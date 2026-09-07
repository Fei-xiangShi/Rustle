//! Audio processing chain
//!
//! Unified audio processing pipeline that combines:
//! - Preamp (gain control before EQ)
//! - 10-band parametric equalizer
//! - Fade envelope
//! - Real-time audio analyzer for visualization
//!

use rodio::Source;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use super::analyzer::{AnalyzingSource, AudioAnalysisData};
use super::equalizer::{Equalizer, EqualizerParams};
use super::fade::{FadeControl, FadeEnvelope};

struct AudioProcessingChainShared {
    preamp_linear_bits: Arc<AtomicU32>,
    eq_params: EqualizerParams,
    analysis_enabled: AtomicBool,
    analysis_decay_bits: AtomicU32,
    active_analysis: RwLock<AudioAnalysisData>,
    idle_analysis: AudioAnalysisData,
}

/// Shared audio processing chain configuration.
///
/// Only persistent DSP configuration lives here. Per-playback runtime state
/// such as fades and analyzer buffers live in `PlaybackProcessingRuntime`.
#[derive(Clone)]
pub struct AudioProcessingChain {
    shared: Arc<AudioProcessingChainShared>,
}

/// Per-playback processing runtime state.
///
/// Each sink gets its own runtime so current playback and preloaded sinks do
/// not share fade or analyzer state.
#[derive(Clone)]
pub struct PlaybackProcessingRuntime {
    fade_control: FadeControl,
    automix_gain_bits: Arc<AtomicU32>,
    bass_control: BassAutomationControl,
    natural_end: Arc<AtomicBool>,
    analysis: AudioAnalysisData,
}

#[derive(Clone)]
struct BassAutomationControl {
    target_bits: Arc<AtomicU32>,
    delay_ms: Arc<AtomicU32>,
    duration_ms: Arc<AtomicU32>,
    generation: Arc<AtomicU32>,
}

impl AudioProcessingChain {
    /// Create a new audio processing chain
    pub fn new() -> Self {
        let idle_analysis = AudioAnalysisData::new();
        idle_analysis.set_enabled(false);

        Self {
            shared: Arc::new(AudioProcessingChainShared {
                preamp_linear_bits: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
                eq_params: EqualizerParams::new(44100),
                analysis_enabled: AtomicBool::new(false),
                analysis_decay_bits: AtomicU32::new(0.85_f32.to_bits()),
                active_analysis: RwLock::new(idle_analysis.clone()),
                idle_analysis,
            }),
        }
    }

    // Preamp controls
    // ========================================================================

    /// Set preamp gain in dB (-12 to +12)
    pub fn set_preamp(&self, db: f32) {
        let db = db.clamp(-12.0, 12.0);
        let linear = if db.abs() < 0.01 {
            1.0
        } else {
            10.0_f32.powf(db / 20.0)
        };
        self.shared
            .preamp_linear_bits
            .store(linear.to_bits(), Ordering::Release);
    }

    // ========================================================================
    // Equalizer controls
    // ========================================================================

    /// Enable or disable the equalizer
    pub fn set_equalizer_enabled(&self, enabled: bool) {
        self.shared.eq_params.set_enabled(enabled);
    }

    /// Set all 10 band gains at once (in dB, typically -12 to +12)
    pub fn set_equalizer_gains(&self, gains: [f32; 10]) {
        self.shared.eq_params.set_gains(gains);
    }

    // ========================================================================
    // Analysis data access
    // ========================================================================

    /// Get audio analysis data for visualization
    pub fn analysis(&self) -> AudioAnalysisData {
        self.shared
            .active_analysis
            .read()
            .map(|analysis| analysis.clone())
            .unwrap_or_else(|_| self.shared.idle_analysis.clone())
    }

    pub fn set_analysis_enabled(&self, enabled: bool) {
        self.shared
            .analysis_enabled
            .store(enabled, Ordering::Release);
        self.analysis().set_enabled(enabled);
    }

    pub fn set_analysis_decay(&self, decay: f32) {
        let decay = decay.clamp(0.0, 0.99);
        self.shared
            .analysis_decay_bits
            .store(decay.to_bits(), Ordering::Release);
        self.analysis().set_decay(decay);
    }

    /// Force EQ coefficients refresh
    /// This marks the EQ parameters as dirty, forcing a recalculation
    /// on the next audio sample. Useful when switching tracks to ensure
    /// the audio processing chain is properly initialized.
    pub fn refresh_eq_coefficients(&self) {
        self.shared.eq_params.mark_dirty();
    }

    // ========================================================================
    // Chain configuration
    // ========================================================================

    /// Update sample rate (called when audio format changes)
    pub fn set_sample_rate(&self, sample_rate: u32) {
        self.shared.eq_params.set_sample_rate(sample_rate);
    }

    // ========================================================================
    // Runtime lifecycle
    // ========================================================================

    pub fn create_runtime(&self) -> PlaybackProcessingRuntime {
        PlaybackProcessingRuntime::new(self.analysis_decay())
    }

    pub fn activate_runtime(&self, runtime: Option<&PlaybackProcessingRuntime>) {
        let analysis = if let Some(runtime) = runtime {
            runtime.prepare_analysis(self.analysis_enabled(), self.analysis_decay());
            runtime.analysis.clone()
        } else {
            self.shared.idle_analysis.reset();
            self.shared.idle_analysis.set_enabled(false);
            self.shared.idle_analysis.clone()
        };

        if let Ok(mut active_analysis) = self.shared.active_analysis.write() {
            *active_analysis = analysis;
        }
    }

    fn analysis_enabled(&self) -> bool {
        self.shared.analysis_enabled.load(Ordering::Acquire)
    }

    fn analysis_decay(&self) -> f32 {
        f32::from_bits(self.shared.analysis_decay_bits.load(Ordering::Acquire))
    }

    // ========================================================================
    // Source processing
    // ========================================================================

    /// Apply the processing chain to an audio source using the provided
    /// per-playback runtime state.
    pub fn apply<S>(
        &self,
        source: S,
        track_gain: f32,
        runtime: PlaybackProcessingRuntime,
    ) -> ProcessedSource<S>
    where
        S: Source<Item = f32>,
    {
        // Update sample rate from source
        self.set_sample_rate(source.sample_rate().get());

        ProcessedSource::new(source, self.shared.clone(), runtime, track_gain)
    }
}

impl Default for AudioProcessingChain {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackProcessingRuntime {
    fn new(analysis_decay: f32) -> Self {
        let analysis = AudioAnalysisData::new();
        analysis.set_enabled(false);
        analysis.set_decay(analysis_decay);

        Self {
            fade_control: FadeControl::new(1.0),
            automix_gain_bits: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            bass_control: BassAutomationControl::new(1.0),
            natural_end: Arc::new(AtomicBool::new(false)),
            analysis,
        }
    }

    pub fn fade_to(&self, volume: f32, duration: std::time::Duration) {
        self.fade_control.fade_to(volume, duration);
    }

    pub(crate) fn fade_from_to(&self, from: f32, volume: f32, duration: std::time::Duration) {
        self.fade_control.fade_from_to(from, volume, duration);
    }

    pub(crate) fn crossfade_to(&self, volume: f32, duration: std::time::Duration) {
        self.fade_control.fade_to_equal_power(volume, duration);
    }

    pub(crate) fn crossfade_from_to(&self, from: f32, volume: f32, duration: std::time::Duration) {
        self.fade_control
            .fade_from_to_equal_power(from, volume, duration);
    }

    pub fn set_fade_volume(&self, volume: f32) {
        self.fade_control.set_volume(volume);
    }

    pub(crate) fn fade_complete(&self) -> bool {
        self.fade_control.is_complete()
    }

    pub(crate) fn set_automix_gain_db(&self, gain_db: f32) {
        let gain_db = if gain_db.is_finite() { gain_db } else { 0.0 };
        let linear = 10.0_f32.powf(gain_db.clamp(-9.0, 9.0) / 20.0);
        self.automix_gain_bits
            .store(linear.to_bits(), Ordering::Release);
    }

    pub(crate) fn automix_gain_db(&self) -> f32 {
        let linear = f32::from_bits(self.automix_gain_bits.load(Ordering::Acquire));
        if linear.is_finite() && linear > 0.0 {
            20.0 * linear.log10()
        } else {
            0.0
        }
    }

    pub(crate) fn set_bass_mix(&self, mix: f32) {
        self.bass_control.set(mix, std::time::Duration::ZERO);
    }

    pub(crate) fn automate_bass_mix(&self, mix: f32, duration: std::time::Duration) {
        self.bass_control.set(mix, duration);
    }

    pub(crate) fn automate_bass_mix_after(
        &self,
        mix: f32,
        delay: std::time::Duration,
        duration: std::time::Duration,
    ) {
        self.bass_control.set_after(mix, delay, duration);
    }

    pub(crate) fn reset_automix(&self) {
        self.set_automix_gain_db(0.0);
        self.reset_automix_transition();
    }

    /// Clear transition-only Bass Swap state while retaining the per-track
    /// Automix loudness anchor.
    pub(crate) fn reset_automix_transition(&self) {
        self.set_bass_mix(1.0);
    }

    pub(crate) fn natural_end_reached(&self) -> bool {
        self.natural_end.load(Ordering::Acquire)
    }

    pub(crate) fn reset_natural_end(&self) {
        self.natural_end.store(false, Ordering::Release);
    }

    fn prepare_analysis(&self, enabled: bool, decay: f32) {
        self.analysis.set_decay(decay);
        self.analysis.reset();
        self.analysis.set_enabled(enabled);
    }
}

impl BassAutomationControl {
    fn new(initial: f32) -> Self {
        Self {
            target_bits: Arc::new(AtomicU32::new(initial.clamp(0.0, 1.0).to_bits())),
            delay_ms: Arc::new(AtomicU32::new(0)),
            duration_ms: Arc::new(AtomicU32::new(0)),
            generation: Arc::new(AtomicU32::new(0)),
        }
    }

    fn set(&self, target: f32, duration: std::time::Duration) {
        self.set_after(target, std::time::Duration::ZERO, duration);
    }

    fn set_after(&self, target: f32, delay: std::time::Duration, duration: std::time::Duration) {
        self.target_bits
            .store(target.clamp(0.0, 1.0).to_bits(), Ordering::Release);
        self.delay_ms.store(
            delay.as_millis().min(u128::from(u32::MAX)) as u32,
            Ordering::Release,
        );
        self.duration_ms.store(
            duration.as_millis().min(u128::from(u32::MAX)) as u32,
            Ordering::Release,
        );
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

type ProcessingPipeline<S> = CompletionSource<
    AnalyzingSource<
        FadeEnvelope<
            SoftClipSource<
                BassSwapSource<AutomixGainSource<Equalizer<PreampSource<TrackGainSource<S>>>>>,
            >,
        >,
    >,
>;

/// Audio source with processing chain applied
///
/// This wraps the source and applies track gain, preamp, EQ, fade, and analysis in sequence.
pub struct ProcessedSource<S>
where
    S: Source<Item = f32>,
{
    /// Inner source with full processing chain applied
    inner: ProcessingPipeline<S>,
}

impl<S> ProcessedSource<S>
where
    S: Source<Item = f32>,
{
    fn new(
        source: S,
        shared: Arc<AudioProcessingChainShared>,
        runtime: PlaybackProcessingRuntime,
        track_gain: f32,
    ) -> Self {
        // Source -> TrackGain -> Preamp -> EQ -> Automix loudness -> Bass Swap
        // -> SoftClip -> Fade -> Analyzer -> natural-end signal.
        let track_gain_source = TrackGainSource::new(source, track_gain);
        let preamp_source = PreampSource::new(track_gain_source, shared.preamp_linear_bits.clone());
        let eq_source = Equalizer::new(preamp_source, shared.eq_params.clone());
        let gain_source = AutomixGainSource::new(eq_source, runtime.automix_gain_bits.clone());
        let bass_source = BassSwapSource::new(gain_source, runtime.bass_control.clone());
        let clip_source = SoftClipSource::new(bass_source);
        let fade_source = FadeEnvelope::new(clip_source, runtime.fade_control.clone());
        let analyzed = AnalyzingSource::new(fade_source, runtime.analysis.clone());
        let completed = CompletionSource::new(analyzed, runtime.natural_end.clone());

        Self { inner: completed }
    }
}

impl<S> Iterator for ProcessedSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> Source for ProcessedSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

/// Per-track gain applied from normalization metadata or waveform analysis.
struct TrackGainSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    gain: f32,
}

impl<S> TrackGainSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, gain: f32) -> Self {
        Self {
            source,
            gain: if gain.is_finite() && gain > 0.0 {
                gain
            } else {
                1.0
            },
        }
    }
}

impl<S> Iterator for TrackGainSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next()?;
        let gain = self.gain;

        if (gain - 1.0).abs() < 0.001 {
            Some(sample)
        } else {
            Some(sample * gain)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for TrackGainSource<S>
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
        self.source.try_seek(pos)
    }
}

/// Shared preamp wrapper driven by UI settings.
struct PreampSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    preamp_linear_bits: Arc<AtomicU32>,
}

impl<S> PreampSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, preamp_linear_bits: Arc<AtomicU32>) -> Self {
        Self {
            source,
            preamp_linear_bits,
        }
    }

    fn preamp_linear(&self) -> f32 {
        f32::from_bits(self.preamp_linear_bits.load(Ordering::Acquire))
    }
}

impl<S> Iterator for PreampSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next()?;
        let gain = self.preamp_linear();

        if (gain - 1.0).abs() < 0.001 {
            Some(sample)
        } else {
            Some(sample * gain)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for PreampSource<S>
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
        self.source.try_seek(pos)
    }
}

struct AutomixGainSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    gain_bits: Arc<AtomicU32>,
}

impl<S> AutomixGainSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, gain_bits: Arc<AtomicU32>) -> Self {
        Self { source, gain_bits }
    }
}

impl<S> Iterator for AutomixGainSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let gain = f32::from_bits(self.gain_bits.load(Ordering::Acquire));
        self.source.next().map(|sample| sample * gain)
    }
}

impl<S> Source for AutomixGainSource<S>
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
        self.source.try_seek(pos)
    }
}

struct BassSwapSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    control: BassAutomationControl,
    low_pass: Vec<f32>,
    channel: usize,
    current_mix: f32,
    start_mix: f32,
    remaining: u64,
    delay_remaining: u64,
    total: u64,
    generation: u32,
    alpha: f32,
}

impl<S> BassSwapSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, control: BassAutomationControl) -> Self {
        let channels = source.channels().get() as usize;
        let sample_rate = source.sample_rate().get() as f32;
        let alpha = 1.0 - (-std::f32::consts::TAU * 400.0 / sample_rate).exp();
        let current_mix = f32::from_bits(control.target_bits.load(Ordering::Acquire));
        let generation = control.generation.load(Ordering::Acquire);
        Self {
            source,
            control,
            low_pass: vec![0.0; channels],
            channel: 0,
            current_mix,
            start_mix: current_mix,
            remaining: 0,
            delay_remaining: 0,
            total: 0,
            generation,
            alpha,
        }
    }

    fn update_automation(&mut self) {
        let generation = self.control.generation.load(Ordering::Acquire);
        if generation != self.generation {
            self.generation = generation;
            self.start_mix = self.current_mix;
            let duration_ms = self.control.duration_ms.load(Ordering::Acquire) as u64;
            let samples_per_second = u64::from(self.source.sample_rate().get())
                .saturating_mul(u64::from(self.source.channels().get()));
            self.delay_remaining = samples_per_second
                .saturating_mul(self.control.delay_ms.load(Ordering::Acquire) as u64)
                .saturating_div(1_000);
            self.total = samples_per_second
                .saturating_mul(duration_ms)
                .saturating_div(1_000);
            self.remaining = self.total;
            if self.total == 0 && self.delay_remaining == 0 {
                self.current_mix = f32::from_bits(self.control.target_bits.load(Ordering::Acquire));
            }
        }
        if self.delay_remaining > 0 {
            self.delay_remaining -= 1;
        } else if self.remaining > 0 {
            let target = f32::from_bits(self.control.target_bits.load(Ordering::Acquire));
            let progress = 1.0 - self.remaining as f32 / self.total.max(1) as f32;
            self.current_mix = self.start_mix + (target - self.start_mix) * progress;
            self.remaining -= 1;
        } else {
            self.current_mix = f32::from_bits(self.control.target_bits.load(Ordering::Acquire));
        }
    }
}

impl<S> Iterator for BassSwapSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.update_automation();
        let sample = self.source.next()?;
        let channel = self.channel.min(self.low_pass.len().saturating_sub(1));
        let low = self.low_pass[channel] + self.alpha * (sample - self.low_pass[channel]);
        self.low_pass[channel] = low;
        self.channel = (self.channel + 1) % self.low_pass.len().max(1);
        Some((sample - low) + low * self.current_mix)
    }
}

impl<S> Source for BassSwapSource<S>
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
        let result = self.source.try_seek(pos);
        if result.is_ok() {
            self.low_pass.fill(0.0);
            self.channel = 0;
        }
        result
    }
}

struct CompletionSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
    ended: Arc<AtomicBool>,
}

impl<S> CompletionSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, ended: Arc<AtomicBool>) -> Self {
        ended.store(false, Ordering::Release);
        Self { source, ended }
    }
}

impl<S> Iterator for CompletionSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next();
        if sample.is_none() {
            self.ended.store(true, Ordering::Release);
        }
        sample
    }
}

impl<S> Source for CompletionSource<S>
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
        let result = self.source.try_seek(pos);
        if result.is_ok() {
            self.ended.store(false, Ordering::Release);
        }
        result
    }
}

/// Final soft-clip stage applied once after gain and EQ.
struct SoftClipSource<S>
where
    S: Source<Item = f32>,
{
    source: S,
}

impl<S> SoftClipSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S) -> Self {
        Self { source }
    }
}

impl<S> Iterator for SoftClipSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.source.next()?;
        if sample.abs() < 0.9 {
            Some(sample)
        } else {
            Some(soft_clip(sample))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<S> Source for SoftClipSource<S>
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

    #[test]
    fn completion_signal_is_set_by_source_exhaustion() {
        let chain = AudioProcessingChain::new();
        let runtime = chain.create_runtime();
        let source = rodio::buffer::SamplesBuffer::new(
            rodio::ChannelCount::new(1).unwrap(),
            rodio::SampleRate::new(1_000).unwrap(),
            vec![0.1, 0.2],
        );
        let mut processed = chain.apply(source, 1.0, runtime.clone());
        assert!(!runtime.natural_end_reached());
        assert!(processed.next().is_some());
        assert!(processed.next().is_some());
        assert!(processed.next().is_none());
        assert!(runtime.natural_end_reached());
    }

    #[test]
    fn invalid_track_gain_is_treated_as_unity() {
        for gain in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let source = rodio::buffer::SamplesBuffer::new(
                rodio::ChannelCount::new(1).unwrap(),
                rodio::SampleRate::new(1_000).unwrap(),
                vec![0.25],
            );
            let mut gained = TrackGainSource::new(source, gain);
            assert_eq!(gained.next(), Some(0.25));
        }
    }

    #[test]
    fn bass_swap_automation_is_per_runtime_and_resettable() {
        let chain = AudioProcessingChain::new();
        let runtime = chain.create_runtime();
        runtime.set_bass_mix(0.0);
        runtime.set_automix_gain_db(-6.0);
        let source = rodio::buffer::SamplesBuffer::new(
            rodio::ChannelCount::new(1).unwrap(),
            rodio::SampleRate::new(1_000).unwrap(),
            vec![0.5; 120],
        );
        let mut processed = chain.apply(source, 1.0, runtime.clone());
        runtime.automate_bass_mix(1.0, std::time::Duration::from_millis(100));
        let output: Vec<_> = processed.by_ref().collect();
        assert!(output[0].abs() < output[100].abs());
        runtime.reset_automix_transition();
        assert!((runtime.automix_gain_db() + 6.0).abs() < 1e-4);
        assert_eq!(
            f32::from_bits(runtime.bass_control.target_bits.load(Ordering::Acquire)),
            1.0
        );
        runtime.reset_automix();
        assert!(runtime.automix_gain_db().abs() < 1e-5);
        assert_eq!(
            f32::from_bits(runtime.bass_control.target_bits.load(Ordering::Acquire)),
            1.0
        );
    }

    #[test]
    fn bass_swap_holds_incoming_bass_until_midpoint_then_releases() {
        let control = BassAutomationControl::new(0.0);
        let source = rodio::buffer::SamplesBuffer::new(
            rodio::ChannelCount::new(1).unwrap(),
            rodio::SampleRate::new(1_000).unwrap(),
            vec![0.5; 140],
        );
        let mut bass = BassSwapSource::new(source, control.clone());
        control.set_after(
            1.0,
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(50),
        );

        for _ in 0..50 {
            bass.next().unwrap();
            assert_eq!(bass.current_mix, 0.0);
        }
        for _ in 0..25 {
            bass.next().unwrap();
        }
        assert!(bass.current_mix > 0.0 && bass.current_mix < 1.0);
        for _ in 0..30 {
            bass.next().unwrap();
        }
        assert_eq!(bass.current_mix, 1.0);
    }
}
