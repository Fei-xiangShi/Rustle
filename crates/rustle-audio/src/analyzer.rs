//! Real-time audio spectrum analyzer
//!
//! Provides FFT-based frequency spectrum analysis with:
//! - 4096-point FFT for high frequency resolution (~11.7Hz per bin at 48kHz)
//! - Logarithmic frequency scale (20Hz - 20kHz)
//! - dB scale with decay smoothing
//! - RMS level metering
//!
//! ## Threading
//!
//! The output callback only copies left/right samples into a fixed,
//! preallocated ring of atomics: no locks, no allocation, no FFT. The FFT,
//! windowing and smoothing run lazily on the reader (UI) side when a value is
//! requested, at most once per new block of audio.

use parking_lot::Mutex;
use rodio::Source;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// FFT size - 4096 gives ~11.7Hz resolution at 48kHz
pub const FFT_SIZE: usize = 4096;

/// Number of spectrum bars for visualization
pub const SPECTRUM_BARS: usize = 128;

/// Minimum frequency (Hz)
const MIN_FREQ: f32 = 20.0;

/// Maximum frequency (Hz)
const MAX_FREQ: f32 = 20000.0;

/// Frames retained by the lock-free capture ring. Twice the FFT window so the
/// writer can never lap the window the reader is copying.
const RING_FRAMES: usize = FFT_SIZE * 2;

/// Audio frames that correspond to one smoothing step. The original analyzer
/// smoothed once per 50%-overlap FFT hop; smoothing is normalized to this
/// hop so the visual decay speed does not depend on the UI refresh rate.
const SMOOTHING_HOP_FRAMES: f32 = (FFT_SIZE / 2) as f32;

/// Minimum post-reset frames before a spectrum is computed.
const MIN_ANALYSIS_FRAMES: u64 = (FFT_SIZE / 4) as u64;

/// Audio analysis data shared between audio thread and UI
#[derive(Clone)]
pub struct AudioAnalysisData {
    capture: Arc<CaptureRing>,
    reader: Arc<Mutex<AnalysisReader>>,
    enabled: Arc<AtomicBool>,
    sample_rate: Arc<AtomicU32>,
    decay_bits: Arc<AtomicU32>,
    reset_generation: Arc<AtomicU32>,
    /// Capture position at the latest reset; earlier frames are stale.
    reset_floor: Arc<AtomicU64>,
}

/// Single-producer capture of the most recent left/right frames.
struct CaptureRing {
    left: Box<[AtomicU32]>,
    right: Box<[AtomicU32]>,
    /// Total frames written; the next frame lands at `frames % RING_FRAMES`.
    frames: AtomicU64,
}

impl CaptureRing {
    fn new() -> Self {
        let slots = || {
            (0..RING_FRAMES)
                .map(|_| AtomicU32::new(0.0_f32.to_bits()))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        Self {
            left: slots(),
            right: slots(),
            frames: AtomicU64::new(0),
        }
    }

    /// Real-time safe: two relaxed stores and one release store per frame.
    fn push_frame(&self, left: f32, right: f32) {
        let frame = self.frames.load(Ordering::Relaxed);
        let slot = (frame % RING_FRAMES as u64) as usize;
        self.left[slot].store(left.to_bits(), Ordering::Relaxed);
        self.right[slot].store(right.to_bits(), Ordering::Relaxed);
        self.frames.store(frame + 1, Ordering::Release);
    }

    fn frames(&self) -> u64 {
        self.frames.load(Ordering::Acquire)
    }

    /// Copy the `count` frames that end at `end` (exclusive).
    fn copy_window(&self, end: u64, count: usize, left: &mut Vec<f32>, right: &mut Vec<f32>) {
        left.clear();
        right.clear();
        let start = end.saturating_sub(count as u64);
        for frame in start..end {
            let slot = (frame % RING_FRAMES as u64) as usize;
            left.push(f32::from_bits(self.left[slot].load(Ordering::Relaxed)));
            right.push(f32::from_bits(self.right[slot].load(Ordering::Relaxed)));
        }
    }
}

/// Reader-side analysis state. Only ever locked by consumers, never by the
/// output callback.
struct AnalysisReader {
    left_rms: f32,
    right_rms: f32,
    spectrum_db: [f32; SPECTRUM_BARS],
    /// Capture position consumed by the last computation.
    analyzed_frames: u64,
    /// Frames written before the last observed reset are ignored.
    floor_frames: u64,
    reset_generation: u32,
    left_scratch: Vec<f32>,
    right_scratch: Vec<f32>,
}

impl AnalysisReader {
    fn new() -> Self {
        Self {
            left_rms: 0.0,
            right_rms: 0.0,
            spectrum_db: [-60.0; SPECTRUM_BARS],
            analyzed_frames: 0,
            floor_frames: 0,
            reset_generation: 0,
            left_scratch: Vec::with_capacity(FFT_SIZE),
            right_scratch: Vec::with_capacity(FFT_SIZE),
        }
    }

    fn clear(&mut self) {
        self.left_rms = 0.0;
        self.right_rms = 0.0;
        self.spectrum_db.fill(-60.0);
    }
}

impl AudioAnalysisData {
    /// Create new audio analysis data
    pub fn new() -> Self {
        Self {
            capture: Arc::new(CaptureRing::new()),
            reader: Arc::new(Mutex::new(AnalysisReader::new())),
            enabled: Arc::new(AtomicBool::new(false)),
            sample_rate: Arc::new(AtomicU32::new(48000)),
            decay_bits: Arc::new(AtomicU32::new(0.85_f32.to_bits())),
            reset_generation: Arc::new(AtomicU32::new(0)),
            reset_floor: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get left channel RMS level (0.0 to 1.0)
    pub fn left_rms(&self) -> f32 {
        self.refreshed(|reader| reader.left_rms)
    }

    /// Get right channel RMS level (0.0 to 1.0)
    pub fn right_rms(&self) -> f32 {
        self.refreshed(|reader| reader.right_rms)
    }

    /// Get spectrum data in dB (SPECTRUM_BARS values, -60 to +12 dB range)
    pub fn spectrum_db(&self) -> [f32; SPECTRUM_BARS] {
        self.refreshed(|reader| reader.spectrum_db)
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Acquire)
    }

    /// Set decay factor (0.0 = instant, 0.99 = very slow)
    pub fn set_decay(&self, decay: f32) {
        self.decay_bits
            .store(decay.clamp(0.0, 0.99).to_bits(), Ordering::Release);
    }

    /// Enable or disable sample capture in the audio callback.
    pub fn set_enabled(&self, enabled: bool) {
        let was_enabled = self.enabled.swap(enabled, Ordering::AcqRel);
        if was_enabled != enabled {
            self.reset();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Reset analysis data (call when playback stops or seeks).
    ///
    /// Lock-free: safe to call from the output callback. The reader drops
    /// its smoothing state and every frame captured before it observes the
    /// new generation.
    pub fn reset(&self) {
        self.reset_floor
            .store(self.capture.frames(), Ordering::Release);
        self.reset_generation.fetch_add(1, Ordering::AcqRel);
    }

    fn decay(&self) -> f32 {
        f32::from_bits(self.decay_bits.load(Ordering::Acquire))
    }

    /// Bring the reader up to date with captured audio, then project a value.
    fn refreshed<T>(&self, project: impl FnOnce(&AnalysisReader) -> T) -> T {
        let mut reader = self.reader.lock();
        let generation = self.reset_generation.load(Ordering::Acquire);
        let end = self.capture.frames();
        if reader.reset_generation != generation {
            let floor = self.reset_floor.load(Ordering::Acquire).min(end);
            reader.reset_generation = generation;
            reader.floor_frames = floor;
            reader.analyzed_frames = floor;
            reader.clear();
        }
        if !self.is_enabled() {
            reader.clear();
            reader.analyzed_frames = end;
            return project(&reader);
        }
        let available = end.saturating_sub(reader.floor_frames);
        if end > reader.analyzed_frames && available >= MIN_ANALYSIS_FRAMES {
            let advanced = end - reader.analyzed_frames;
            let window = (available.min(FFT_SIZE as u64)) as usize;
            self.analyze(&mut reader, end, window, advanced);
        }
        project(&reader)
    }

    fn analyze(&self, reader: &mut AnalysisReader, end: u64, window: usize, advanced: u64) {
        let mut left = std::mem::take(&mut reader.left_scratch);
        let mut right = std::mem::take(&mut reader.right_scratch);
        self.capture.copy_window(end, window, &mut left, &mut right);
        reader.analyzed_frames = end;

        // Normalize per-hop smoothing to the amount of new audio.
        let steps = (advanced as f32 / SMOOTHING_HOP_FRAMES).max(0.0);
        let rms_keep = 0.7_f32.powf(steps);
        let rms_frames = left.len().clamp(1, FFT_SIZE / 2);
        let rms = |samples: &[f32]| {
            let tail = &samples[samples.len().saturating_sub(rms_frames)..];
            let sum_sq: f32 = tail.iter().map(|sample| sample * sample).sum();
            (sum_sq / tail.len().max(1) as f32).sqrt().min(1.0)
        };
        reader.left_rms = reader.left_rms * rms_keep + rms(&left) * (1.0 - rms_keep);
        reader.right_rms = reader.right_rms * rms_keep + rms(&right) * (1.0 - rms_keep);

        let mut mono: Vec<f32> = left
            .iter()
            .zip(&right)
            .map(|(left, right)| (left + right) * 0.5)
            .collect();
        // The FFT needs a power-of-two input; zero-pad a short post-reset
        // window at the front so the newest audio stays aligned.
        if mono.len() < FFT_SIZE {
            let mut padded = vec![0.0; FFT_SIZE - mono.len()];
            padded.extend_from_slice(&mono);
            mono = padded;
        }
        let windowed = hann_window(&mono);
        let sample_rate = self.sample_rate();
        let decay = self.decay().powf(steps);
        let attack_keep = 0.3_f32.powf(steps.min(1.0));
        if let Ok(spectrum) = samples_fft_to_spectrum(
            &windowed,
            sample_rate,
            FrequencyLimit::Range(MIN_FREQ, MAX_FREQ.min(sample_rate as f32 / 2.0 - 1.0)),
            Some(&divide_by_N_sqrt),
        ) {
            let freq_data = spectrum.data();
            let mut freq_idx = 0;
            for bar_idx in 0..SPECTRUM_BARS {
                let t0 = bar_idx as f32 / SPECTRUM_BARS as f32;
                let t1 = (bar_idx + 1) as f32 / SPECTRUM_BARS as f32;
                let freq_low = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t0);
                let freq_high = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t1);

                let mut max_mag: f32 = 0.0;
                while freq_idx < freq_data.len() && freq_data[freq_idx].0.val() < freq_low {
                    freq_idx += 1;
                }
                let mut scan_idx = freq_idx;
                while scan_idx < freq_data.len() {
                    let (freq, mag) = &freq_data[scan_idx];
                    let f = freq.val();
                    if f >= freq_high {
                        break;
                    }
                    if f >= freq_low {
                        max_mag = max_mag.max(mag.val());
                    }
                    scan_idx += 1;
                }
                freq_idx = scan_idx;

                let db = if max_mag > 0.0 {
                    (20.0 * max_mag.log10()).clamp(-60.0, 12.0)
                } else {
                    -60.0
                };
                let current = reader.spectrum_db[bar_idx];
                reader.spectrum_db[bar_idx] = if db > current {
                    // Attack: fast rise
                    current * attack_keep + db * (1.0 - attack_keep)
                } else {
                    // Decay: smooth fall
                    current * decay + db * (1.0 - decay)
                };
            }
        }

        reader.left_scratch = left;
        reader.right_scratch = right;
    }
}

impl Default for AudioAnalysisData {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio analyzer source wrapper
pub struct AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    analysis: AudioAnalysisData,
    /// Channel index of the next interleaved sample.
    current_channel: u16,
    /// Left sample of the frame being assembled.
    pending_left: f32,
}

impl<S> AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    /// Create a new analyzing source
    pub fn new(source: S, analysis: AudioAnalysisData) -> Self {
        analysis
            .sample_rate
            .store(source.sample_rate().get(), Ordering::Release);
        Self {
            source,
            analysis,
            current_channel: 0,
            pending_left: 0.0,
        }
    }

    fn capture(&mut self, sample: f32) {
        let channels = self.source.channels().get();
        match self.current_channel {
            0 if channels == 1 => self.analysis.capture.push_frame(sample, sample),
            0 => self.pending_left = sample,
            1 => self.analysis.capture.push_frame(self.pending_left, sample),
            _ => {}
        }
        self.current_channel = (self.current_channel + 1) % channels.max(1);
    }
}

impl<S> Iterator for AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next()?;
        if self.analysis.is_enabled() {
            self.capture(sample);
        } else {
            let channels = self.source.channels().get().max(1);
            self.current_channel = (self.current_channel + 1) % channels;
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.source.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.source.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.source.total_duration()
    }

    fn try_seek(&mut self, pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        // Seeking starts a new frame boundary and invalidates captured audio.
        self.analysis.reset();
        self.current_channel = 0;
        self.source.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Sine {
        frame: u64,
        channel: u16,
        channels: u16,
        sample_rate: u32,
        frequency: f32,
        frames: u64,
    }

    impl Iterator for Sine {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            if self.frame >= self.frames {
                return None;
            }
            let t = self.frame as f32 / self.sample_rate as f32;
            let value = (std::f32::consts::TAU * self.frequency * t).sin() * 0.5;
            self.channel += 1;
            if self.channel == self.channels {
                self.channel = 0;
                self.frame += 1;
            }
            Some(value)
        }
    }

    impl Source for Sine {
        fn current_span_len(&self) -> Option<usize> {
            None
        }

        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(self.channels).expect("non-zero channels")
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(self.sample_rate).expect("non-zero rate")
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn sine(frames: u64) -> Sine {
        Sine {
            frame: 0,
            channel: 0,
            channels: 2,
            sample_rate: 48_000,
            frequency: 1_000.0,
            frames,
        }
    }

    #[test]
    fn capture_feeds_reader_side_spectrum_and_rms() {
        let analysis = AudioAnalysisData::new();
        analysis.set_enabled(true);
        let source = AnalyzingSource::new(sine(FFT_SIZE as u64 * 2), analysis.clone());
        let samples: Vec<f32> = source.collect();
        assert_eq!(samples.len(), FFT_SIZE * 4);

        assert!(analysis.left_rms() > 0.05);
        assert!(analysis.right_rms() > 0.05);
        let spectrum = analysis.spectrum_db();
        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .expect("bars");
        // 1 kHz sits in the middle of the 20 Hz..20 kHz log scale.
        let t = (1_000.0_f32 / MIN_FREQ).ln() / (MAX_FREQ / MIN_FREQ).ln();
        let expected = (t * SPECTRUM_BARS as f32) as usize;
        assert!(
            peak.abs_diff(expected) <= 2,
            "peak bar {peak}, expected ~{expected}"
        );
    }

    #[test]
    fn reset_and_disable_return_silence_without_touching_the_callback_path() {
        let analysis = AudioAnalysisData::new();
        analysis.set_enabled(true);
        let _: Vec<f32> = AnalyzingSource::new(sine(FFT_SIZE as u64), analysis.clone()).collect();
        assert!(analysis.left_rms() > 0.0);

        analysis.reset();
        assert_eq!(analysis.left_rms(), 0.0);
        assert!(analysis.spectrum_db().iter().all(|db| *db == -60.0));

        let _: Vec<f32> = AnalyzingSource::new(sine(FFT_SIZE as u64), analysis.clone()).collect();
        analysis.set_enabled(false);
        assert_eq!(analysis.right_rms(), 0.0);
    }
}
