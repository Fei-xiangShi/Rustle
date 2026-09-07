//! Audio player - playback control and audio output
//!
//! Provides:
//! - Play/pause/seek/volume control
//! - Streaming buffer playback
//! - Audio processing chain integration
//! - Pre-decoded Sink creation for seamless track switching

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{
    Decoder, DeviceSinkBuilder as OutputStreamBuilder, MixerDeviceSink as OutputStream,
    Player as Sink, Source,
};

use super::automix::{TransitionDirective, TransitionKind};
use super::chain::{AudioProcessingChain, PlaybackProcessingRuntime};
use super::streaming::{SharedBuffer, StreamingBuffer, StreamingReaderCancellation};

pub(crate) type PreparedStreamingSource = Decoder<StreamingBuffer>;

/// Build the decoder outside the audio control actor.
///
/// Streaming producers must complete the strict Range probe and startup
/// high-watermark contract before submitting preparation. Decoder probing can
/// still call `Read + Seek`, so callers run this function on a preparation
/// worker rather than blocking lifecycle commands.
pub(crate) fn prepare_streaming_source(
    buffer: StreamingBuffer,
) -> Result<PreparedStreamingSource, String> {
    let shared = buffer.shared();
    let byte_len = shared.total_size();
    if byte_len == 0 {
        return Err(
            "UnsupportedStreaming: source preparation requires a validated total size".to_string(),
        );
    }
    if shared.is_cancelled() {
        return Err("Cancelled: streaming source preparation was cancelled".to_string());
    }
    if let Some(error) = shared.error_message() {
        return Err(error);
    }

    Decoder::builder()
        .with_data(buffer)
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
        .map_err(|e| format!("Failed to decode streaming audio: {}", e))
}

/// Cached audio devices to avoid repeated enumeration (which triggers Jack/ALSA warnings)
static AUDIO_DEVICES_CACHE: OnceLock<Vec<AudioDevice>> = OnceLock::new();

/// Playback status
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    /// Buffering - waiting for more data before playback can continue.
    /// Used when playback catches up with download, or when seeking to unbuffered position.
    Buffering {
        position: Duration,
    },
}

/// Current playback info
#[derive(Debug, Clone)]
pub struct PlaybackInfo {
    pub status: PlaybackStatus,
    pub position: Duration,
    pub duration: Duration,
    pub volume: f32,
}

impl Default for PlaybackInfo {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            volume: 1.0,
        }
    }
}

/// Audio player state (shared between threads)
struct PlayerState {
    status: PlaybackStatus,
    duration: Duration,
    volume: f32,
    paused_position: Option<Duration>,
    current_track_gain: f32,
    device_name: Option<String>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            duration: Duration::ZERO,
            volume: 1.0,
            paused_position: None,
            current_track_gain: 1.0,
            device_name: None,
        }
    }
}

struct OutgoingTransition {
    sink: Sink,
    runtime: PlaybackProcessingRuntime,
    reader_cancellation: Option<StreamingReaderCancellation>,
    shared_buffer: Option<SharedBuffer>,
    _group: super::automix::ScheduleGroup,
    advanced: bool,
}

pub(crate) struct DetachedStreamingPlayback {
    pub sink: Sink,
    pub duration: Duration,
    pub cache_path: Option<PathBuf>,
    pub track_gain: f32,
    pub was_paused: bool,
}

fn prepare_transition_for_handoff<F>(
    transition: TransitionDirective,
    current_position: Duration,
    mut try_seek_entry: F,
) -> TransitionDirective
where
    F: FnMut(Duration) -> bool,
{
    if transition.kind != TransitionKind::Automix {
        return transition;
    }
    let scheduler_late = transition.expected_exit.is_some_and(|expected| {
        current_position
            > expected.saturating_add(Duration::from_millis(super::automix::SCHEDULER_HORIZON_MS))
    });
    if scheduler_late {
        return TransitionDirective::baseline_natural(transition.group);
    }
    if transition.entry > Duration::ZERO && !try_seek_entry(transition.entry) {
        return TransitionDirective::baseline_natural(transition.group);
    }
    transition
}

/// Audio player - simplified, focused on playback control
/// Preloading is managed externally by PreloadManager
pub struct AudioPlayer {
    _stream: OutputStream,
    current_sink: Option<Sink>,
    current_path: Option<PathBuf>,
    state: Arc<Mutex<PlayerState>>,
    chain: AudioProcessingChain,
    current_runtime: Option<PlaybackProcessingRuntime>,
    current_reader_cancellation: Option<StreamingReaderCancellation>,
    is_streaming: bool,
    detached_seek_position: Option<Duration>,
    position_offset: Duration,
    pending_pause_fade: bool,
    outgoing_transition: Option<OutgoingTransition>,
    last_transition_group: Option<super::automix::ScheduleGroup>,
}

impl AudioPlayer {
    const FADE_DURATION: Duration = Duration::from_millis(300);

    /// Create a new audio player with default output device
    pub fn new(chain: AudioProcessingChain) -> Result<Self, String> {
        Self::with_device(None, chain)
    }

    /// Create a new audio player with specified output device
    pub fn with_device(
        device_name: Option<&str>,
        chain: AudioProcessingChain,
    ) -> Result<Self, String> {
        let stream = if let Some(name) = device_name {
            Self::create_stream_for_device(name)?
        } else {
            OutputStreamBuilder::open_default_sink()
                .map_err(|e| format!("Failed to create audio output: {}", e))?
        };

        let state = PlayerState {
            device_name: device_name.map(str::to_string),
            ..PlayerState::default()
        };

        Ok(Self {
            _stream: stream,
            current_sink: None,
            current_path: None,
            state: Arc::new(Mutex::new(state)),
            chain,
            current_runtime: None,
            current_reader_cancellation: None,
            is_streaming: false,
            detached_seek_position: None,
            position_offset: Duration::ZERO,
            pending_pause_fade: false,
            outgoing_transition: None,
            last_transition_group: None,
        })
    }

    /// Create output stream for a specific device by name
    fn create_stream_for_device(device_name: &str) -> Result<OutputStream, String> {
        let host = rodio::cpal::default_host();

        let device = host
            .output_devices()
            .map_err(|e| format!("Failed to enumerate devices: {}", e))?
            .find(|device| {
                device
                    .description()
                    .map(|description| description.name() == device_name)
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("Device not found: {}", device_name))?;

        let config = device
            .default_output_config()
            .map_err(|e| format!("Failed to get device config: {}", e))?;
        let sample_rate = rodio::SampleRate::new(config.sample_rate())
            .ok_or_else(|| "Output device reported a zero sample rate".to_string())?;

        OutputStreamBuilder::from_device(device)
            .map_err(|e| format!("Failed to create stream builder: {}", e))?
            .with_sample_rate(sample_rate)
            .open_stream()
            .map_err(|e| format!("Failed to open stream: {}", e))
    }

    /// Switch to a different audio output device
    pub fn switch_device(
        &mut self,
        device_name: Option<&str>,
    ) -> Result<Option<(PathBuf, Duration, bool)>, String> {
        let playback_state = self.current_path.clone().map(|path| {
            let info = self.get_info();
            let was_playing = info.status == PlaybackStatus::Playing;
            let position = info.position;
            (path, position, was_playing)
        });

        self.stop();

        let stream = if let Some(name) = device_name {
            Self::create_stream_for_device(name)?
        } else {
            OutputStreamBuilder::open_default_sink()
                .map_err(|e| format!("Failed to create audio output: {}", e))?
        };

        {
            let mut state = self.state.lock().unwrap();
            state.device_name = device_name.map(|s| s.to_string());
        }

        self._stream = stream;

        tracing::info!("Switched audio device to: {:?}", device_name);
        Ok(playback_state)
    }

    /// Get current sink volume controlled by the user setting.
    fn get_sink_volume(&self) -> f32 {
        self.state.lock().unwrap().volume
    }

    fn decode_local_file(path: &Path) -> Result<Decoder<BufReader<File>>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let reader = BufReader::new(file);

        Decoder::builder()
            .with_data(reader)
            .with_byte_len(file_len)
            .with_seekable(true)
            .build()
            .map_err(|e| format!("Failed to decode audio: {}", e))
    }

    fn prepare_runtime(&self, fade_in: bool) -> PlaybackProcessingRuntime {
        let runtime = self.chain.create_runtime();
        runtime.set_fade_volume(if fade_in { 0.0 } else { 1.0 });
        runtime
    }

    fn try_seek_on_start(sink: &Sink, position: Duration, path: &Path) -> Option<String> {
        if position.is_zero() {
            return None;
        }

        if let Err(err) = sink.try_seek(position) {
            tracing::warn!(
                "Failed to seek to {:?} while starting {:?}: {}",
                position,
                path,
                err
            );
            return Some("Seek not supported for this format".to_string());
        }

        None
    }

    /// Play a file with fade in option
    pub fn play_with_fade(
        &mut self,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    ) -> Result<(), String> {
        self.stop();
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(fade_in);
        self.chain.activate_runtime(Some(&runtime));

        let source = Self::decode_local_file(&path)?;
        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);

        let volume = self.get_sink_volume();
        sink.set_volume(volume);

        if fade_in {
            runtime.fade_to(1.0, Self::FADE_DURATION);
        }

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Playing;
            state.duration = duration;
            state.paused_position = None;
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        self.current_path = Some(path.clone());
        self.current_runtime = Some(runtime);
        self.is_streaming = false;
        self.position_offset = Duration::ZERO;

        tracing::info!("Playing audio, duration: {:?}", duration);
        Ok(())
    }

    /// Create a preload sink for external use (by PreloadManager)
    /// Returns (Sink, Duration) - sink is paused and ready for playback
    pub fn create_preload_sink(
        &self,
        path: &Path,
        track_gain: f32,
    ) -> Result<(Sink, Duration, PlaybackProcessingRuntime), String> {
        let source = Self::decode_local_file(path)?;
        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        let runtime = self.prepare_runtime(false);
        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);
        sink.set_volume(self.get_sink_volume());
        sink.pause(); // Start paused

        Ok((sink, duration, runtime))
    }

    /// Load a local file into paused state at a target position.
    pub fn load_paused(
        &mut self,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    ) -> Result<(), String> {
        self.stop();
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(false);
        self.chain.activate_runtime(Some(&runtime));

        let source = Self::decode_local_file(&path)?;
        let duration = source.total_duration().unwrap_or(Duration::ZERO);
        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);
        sink.set_volume(self.get_sink_volume());
        sink.pause();

        let _ = Self::try_seek_on_start(&sink, position, &path);

        let paused_position = sink.get_pos();

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Paused;
            state.duration = duration;
            state.paused_position = Some(paused_position);
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        self.current_path = Some(path.clone());
        self.current_runtime = Some(runtime);
        self.is_streaming = false;
        self.position_offset = Duration::ZERO;

        tracing::info!(
            "Loaded paused audio at {:?}, duration: {:?}",
            paused_position,
            duration
        );
        Ok(())
    }

    /// Load a streaming source into paused state at a target position.
    pub(crate) fn load_prepared_streaming_paused(
        &mut self,
        source: PreparedStreamingSource,
        reader_cancellation: StreamingReaderCancellation,
        duration: Duration,
        cache_path: Option<PathBuf>,
        position: Duration,
        track_gain: f32,
    ) -> Result<Option<String>, String> {
        self.stop();
        if !reader_cancellation.activate() {
            return Err("streaming reader demand lease is owned by another decoder".to_string());
        }
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(false);
        self.chain.activate_runtime(Some(&runtime));

        tracing::info!(
            "load_prepared_streaming_paused: cache_path={:?}",
            cache_path
        );

        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);
        sink.set_volume(self.get_sink_volume());
        sink.pause();

        // The preparation worker already sought the decoder. Issuing a Rodio
        // Sink seek here would synchronously wait for the mixer on the control
        // actor and can deadlock behind a blocked streaming read.
        let paused_position = position;

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Paused;
            state.duration = duration;
            state.paused_position = Some(paused_position);
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        self.current_path = cache_path;
        self.current_runtime = Some(runtime);
        self.current_reader_cancellation = Some(reader_cancellation);
        self.is_streaming = true;
        self.position_offset = position;

        tracing::info!(
            "Loaded paused streaming audio at {:?}, duration: {:?}",
            paused_position,
            duration
        );
        Ok(None)
    }

    /// Play a local file from a target position.
    ///
    /// Returns `Ok(Some(error))` when playback started but the initial seek failed.
    pub fn play_from_position_with_fade(
        &mut self,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    ) -> Result<Option<String>, String> {
        self.stop();
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(fade_in);
        self.chain.activate_runtime(Some(&runtime));

        let source = Self::decode_local_file(&path)?;
        let duration = source.total_duration().unwrap_or(Duration::ZERO);
        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);

        let volume = self.get_sink_volume();
        sink.set_volume(volume);
        let seek_error = Self::try_seek_on_start(&sink, position, &path);

        if fade_in {
            runtime.fade_to(1.0, Self::FADE_DURATION);
        }

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Playing;
            state.duration = duration;
            state.paused_position = None;
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        self.current_path = Some(path.clone());
        self.current_runtime = Some(runtime);
        self.is_streaming = false;
        self.position_offset = Duration::ZERO;

        tracing::info!(
            "Playing audio from position {:?}, duration: {:?}",
            position,
            duration
        );
        Ok(seek_error)
    }

    /// Create a preload sink from a decoder prepared off the control actor.
    ///
    /// Returns (Sink, Duration) - sink is paused and ready for playback
    pub(crate) fn create_preload_sink_prepared_streaming(
        &self,
        source: PreparedStreamingSource,
        duration: Duration,
        track_gain: f32,
    ) -> Result<(Sink, Duration, PlaybackProcessingRuntime), String> {
        // Use provided duration since streaming buffer may not know total duration
        let actual_duration = source.total_duration().unwrap_or(duration);

        let runtime = self.prepare_runtime(false);
        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);
        sink.set_volume(self.get_sink_volume());
        sink.pause(); // Start paused

        Ok((sink, actual_duration, runtime))
    }

    /// Play a preloaded sink (from PreloadManager)
    ///
    pub(crate) fn preloaded_will_overlap(
        &self,
        transition: &TransitionDirective,
    ) -> Result<bool, String> {
        if self.last_transition_group == Some(transition.group) {
            return Err(format!(
                "stale Automix transition group {}",
                transition.group.0
            ));
        }
        Ok(self.current_sink.is_some()
            && self
                .state
                .lock()
                .map(|state| state.status == PlaybackStatus::Playing)
                .unwrap_or(false))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "method mirrors the immutable preload and transition handoff contract; remove when PreloadedHandoff owns the payload"
    )]
    pub fn play_preloaded_sink(
        &mut self,
        sink: Sink,
        duration: Duration,
        path: PathBuf,
        is_streaming: bool,
        fade_in: bool,
        track_gain: f32,
        runtime: PlaybackProcessingRuntime,
        reader_cancellation: Option<StreamingReaderCancellation>,
        outgoing_shared_buffer: Option<SharedBuffer>,
        mut transition: TransitionDirective,
    ) -> Result<(), String> {
        self.pending_pause_fade = false;
        let can_overlap = self.preloaded_will_overlap(&transition)?;

        if can_overlap {
            if is_streaming
                && !reader_cancellation
                    .as_ref()
                    .is_some_and(StreamingReaderCancellation::activate)
            {
                return Err("streaming preload shares an active retained-window lease".to_string());
            }
            self.last_transition_group = Some(transition.group);
            if let Some(previous) = self.outgoing_transition.take() {
                if let Some(cancellation) = previous.reader_cancellation {
                    cancellation.cancel();
                }
                if let Some(buffer) = previous.shared_buffer {
                    buffer.clear_buffer_callback();
                }
                previous.sink.stop();
            }
            let old_sink = self.current_sink.take().expect("current sink exists");
            let old_runtime = self
                .current_runtime
                .take()
                .unwrap_or_else(|| self.prepare_runtime(false));
            let old_reader_cancellation = self.current_reader_cancellation.take();
            let outgoing_track_gain = self
                .state
                .lock()
                .map(|state| state.current_track_gain)
                .unwrap_or(1.0);
            let outgoing_automix_gain_db = old_runtime.automix_gain_db();
            transition = prepare_transition_for_handoff(
                transition,
                self.position_offset.saturating_add(old_sink.get_pos()),
                |entry| !is_streaming && sink.try_seek(entry).is_ok(),
            );
            let crossfade = transition.duration;
            let advanced = transition.kind == TransitionKind::Automix;
            old_sink.set_speed(1.0);
            old_runtime.reset_automix_transition();
            runtime.reset_automix();
            if advanced {
                sink.set_speed(transition.automation.rate);
                runtime.set_automix_gain_db(super::automix::effective_automix_gain_db(
                    transition.automation.gain_db,
                    outgoing_track_gain,
                    outgoing_automix_gain_db,
                    track_gain,
                ));
                if transition.automation.bass_swap {
                    let midpoint = crossfade.mul_f32(0.5);
                    let release = Duration::from_millis(600).min(crossfade.mul_f32(0.25));
                    old_runtime.set_bass_mix(1.0);
                    old_runtime.automate_bass_mix(0.0, midpoint);
                    runtime.set_bass_mix(0.0);
                    runtime.automate_bass_mix_after(1.0, midpoint, release);
                }
            }
            old_runtime.crossfade_to(0.0, crossfade);
            sink.set_volume(self.get_sink_volume());
            sink.play();
            // A paused preload may not process the preceding control command
            // before playback starts. Carry the zero anchor in the ramp itself
            // so latest-value command coalescing cannot turn this into a
            // full-volume handoff.
            runtime.crossfade_from_to(0.0, 1.0, crossfade);
            self.outgoing_transition = Some(OutgoingTransition {
                sink: old_sink,
                runtime: old_runtime,
                reader_cancellation: old_reader_cancellation,
                shared_buffer: outgoing_shared_buffer,
                _group: transition.group,
                advanced,
            });
        } else {
            if let Some(buffer) = outgoing_shared_buffer {
                buffer.clear_buffer_callback();
            }
            self.stop();
            if is_streaming
                && !reader_cancellation
                    .as_ref()
                    .is_some_and(StreamingReaderCancellation::activate)
            {
                return Err(
                    "streaming preload could not acquire retained-window demand".to_string()
                );
            }
            sink.set_volume(self.get_sink_volume());
            if fade_in {
                runtime.set_fade_volume(0.0);
                sink.play();
                runtime.fade_to(1.0, Self::FADE_DURATION);
            } else {
                runtime.set_fade_volume(1.0);
                sink.play();
            }
        }
        self.chain.activate_runtime(Some(&runtime));

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Playing;
            state.duration = duration;
            state.paused_position = None;
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        self.current_path = Some(path.clone());
        self.current_runtime = Some(runtime);
        self.current_reader_cancellation = reader_cancellation;
        self.is_streaming = is_streaming;
        self.position_offset = Duration::ZERO;

        tracing::info!(
            "Playing preloaded audio, duration: {:?}, streaming: {}",
            duration,
            is_streaming
        );
        Ok(())
    }

    /// Play a streaming source whose decoder was prepared off the control actor.
    pub(crate) fn play_prepared_streaming(
        &mut self,
        source: PreparedStreamingSource,
        reader_cancellation: StreamingReaderCancellation,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    ) -> Result<(), String> {
        self.stop();
        if !reader_cancellation.activate() {
            return Err("streaming reader demand lease is owned by another decoder".to_string());
        }
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(fade_in);
        self.chain.activate_runtime(Some(&runtime));

        tracing::info!("play_prepared_streaming: cache_path={:?}", cache_path);

        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);

        let volume = self.get_sink_volume();
        sink.set_volume(volume);

        if fade_in {
            runtime.fade_to(1.0, Self::FADE_DURATION);
        }

        {
            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Playing;
            state.duration = duration;
            state.paused_position = None;
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(sink);
        // Store cache path for seek fallback (when streaming seek fails, we can reload from file)
        self.current_path = cache_path;
        self.current_runtime = Some(runtime);
        self.current_reader_cancellation = Some(reader_cancellation);
        self.is_streaming = true;
        self.position_offset = Duration::ZERO;

        tracing::info!("Playing streaming audio, duration: {:?}", duration);
        Ok(())
    }

    /// Pause playback
    pub fn pause(&mut self) {
        self.pause_with_fade(false);
    }

    /// Pause playback with optional fade out
    pub fn pause_with_fade(&mut self, fade_out: bool) {
        if let Some(sink) = self.current_sink.as_ref() {
            if fade_out && let Some(runtime) = self.current_runtime.as_ref() {
                runtime.fade_to(0.0, Self::FADE_DURATION);
                self.pending_pause_fade = true;
                return;
            }

            let current_pos = self.position_offset.saturating_add(sink.get_pos());
            sink.pause();
            sink.set_volume(self.get_sink_volume());

            let mut state = self.state.lock().unwrap();
            state.status = PlaybackStatus::Paused;
            state.paused_position = Some(current_pos);
            self.pending_pause_fade = false;
        }
    }

    /// Complete a requested fade-out once the sample-driven envelope reaches
    /// zero. The audio thread polls this without sleeping.
    pub fn poll_pending_pause(&mut self) -> bool {
        if !self.pending_pause_fade {
            return false;
        }
        let Some(runtime) = self.current_runtime.as_ref() else {
            self.pending_pause_fade = false;
            return false;
        };
        if !runtime.fade_complete() {
            return false;
        }
        if let Some(sink) = self.current_sink.as_ref() {
            let current_pos = self.position_offset.saturating_add(sink.get_pos());
            sink.pause();
            sink.set_volume(self.get_sink_volume());
            if let Ok(mut state) = self.state.lock() {
                state.status = PlaybackStatus::Paused;
                state.paused_position = Some(current_pos);
            }
        }
        self.pending_pause_fade = false;
        true
    }

    pub fn poll_transition(&mut self) -> bool {
        let complete = self.outgoing_transition.as_ref().is_some_and(|transition| {
            transition.runtime.fade_complete()
                || transition.runtime.natural_end_reached()
                || transition.sink.empty()
        });
        if complete && let Some(transition) = self.outgoing_transition.take() {
            if let Some(cancellation) = transition.reader_cancellation {
                cancellation.cancel();
            }
            if let Some(buffer) = transition.shared_buffer {
                buffer.clear_buffer_callback();
            }
            transition.sink.stop();
            if transition.advanced {
                if let Some(sink) = self.current_sink.as_ref() {
                    sink.set_speed(1.0);
                }
                if let Some(runtime) = self.current_runtime.as_ref() {
                    runtime.reset_automix_transition();
                }
            }
        }
        complete
    }

    /// Resume playback
    pub fn resume(&mut self) {
        self.resume_with_fade(false);
    }

    /// Resume playback with optional fade in
    pub fn resume_with_fade(&mut self, fade_in: bool) {
        if let Some(sink) = &self.current_sink {
            let interrupted_pause_fade = self.pending_pause_fade;
            self.pending_pause_fade = false;
            let target_volume = self.get_sink_volume();

            if fade_in {
                // User volume remains on the Sink. Fade is owned exclusively
                // by the sample-driven DSP envelope so later volume mailbox
                // updates cannot bypass or cancel the ramp.
                sink.set_volume(target_volume);
                sink.play();
                if let Some(runtime) = self.current_runtime.as_ref() {
                    if interrupted_pause_fade {
                        runtime.fade_to(1.0, Self::FADE_DURATION);
                    } else {
                        runtime.fade_from_to(0.0, 1.0, Self::FADE_DURATION);
                    }
                }
            } else {
                if let Some(runtime) = self.current_runtime.as_ref() {
                    runtime.set_fade_volume(1.0);
                }
                sink.set_volume(target_volume);
                sink.play();
            }

            {
                let mut state = self.state.lock().unwrap();
                state.status = PlaybackStatus::Playing;
                state.paused_position = None;
            }
        }
    }

    pub fn pause_sink(&self) {
        if let Some(sink) = &self.current_sink {
            sink.pause();
        }
    }

    pub fn play_sink(&self) {
        if let Some(sink) = &self.current_sink {
            sink.play();
            if let Ok(mut state) = self.state.lock() {
                state.status = PlaybackStatus::Playing;
                state.paused_position = None;
            }
        }
    }

    /// Stop playback
    pub fn stop(&mut self) {
        self.pending_pause_fade = false;
        self.last_transition_group = None;
        if let Some(transition) = self.outgoing_transition.take() {
            if let Some(cancellation) = transition.reader_cancellation {
                cancellation.cancel();
            }
            if let Some(buffer) = transition.shared_buffer {
                buffer.clear_buffer_callback();
            }
            transition.sink.stop();
        }
        if let Some(cancellation) = self.current_reader_cancellation.take() {
            cancellation.cancel();
        }
        if let Some(sink) = self.current_sink.take() {
            sink.set_speed(1.0);
            sink.stop();
        }
        self.current_runtime = None;
        self.current_path = None;
        self.is_streaming = false;
        self.detached_seek_position = None;
        self.position_offset = Duration::ZERO;
        self.chain.activate_runtime(None);
        let mut state = self.state.lock().unwrap();
        state.status = PlaybackStatus::Stopped;
        state.current_track_gain = 1.0;
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        {
            let mut state = self.state.lock().unwrap();
            state.volume = volume;
        }

        if let Some(sink) = &self.current_sink {
            sink.set_volume(self.get_sink_volume());
        }
    }

    /// Seek to position
    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        self.pending_pause_fade = false;
        self.last_transition_group = None;
        if let Some(transition) = self.outgoing_transition.take() {
            if let Some(cancellation) = transition.reader_cancellation {
                cancellation.cancel();
            }
            if let Some(buffer) = transition.shared_buffer {
                buffer.clear_buffer_callback();
            }
            transition.sink.stop();
        }
        if let Some(runtime) = self.current_runtime.as_ref() {
            runtime.set_fade_volume(1.0);
            runtime.reset_automix_transition();
            runtime.reset_natural_end();
        }
        if let Some(sink) = &mut self.current_sink {
            sink.set_speed(1.0);
            match sink.try_seek(position) {
                Ok(_) => {
                    self.position_offset = Duration::ZERO;
                    let new_position = sink.get_pos();
                    let mut state = self.state.lock().unwrap();
                    if matches!(state.status, PlaybackStatus::Paused) {
                        state.paused_position = Some(new_position);
                    }
                    tracing::debug!("Seek to {:?} successful", position);
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!("Direct seek failed: {:?}, will try reload", e);
                }
            }
        } else {
            return Err("No audio loaded".to_string());
        }

        // Direct seek failed, try reloading the file
        let path = match self.current_path.clone() {
            Some(p) => p,
            None => {
                return Err("Seek failed: end of stream (streaming playback)".to_string());
            }
        };

        tracing::info!("Attempting reload workaround for seek to {:?}", position);
        let (volume, was_playing, track_gain) = {
            let state = self.state.lock().unwrap();
            (
                state.volume,
                state.status == PlaybackStatus::Playing,
                state.current_track_gain,
            )
        };

        if let Some(old_sink) = self.current_sink.take() {
            old_sink.stop();
        }

        let source = Self::decode_local_file(&path)?;
        let duration = source.total_duration();

        let runtime = self
            .current_runtime
            .clone()
            .unwrap_or_else(|| self.prepare_runtime(false));
        self.chain.activate_runtime(Some(&runtime));
        let processed = self.chain.apply(source, track_gain, runtime.clone());

        let new_sink = Sink::connect_new(self._stream.mixer());
        new_sink.append(processed);
        new_sink.set_volume(volume);

        let seek_failed = if let Err(seek_err) = new_sink.try_seek(position) {
            tracing::warn!("Seek after reload also failed: {:?}", seek_err);
            true
        } else {
            false
        };

        if !was_playing {
            new_sink.pause();
        }

        let new_position = new_sink.get_pos();

        {
            let mut state = self.state.lock().unwrap();
            state.duration = duration.unwrap_or(Duration::ZERO);
            state.status = if was_playing {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Paused
            };
            state.paused_position = if was_playing {
                None
            } else {
                Some(new_position)
            };
            state.current_track_gain = track_gain;
        }

        self.current_sink = Some(new_sink);
        self.current_runtime = Some(runtime);
        self.position_offset = Duration::ZERO;

        if seek_failed {
            Err("Seek not supported for this format".to_string())
        } else {
            Ok(())
        }
    }

    /// Transfer a streaming Sink to the bounded seek worker. No Rodio seek
    /// order is issued on the control actor.
    pub(crate) fn take_streaming_sink_for_seek(
        &mut self,
        position: Duration,
    ) -> Result<DetachedStreamingPlayback, String> {
        if !self.is_streaming {
            return Err("current source is not streaming".to_string());
        }
        self.pending_pause_fade = false;
        self.last_transition_group = None;
        if let Some(transition) = self.outgoing_transition.take() {
            if let Some(cancellation) = transition.reader_cancellation {
                cancellation.cancel();
            }
            if let Some(buffer) = transition.shared_buffer {
                buffer.clear_buffer_callback();
            }
            transition.sink.stop();
        }
        if let Some(runtime) = self.current_runtime.as_ref() {
            runtime.set_fade_volume(1.0);
            runtime.reset_automix_transition();
            runtime.reset_natural_end();
        }
        let sink = self
            .current_sink
            .take()
            .ok_or_else(|| "streaming seek is already pending".to_string())?;
        let (duration, track_gain, was_paused) = self
            .state
            .lock()
            .map(|state| {
                (
                    state.duration,
                    state.current_track_gain,
                    matches!(state.status, PlaybackStatus::Paused),
                )
            })
            .unwrap_or((Duration::ZERO, 1.0, false));
        sink.set_speed(1.0);
        self.detached_seek_position = Some(position);
        Ok(DetachedStreamingPlayback {
            sink,
            duration,
            cache_path: self.current_path.clone(),
            track_gain,
            was_paused,
        })
    }

    /// Wake the currently installed streaming decoder before its detached
    /// Sink is stopped or replaced.
    pub(crate) fn cancel_current_streaming_reader(&mut self) {
        if let Some(cancellation) = self.current_reader_cancellation.take() {
            cancellation.cancel();
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "preseek installation validates and commits one streaming source; remove when PreseekedSource owns all installation metadata"
    )]
    pub(crate) fn install_preseeked_streaming_source(
        &mut self,
        source: PreparedStreamingSource,
        reader_cancellation: StreamingReaderCancellation,
        duration: Duration,
        cache_path: Option<PathBuf>,
        position: Duration,
        track_gain: f32,
        paused: bool,
    ) -> Result<(), String> {
        self.stop();
        if !reader_cancellation.activate() {
            return Err(
                "streaming seek reader could not acquire retained-window demand".to_string(),
            );
        }
        self.chain.refresh_eq_coefficients();
        let runtime = self.prepare_runtime(false);
        self.chain.activate_runtime(Some(&runtime));
        let processed = self.chain.apply(source, track_gain, runtime.clone());
        let sink = Sink::connect_new(self._stream.mixer());
        sink.append(processed);
        sink.set_volume(self.get_sink_volume());
        if paused {
            sink.pause();
        }
        {
            let mut state = self.state.lock().unwrap();
            state.status = if paused {
                PlaybackStatus::Paused
            } else {
                PlaybackStatus::Playing
            };
            state.duration = duration;
            state.paused_position = paused.then_some(position);
            state.current_track_gain = track_gain;
        }
        self.current_sink = Some(sink);
        self.current_path = cache_path;
        self.current_runtime = Some(runtime);
        self.current_reader_cancellation = Some(reader_cancellation);
        self.is_streaming = true;
        self.detached_seek_position = None;
        self.position_offset = position;
        Ok(())
    }

    /// Get current playback info
    pub fn get_info(&self) -> PlaybackInfo {
        let state = self.state.lock().unwrap();

        let position = if let Some(sink) = &self.current_sink {
            if matches!(state.status, PlaybackStatus::Paused) {
                state.paused_position.unwrap_or_else(|| sink.get_pos())
            } else {
                self.position_offset.saturating_add(sink.get_pos())
            }
        } else {
            self.detached_seek_position.unwrap_or(Duration::ZERO)
        };

        // Don't change status based on sink.empty() - it's unreliable
        // The is_finished() method handles proper finish detection
        let status = state.status.clone();

        PlaybackInfo {
            status,
            position,
            duration: state.duration,
            volume: state.volume,
        }
    }

    /// Check if playback finished
    pub fn is_finished(&self) -> bool {
        if self
            .current_runtime
            .as_ref()
            .is_some_and(PlaybackProcessingRuntime::natural_end_reached)
        {
            return true;
        }
        if let Some(sink) = &self.current_sink {
            let state = self.state.lock().unwrap();
            let position = self.position_offset.saturating_add(sink.get_pos());
            let duration = state.duration;

            // Don't consider finished if we just started or if paused/stopped
            if state.status != PlaybackStatus::Playing {
                return false;
            }

            // Need valid duration to determine if finished
            if duration.as_secs_f32() <= 0.0 {
                return false;
            }

            // Don't consider finished if position is very early
            if position.as_secs_f32() < 5.0 {
                return false;
            }

            // Check if we've reached near the end of the track
            // Use a small tolerance (0.5s) to account for timing variations
            if position.as_secs_f32() >= duration.as_secs_f32() - 0.5 {
                tracing::debug!(
                    "is_finished: reached end at {:.1}s / {:.1}s",
                    position.as_secs_f32(),
                    duration.as_secs_f32()
                );
                return true;
            }

            // Also check if sink is empty AND we're very close to the end (95%)
            // Also check if sink is empty and we're past 95% of duration
            if sink.empty() && position.as_secs_f32() > duration.as_secs_f32() * 0.95 {
                tracing::debug!(
                    "is_finished: sink empty near end at {:.1}s / {:.1}s",
                    position.as_secs_f32(),
                    duration.as_secs_f32()
                );
                return true;
            }

            false
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::automix::{AdvancedAutomation, ScheduleGroup};

    #[test]
    fn streaming_preparation_rejects_missing_validated_length_without_waiting() {
        let shared = crate::audio::streaming::SharedBuffer::new(0);
        let started = std::time::Instant::now();
        let error = match prepare_streaming_source(StreamingBuffer::new(shared)) {
            Ok(_) => panic!("source without validated length must be rejected"),
            Err(error) => error,
        };

        assert!(error.starts_with("UnsupportedStreaming:"));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn scheduler_late_fallback_does_not_seek_the_automix_entry() {
        let group = ScheduleGroup(9);
        let transition = TransitionDirective {
            kind: TransitionKind::Automix,
            duration: Duration::from_secs(3),
            entry: Duration::from_secs(12),
            expected_exit: Some(Duration::from_secs(20)),
            group,
            automation: AdvancedAutomation {
                rate: 1.02,
                gain_db: 3.0,
                bass_swap: true,
            },
        };
        let mut seek_calls = 0;
        let resolved =
            prepare_transition_for_handoff(transition, Duration::from_millis(21_501), |_| {
                seek_calls += 1;
                true
            });

        assert_eq!(seek_calls, 0);
        assert_eq!(resolved, TransitionDirective::baseline_natural(group));
    }

    #[test]
    fn entry_seek_failure_falls_back_before_advanced_automation() {
        let group = ScheduleGroup(10);
        let transition = TransitionDirective {
            kind: TransitionKind::Automix,
            duration: Duration::from_secs(3),
            entry: Duration::from_secs(12),
            expected_exit: Some(Duration::from_secs(20)),
            group,
            automation: AdvancedAutomation {
                rate: 1.02,
                gain_db: 3.0,
                bass_swap: true,
            },
        };
        let resolved =
            prepare_transition_for_handoff(transition, Duration::from_secs(20), |_| false);

        assert_eq!(resolved, TransitionDirective::baseline_natural(group));
    }
}

// ============ Audio Device Discovery ============

/// Audio device info with internal name and display name
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

/// Get list of available audio output devices
pub fn get_audio_devices() -> Vec<AudioDevice> {
    AUDIO_DEVICES_CACHE
        .get_or_init(|| {
            let devices = get_cpal_devices();
            if !devices.is_empty() {
                return devices;
            }

            let pa_devices = get_pulseaudio_devices();
            if !pa_devices.is_empty() {
                return pa_devices;
            }

            get_alsa_devices()
        })
        .clone()
}

fn get_pulseaudio_devices() -> Vec<AudioDevice> {
    let mut devices = Vec::new();

    if let Ok(output) = std::process::Command::new("pactl")
        .args(["list", "sinks"])
        .output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.starts_with("Name:") {
                current_name = line.trim_start_matches("Name:").trim().to_string();
            } else if line.starts_with("Description:") && !current_name.is_empty() {
                let description = line.trim_start_matches("Description:").trim().to_string();
                devices.push(AudioDevice {
                    name: current_name.clone(),
                    description,
                });
                current_name.clear();
            }
        }
    }

    devices
}

fn get_alsa_devices() -> Vec<AudioDevice> {
    let mut devices = Vec::new();

    if let Ok(output) = std::process::Command::new("aplay").args(["-l"]).output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            if line.starts_with("card ")
                && let Some((card_info, device_info)) = line.split_once(", device ")
            {
                let card_num = card_info
                    .trim_start_matches("card ")
                    .split(':')
                    .next()
                    .unwrap_or("0")
                    .trim();

                let device_num = device_info.split(':').next().unwrap_or("0").trim();

                let description = if let Some(start) = line.find('[') {
                    if let Some(end) = line.rfind(']') {
                        line[start + 1..end].to_string()
                    } else {
                        line.to_string()
                    }
                } else {
                    line.to_string()
                };

                let name = format!("hw:{},{}", card_num, device_num);

                devices.push(AudioDevice { name, description });
            }
        }
    }

    if devices.is_empty() {
        devices = get_cpal_devices();
    }

    devices
}

fn get_cpal_devices() -> Vec<AudioDevice> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    let host = rodio::cpal::default_host();
    let mut devices = Vec::new();

    if let Ok(output_devices) = host.output_devices() {
        for device in output_devices {
            if let Ok(description) = device.description() {
                let name = description.name().to_string();
                let name_lower = name.to_lowercase();

                if name_lower.contains("jack")
                    || name_lower.contains("oss")
                    || name_lower.contains("/dev/dsp")
                    || name == "default"
                    || name == "pipewire"
                    || name == "pulse"
                {
                    continue;
                }

                if device.default_output_config().is_ok() {
                    devices.push(AudioDevice {
                        name: name.clone(),
                        description: name,
                    });
                }
            }
        }
    }

    devices
}

/// Classified playback error types for UI display
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackError {
    FileNotFound(String),
    UnsupportedStreaming(String),
    UnhealthyPreload(String),
    UnsupportedFormat(String),
    NetworkError(String),
    IoError(String),
    DecodeError(String),
}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaybackError::FileNotFound(m) => write!(f, "File not found: {}", m),
            PlaybackError::UnsupportedStreaming(m) => write!(f, "Unsupported streaming: {}", m),
            PlaybackError::UnhealthyPreload(m) => write!(f, "Unhealthy preload: {}", m),
            PlaybackError::UnsupportedFormat(m) => write!(f, "Unsupported format: {}", m),
            PlaybackError::NetworkError(m) => write!(f, "Network error: {}", m),
            PlaybackError::IoError(m) => write!(f, "IO error: {}", m),
            PlaybackError::DecodeError(m) => write!(f, "Decode error: {}", m),
        }
    }
}

/// Classify an error message from rodio/IO into a PlaybackError.
pub fn classify_playback_error(error_msg: &str) -> PlaybackError {
    let lower = error_msg.to_lowercase();
    if lower.contains("not found") || lower.contains("no such file") {
        PlaybackError::FileNotFound(error_msg.to_string())
    } else if lower.contains("unsupportedstreaming")
        || lower.contains("unsupported streaming")
        || lower.contains("content-range")
        || lower.contains("expected http 206")
    {
        PlaybackError::UnsupportedStreaming(error_msg.to_string())
    } else if lower.contains("network:")
        || lower.contains("http request")
        || lower.contains("connection")
        || lower.contains("timed out")
    {
        PlaybackError::NetworkError(error_msg.to_string())
    } else if lower.contains("unsupportedformat")
        || lower.contains("unsupported format")
        || lower.contains("unsupported codec")
        || lower.contains("unrecognized format")
    {
        PlaybackError::UnsupportedFormat(error_msg.to_string())
    } else if lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("i/o")
        || lower.contains("io error")
        || lower.contains("cache write")
    {
        PlaybackError::IoError(error_msg.to_string())
    } else {
        PlaybackError::DecodeError(error_msg.to_string())
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn playback_errors_keep_streaming_network_format_decode_and_io_distinct() {
        assert!(matches!(
            classify_playback_error("UnsupportedStreaming: expected HTTP 206"),
            PlaybackError::UnsupportedStreaming(_)
        ));
        assert!(matches!(
            classify_playback_error("Network: Range request timed out"),
            PlaybackError::NetworkError(_)
        ));
        assert!(matches!(
            classify_playback_error("UnsupportedFormat: unknown audio prefix"),
            PlaybackError::UnsupportedFormat(_)
        ));
        assert!(matches!(
            classify_playback_error("Failed to decode packet"),
            PlaybackError::DecodeError(_)
        ));
        assert!(matches!(
            classify_playback_error("I/O: cache write failed"),
            PlaybackError::IoError(_)
        ));
    }
}
