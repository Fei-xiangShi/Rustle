//! Fade envelope - self-driving volume automation
//!
//! A pure audio processor that smoothly transitions volume without
//! knowing anything about playback state. It just does one thing:
//! smoothly interpolate volume from current to target.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rodio::Source;

/// Shared fade control handle
///
/// Clone this to control fade from outside the audio thread.
/// All operations are lock-free using atomics.
#[derive(Clone)]
pub struct FadeControl {
    inner: Arc<FadeControlInner>,
}

struct FadeControlInner {
    /// Target volume (0.0 - 1.0), stored as u32 bits
    target_volume: AtomicU32,
    /// Fade duration in milliseconds
    fade_duration_ms: AtomicU32,
    /// Optional explicit ramp start. NaN means continue from the envelope's
    /// current sample gain.
    requested_start_volume: AtomicU32,
    /// Generation counter - incremented on each fade_to() call
    /// Used to detect new fade requests
    generation: AtomicU32,
    /// Generation whose ramp has reached its target.
    completed_generation: AtomicU32,
    /// 0 = cubic UI fade, 1 = equal-power transition fade.
    curve: AtomicU32,
}

impl FadeControl {
    /// Create a new fade control with initial volume
    pub fn new(initial_volume: f32) -> Self {
        Self {
            inner: Arc::new(FadeControlInner {
                target_volume: AtomicU32::new(initial_volume.to_bits()),
                fade_duration_ms: AtomicU32::new(0),
                requested_start_volume: AtomicU32::new(f32::NAN.to_bits()),
                generation: AtomicU32::new(0),
                completed_generation: AtomicU32::new(0),
                curve: AtomicU32::new(0),
            }),
        }
    }

    /// Start a fade to target volume over duration
    ///
    /// This is the only API you need. Examples:
    /// - Fade in: `fade_to(1.0, Duration::from_millis(300))`
    /// - Fade out: `fade_to(0.0, Duration::from_millis(300))`
    /// - Instant: `fade_to(0.5, Duration::ZERO)`
    pub fn fade_to(&self, volume: f32, duration: Duration) {
        self.fade_to_with_curve(None, volume, duration, 0);
    }

    pub(crate) fn fade_to_equal_power(&self, volume: f32, duration: Duration) {
        self.fade_to_with_curve(None, volume, duration, 1);
    }

    pub(crate) fn fade_from_to_equal_power(&self, from: f32, volume: f32, duration: Duration) {
        self.fade_to_with_curve(Some(from), volume, duration, 1);
    }

    pub(crate) fn fade_from_to(&self, from: f32, volume: f32, duration: Duration) {
        self.fade_to_with_curve(Some(from), volume, duration, 0);
    }

    fn fade_to_with_curve(&self, from: Option<f32>, volume: f32, duration: Duration, curve: u32) {
        let volume = volume.clamp(0.0, 1.0);
        self.inner.requested_start_volume.store(
            from.map_or(f32::NAN, |value| value.clamp(0.0, 1.0))
                .to_bits(),
            Ordering::Release,
        );
        self.inner
            .target_volume
            .store(volume.to_bits(), Ordering::Release);
        self.inner
            .fade_duration_ms
            .store(duration.as_millis() as u32, Ordering::Release);
        self.inner.curve.store(curve, Ordering::Release);
        self.inner.generation.fetch_add(1, Ordering::Release);
    }

    /// Set volume instantly (no fade)
    pub fn set_volume(&self, volume: f32) {
        self.fade_to(volume, Duration::ZERO);
    }

    /// Get current target volume
    pub fn target_volume(&self) -> f32 {
        f32::from_bits(self.inner.target_volume.load(Ordering::Acquire))
    }

    fn generation(&self) -> u32 {
        self.inner.generation.load(Ordering::Acquire)
    }

    fn fade_duration_ms(&self) -> u32 {
        self.inner.fade_duration_ms.load(Ordering::Acquire)
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.inner.completed_generation.load(Ordering::Acquire)
            == self.inner.generation.load(Ordering::Acquire)
    }
}

impl Default for FadeControl {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Fade envelope source wrapper
///
/// Wraps any audio source and applies smooth volume transitions.
/// Runs entirely in the audio thread - no external tick needed.
pub struct FadeEnvelope<S>
where
    S: Source<Item = f32>,
{
    source: S,
    control: FadeControl,
    /// Current volume (what we're outputting now)
    current_volume: f32,
    /// Volume at the start of current fade
    fade_start_volume: f32,
    /// Samples remaining in current fade
    fade_samples_remaining: u32,
    /// Total samples for current fade
    fade_samples_total: u32,
    /// Last seen generation (to detect new fade requests)
    last_generation: u32,
    /// Cached sample rate
    sample_rate: u32,
    /// Cached channel count. `Source::next()` yields interleaved samples, so
    /// wall-clock fade duration must count every channel sample in a frame.
    channels: u16,
    equal_power: bool,
}

impl<S> FadeEnvelope<S>
where
    S: Source<Item = f32>,
{
    /// Create a new fade envelope wrapping a source
    pub fn new(source: S, control: FadeControl) -> Self {
        let sample_rate = source.sample_rate().get();
        let channels = source.channels().get();
        let initial_volume = control.target_volume();
        let generation = control.generation();

        Self {
            source,
            control,
            current_volume: initial_volume,
            fade_start_volume: initial_volume,
            fade_samples_remaining: 0,
            fade_samples_total: 0,
            last_generation: generation,
            sample_rate,
            channels,
            equal_power: false,
        }
    }

    /// Check for new fade requests and start fade if needed
    fn check_fade_request(&mut self) {
        let current_gen = self.control.generation();
        if current_gen != self.last_generation {
            self.last_generation = current_gen;

            let duration_ms = self.control.fade_duration_ms();
            let target = self.control.target_volume();
            self.equal_power = self.control.inner.curve.load(Ordering::Acquire) == 1;
            let requested_start = f32::from_bits(
                self.control
                    .inner
                    .requested_start_volume
                    .load(Ordering::Acquire),
            );
            if requested_start.is_finite() {
                self.current_volume = requested_start.clamp(0.0, 1.0);
            }

            if duration_ms == 0 {
                // Instant change
                self.current_volume = target;
                self.fade_samples_remaining = 0;
                self.control
                    .inner
                    .completed_generation
                    .store(current_gen, Ordering::Release);
            } else {
                // Start new fade
                self.fade_start_volume = self.current_volume;
                let total_samples = self.sample_rate as u64
                    * u64::from(self.channels.max(1))
                    * u64::from(duration_ms)
                    / 1000;
                self.fade_samples_total = total_samples.clamp(1, u64::from(u32::MAX)) as u32;
                self.fade_samples_remaining = self.fade_samples_total;
            }
        }
    }

    /// Update current volume based on fade progress
    fn update_volume(&mut self) {
        if self.fade_samples_remaining > 0 {
            self.fade_samples_remaining -= 1;

            let target = self.control.target_volume();
            let progress =
                1.0 - (self.fade_samples_remaining as f32 / self.fade_samples_total as f32);

            if self.equal_power {
                let angle = progress * std::f32::consts::FRAC_PI_2;
                self.current_volume = if target >= self.fade_start_volume {
                    self.fade_start_volume + (target - self.fade_start_volume) * angle.sin()
                } else {
                    target + (self.fade_start_volume - target) * angle.cos()
                };
            } else {
                let eased = 1.0 - (1.0 - progress).powi(3);
                self.current_volume =
                    self.fade_start_volume + (target - self.fade_start_volume) * eased;
            }

            // Snap to target when done
            if self.fade_samples_remaining == 0 {
                self.current_volume = target;
                self.control
                    .inner
                    .completed_generation
                    .store(self.last_generation, Ordering::Release);
            }
        }
    }
}

impl<S> Iterator for FadeEnvelope<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Check for new fade requests (lock-free)
        self.check_fade_request();

        // Update volume based on fade progress
        self.update_volume();

        // Apply volume to sample
        self.source
            .next()
            .map(|sample| sample * self.current_volume)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for FadeEnvelope<S>
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

    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.source.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSource {
        samples: std::vec::IntoIter<f32>,
        sample_rate: u32,
        channels: u16,
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
    fn fade_progresses_in_samples_and_reports_completion() {
        let control = FadeControl::new(1.0);
        let source = TestSource {
            samples: vec![1.0; 8].into_iter(),
            sample_rate: 1_000,
            channels: 1,
        };
        let mut envelope = FadeEnvelope::new(source, control.clone());
        control.fade_to(0.0, Duration::from_millis(4));

        let values: Vec<_> = (&mut envelope).collect();
        assert!(values[0] < 1.0);
        assert_eq!(values[3], 0.0);
        assert!(control.is_complete());
    }

    #[test]
    fn a_new_fade_interrupts_an_old_ramp_without_sleeping() {
        let control = FadeControl::new(1.0);
        let source = TestSource {
            samples: vec![1.0; 4].into_iter(),
            sample_rate: 1_000,
            channels: 1,
        };
        let mut envelope = FadeEnvelope::new(source, control.clone());
        control.fade_to(0.0, Duration::from_millis(10));
        let first = envelope.next().unwrap();
        control.set_volume(1.0);
        let second = envelope.next().unwrap();

        assert!(first < 1.0);
        assert_eq!(second, 1.0);
        assert!(control.is_complete());
    }

    #[test]
    fn explicit_start_restarts_a_fade_after_the_source_is_already_running() {
        let control = FadeControl::new(1.0);
        let source = TestSource {
            samples: vec![1.0; 8].into_iter(),
            sample_rate: 1_000,
            channels: 1,
        };
        let mut envelope = FadeEnvelope::new(source, control.clone());
        assert_eq!(envelope.next().unwrap(), 1.0);

        control.fade_from_to(0.0, 1.0, Duration::from_millis(4));
        let faded: Vec<_> = (&mut envelope).take(4).collect();

        assert!(faded[0] < 1.0);
        assert!(faded.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(faded[3], 1.0);
        assert!(control.is_complete());
    }

    #[test]
    fn stereo_fade_duration_is_measured_in_frames_not_interleaved_samples() {
        let control = FadeControl::new(1.0);
        let source = TestSource {
            samples: vec![1.0; 8].into_iter(),
            sample_rate: 1_000,
            channels: 2,
        };
        let mut envelope = FadeEnvelope::new(source, control.clone());
        control.fade_to(0.0, Duration::from_millis(4));

        let values: Vec<_> = (&mut envelope).collect();
        assert!(values[3] > 0.0);
        assert_eq!(values[7], 0.0);
        assert!(control.is_complete());
    }

    #[test]
    fn equal_power_pair_preserves_unit_power() {
        let outgoing_control = FadeControl::new(1.0);
        let incoming_control = FadeControl::new(0.0);
        let source = || TestSource {
            samples: vec![1.0; 4].into_iter(),
            sample_rate: 1_000,
            channels: 1,
        };
        let mut outgoing = FadeEnvelope::new(source(), outgoing_control.clone());
        let mut incoming = FadeEnvelope::new(source(), incoming_control.clone());
        outgoing_control.fade_to_equal_power(0.0, Duration::from_millis(4));
        incoming_control.fade_to_equal_power(1.0, Duration::from_millis(4));

        for (out, input) in (&mut outgoing).zip(&mut incoming) {
            assert!((out * out + input * input - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn explicit_equal_power_start_survives_coalesced_preload_commands() {
        let control = FadeControl::new(1.0);
        let source = TestSource {
            samples: vec![1.0; 4].into_iter(),
            sample_rate: 1_000,
            channels: 1,
        };
        let mut envelope = FadeEnvelope::new(source, control.clone());

        // A paused preload may not consume a sample between these commands.
        // The final ramp must therefore carry its zero start explicitly.
        control.set_volume(0.0);
        control.fade_from_to_equal_power(0.0, 1.0, Duration::from_millis(4));

        let values: Vec<_> = (&mut envelope).collect();
        assert!(values[0] < 0.5);
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(values[3], 1.0);
        assert!(control.is_complete());
    }
}
