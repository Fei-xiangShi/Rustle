//! Audio thread communication types
//!
//! This module provides commands and events for the audio thread architecture:
//! - `AudioCommand` - Commands sent from UI thread to audio thread
//! - `AudioEvent` - Events sent from audio thread to UI thread
//! - `SharedPlaybackState` - Thread-safe state for non-blocking UI reads
//!
//! For streaming download events, see `crate::audio::streaming::StreamingEvent`.
//!
//! ## Architecture
//! ```text
//! UI Thread (AudioHandle) --[AudioCommand]--> Audio Thread (AudioPlayer)
//! UI Thread              <--[AudioEvent]---- Audio Thread
//! UI Thread              <--[SharedState]--- Audio Thread (non-blocking reads)
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::PlaybackStatus;
use super::identity::{PlaybackContext, PreloadIdentity, SeekNonce};
use super::player::PlaybackError;
use super::streaming::StreamingBuffer;

// ============ Commands (UI -> Audio Thread) ============

/// Commands sent from UI thread to audio thread
///
/// All commands are processed asynchronously - the UI thread sends and returns immediately.
/// Results are communicated back via `AudioEvent`.
pub enum AudioCommand {
    /// Play a local file
    Play {
        context: PlaybackContext,
        request_id: u64,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    },
    /// Load a local file into paused state at a target position
    LoadPaused {
        context: PlaybackContext,
        request_id: u64,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    },
    /// Load a streaming source into paused state at a target position
    LoadPausedStreaming {
        context: PlaybackContext,
        request_id: u64,
        buffer: StreamingBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        position: Duration,
        track_gain: f32,
    },
    /// Play a local file from a target position
    PlayAt {
        context: PlaybackContext,
        request_id: u64,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    },
    /// Play from streaming buffer (for NCM songs)
    PlayStreaming {
        context: PlaybackContext,
        request_id: u64,
        buffer: StreamingBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    },
    Pause {
        context: PlaybackContext,
        fade_out: bool,
    },
    /// Resume playback
    Resume {
        context: PlaybackContext,
        fade_in: bool,
    },
    /// Stop playback
    Stop { context: PlaybackContext },
    /// Seek to position
    Seek {
        context: PlaybackContext,
        nonce: SeekNonce,
        position: Duration,
    },
    /// Create preload sink for a local file (async, returns via PreloadReady event)
    CreatePreloadSink {
        identity: PreloadIdentity,
        path: PathBuf,
        track_gain: f32,
    },
    /// Create preload sink for streaming (async, returns via PreloadReady event)
    CreatePreloadSinkStreaming {
        identity: PreloadIdentity,
        buffer: StreamingBuffer,
        duration: Duration,
        track_gain: f32,
    },
    /// Play a preloaded sink by immutable identity.
    /// `context` is the outgoing owner; the audio thread promotes a new
    /// generation only after validating the preload's current health.
    PlayPreloaded {
        context: PlaybackContext,
        identity: PreloadIdentity,
        playback_request_id: u64,
        fade_in: bool,
        transition: super::automix::TransitionDirective,
    },
    /// Schedule an already-ready preload against the outgoing sink's audio clock.
    /// The playback generation is promoted only when the deadline is reached.
    SchedulePreloadedTransition {
        owner: PlaybackContext,
        identity: PreloadIdentity,
        playback_request_id: u64,
        trigger_at: Duration,
        fade_in: bool,
        transition: super::automix::TransitionDirective,
    },
    /// Cancel the scheduled transition owned by this outgoing playback context.
    CancelScheduledTransition { owner: PlaybackContext },
    /// Release a preloaded sink by immutable identity without playing it
    ReleasePreload { identity: PreloadIdentity },
    /// Switch audio output device
    SwitchDevice { device_name: Option<String> },
    /// Wake the audio thread to drain latest-value control mailboxes.
    LatestMailboxWake,
    /// Buffer data available wake-up marker. The latest payload is held in
    /// `BufferDataMailbox`, keeping high-frequency progress out of payload FIFO.
    BufferDataAvailable,
}

impl std::fmt::Debug for AudioCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Play { .. } => f.debug_struct("Play").finish_non_exhaustive(),
            Self::LoadPaused { .. } => f.debug_struct("LoadPaused").finish_non_exhaustive(),
            Self::LoadPausedStreaming {
                request_id,
                duration,
                cache_path,
                position,
                track_gain,
                ..
            } => f
                .debug_struct("LoadPausedStreaming")
                .field("request_id", request_id)
                .field("duration", duration)
                .field("cache_path", cache_path)
                .field("position", position)
                .field("track_gain", track_gain)
                .finish_non_exhaustive(),
            Self::PlayAt {
                request_id,
                path,
                position,
                fade_in,
                track_gain,
                ..
            } => f
                .debug_struct("PlayAt")
                .field("request_id", request_id)
                .field("path", path)
                .field("position", position)
                .field("fade_in", fade_in)
                .field("track_gain", track_gain)
                .finish(),
            Self::PlayStreaming {
                request_id,
                duration,
                cache_path,
                fade_in,
                track_gain,
                ..
            } => f
                .debug_struct("PlayStreaming")
                .field("request_id", request_id)
                .field("duration", duration)
                .field("cache_path", cache_path)
                .field("fade_in", fade_in)
                .field("track_gain", track_gain)
                .finish_non_exhaustive(),
            Self::Pause { fade_out, .. } => {
                f.debug_struct("Pause").field("fade_out", fade_out).finish()
            }
            Self::Resume { fade_in, .. } => {
                f.debug_struct("Resume").field("fade_in", fade_in).finish()
            }
            Self::Stop { .. } => write!(f, "Stop"),
            Self::Seek { position, .. } => {
                f.debug_struct("Seek").field("position", position).finish()
            }
            Self::CreatePreloadSink {
                identity,
                path,
                track_gain,
            } => f
                .debug_struct("CreatePreloadSink")
                .field("identity", identity)
                .field("path", path)
                .field("track_gain", track_gain)
                .finish(),
            Self::CreatePreloadSinkStreaming {
                identity,
                duration,
                track_gain,
                ..
            } => f
                .debug_struct("CreatePreloadSinkStreaming")
                .field("identity", identity)
                .field("duration", duration)
                .field("track_gain", track_gain)
                .finish_non_exhaustive(),
            Self::PlayPreloaded { .. } => f.debug_struct("PlayPreloaded").finish_non_exhaustive(),
            Self::SchedulePreloadedTransition {
                identity,
                trigger_at,
                transition,
                ..
            } => f
                .debug_struct("SchedulePreloadedTransition")
                .field("identity", identity)
                .field("trigger_at", trigger_at)
                .field("group", &transition.group)
                .finish_non_exhaustive(),
            Self::CancelScheduledTransition { .. } => f
                .debug_struct("CancelScheduledTransition")
                .finish_non_exhaustive(),
            Self::ReleasePreload { identity } => f
                .debug_struct("ReleasePreload")
                .field("identity", identity)
                .finish(),
            Self::SwitchDevice { device_name } => f
                .debug_struct("SwitchDevice")
                .field("device_name", device_name)
                .finish(),
            Self::LatestMailboxWake => write!(f, "LatestMailboxWake"),
            Self::BufferDataAvailable => f.debug_struct("BufferDataAvailable").finish(),
        }
    }
}

// ============ Events (Audio Thread -> UI) ============

/// Events emitted by the audio thread
///
/// These events notify the UI of state changes and operation results.
/// The UI should handle these in its message loop.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Playback started for a track
    Started {
        context: PlaybackContext,
        request_id: u64,
        path: Option<PathBuf>,
    },
    /// Playback paused
    Paused {
        context: PlaybackContext,
        request_id: Option<u64>,
        position: Duration,
    },
    /// Playback resumed
    Resumed {
        context: PlaybackContext,
    },
    /// Playback stopped
    Stopped {
        context: PlaybackContext,
    },
    /// Seek completed successfully
    SeekComplete {
        context: PlaybackContext,
        nonce: SeekNonce,
        position: Duration,
    },
    /// Seek failed
    SeekFailed {
        context: PlaybackContext,
        nonce: SeekNonce,
        error: PlaybackError,
    },
    /// Seek started
    SeekStarted {
        context: PlaybackContext,
        nonce: SeekNonce,
        target_position: Duration,
    },
    /// State changed
    StateChanged {
        context: PlaybackContext,
        old_status: PlaybackStatus,
        new_status: PlaybackStatus,
    },
    /// Contiguous disk-cache progress update.
    CacheProgress {
        context: PlaybackContext,
        cached: u64,
        total: u64,
        progress: f32,
    },
    /// Entered buffering state
    BufferingStarted {
        context: PlaybackContext,
        position: Duration,
    },
    /// Buffering ended, playback resumed
    BufferingEnded {
        context: PlaybackContext,
    },
    /// Preload sink ready
    PreloadReady {
        identity: PreloadIdentity,
        duration: Duration,
        path: PathBuf,
    },
    /// Preload failed
    PreloadFailed {
        identity: PreloadIdentity,
        error: PlaybackError,
    },
    /// Device switched successfully
    DeviceSwitched {
        /// State to restore: (path, position, was_playing)
        restore_state: Option<(PathBuf, Duration, bool)>,
    },
    /// Device switch failed
    DeviceSwitchFailed {
        error: PlaybackError,
    },
    Finished {
        context: PlaybackContext,
    },
    /// Error occurred
    Error {
        context: PlaybackContext,
        request_id: Option<u64>,
        error: PlaybackError,
    },
}

/// Inner state protected by RwLock
#[derive(Debug, Clone)]
struct PlaybackStateInner {
    /// Current playback status
    pub status: PlaybackStatus,
    /// Current playback position
    pub position: Duration,
    /// Total audio duration
    pub duration: Duration,
    /// Volume (0.0 - 1.0)
    pub volume: f32,
    /// Current playing file path
    pub current_path: Option<PathBuf>,
    /// Contiguous disk-cache progress (0.0 - 1.0), None for local files.
    pub cache_progress: Option<f32>,
    pub cached_bytes: u64,
    pub total_cache_bytes: u64,
    /// Bytes immediately readable ahead of the active decoder.
    pub buffered_ahead_bytes: u64,
}

impl Default for PlaybackStateInner {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            volume: 1.0,
            current_path: None,
            cache_progress: None,
            cached_bytes: 0,
            total_cache_bytes: 0,
            buffered_ahead_bytes: 0,
        }
    }
}

/// Thread-safe shared playback state
///
/// UI thread reads this without blocking.
/// Audio thread updates it after each operation.
#[derive(Clone)]
pub struct SharedPlaybackState {
    inner: Arc<RwLock<PlaybackStateInner>>,
}

impl std::fmt::Debug for SharedPlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.read();
        f.debug_struct("SharedPlaybackState")
            .field("status", &inner.status)
            .field("position", &inner.position)
            .field("duration", &inner.duration)
            .field("volume", &inner.volume)
            .finish()
    }
}

impl Default for SharedPlaybackState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedPlaybackState {
    /// Create new shared state
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PlaybackStateInner::default())),
        }
    }

    /// Get current playback info
    pub fn get_info(&self) -> super::PlaybackInfo {
        let inner = self.inner.read();
        super::PlaybackInfo {
            status: inner.status.clone(),
            position: inner.position,
            duration: inner.duration,
            volume: inner.volume,
        }
    }

    /// Check if stopped
    pub fn is_stopped(&self) -> bool {
        let inner = self.inner.read();
        inner.status == PlaybackStatus::Stopped
    }

    /// Get display positio
    pub fn display_position(&self) -> Duration {
        let inner = self.inner.read();
        inner.position
    }

    /// Get contiguous disk-cache progress for the UI secondary track.
    pub fn cache_progress(&self) -> Option<f32> {
        self.inner.read().cache_progress
    }

    /// Get current path
    pub fn current_path(&self) -> Option<PathBuf> {
        self.inner.read().current_path.clone()
    }

    // ---- Update methods (called by audio thread) ----
    /// Update status
    pub fn set_status(&self, status: PlaybackStatus) {
        self.inner.write().status = status;
    }

    /// Update position
    pub fn set_position(&self, position: Duration) {
        self.inner.write().position = position;
    }

    /// Update volume
    pub fn set_volume(&self, volume: f32) {
        self.inner.write().volume = volume;
    }

    /// Update current path
    pub fn set_current_path(&self, path: Option<PathBuf>) {
        self.inner.write().current_path = path;
    }

    pub fn set_cache_bytes(&self, cached: u64, total: u64) {
        let mut inner = self.inner.write();
        inner.cached_bytes = cached;
        inner.total_cache_bytes = total;
        if total > 0 {
            inner.cache_progress = Some(cached as f32 / total as f32);
        } else {
            inner.cache_progress = None;
        }
    }

    pub fn set_buffered_ahead_bytes(&self, buffered_ahead_bytes: u64) {
        self.inner.write().buffered_ahead_bytes = buffered_ahead_bytes;
    }

    /// Update from PlaybackInfo
    pub fn update_from_info(&self, info: &super::PlaybackInfo) {
        let mut inner = self.inner.write();
        inner.status = info.status.clone();
        inner.position = info.position;
        inner.duration = info.duration;
        inner.volume = info.volume;
    }
}

/// Latest retained buffer progress payload.
#[derive(Debug, Clone)]
pub struct BufferDataUpdate {
    pub context: PlaybackContext,
    pub cached: u64,
    pub total: u64,
}

#[derive(Debug)]
struct BufferDataMailboxInner {
    latest: Mutex<Option<BufferDataUpdate>>,
    dirty: AtomicBool,
    wake_tx: AudioCommandSender,
}

/// Race-safe latest-value mailbox for high-frequency buffer progress.
#[derive(Clone, Debug)]
pub struct BufferDataMailbox {
    inner: Arc<BufferDataMailboxInner>,
}

#[derive(Debug, Clone)]
pub struct LatestControlMailbox {
    inner: Arc<LatestControlMailboxInner>,
}

#[derive(Debug)]
struct LatestControlMailboxInner {
    latest_volume: Mutex<Option<f32>>,
    tick_pending: AtomicBool,
    shutdown: Mutex<Option<PlaybackContext>>,
    wake_enqueued: AtomicBool,
    wake_tx: AudioCommandSender,
}

impl LatestControlMailbox {
    pub fn new(wake_tx: AudioCommandSender) -> Self {
        Self {
            inner: Arc::new(LatestControlMailboxInner {
                latest_volume: Mutex::new(None),
                tick_pending: AtomicBool::new(false),
                shutdown: Mutex::new(None),
                wake_enqueued: AtomicBool::new(false),
                wake_tx,
            }),
        }
    }

    fn wake(&self) {
        if !self.inner.wake_enqueued.swap(true, Ordering::AcqRel)
            && self
                .inner
                .wake_tx
                .try_send(AudioCommand::LatestMailboxWake)
                .is_err()
        {
            // A full FIFO is recoverable: the audio thread drains the
            // mailbox after every critical command. Clear the marker so a
            // later producer can retry when capacity becomes available.
            self.inner.wake_enqueued.store(false, Ordering::Release);
        }
    }

    pub fn publish_volume(&self, volume: f32) {
        *self.inner.latest_volume.lock() = Some(volume);
        self.wake();
    }

    pub fn publish_tick(&self) {
        self.inner.tick_pending.store(true, Ordering::Release);
        self.wake();
    }

    pub fn publish_shutdown(&self, context: PlaybackContext) {
        *self.inner.shutdown.lock() = Some(context);
        self.wake();
    }

    pub fn take(&self) -> (Option<f32>, bool, Option<PlaybackContext>) {
        let volume = self.inner.latest_volume.lock().take();
        let tick = self.inner.tick_pending.swap(false, Ordering::AcqRel);
        let shutdown = self.inner.shutdown.lock().take();
        self.inner.wake_enqueued.store(false, Ordering::Release);
        (volume, tick, shutdown)
    }
}

impl BufferDataMailbox {
    pub fn new(wake_tx: AudioCommandSender) -> Self {
        Self {
            inner: Arc::new(BufferDataMailboxInner {
                latest: Mutex::new(None),
                dirty: AtomicBool::new(false),
                wake_tx,
            }),
        }
    }

    pub fn publish(&self, update: BufferDataUpdate) {
        {
            let mut latest = self.inner.latest.lock();
            let replace = latest.as_ref().is_none_or(|current| {
                update.context.generation.0 > current.context.generation.0
                    || (update.context.generation == current.context.generation
                        && update.cached >= current.cached)
            });
            if replace {
                *latest = Some(update);
            }
        }

        if !self.inner.dirty.swap(true, Ordering::AcqRel) {
            let _ = self
                .inner
                .wake_tx
                .try_send(AudioCommand::BufferDataAvailable);
        }
    }

    pub fn take_latest(&self) -> Option<BufferDataUpdate> {
        // Keep the payload lock while clearing the dirty flag. A producer can
        // therefore either publish before this take and be consumed here, or
        // publish after it and observe the cleared flag to enqueue a wake-up.
        let mut latest = self.inner.latest.lock();
        let update = latest.take();
        self.inner.dirty.store(false, Ordering::Release);
        update
    }
}

/// Sender for audio commands (held by AudioHandle)
pub type AudioCommandSender = tokio::sync::mpsc::Sender<AudioCommand>;

/// Receiver for audio commands (held by audio thread)
pub type AudioCommandReceiver = tokio::sync::mpsc::Receiver<AudioCommand>;

/// Sender for audio events (held by audio thread)
pub type AudioEventSender = tokio::sync::mpsc::UnboundedSender<AudioEvent>;

/// Receiver for audio events (held by App)
pub type AudioEventReceiver = tokio::sync::mpsc::UnboundedReceiver<AudioEvent>;

/// Create a new audio command channel
pub fn audio_command_channel() -> (AudioCommandSender, AudioCommandReceiver) {
    tokio::sync::mpsc::channel(64)
}

/// Create a new audio event channel
pub fn audio_event_channel() -> (AudioEventSender, AudioEventReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_controls_overwrite_volume_and_coalesce_ticks() {
        let (tx, mut rx) = audio_command_channel();
        let mailbox = LatestControlMailbox::new(tx);
        mailbox.publish_volume(0.1);
        mailbox.publish_volume(0.9);
        mailbox.publish_tick();
        mailbox.publish_tick();

        let (volume, tick, shutdown) = mailbox.take();
        assert_eq!(volume, Some(0.9));
        assert!(tick);
        assert!(shutdown.is_none());
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn critical_command_fifo_is_bounded() {
        let (tx, _rx) = audio_command_channel();
        for _ in 0..64 {
            tx.try_send(AudioCommand::LatestMailboxWake).unwrap();
        }
        assert!(tx.try_send(AudioCommand::LatestMailboxWake).is_err());
    }

    #[test]
    fn buffer_mailbox_retains_latest_update_when_wake_queue_is_full() {
        let (tx, _rx) = audio_command_channel();
        let mailbox = BufferDataMailbox::new(tx.clone());
        for _ in 0..64 {
            tx.try_send(AudioCommand::LatestMailboxWake).unwrap();
        }
        let controller = super::super::identity::PlaybackGenerationController::new();
        let context = controller.activate_generation();

        mailbox.publish(BufferDataUpdate {
            context: context.clone(),
            cached: 3,
            total: 10,
        });

        let update = mailbox.take_latest().expect("latest update must survive");
        assert_eq!(update.context, context);
        assert_eq!(update.cached, 3);
        assert_eq!(update.total, 10);
    }

    #[test]
    fn cache_progress_and_decoder_reserve_are_independent() {
        let state = SharedPlaybackState::new();
        state.set_cache_bytes(5, 10);
        state.set_buffered_ahead_bytes(1234);
        assert_eq!(state.cache_progress(), Some(0.5));

        state.set_buffered_ahead_bytes(0);
        assert_eq!(state.cache_progress(), Some(0.5));
        state.set_cache_bytes(0, 0);
        assert_eq!(state.cache_progress(), None);
    }

    #[test]
    fn audio_error_events_retain_the_typed_playback_error() {
        let controller = super::super::identity::PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let event = AudioEvent::Error {
            context,
            request_id: Some(7),
            error: PlaybackError::Cancelled("stale generation".to_string()),
        };

        let AudioEvent::Error { error, .. } = event else {
            panic!("constructed the wrong audio event variant");
        };
        assert!(matches!(error, PlaybackError::Cancelled(_)));
        assert_eq!(
            error.code(),
            rustle_application::error::ErrorCode::OperationCancelled
        );
    }
}
