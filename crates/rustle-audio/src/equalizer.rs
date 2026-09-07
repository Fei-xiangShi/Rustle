//! 10-band parametric equalizer using biquad filters
//!
//! Standard 10-band EQ frequencies:
//! 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz

use rodio::Source;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

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

/// Shared equalizer parameters that can be updated in real-time
#[derive(Clone)]
pub struct EqualizerParams {
    inner: Arc<RwLock<EqualizerParamsInner>>,
    enabled: Arc<AtomicBool>,
    coeffs_dirty: Arc<AtomicBool>,
}

struct EqualizerParamsInner {
    gains: [f32; 10],
    sample_rate: u32,
}

impl EqualizerParams {
    /// Create new equalizer parameters
    pub fn new(sample_rate: u32) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EqualizerParamsInner {
                gains: [0.0; 10],
                sample_rate,
            })),
            enabled: Arc::new(AtomicBool::new(false)),
            coeffs_dirty: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Enable or disable the equalizer
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        self.coeffs_dirty.store(true, Ordering::Release);
    }

    /// Check if equalizer is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Set all 10 band gains at once (in dB, typically -12 to +12)
    pub fn set_gains(&self, gains: [f32; 10]) {
        if let Ok(mut inner) = self.inner.write() {
            inner.gains = gains;
        }
        self.coeffs_dirty.store(true, Ordering::Release);
    }

    /// Update sample rate (call when audio format changes)
    pub fn set_sample_rate(&self, sample_rate: u32) {
        if let Ok(mut inner) = self.inner.write()
            && inner.sample_rate != sample_rate
        {
            inner.sample_rate = sample_rate;
            self.coeffs_dirty.store(true, Ordering::Release);
        }
    }

    /// 标记系数为脏，强制下次采样时重新计算
    /// 切换曲目时使用，确保 EQ 正确初始化
    pub fn mark_dirty(&self) {
        self.coeffs_dirty.store(true, Ordering::Release);
    }
}

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
}

impl<S> Equalizer<S>
where
    S: Source<Item = f32>,
{
    /// Create a new equalizer wrapping the given source
    pub fn new(source: S, params: EqualizerParams) -> Self {
        let channels = source.channels().get();
        let sample_rate = source.sample_rate().get();

        // Update params with actual sample rate
        params.set_sample_rate(sample_rate);

        // 强制重新计算系数，新实例的系数是默认值
        params.mark_dirty();

        let mut eq = Self {
            source,
            params,
            coeffs: [BiquadCoeffs::default(); 10],
            states: vec![[BiquadState::default(); 10]; usize::from(channels.max(1))],
            current_channel: 0,
            channels,
            enabled: false,
        };

        eq.update_coefficients();
        eq
    }

    /// Update filter coefficients from current parameters
    fn update_coefficients(&mut self) {
        let coeffs_dirty = self.params.coeffs_dirty.swap(false, Ordering::AcqRel);
        if !coeffs_dirty {
            return;
        }

        let (enabled, gains, sample_rate) = {
            let inner = self.params.inner.read().unwrap();
            (self.params.is_enabled(), inner.gains, inner.sample_rate)
        };
        self.enabled = enabled;

        if !enabled {
            // Set all filters to unity gain (bypass)
            self.coeffs.fill(unity_coeffs());
            return;
        }

        // Q factor for each band - wider at low frequencies, narrower at high
        let q_values: [f32; 10] = [0.7, 0.8, 1.0, 1.2, 1.4, 1.4, 1.4, 1.2, 1.0, 0.8];

        for (i, &freq) in EQ_FREQUENCIES.iter().enumerate() {
            self.coeffs[i] = calc_peaking_eq(freq, gains[i], sample_rate as f32, q_values[i]);
        }
    }
}

impl<S> Iterator for Equalizer<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if coefficients need updating
        if self.params.coeffs_dirty.load(Ordering::Acquire) {
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

        // Soft clip to prevent harsh distortion
        output = soft_clip(output);

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

/// Soft clipping function to prevent harsh digital clipping
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.9 {
        x
    } else if x > 0.0 {
        0.9 + 0.1 * ((x - 0.9) / 0.1).tanh()
    } else {
        -0.9 - 0.1 * ((-x - 0.9) / 0.1).tanh()
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
        let params = EqualizerParams::new(48_000);
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
        let params = EqualizerParams::new(48_000);
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
