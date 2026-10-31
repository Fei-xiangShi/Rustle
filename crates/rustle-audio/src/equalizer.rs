//! 10-band parametric equalizer using biquad filters
//!
//! Standard 10-band EQ frequencies:
//! 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz

use rodio::Source;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Standard 10-band equalizer center frequencies in Hz
pub const EQ_FREQUENCIES: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Biquad filter coefficients
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// Biquad filter state for one channel
#[derive(Clone, Copy, Default)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    fn process(&mut self, coeffs: &BiquadCoeffs, input: f32) -> f32 {
        let output = coeffs.b0 * input + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
            - coeffs.a1 * self.y1
            - coeffs.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;

        output
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Calculate peaking EQ filter coefficients
/// gain_db: gain in decibels (-12 to +12 typical)
/// freq: center frequency in Hz
/// sample_rate: audio sample rate
/// q: quality factor (bandwidth), typically 1.0-2.0 for EQ
fn calc_peaking_eq(freq: f32, gain_db: f32, sample_rate: f32, q: f32) -> BiquadCoeffs {
    // A peaking filter at or above Nyquist is undefined for the current
    // sample rate. Keep the band at unity instead of silently moving it to a
    // different frequency or producing unstable coefficients.
    if gain_db.abs() < 0.01
        || !sample_rate.is_finite()
        || sample_rate <= 0.0
        || !freq.is_finite()
        || freq >= 0.95 * (sample_rate * 0.5)
    {
        return unity_coeffs();
    }

    let a = 10.0_f32.powf(gain_db / 40.0);
    let omega = 2.0 * std::f32::consts::PI * freq / sample_rate;
    let sin_omega = omega.sin();
    let cos_omega = omega.cos();
    let alpha = sin_omega / (2.0 * q);

    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * cos_omega;
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * cos_omega;
    let a2 = 1.0 - alpha / a;

    // Normalize coefficients
    BiquadCoeffs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

fn unity_coeffs() -> BiquadCoeffs {
    BiquadCoeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    }
}

/// Shared equalizer parameters that can be updated in real-time.
///
/// Only user settings are shared. Every `Equalizer` instance owns its sample
/// rate (taken from its own source) and tracks the settings version it last
/// applied, so concurrent sinks (current, preloaded, crossfading) never
/// steal each other's update or compute coefficients for the wrong rate.
#[derive(Clone)]
pub struct EqualizerParams {
    inner: Arc<EqualizerParamsInner>,
}

struct EqualizerParamsInner {
    gain_bits: [AtomicU32; 10],
    enabled: AtomicBool,
    /// Bumped after every settings store; readers compare against the
    /// version they applied. Lock-free so the output callback never waits.
    version: AtomicU64,
}

impl Default for EqualizerParams {
    fn default() -> Self {
        Self::new()
    }
}

impl EqualizerParams {
    /// Create new equalizer parameters
    pub fn new() -> Self {
        Self {
            inner: Arc::new(EqualizerParamsInner {
                gain_bits: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
                enabled: AtomicBool::new(false),
                version: AtomicU64::new(1),
            }),
        }
    }

    /// Enable or disable the equalizer
    pub fn set_enabled(&self, enabled: bool) {
        self.inner.enabled.store(enabled, Ordering::Release);
        self.bump();
    }

    /// Check if equalizer is enabled
    pub fn is_enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Acquire)
    }

    /// Set all 10 band gains at once (in dB, typically -12 to +12)
    pub fn set_gains(&self, gains: [f32; 10]) {
        for (slot, gain) in self.inner.gain_bits.iter().zip(gains) {
            slot.store(gain.to_bits(), Ordering::Release);
        }
        self.bump();
    }

    /// Force every equalizer instance to recompute its coefficients.
    pub fn mark_dirty(&self) {
        self.bump();
    }

    fn bump(&self) {
        self.inner.version.fetch_add(1, Ordering::AcqRel);
    }

    fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }

    fn gains(&self) -> [f32; 10] {
        std::array::from_fn(|index| {
            f32::from_bits(self.inner.gain_bits[index].load(Ordering::Acquire))
        })
    }
}

/// Samples between checks for a source sample-rate change (span boundary).
const SAMPLE_RATE_CHECK_INTERVAL: u32 = 1024;

/// 10-band equalizer Source wrapper
pub struct Equalizer<S>
where
    S: Source<Item = f32>,
{
    source: S,
    params: EqualizerParams,
    // Filter coefficients for each band
    coeffs: [BiquadCoeffs; 10],
    // Filter state for each band, per actual source channel.
    states: Vec<[BiquadState; 10]>,
    // Current channel being processed (for interleaved stereo)
    current_channel: usize,
    channels: u16,
    enabled: bool,
    /// Settings version reflected in `coeffs`.
    applied_version: u64,
    /// Sample rate `coeffs` were computed for (this source's own rate).
    sample_rate: u32,
    samples_until_rate_check: u32,
}

impl<S> Equalizer<S>
where
    S: Source<Item = f32>,
{
    /// Create a new equalizer wrapping the given source
    pub fn new(source: S, params: EqualizerParams) -> Self {
        let channels = source.channels().get();
        let sample_rate = source.sample_rate().get();

        let mut eq = Self {
            source,
            params,
            coeffs: [BiquadCoeffs::default(); 10],
            states: vec![[BiquadState::default(); 10]; usize::from(channels.max(1))],
            current_channel: 0,
            channels,
            enabled: false,
            applied_version: 0,
            sample_rate,
            samples_until_rate_check: SAMPLE_RATE_CHECK_INTERVAL,
        };

        eq.update_coefficients();
        eq
    }

    /// Recompute filter coefficients from the shared settings and this
    /// source's own sample rate.
    fn update_coefficients(&mut self) {
        // Read the version first: a concurrent store bumps it afterwards, so
        // a torn read is always followed by another recompute.
        self.applied_version = self.params.version();
        let enabled = self.params.is_enabled();
        self.enabled = enabled;

        if !enabled {
            // Set all filters to unity gain (bypass)
            self.coeffs.fill(unity_coeffs());
            return;
        }

        let gains = self.params.gains();
        // Q factor for each band - wider at low frequencies, narrower at high
        let q_values: [f32; 10] = [0.7, 0.8, 1.0, 1.2, 1.4, 1.4, 1.4, 1.2, 1.0, 0.8];

        for (i, &freq) in EQ_FREQUENCIES.iter().enumerate() {
            self.coeffs[i] = calc_peaking_eq(freq, gains[i], self.sample_rate as f32, q_values[i]);
        }
    }
}

impl<S> Iterator for Equalizer<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.samples_until_rate_check == 0 {
            self.samples_until_rate_check = SAMPLE_RATE_CHECK_INTERVAL;
            let sample_rate = self.source.sample_rate().get();
            if sample_rate != self.sample_rate {
                self.sample_rate = sample_rate;
                self.applied_version = 0;
            }
        } else {
            self.samples_until_rate_check -= 1;
        }
        if self.params.version() != self.applied_version {
            self.update_coefficients();
        }

        let sample = self.source.next()?;

        // Check if EQ is enabled
        if !self.enabled {
            // Bypass - return original sample
            self.current_channel = (self.current_channel + 1) % usize::from(self.channels.max(1));
            return Some(sample);
        }

        // Process through all 10 bands in series
        let channel = self
            .current_channel
            .min(self.states.len().saturating_sub(1));
        let mut output = sample;

        for (i, coeff) in self.coeffs.iter().enumerate() {
            output = self.states[channel][i].process(coeff, output);
        }

        // Peak protection is applied once, after the whole per-track chain
        // (`SoftClipSource`); clipping here as well would double-process.
        self.current_channel = (self.current_channel + 1) % usize::from(self.channels.max(1));
        Some(output)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for Equalizer<S>
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
        // Reset filter states when seeking to avoid audio artifacts
        for channel_states in &mut self.states {
            for state in channel_states {
                state.reset();
            }
        }
        self.source.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct TestSource {
        samples: std::vec::IntoIter<f32>,
        channels: u16,
        sample_rate: u32,
    }

    impl Iterator for TestSource {
        type Item = f32;

        fn next(&mut self) -> Option<Self::Item> {
            self.samples.next()
        }
    }

    impl Source for TestSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.samples.len())
        }

        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(self.channels).expect("test channel count must be non-zero")
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(self.sample_rate).expect("test sample rate must be non-zero")
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }

        fn try_seek(&mut self, _pos: Duration) -> Result<(), rodio::source::SeekError> {
            Err(rodio::source::SeekError::NotSupported {
                underlying_source: "test source is not seekable",
            })
        }
    }

    #[test]
    fn allocates_filter_history_for_every_source_channel() {
        let params = EqualizerParams::new();
        let mono = Equalizer::new(
            TestSource {
                samples: vec![0.0].into_iter(),
                channels: 1,
                sample_rate: 48_000,
            },
            params.clone(),
        );
        assert_eq!(mono.states.len(), 1);

        let surround = Equalizer::new(
            TestSource {
                samples: vec![0.0; 6].into_iter(),
                channels: 6,
                sample_rate: 48_000,
            },
            params,
        );
        assert_eq!(surround.states.len(), 6);
    }

    #[test]
    fn concurrent_instances_each_apply_updates_at_their_own_rate() {
        let params = EqualizerParams::new();
        params.set_enabled(true);
        let source = |sample_rate| TestSource {
            samples: vec![0.0; 4096].into_iter(),
            channels: 2,
            sample_rate,
        };
        let mut playing = Equalizer::new(source(44_100), params.clone());
        // Creating a preload at another rate must not retune the playing EQ.
        let mut preload = Equalizer::new(source(48_000), params.clone());

        params.set_gains([0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        // Whichever instance runs first, both must observe the update.
        let _ = preload.next();
        let _ = playing.next();

        assert_eq!(playing.sample_rate, 44_100);
        assert_eq!(preload.sample_rate, 48_000);
        assert_eq!(
            playing.coeffs[2],
            calc_peaking_eq(125.0, 6.0, 44_100.0, 1.0)
        );
        assert_eq!(
            preload.coeffs[2],
            calc_peaking_eq(125.0, 6.0, 48_000.0, 1.0)
        );
    }

    #[test]
    fn bands_at_nyquist_boundary_are_unity_and_finite() {
        let coeffs = calc_peaking_eq(16_000.0, 12.0, 32_000.0, 0.8);
        assert_eq!(coeffs, unity_coeffs());
        assert!(
            [coeffs.b0, coeffs.b1, coeffs.b2, coeffs.a1, coeffs.a2]
                .into_iter()
                .all(f32::is_finite)
        );

        let active = calc_peaking_eq(8_000.0, 12.0, 48_000.0, 0.8);
        assert_ne!(active, unity_coeffs());
    }

    #[test]
    fn multichannel_filter_history_does_not_bleed_between_channels() {
        let params = EqualizerParams::new();
        params.set_enabled(true);
        params.set_gains([0.0, 0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let source = TestSource {
            // Two frames of 5.1 audio. Only channel 1 receives an impulse.
            samples: vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0].into_iter(),
            channels: 6,
            sample_rate: 48_000,
        };
        let mut eq = Equalizer::new(source, params);
        let output: Vec<_> = (&mut eq).collect();

        // Channel 2's second-frame sample must remain exactly silent; sharing
        // a stereo right-channel state would leak channel 1's impulse here.
        assert_eq!(output[8], 0.0);
    }
}
