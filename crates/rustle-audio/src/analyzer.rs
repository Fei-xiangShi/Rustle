//! Real-time audio spectrum analyzer
//!
//! Provides FFT-based frequency spectrum analysis with:
//! - 4096-point FFT for high frequency resolution (~11.7Hz per bin at 48kHz)
//! - Logarithmic frequency scale (20Hz - 20kHz)
//! - dB scale with decay smoothing
//! - RMS level metering

use rodio::Source;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

/// FFT size - 4096 gives ~11.7Hz resolution at 48kHz
pub const FFT_SIZE: usize = 4096;

/// Number of spectrum bars for visualization
pub const SPECTRUM_BARS: usize = 128;

/// Minimum frequency (Hz)
const MIN_FREQ: f32 = 20.0;

/// Maximum frequency (Hz)
const MAX_FREQ: f32 = 20000.0;

/// Audio analysis data shared between audio thread and UI
#[derive(Clone)]
pub struct AudioAnalysisData {
    snapshot: Arc<RwLock<AudioAnalysisSnapshot>>,
    enabled: Arc<AtomicBool>,
    sample_rate: Arc<AtomicU32>,
    decay_bits: Arc<AtomicU32>,
    reset_generation: Arc<AtomicU32>,
}

struct AudioAnalysisSnapshot {
    /// Left channel RMS level (0.0 to 1.0)
    left_rms: f32,
    /// Right channel RMS level (0.0 to 1.0)
    right_rms: f32,
    /// Spectrum magnitude in dB for each bar (smoothed with decay)
    spectrum_db: [f32; SPECTRUM_BARS],
}

impl Default for AudioAnalysisSnapshot {
    fn default() -> Self {
        Self {
            left_rms: 0.0,
            right_rms: 0.0,
            spectrum_db: [-60.0; SPECTRUM_BARS],
        }
    }
}

struct AudioAnalysisProcessor {
    /// Smoothed left/right RMS values.
    left_rms: f32,
    right_rms: f32,
    /// Spectrum magnitude in dB for each bar (smoothed with decay)
    spectrum_db: [f32; SPECTRUM_BARS],
    /// Sample buffer for FFT (mono mixed)
    sample_buffer: Vec<f32>,
    /// Left channel samples for RMS
    left_samples: Vec<f32>,
    /// Right channel samples for RMS
    right_samples: Vec<f32>,
    /// Current channel index (for interleaved stereo)
    current_channel: usize,
    /// Number of channels
    channels: u16,
    /// Sample rate
    sample_rate: u32,
}

impl Default for AudioAnalysisProcessor {
    fn default() -> Self {
        Self {
            left_rms: 0.0,
            right_rms: 0.0,
            spectrum_db: [-60.0; SPECTRUM_BARS],
            sample_buffer: Vec::with_capacity(FFT_SIZE),
            left_samples: Vec::with_capacity(FFT_SIZE / 2),
            right_samples: Vec::with_capacity(FFT_SIZE / 2),
            current_channel: 0,
            channels: 2,
            sample_rate: 48000,
        }
    }
}

impl AudioAnalysisData {
    /// Create new audio analysis data
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(AudioAnalysisSnapshot::default())),
            enabled: Arc::new(AtomicBool::new(false)),
            sample_rate: Arc::new(AtomicU32::new(48000)),
            decay_bits: Arc::new(AtomicU32::new(0.85_f32.to_bits())),
            reset_generation: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Get left channel RMS level (0.0 to 1.0)
    pub fn left_rms(&self) -> f32 {
        self.snapshot.read().map(|i| i.left_rms).unwrap_or(0.0)
    }

    /// Get right channel RMS level (0.0 to 1.0)
    pub fn right_rms(&self) -> f32 {
        self.snapshot.read().map(|i| i.right_rms).unwrap_or(0.0)
    }

    /// Get spectrum data in dB (SPECTRUM_BARS values, -60 to +12 dB range)
    pub fn spectrum_db(&self) -> [f32; SPECTRUM_BARS] {
        self.snapshot
            .read()
            .map(|i| i.spectrum_db)
            .unwrap_or([-60.0; SPECTRUM_BARS])
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

    /// Enable or disable analysis in the audio callback.
    pub fn set_enabled(&self, enabled: bool) {
        let was_enabled = self.enabled.swap(enabled, Ordering::AcqRel);
        if was_enabled != enabled {
            self.reset();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Reset analysis data (call when playback stops)
    pub fn reset(&self) {
        self.reset_generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.left_rms = 0.0;
            snapshot.right_rms = 0.0;
            snapshot.spectrum_db.fill(-60.0);
        }
    }

    fn decay(&self) -> f32 {
        f32::from_bits(self.decay_bits.load(Ordering::Acquire))
    }

    fn reset_generation(&self) -> u32 {
        self.reset_generation.load(Ordering::Acquire)
    }

    fn publish(
        &self,
        sample_rate: u32,
        left_rms: f32,
        right_rms: f32,
        spectrum_db: &[f32; SPECTRUM_BARS],
    ) {
        self.sample_rate.store(sample_rate, Ordering::Release);
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.left_rms = left_rms;
            snapshot.right_rms = right_rms;
            snapshot.spectrum_db = *spectrum_db;
        }
    }
}

impl Default for AudioAnalysisData {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioAnalysisProcessor {
    fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            ..Self::default()
        }
    }

    fn reset(&mut self) {
        self.left_rms = 0.0;
        self.right_rms = 0.0;
        self.spectrum_db.fill(-60.0);
        self.sample_buffer.clear();
        self.left_samples.clear();
        self.right_samples.clear();
        self.current_channel = 0;
    }

    fn process_sample(&mut self, sample: f32, analysis: &AudioAnalysisData) {
        let channel = self.current_channel;
        let channels = self.channels as usize;

        if channel == 0 {
            self.left_samples.push(sample);
            self.sample_buffer.push(sample);
        } else if channel == 1 {
            self.right_samples.push(sample);
            if let Some(last) = self.sample_buffer.last_mut() {
                *last = (*last + sample) * 0.5;
            }
        }

        self.current_channel = (channel + 1) % channels;

        if self.sample_buffer.len() >= FFT_SIZE {
            let (left_rms, right_rms) = self.perform_fft(analysis.decay());
            analysis.publish(self.sample_rate, left_rms, right_rms, &self.spectrum_db);
        }
    }

    /// Perform FFT analysis and update spectrum
    fn perform_fft(&mut self, decay: f32) -> (f32, f32) {
        let mut left_rms = 0.0;
        let mut right_rms = 0.0;

        // Calculate RMS first
        if !self.left_samples.is_empty() {
            let sum_sq: f32 = self.left_samples.iter().map(|s| s * s).sum();
            let rms = (sum_sq / self.left_samples.len() as f32).sqrt();
            left_rms = self.left_rms * 0.7 + rms.min(1.0) * 0.3;
        }
        if !self.right_samples.is_empty() {
            let sum_sq: f32 = self.right_samples.iter().map(|s| s * s).sum();
            let rms = (sum_sq / self.right_samples.len() as f32).sqrt();
            right_rms = self.right_rms * 0.7 + rms.min(1.0) * 0.3;
        }

        // Apply Hann window to samples
        let windowed = hann_window(&self.sample_buffer[..FFT_SIZE]);

        // Perform FFT
        if let Ok(spectrum) = samples_fft_to_spectrum(
            &windowed,
            self.sample_rate,
            FrequencyLimit::Range(MIN_FREQ, MAX_FREQ),
            Some(&divide_by_N_sqrt),
        ) {
            // Map FFT bins to logarithmic frequency bars
            let freq_data = spectrum.data();
            let mut freq_idx = 0;

            for bar_idx in 0..SPECTRUM_BARS {
                // Calculate frequency range for this bar (logarithmic scale)
                let t0 = bar_idx as f32 / SPECTRUM_BARS as f32;
                let t1 = (bar_idx + 1) as f32 / SPECTRUM_BARS as f32;
                let freq_low = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t0);
                let freq_high = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t1);

                // Find max magnitude in this frequency range
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
                    if f >= freq_low && f < freq_high {
                        max_mag = max_mag.max(mag.val());
                    }
                    scan_idx += 1;
                }
                freq_idx = scan_idx;

                // Convert to dB (with floor at -60dB)
                let db = if max_mag > 0.0 {
                    (20.0 * max_mag.log10()).clamp(-60.0, 12.0)
                } else {
                    -60.0
                };

                // Apply decay smoothing
                let current = self.spectrum_db[bar_idx];
                self.spectrum_db[bar_idx] = if db > current {
                    // Attack: fast rise
                    current * 0.3 + db * 0.7
                } else {
                    // Decay: smooth fall
                    current * decay + db * (1.0 - decay)
                };
            }
        }

        // Keep overlap for smoother updates (50% overlap)
        let overlap = FFT_SIZE / 2;
        self.sample_buffer.drain(0..overlap);
        self.left_samples.clear();
        self.right_samples.clear();
        self.left_rms = left_rms;
        self.right_rms = right_rms;

        (left_rms, right_rms)
    }
}

/// Audio analyzer source wrapper
pub struct AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    analysis: AudioAnalysisData,
    processor: AudioAnalysisProcessor,
    last_reset_generation: u32,
}

impl<S> AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    /// Create a new analyzing source
    pub fn new(source: S, analysis: AudioAnalysisData) -> Self {
        let processor =
            AudioAnalysisProcessor::new(source.sample_rate().get(), source.channels().get());
        let last_reset_generation = analysis.reset_generation();

        Self {
            source,
            analysis,
            processor,
            last_reset_generation,
        }
    }

    fn sync_control_state(&mut self) {
        let reset_generation = self.analysis.reset_generation();
        if reset_generation != self.last_reset_generation {
            self.last_reset_generation = reset_generation;
            self.processor.reset();
        }
    }
}

impl<S> Iterator for AnalyzingSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next()?;
        self.sync_control_state();
        if self.analysis.is_enabled() {
            self.processor.process_sample(sample, &self.analysis);
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
        // Reset analysis buffers when seeking
        self.analysis.reset();
        self.processor.reset();
        self.source.try_seek(pos)
    }
}
