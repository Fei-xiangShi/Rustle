//! Audio handle for non-blocking audio control from UI thread
//!
//! `AudioHandle` provides a non-blocking interface to control audio playback.
//! All methods send commands to the audio thread and return immediately.
//! State is read from `SharedPlaybackState` without blocking.

use std::path::PathBuf;
use std::time::Duration;

use super::PlaybackInfo;
use super::events::{AudioCommand, AudioCommandSender, LatestControlMailbox, SharedPlaybackState};
use super::identity::{PlaybackContext, PlaybackGenerationController, PreloadIdentity};
use super::player::{PlaybackError, PlaybackResult};
use super::streaming::StreamingBuffer;

fn command_queue_error<T>(error: tokio::sync::mpsc::error::TrySendError<T>) -> PlaybackError {
    match error {
        tokio::sync::mpsc::error::TrySendError::Full(_) => {
            PlaybackError::ControlUnavailable("audio command queue is full".to_string())
        }
        tokio::sync::mpsc::error::TrySendError::Closed(_) => {
            PlaybackError::ControlUnavailable("audio command queue is closed".to_string())
        }
    }
}

/// Handle for controlling audio from UI thread
///
/// All methods are non-blocking - they send commands to the audio thread
/// and return immediately. Results are communicated via `AudioEvent`.
///
/// State queries (get_info, is_playing, etc.) read from shared state
/// without blocking, even if the audio thread is busy.
#[derive(Clone)]
pub struct AudioHandle {
    command_tx: AudioCommandSender,
    latest_controls: LatestControlMailbox,
    state: SharedPlaybackState,
    generation: PlaybackGenerationController,
}

impl std::fmt::Debug for AudioHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioHandle")
            .field("state", &self.state)
            .finish()
    }
}

impl AudioHandle {
    fn send_critical<F>(&self, build: F) -> PlaybackResult<()>
    where
        F: FnOnce() -> AudioCommand,
    {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;
        permit.send(build());
        Ok(())
    }

    fn next_playback_request_id(&self) -> u64 {
        self.generation.next_request_id()
    }

    fn send_with_context<F>(&self, context: &PlaybackContext, build: F) -> PlaybackResult<u64>
    where
        F: FnOnce(PlaybackContext, u64) -> AudioCommand,
    {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;

        if !self.generation.accepts(context) {
            return Err(PlaybackError::Cancelled(
                "playback context is stale or cancelled".to_string(),
            ));
        }

        let request_id = self.next_playback_request_id();
        permit.send(build(context.clone(), request_id));
        Ok(request_id)
    }

    pub fn current_context(&self) -> Option<super::identity::PlaybackContext> {
        self.generation.active_context()
    }

    pub fn accepts_context(&self, context: &super::identity::PlaybackContext) -> bool {
        self.generation.accepts(context)
    }

    pub(crate) fn generation_controller(&self) -> PlaybackGenerationController {
        self.generation.clone()
    }

    pub fn accepts_seek(
        &self,
        context: &super::identity::PlaybackContext,
        nonce: super::identity::SeekNonce,
    ) -> bool {
        self.generation.accepts_seek(context, nonce)
    }

    /// Create a new audio handle
    pub fn new(
        command_tx: AudioCommandSender,
        latest_controls: LatestControlMailbox,
        state: SharedPlaybackState,
    ) -> Self {
        Self {
            command_tx,
            latest_controls,
            state,
            generation: PlaybackGenerationController::new(),
        }
    }

    // ============ Playback Control ============

    /// Start an async source-resolution request as a new playback generation.
    ///
    /// The stop command and the returned context share the same generation, so
    /// a later resolved source can be enqueued without cancelling its download.
    pub fn begin_playback_resolution(&self) -> PlaybackResult<PlaybackContext> {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;
        let context = self.generation.activate_generation();
        permit.send(AudioCommand::Stop {
            context: context.clone(),
        });
        Ok(context)
    }

    /// Play a local file with optional fade in.
    pub fn play_with_fade(
        &self,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        let request_id = self.next_playback_request_id();
        self.send_critical(|| AudioCommand::Play {
            context: self.generation.activate_generation(),
            request_id,
            path,
            fade_in,
            track_gain,
        })?;
        Ok(request_id)
    }

    pub fn play_with_fade_in_context(
        &self,
        context: &PlaybackContext,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        self.send_with_context(context, |context, request_id| AudioCommand::Play {
            context,
            request_id,
            path,
            fade_in,
            track_gain,
        })
    }

    pub fn load_paused(
        &self,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        let request_id = self.next_playback_request_id();
        self.send_critical(|| AudioCommand::LoadPaused {
            context: self.generation.activate_generation(),
            request_id,
            path,
            position,
            track_gain,
        })?;
        Ok(request_id)
    }

    pub fn load_paused_in_context(
        &self,
        context: &PlaybackContext,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        self.send_with_context(context, |context, request_id| AudioCommand::LoadPaused {
            context,
            request_id,
            path,
            position,
            track_gain,
        })
    }

    pub fn load_streaming_paused_in_context(
        &self,
        context: &PlaybackContext,
        buffer: StreamingBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        position: Duration,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        self.send_with_context(context, |context, request_id| {
            AudioCommand::LoadPausedStreaming {
                context,
                request_id,
                buffer,
                duration,
                cache_path,
                position,
                track_gain,
            }
        })
    }

    pub fn play_from_position_with_fade(
        &self,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        let request_id = self.next_playback_request_id();
        self.send_critical(|| AudioCommand::PlayAt {
            context: self.generation.activate_generation(),
            request_id,
            path,
            position,
            fade_in,
            track_gain,
        })?;
        Ok(request_id)
    }

    pub fn play_from_position_with_fade_in_context(
        &self,
        context: &PlaybackContext,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        self.send_with_context(context, |context, request_id| AudioCommand::PlayAt {
            context,
            request_id,
            path,
            position,
            fade_in,
            track_gain,
        })
    }

    pub fn play_streaming(
        &self,
        buffer: StreamingBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        let request_id = self.next_playback_request_id();
        self.send_critical(|| AudioCommand::PlayStreaming {
            context: self.generation.activate_generation(),
            request_id,
            buffer,
            duration,
            cache_path,
            fade_in,
            track_gain,
        })?;
        Ok(request_id)
    }

    pub fn play_streaming_in_context(
        &self,
        context: &PlaybackContext,
        buffer: StreamingBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    ) -> PlaybackResult<u64> {
        self.send_with_context(context, |context, request_id| AudioCommand::PlayStreaming {
            context,
            request_id,
            buffer,
            duration,
            cache_path,
            fade_in,
            track_gain,
        })
    }

    /// Pause playback with optional fade out
    ///
    /// Sends Pause command to audio thread.
    /// Note: Audio Thread will pause Sink before data runs out, so no interrupt needed.
    pub fn pause_with_fade(&self, fade_out: bool) -> PlaybackResult<()> {
        let context = self.generation.active_context().ok_or_else(|| {
            PlaybackError::SourceUnavailable(
                "pause requires active playback generation".to_string(),
            )
        })?;
        self.send_critical(|| AudioCommand::Pause { context, fade_out })
    }

    /// Resume playback with optional fade in
    pub fn resume_with_fade(&self, fade_in: bool) -> PlaybackResult<()> {
        let context = self.generation.active_context().ok_or_else(|| {
            PlaybackError::SourceUnavailable(
                "resume requires active playback generation".to_string(),
            )
        })?;
        self.send_critical(|| AudioCommand::Resume { context, fade_in })
    }

    /// Stop playback
    ///
    /// Sends Stop command to audio thread.
    pub fn stop(&self) -> PlaybackResult<()> {
        self.begin_playback_resolution().map(|_| ())
    }

    /// Seek to position
    ///
    /// Sends Seek command and returns immediately.
    /// Listen for `AudioEvent::SeekComplete` or `AudioEvent::SeekFailed`.
    ///
    /// The shared state position is updated immediately to the target position,
    /// so UI shows the target position while seek is in progress (prevents
    /// "bounce back" effect during buffering).
    pub fn seek(&self, position: Duration) -> PlaybackResult<()> {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;
        let (context, nonce) = self.generation.seek_context().ok_or_else(|| {
            PlaybackError::SourceUnavailable("seek requires active playback generation".to_string())
        })?;
        // Update position immediately so UI shows target position during seek.
        // This happens only after capacity and identity have both been secured.
        self.state.set_position(position);
        permit.send(AudioCommand::Seek {
            context,
            nonce,
            position,
        });
        Ok(())
    }

    /// Set volume
    pub fn set_volume(&self, volume: f32) {
        self.latest_controls.publish_volume(volume);
    }

    /// Tick handler - checks buffer status and syncs position
    pub fn tick(&self) {
        self.latest_controls.publish_tick();
    }
    // ============ Preloading ============

    /// Reserve an immutable identity before starting a preload operation.
    pub fn reserve_preload_identity(&self) -> Option<PreloadIdentity> {
        self.generation.reserve_preload_identity()
    }

    /// Reserve a new immutable sink identity from an accepted preload identity.
    pub fn reserve_preload_handoff(&self, parent: &PreloadIdentity) -> Option<PreloadIdentity> {
        self.generation.reserve_preload_handoff(parent)
    }

    /// Request creation of a preload sink for a local file using its origin identity.
    pub fn create_preload_sink(
        &self,
        identity: PreloadIdentity,
        path: PathBuf,
        track_gain: f32,
    ) -> PlaybackResult<()> {
        self.send_critical(|| AudioCommand::CreatePreloadSink {
            identity,
            path,
            track_gain,
        })
    }

    /// Request creation of a preload sink for streaming using its origin identity.
    pub fn create_preload_sink_streaming(
        &self,
        identity: PreloadIdentity,
        buffer: StreamingBuffer,
        duration: Duration,
        track_gain: f32,
    ) -> PlaybackResult<()> {
        self.send_critical(|| AudioCommand::CreatePreloadSinkStreaming {
            identity,
            buffer,
            duration,
            track_gain,
        })
    }

    /// Play a preloaded sink by immutable identity.
    ///
    /// The sink must have been created via `create_preload_sink` or
    /// `create_preload_sink_streaming` and received `AudioEvent::PreloadReady`.
    pub fn play_preloaded(
        &self,
        identity: PreloadIdentity,
        fade_in: bool,
        transition: Option<super::automix::TransitionDirective>,
    ) -> PlaybackResult<u64> {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;
        let playback_request_id = self.next_playback_request_id();
        let transition = transition.unwrap_or_else(|| {
            super::automix::TransitionDirective::manual(super::automix::ScheduleGroup(
                playback_request_id,
            ))
        });
        let context = self
            .generation
            .active_context()
            .filter(|_| self.generation.accepts_preload(&identity))
            .ok_or_else(|| {
                PlaybackError::Cancelled(
                    "preloaded audio identity is stale or cancelled".to_string(),
                )
            })?;
        permit.send(AudioCommand::PlayPreloaded {
            context,
            identity,
            playback_request_id,
            fade_in,
            transition,
        });
        Ok(playback_request_id)
    }

    /// Schedule a ready preload against the current sink's audio clock.
    ///
    /// Unlike immediate promotion, this deliberately keeps the outgoing
    /// playback generation active until the audio thread reaches `trigger_at`.
    pub fn schedule_preloaded_transition(
        &self,
        identity: PreloadIdentity,
        trigger_at: Duration,
        fade_in: bool,
        transition: super::automix::TransitionDirective,
    ) -> PlaybackResult<u64> {
        let permit = self.command_tx.try_reserve().map_err(command_queue_error)?;
        let owner = self.generation.active_context().ok_or_else(|| {
            PlaybackError::SourceUnavailable(
                "scheduled transition requires active playback".to_string(),
            )
        })?;
        if !self.generation.accepts_preload(&identity) {
            return Err(PlaybackError::Cancelled(
                "preloaded audio identity is stale or cancelled".to_string(),
            ));
        }
        let playback_request_id = self.next_playback_request_id();
        permit.send(AudioCommand::SchedulePreloadedTransition {
            owner,
            identity,
            playback_request_id,
            trigger_at,
            fade_in,
            transition,
        });
        Ok(playback_request_id)
    }

    pub fn cancel_scheduled_transition(&self) -> PlaybackResult<()> {
        let owner = self.generation.active_context().ok_or_else(|| {
            PlaybackError::SourceUnavailable(
                "scheduled transition cancellation requires active playback".to_string(),
            )
        })?;
        self.send_critical(|| AudioCommand::CancelScheduledTransition { owner })
    }

    /// Release a preloaded sink by request_id without playing it
    pub fn release_preload(&self, identity: PreloadIdentity) -> PlaybackResult<()> {
        self.send_critical(|| AudioCommand::ReleasePreload { identity })
    }

    pub(crate) fn shutdown(&self) {
        self.generation.cancel_active();
        let context = self
            .generation
            .active_context()
            .unwrap_or_else(|| self.generation.activate_generation());
        self.latest_controls.publish_shutdown(context);
    }

    // ============ Device Control ============

    /// Switch audio output device
    ///
    /// Listen for the typed output-recovery lifecycle events.
    pub fn switch_device(&self, device_name: Option<String>) -> PlaybackResult<()> {
        self.send_critical(|| AudioCommand::SwitchDevice { device_name })
    }

    /// Retry the most recently failed output reconstruction.
    pub fn retry_output(&self) -> PlaybackResult<()> {
        self.send_critical(|| AudioCommand::RetryOutput)
    }

    // ============ State Queries (non-blocking reads) ============

    /// Get current playback info
    ///
    /// Reads from shared state, does not communicate with audio thread.
    pub fn get_info(&self) -> PlaybackInfo {
        self.state.get_info()
    }

    /// Get display position
    ///
    /// Returns target position during pending seek, otherwise actual position.
    /// Use this for UI display to show immediate feedback during seek.
    pub fn display_position(&self) -> Duration {
        self.state.display_position()
    }

    /// Check if player has no loaded audio
    pub fn is_empty(&self) -> bool {
        self.state.current_path().is_none() && self.state.is_stopped()
    }

    /// Get contiguous disk-cache progress for streaming playback.
    ///
    /// Returns None for local files, Some(0.0-1.0) for streaming.
    /// This is the single source of truth for the UI secondary track.
    pub fn cache_progress(&self) -> Option<f32> {
        self.state.cache_progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_capacity_error_does_not_activate_a_generation() {
        let (tx, _rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx.clone(), latest, SharedPlaybackState::new());
        while tx.try_send(AudioCommand::LatestMailboxWake).is_ok() {}

        let result = handle.play_with_fade(PathBuf::from("missing.mp3"), false, 1.0);
        assert!(matches!(
            result.unwrap_err(),
            PlaybackError::ControlUnavailable(_)
        ));
        assert!(handle.current_context().is_none());
    }

    #[test]
    fn resolution_context_stops_then_starts_without_advancing_generation() {
        let (tx, mut rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());

        let context = handle.begin_playback_resolution().unwrap();
        let request_id = handle
            .play_with_fade_in_context(&context, PathBuf::from("cached.flac"), false, 1.0)
            .unwrap();

        assert_eq!(handle.current_context(), Some(context.clone()));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AudioCommand::Stop { context: stopped } if stopped == context
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AudioCommand::Play {
                context: started,
                request_id: queued_request_id,
                ..
            } if started == context && queued_request_id == request_id
        ));
    }

    #[test]
    fn stale_resolution_context_cannot_start_playback() {
        let (tx, mut rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());

        let stale = handle.begin_playback_resolution().unwrap();
        let current = handle.begin_playback_resolution().unwrap();
        let error = handle
            .play_with_fade_in_context(&stale, PathBuf::from("stale.flac"), false, 1.0)
            .unwrap_err();

        assert!(matches!(error, PlaybackError::Cancelled(_)));
        assert_eq!(handle.current_context(), Some(current));
        assert!(matches!(rx.try_recv().unwrap(), AudioCommand::Stop { .. }));
        assert!(matches!(rx.try_recv().unwrap(), AudioCommand::Stop { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn full_queue_cannot_begin_or_cancel_playback_resolution() {
        let (tx, _rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx.clone(), latest, SharedPlaybackState::new());
        let active = handle.begin_playback_resolution().unwrap();
        while tx.try_send(AudioCommand::LatestMailboxWake).is_ok() {}

        assert!(matches!(
            handle.begin_playback_resolution().unwrap_err(),
            PlaybackError::ControlUnavailable(_)
        ));
        assert_eq!(handle.current_context(), Some(active.clone()));
        assert!(!active.cancellation.is_cancelled());
    }

    #[test]
    fn controls_without_active_playback_return_errors_instead_of_panicking() {
        let (tx, _rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());

        assert!(matches!(
            handle.pause_with_fade(false).unwrap_err(),
            PlaybackError::SourceUnavailable(_)
        ));
        assert!(matches!(
            handle.resume_with_fade(false).unwrap_err(),
            PlaybackError::SourceUnavailable(_)
        ));
        assert!(matches!(
            handle.seek(Duration::ZERO).unwrap_err(),
            PlaybackError::SourceUnavailable(_)
        ));
    }

    #[test]
    fn stale_preload_promotion_does_not_replace_the_active_generation() {
        let (tx, _rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());
        handle.generation.activate_generation();
        let stale = handle.generation.reserve_preload_identity().unwrap();
        let current = handle.generation.activate_generation();

        let error = handle.play_preloaded(stale, false, None).unwrap_err();

        assert!(matches!(error, PlaybackError::Cancelled(_)));
        assert_eq!(handle.current_context(), Some(current));
    }

    #[test]
    fn immediate_preload_command_defers_generation_promotion_to_audio_thread() {
        let (tx, mut rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());
        let outgoing = handle.generation.activate_generation();
        let identity = handle.generation.reserve_preload_identity().unwrap();

        handle
            .play_preloaded(identity.clone(), false, None)
            .unwrap();

        assert_eq!(handle.current_context(), Some(outgoing.clone()));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AudioCommand::PlayPreloaded {
                context,
                identity: queued_identity,
                ..
            } if context == outgoing && queued_identity == identity
        ));
    }

    #[test]
    fn full_queue_does_not_advance_seek_nonce_or_position() {
        let (tx, mut rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let state = SharedPlaybackState::new();
        let handle = AudioHandle::new(tx.clone(), latest, state.clone());
        handle.generation.activate_generation();
        while tx.try_send(AudioCommand::LatestMailboxWake).is_ok() {}

        assert!(matches!(
            handle.seek(Duration::from_secs(12)).unwrap_err(),
            PlaybackError::ControlUnavailable(_)
        ));
        assert_eq!(state.get_info().position, Duration::ZERO);
        rx.try_recv().unwrap();
        let (_, nonce) = handle.generation.seek_context().unwrap();
        assert_eq!(nonce, super::super::identity::SeekNonce(1));
    }

    #[test]
    fn scheduling_preload_keeps_outgoing_generation_active() {
        let (tx, mut rx) = super::super::events::audio_command_channel();
        let latest = LatestControlMailbox::new(tx.clone());
        let handle = AudioHandle::new(tx, latest, SharedPlaybackState::new());
        let outgoing = handle.generation.activate_generation();
        let identity = handle.generation.reserve_preload_identity().unwrap();

        handle
            .schedule_preloaded_transition(
                identity,
                Duration::from_secs(20),
                false,
                super::super::automix::TransitionDirective::baseline_natural(
                    super::super::automix::ScheduleGroup(9),
                ),
            )
            .unwrap();

        assert_eq!(handle.current_context(), Some(outgoing));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AudioCommand::SchedulePreloadedTransition { .. }
        ));
    }
}
