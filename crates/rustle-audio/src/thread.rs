//! Audio thread implementation
//!
//! This module runs the AudioPlayer in a dedicated thread, processing
//! commands from the UI thread and sending events back.
//!
//! The audio thread may block on streaming operations (e.g., seeking to
//! unbuffered positions), but this doesn't affect the UI thread.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rodio::{Player as Sink, Source};

use super::PlaybackStatus;
use super::chain::{AudioProcessingChain, PlaybackProcessingRuntime};
use super::events::{
    AudioCommand, AudioCommandReceiver, AudioEvent, AudioEventSender, BufferDataMailbox,
    BufferDataUpdate, LatestControlMailbox, SharedPlaybackState, audio_command_channel,
    audio_event_channel,
};
use super::handle::AudioHandle;
use super::player::{
    AudioPlayer, DetachedStreamingPlayback, PlaybackError, PlaybackResult, PreparedStreamingSource,
    prepare_streaming_source,
};
use super::streaming::{
    SharedBuffer, SharedBufferHealth, StreamingBuffer, StreamingReaderCancellation,
};

pub type WorkerTask = Box<dyn FnOnce() + Send + 'static>;
pub type WorkerSpawner = fn(
    thread_name: &'static str,
    operation: &'static str,
    worker: WorkerTask,
) -> std::io::Result<JoinHandle<()>>;

const STREAMING_PREPARATION_QUEUE_CAPACITY: usize = 4;
const STREAMING_PREPARATION_RESULT_CAPACITY: usize = 8;
const STREAMING_SEEK_QUEUE_CAPACITY: usize = 1;
const STREAMING_SEEK_RESULT_CAPACITY: usize = 2;
const CONTROL_MAINTENANCE_INTERVAL: Duration =
    Duration::from_millis(super::automix::SCHEDULER_POLL_MS);

#[derive(Default)]
struct FinishedGuard {
    generation: Option<super::identity::PlaybackGeneration>,
    emitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinishReason {
    Natural,
    Stop,
    SeekRebuild,
    TransitionDisposed,
    Stale,
}

impl FinishedGuard {
    fn reset(&mut self) {
        self.generation = None;
        self.emitted = false;
    }

    fn try_mark(
        &mut self,
        generation: super::identity::PlaybackGeneration,
        reason: FinishReason,
    ) -> bool {
        if reason != FinishReason::Natural {
            return false;
        }
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.emitted = false;
        }
        if self.emitted {
            false
        } else {
            self.emitted = true;
            true
        }
    }
}

/// Preloaded sink with metadata
struct PreloadedSink {
    identity: super::identity::PreloadIdentity,
    sink: Sink,
    duration: Duration,
    path: PathBuf,
    track_gain: f32,
    runtime: PlaybackProcessingRuntime,
    is_streaming: bool,
    shared_buffer: Option<SharedBuffer>,
    reader_cancellation: Option<StreamingReaderCancellation>,
}

struct ScheduledPreloadedTransition {
    owner: super::identity::PlaybackContext,
    identity: super::identity::PreloadIdentity,
    playback_request_id: u64,
    fade_in: bool,
    transition: super::automix::TransitionDirective,
}

#[derive(Clone, Copy)]
enum PlaybackPreparationMode {
    Play { fade_in: bool },
    LoadPaused { position: Duration },
}

struct PlaybackPreparationRequest {
    context: super::identity::PlaybackContext,
    request_id: u64,
    buffer: StreamingBuffer,
    duration: Duration,
    cache_path: Option<PathBuf>,
    track_gain: f32,
    mode: PlaybackPreparationMode,
}

struct PreparedPlayback {
    context: super::identity::PlaybackContext,
    request_id: u64,
    shared_buffer: SharedBuffer,
    reader_cancellation: StreamingReaderCancellation,
    source: PlaybackResult<PreparedStreamingSource>,
    duration: Duration,
    cache_path: Option<PathBuf>,
    track_gain: f32,
    mode: PlaybackPreparationMode,
}

struct PreloadPreparationRequest {
    identity: super::identity::PreloadIdentity,
    buffer: StreamingBuffer,
    duration: Duration,
    track_gain: f32,
}

struct PreparedPreload {
    identity: super::identity::PreloadIdentity,
    shared_buffer: SharedBuffer,
    reader_cancellation: StreamingReaderCancellation,
    source: PlaybackResult<PreparedStreamingSource>,
    duration: Duration,
    track_gain: f32,
}

enum StreamingPreparationResult {
    Playback(PreparedPlayback),
    Preload(PreparedPreload),
}

enum ControlEvent {
    Command(AudioCommand),
    Prepared(StreamingPreparationResult),
    StreamingSeekFinished(StreamingSeekResult),
    Maintenance,
    Closed,
}

async fn next_control_event(
    command_rx: &mut AudioCommandReceiver,
    preparation_result_rx: &mut tokio::sync::mpsc::Receiver<StreamingPreparationResult>,
    seek_result_rx: &mut tokio::sync::mpsc::Receiver<StreamingSeekResult>,
    maintenance_deadline: tokio::time::Instant,
) -> ControlEvent {
    tokio::select! {
        command = command_rx.recv() => match command {
            Some(command) => ControlEvent::Command(command),
            None => ControlEvent::Closed,
        },
        Some(prepared) = preparation_result_rx.recv() => ControlEvent::Prepared(prepared),
        Some(result) = seek_result_rx.recv() => ControlEvent::StreamingSeekFinished(result),
        _ = tokio::time::sleep_until(maintenance_deadline) => ControlEvent::Maintenance,
    }
}

struct StreamingSeekRequest {
    context: super::identity::PlaybackContext,
    nonce: super::identity::SeekNonce,
    position: Duration,
    sink: Sink,
    buffer: StreamingBuffer,
    reader_cancellation: StreamingReaderCancellation,
    shared_buffer: SharedBuffer,
    duration: Duration,
    cache_path: Option<PathBuf>,
    track_gain: f32,
    was_paused: bool,
}

struct StreamingSeekResult {
    context: super::identity::PlaybackContext,
    nonce: super::identity::SeekNonce,
    target_position: Duration,
    source: PlaybackResult<PreparedStreamingSource>,
    sink: Sink,
    reader_cancellation: StreamingReaderCancellation,
    shared_buffer: SharedBuffer,
    duration: Duration,
    cache_path: Option<PathBuf>,
    track_gain: f32,
    was_paused: bool,
}

#[derive(Clone)]
struct DeferredStreamingSeek {
    context: super::identity::PlaybackContext,
    nonce: super::identity::SeekNonce,
    position: Duration,
}

struct StreamingSeekWorker {
    request_tx: std::sync::mpsc::SyncSender<StreamingSeekRequest>,
    active_cancellation: std::sync::Arc<parking_lot::Mutex<Option<StreamingReaderCancellation>>>,
}

fn cancel_streaming_seek_runtime(
    reader_cancellation: &StreamingReaderCancellation,
    _shared_buffer: &SharedBuffer,
) {
    reader_cancellation.cancel();
}

impl StreamingSeekWorker {
    fn new(
        result_tx: tokio::sync::mpsc::Sender<StreamingSeekResult>,
        spawn_worker: WorkerSpawner,
    ) -> PlaybackResult<Self> {
        let (request_tx, request_rx) =
            std::sync::mpsc::sync_channel::<StreamingSeekRequest>(STREAMING_SEEK_QUEUE_CAPACITY);
        let active_cancellation =
            std::sync::Arc::new(parking_lot::Mutex::new(None::<StreamingReaderCancellation>));
        spawn_worker(
            "audio-streaming-seek",
            "prepare_streaming_seek",
            Box::new(move || {
                while let Ok(request) = request_rx.recv() {
                    let StreamingSeekRequest {
                        context,
                        nonce,
                        position,
                        sink,
                        buffer,
                        reader_cancellation,
                        shared_buffer,
                        duration,
                        cache_path,
                        track_gain,
                        was_paused,
                    } = request;
                    let started = Instant::now();
                    let source = prepare_streaming_source(buffer).and_then(|mut source| {
                        source
                            .try_seek(position)
                            .map_err(|error| PlaybackError::SeekUnsupported(error.to_string()))?;
                        Ok(source)
                    });
                    tracing::debug!(
                        generation = context.generation.0,
                        nonce = nonce.0,
                        elapsed_ms = started.elapsed().as_millis(),
                        success = source.is_ok(),
                        "Streaming seek worker completed"
                    );
                    let completed = StreamingSeekResult {
                        context,
                        nonce,
                        target_position: position,
                        source,
                        sink,
                        reader_cancellation,
                        shared_buffer,
                        duration,
                        cache_path,
                        track_gain,
                        was_paused,
                    };
                    if let Err(error) = result_tx.blocking_send(completed) {
                        let failed = error.0;
                        cancel_streaming_seek_runtime(
                            &failed.reader_cancellation,
                            &failed.shared_buffer,
                        );
                        failed.sink.stop();
                        break;
                    }
                }
            }),
        )
        .map_err(|error| PlaybackError::ControlUnavailable(error.to_string()))?;
        Ok(Self {
            request_tx,
            active_cancellation,
        })
    }

    fn try_submit(
        &self,
        request: StreamingSeekRequest,
    ) -> Result<(), Box<(PlaybackError, StreamingSeekRequest)>> {
        let reader_cancellation = request.reader_cancellation.clone();
        *self.active_cancellation.lock() = Some(reader_cancellation);
        self.request_tx.try_send(request).map_err(|error| {
            self.active_cancellation.lock().take();
            match error {
                std::sync::mpsc::TrySendError::Full(request) => Box::new((
                    PlaybackError::ControlUnavailable(
                        "streaming seek worker queue is full".to_string(),
                    ),
                    request,
                )),
                std::sync::mpsc::TrySendError::Disconnected(request) => Box::new((
                    PlaybackError::ControlUnavailable(
                        "streaming seek worker is unavailable".to_string(),
                    ),
                    request,
                )),
            }
        })
    }

    fn cancel_active(&self) {
        if let Some(cancellation) = self.active_cancellation.lock().as_ref() {
            cancellation.cancel();
        }
    }

    fn clear_active(&self) {
        self.active_cancellation.lock().take();
    }
}

struct StreamingPreparationPool {
    playback_tx: std::sync::mpsc::SyncSender<PlaybackPreparationRequest>,
    preload_tx: std::sync::mpsc::SyncSender<PreloadPreparationRequest>,
    active_playback_cancellation:
        std::sync::Arc<parking_lot::Mutex<Option<StreamingReaderCancellation>>>,
}

impl StreamingPreparationPool {
    fn new(
        result_tx: tokio::sync::mpsc::Sender<StreamingPreparationResult>,
        spawn_worker: WorkerSpawner,
    ) -> PlaybackResult<Self> {
        let (playback_tx, playback_rx) =
            std::sync::mpsc::sync_channel(STREAMING_PREPARATION_QUEUE_CAPACITY);
        let active_playback_cancellation =
            std::sync::Arc::new(parking_lot::Mutex::new(None::<StreamingReaderCancellation>));
        let playback_result_tx = result_tx.clone();
        spawn_worker(
            "audio-playback-prepare",
            "prepare_streaming_playback",
            Box::new(move || {
                while let Ok(request) = playback_rx.recv() {
                    let PlaybackPreparationRequest {
                        context,
                        request_id,
                        buffer,
                        duration,
                        cache_path,
                        track_gain,
                        mode,
                    } = request;
                    let shared_buffer = buffer.shared().clone();
                    let reader_cancellation = buffer.reader_cancellation();
                    let started = Instant::now();
                    let source = prepare_streaming_source(buffer).and_then(|mut source| {
                        if let PlaybackPreparationMode::LoadPaused { position } = mode {
                            source.try_seek(position).map_err(|error| {
                                PlaybackError::SeekUnsupported(error.to_string())
                            })?;
                        }
                        Ok(source)
                    });
                    tracing::debug!(
                        request_id,
                        generation = context.generation.0,
                        elapsed_ms = started.elapsed().as_millis(),
                        success = source.is_ok(),
                        "Streaming playback preparation completed"
                    );
                    if playback_result_tx
                        .blocking_send(StreamingPreparationResult::Playback(PreparedPlayback {
                            context,
                            request_id,
                            shared_buffer,
                            reader_cancellation,
                            source,
                            duration,
                            cache_path,
                            track_gain,
                            mode,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        )
        .map_err(|error| PlaybackError::ControlUnavailable(error.to_string()))?;

        let (preload_tx, preload_rx) =
            std::sync::mpsc::sync_channel(STREAMING_PREPARATION_QUEUE_CAPACITY);
        spawn_worker(
            "audio-preload-prepare",
            "prepare_streaming_preload",
            Box::new(move || {
                while let Ok(request) = preload_rx.recv() {
                    let PreloadPreparationRequest {
                        identity,
                        buffer,
                        duration,
                        track_gain,
                    } = request;
                    let shared_buffer = buffer.shared().clone();
                    let reader_cancellation = buffer.reader_cancellation();
                    let started = Instant::now();
                    let source = prepare_streaming_source(buffer);
                    tracing::debug!(
                        request_id = identity.request_id,
                        generation = identity.generation.0,
                        elapsed_ms = started.elapsed().as_millis(),
                        success = source.is_ok(),
                        "Streaming preload preparation completed"
                    );
                    if result_tx
                        .blocking_send(StreamingPreparationResult::Preload(PreparedPreload {
                            identity,
                            shared_buffer,
                            reader_cancellation,
                            source,
                            duration,
                            track_gain,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        )
        .map_err(|error| PlaybackError::ControlUnavailable(error.to_string()))?;

        Ok(Self {
            playback_tx,
            preload_tx,
            active_playback_cancellation,
        })
    }

    fn prepare_playback(&self, request: PlaybackPreparationRequest) -> PlaybackResult<()> {
        let reader_cancellation = request.buffer.reader_cancellation();
        *self.active_playback_cancellation.lock() = Some(reader_cancellation);
        self.playback_tx.try_send(request).map_err(|error| {
            self.active_playback_cancellation.lock().take();
            match error {
                std::sync::mpsc::TrySendError::Full(_) => PlaybackError::ControlUnavailable(
                    "streaming playback preparation queue is full".to_string(),
                ),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    PlaybackError::ControlUnavailable(
                        "streaming playback preparation worker is unavailable".to_string(),
                    )
                }
            }
        })
    }

    fn cancel_active_playback(&self) {
        if let Some(cancellation) = self.active_playback_cancellation.lock().as_ref() {
            cancellation.cancel();
        }
    }

    fn clear_active_playback(&self, completed: &StreamingReaderCancellation) {
        let mut active = self.active_playback_cancellation.lock();
        if active
            .as_ref()
            .is_some_and(|cancellation| cancellation.same_reader(completed))
        {
            active.take();
        }
    }

    fn prepare_preload(&self, request: PreloadPreparationRequest) -> PlaybackResult<()> {
        self.preload_tx
            .try_send(request)
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => PlaybackError::ControlUnavailable(
                    "streaming preload preparation queue is full".to_string(),
                ),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    PlaybackError::ControlUnavailable(
                        "streaming preload preparation worker is unavailable".to_string(),
                    )
                }
            })
    }
}

pub struct AudioThreadHandle {
    pub handle: AudioHandle,
    pub event_rx: Option<super::events::AudioEventReceiver>,
    thread_handle: Option<JoinHandle<()>>,
}

impl AudioThreadHandle {
    pub fn take_event_rx(&mut self) -> Option<super::events::AudioEventReceiver> {
        self.event_rx.take()
    }
}

impl Drop for AudioThreadHandle {
    fn drop(&mut self) {
        self.handle.shutdown();
        // Intentionally detach in every case. The audio thread owns its backend
        // resources and must never be joined from arbitrary UI teardown paths.
        let _ = self.thread_handle.take();
    }
}

/// Spawn the audio thread
///
/// Creates an AudioPlayer in a dedicated thread and returns handles for
/// communication. The audio thread processes commands and sends events.
///
/// # Arguments
/// * `device_name` - Optional audio output device name
/// * `chain` - Audio processing chain (EQ, preamp, etc.)
///
/// # Returns
/// * `AudioThreadHandle` containing the handle and event receiver
pub fn spawn_audio_thread_with(
    device_name: Option<&str>,
    chain: AudioProcessingChain,
    spawn_worker: WorkerSpawner,
) -> PlaybackResult<AudioThreadHandle> {
    // Create channels
    let (command_tx, command_rx) = audio_command_channel();
    let (event_tx, event_rx) = audio_event_channel();
    let latest_controls = LatestControlMailbox::new(command_tx.clone());
    let (preparation_result_tx, preparation_result_rx) =
        tokio::sync::mpsc::channel(STREAMING_PREPARATION_RESULT_CAPACITY);
    let preparation_pool = StreamingPreparationPool::new(preparation_result_tx, spawn_worker)?;
    let (seek_result_tx, seek_result_rx) =
        tokio::sync::mpsc::channel(STREAMING_SEEK_RESULT_CAPACITY);
    let seek_worker = StreamingSeekWorker::new(seek_result_tx, spawn_worker)?;

    // Create shared state
    let state = SharedPlaybackState::new();
    let state_clone = state.clone();

    // The mailbox owns the only callback wake sender; the audio thread itself
    // does not need a command sender once the handle has been created.
    let buffer_mailbox = BufferDataMailbox::new(command_tx.clone());

    // Create handle for UI
    let handle = AudioHandle::new(command_tx, latest_controls.clone(), state);
    let generation_controller = handle.generation_controller();

    // Clone device name for thread
    let device_name_owned = device_name.map(|s| s.to_string());

    // Spawn audio thread
    let thread_handle = spawn_worker(
        "audio-player",
        "audio_control_actor",
        Box::new(move || {
            // Create player in audio thread
            let player_result = if let Some(ref name) = device_name_owned {
                AudioPlayer::with_device(Some(name), chain)
            } else {
                AudioPlayer::new(chain)
            };

            match player_result {
                Ok(player) => {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build();
                    match runtime {
                        Ok(runtime) => runtime.block_on(audio_thread_main(
                            player,
                            command_rx,
                            buffer_mailbox,
                            latest_controls,
                            event_tx,
                            state_clone,
                            generation_controller,
                            preparation_pool,
                            preparation_result_rx,
                            seek_worker,
                            seek_result_rx,
                        )),
                        Err(error) => {
                            tracing::error!("Failed to create audio control runtime: {}", error);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create audio player: {}", e);
                }
            }
        }),
    )
    .map_err(|error| PlaybackError::ControlUnavailable(error.to_string()))?;

    Ok(AudioThreadHandle {
        handle,
        event_rx: Some(event_rx),
        thread_handle: Some(thread_handle),
    })
}

/// Main loop for the audio thread
///
/// Processes commands from the UI thread and updates shared state.
/// The control actor waits on ordered lifecycle commands, immutable streaming
/// preparation results, or a coalesced maintenance deadline. Periodic work is
/// never represented as a critical command.
#[expect(
    clippy::too_many_arguments,
    reason = "audio actor entry owns the complete runtime channel set; remove when AudioThreadContext owns those channels"
)]
async fn audio_thread_main(
    mut player: AudioPlayer,
    mut command_rx: AudioCommandReceiver,
    buffer_mailbox: BufferDataMailbox,
    latest_controls: LatestControlMailbox,
    event_tx: AudioEventSender,
    state: SharedPlaybackState,
    generation: super::identity::PlaybackGenerationController,
    preparation_pool: StreamingPreparationPool,
    mut preparation_result_rx: tokio::sync::mpsc::Receiver<StreamingPreparationResult>,
    seek_worker: StreamingSeekWorker,
    mut seek_result_rx: tokio::sync::mpsc::Receiver<StreamingSeekResult>,
) {
    tracing::info!("Audio thread started");

    // Storage for preloaded sinks (request_id -> PreloadedSink)
    let mut preloaded_sinks: HashMap<u64, PreloadedSink> = HashMap::new();

    // Current streaming buffer reference (for data availability checks)
    // Set when PlayStreaming command is processed, cleared on Play/Stop
    let mut current_buffer: Option<SharedBuffer> = None;

    let mut current_context: Option<super::identity::PlaybackContext> = None;
    let mut pause_after_preparation: Option<super::identity::PlaybackContext> = None;
    let mut finished_guard = FinishedGuard::default();
    let mut scheduled_transition: Option<ScheduledPreloadedTransition> = None;
    let mut deferred_streaming_seeks: HashMap<
        super::identity::PlaybackGeneration,
        DeferredStreamingSeek,
    > = HashMap::new();
    let mut transition_scheduler = super::automix::AudioClockScheduler::new(Duration::from_millis(
        super::automix::SCHEDULER_HORIZON_MS,
    ));

    let mut next_maintenance = tokio::time::Instant::now() + CONTROL_MAINTENANCE_INTERVAL;

    loop {
        let event = next_control_event(
            &mut command_rx,
            &mut preparation_result_rx,
            &mut seek_result_rx,
            next_maintenance,
        )
        .await;
        let now = tokio::time::Instant::now();
        let maintenance_due = now >= next_maintenance;
        if maintenance_due {
            // Missed timer wake-ups are intentionally coalesced. Never replay a
            // backlog of periodic ticks after the actor was busy.
            let lateness = now.saturating_duration_since(next_maintenance);
            if lateness > CONTROL_MAINTENANCE_INTERVAL {
                tracing::warn!(
                    lateness_ms = lateness.as_millis(),
                    "Audio control maintenance deadline was delayed"
                );
            }
            next_maintenance = now + CONTROL_MAINTENANCE_INTERVAL;
        }

        let cmd = match event {
            ControlEvent::Closed => {
                seek_worker.cancel_active();
                preparation_pool.cancel_active_playback();
                cancel_current_streaming_buffer(&mut current_buffer);
                for (_, preloaded) in preloaded_sinks.drain() {
                    cancel_preloaded_streaming_buffer(&preloaded);
                }
                player.stop();
                break;
            }
            ControlEvent::Maintenance => {
                if run_control_maintenance(
                    &mut player,
                    &buffer_mailbox,
                    &latest_controls,
                    &event_tx,
                    &state,
                    &generation,
                    &preparation_pool,
                    &mut current_buffer,
                    &mut current_context,
                    &mut finished_guard,
                    &mut preloaded_sinks,
                    &mut scheduled_transition,
                    &mut transition_scheduler,
                    maintenance_due,
                ) {
                    break;
                }
                continue;
            }
            ControlEvent::Prepared(prepared) => {
                handle_streaming_preparation_result(
                    prepared,
                    &mut player,
                    &buffer_mailbox,
                    &event_tx,
                    &state,
                    &generation,
                    &preparation_pool,
                    &mut current_buffer,
                    &mut current_context,
                    &mut pause_after_preparation,
                    &mut finished_guard,
                    &mut preloaded_sinks,
                    &mut scheduled_transition,
                    &mut transition_scheduler,
                );
                if run_control_maintenance(
                    &mut player,
                    &buffer_mailbox,
                    &latest_controls,
                    &event_tx,
                    &state,
                    &generation,
                    &preparation_pool,
                    &mut current_buffer,
                    &mut current_context,
                    &mut finished_guard,
                    &mut preloaded_sinks,
                    &mut scheduled_transition,
                    &mut transition_scheduler,
                    maintenance_due,
                ) {
                    break;
                }
                continue;
            }
            ControlEvent::StreamingSeekFinished(result) => {
                handle_streaming_seek_result(
                    result,
                    &mut player,
                    &seek_worker,
                    &buffer_mailbox,
                    &event_tx,
                    &state,
                    &generation,
                    &mut current_buffer,
                    &mut current_context,
                    &mut deferred_streaming_seeks,
                );
                if run_control_maintenance(
                    &mut player,
                    &buffer_mailbox,
                    &latest_controls,
                    &event_tx,
                    &state,
                    &generation,
                    &preparation_pool,
                    &mut current_buffer,
                    &mut current_context,
                    &mut finished_guard,
                    &mut preloaded_sinks,
                    &mut scheduled_transition,
                    &mut transition_scheduler,
                    maintenance_due,
                ) {
                    break;
                }
                continue;
            }
            ControlEvent::Command(cmd) => cmd,
        };

        if matches!(
            &cmd,
            AudioCommand::Play { .. }
                | AudioCommand::LoadPaused { .. }
                | AudioCommand::LoadPausedStreaming { .. }
                | AudioCommand::PlayAt { .. }
                | AudioCommand::PlayStreaming { .. }
                | AudioCommand::Stop { .. }
                | AudioCommand::Seek { .. }
                | AudioCommand::PlayPreloaded { .. }
                | AudioCommand::SwitchDevice { .. }
        ) {
            cancel_scheduled_transition(
                &mut scheduled_transition,
                &mut transition_scheduler,
                &mut preloaded_sinks,
            );
        }
        if matches!(
            &cmd,
            AudioCommand::Play { .. }
                | AudioCommand::LoadPaused { .. }
                | AudioCommand::LoadPausedStreaming { .. }
                | AudioCommand::PlayAt { .. }
                | AudioCommand::PlayStreaming { .. }
                | AudioCommand::Stop { .. }
                | AudioCommand::PlayPreloaded { .. }
                | AudioCommand::SwitchDevice { .. }
        ) {
            seek_worker.cancel_active();
            preparation_pool.cancel_active_playback();
        }
        match cmd {
            AudioCommand::Play {
                context,
                request_id,
                path,
                fade_in,
                track_gain,
            } => {
                if !generation.accepts(&context) {
                    let _ = finished_guard.try_mark(context.generation, FinishReason::Stale);
                    continue;
                }
                finished_guard.reset();
                cancel_current_streaming_buffer(&mut current_buffer);
                // Local file playback
                current_context = Some(context.clone());
                handle_play(
                    &mut player,
                    &event_tx,
                    &state,
                    context,
                    request_id,
                    path,
                    fade_in,
                    track_gain,
                );
            }

            AudioCommand::LoadPaused {
                context,
                request_id,
                path,
                position,
                track_gain,
            } => {
                if !generation.accepts(&context) {
                    continue;
                }
                finished_guard.reset();
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = Some(context.clone());
                handle_load_paused(
                    &mut player,
                    &event_tx,
                    &state,
                    context,
                    request_id,
                    path,
                    position,
                    track_gain,
                );
            }

            AudioCommand::LoadPausedStreaming {
                context,
                request_id,
                buffer,
                duration,
                cache_path,
                position,
                track_gain,
            } => {
                if !generation.accepts(&context) {
                    continue;
                }
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = None;
                player.stop();
                update_state_from_player(&player, &state);
                state.set_current_path(None);
                let error_context = context.clone();
                pause_after_preparation = None;
                if let Err(error) = preparation_pool.prepare_playback(PlaybackPreparationRequest {
                    context,
                    request_id,
                    buffer,
                    duration,
                    cache_path,
                    track_gain,
                    mode: PlaybackPreparationMode::LoadPaused { position },
                }) {
                    tracing::warn!(request_id, %error, "Streaming playback preparation rejected");
                    let _ = event_tx.send(AudioEvent::Error {
                        context: error_context,
                        request_id: Some(request_id),
                        error,
                    });
                }
            }

            AudioCommand::PlayAt {
                context,
                request_id,
                path,
                position,
                fade_in,
                track_gain,
            } => {
                if !generation.accepts(&context) {
                    continue;
                }
                finished_guard.reset();
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = Some(context.clone());
                handle_play_at(
                    &mut player,
                    &event_tx,
                    &state,
                    context,
                    request_id,
                    path,
                    position,
                    fade_in,
                    track_gain,
                );
            }

            AudioCommand::PlayStreaming {
                context,
                request_id,
                buffer,
                duration,
                cache_path,
                fade_in,
                track_gain,
            } => {
                if !generation.accepts(&context) {
                    continue;
                }
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = None;
                player.stop();
                update_state_from_player(&player, &state);
                state.set_current_path(None);
                let error_context = context.clone();
                pause_after_preparation = None;
                if let Err(error) = preparation_pool.prepare_playback(PlaybackPreparationRequest {
                    context,
                    request_id,
                    buffer,
                    duration,
                    cache_path,
                    track_gain,
                    mode: PlaybackPreparationMode::Play { fade_in },
                }) {
                    tracing::warn!(request_id, %error, "Streaming playback preparation rejected");
                    let _ = event_tx.send(AudioEvent::Error {
                        context: error_context,
                        request_id: Some(request_id),
                        error,
                    });
                }
            }

            AudioCommand::Pause { context, fade_out } => {
                if !generation.accepts(&context) {
                    continue;
                }
                if current_context.as_ref() != Some(&context) {
                    pause_after_preparation = Some(context.clone());
                }
                if fade_out {
                    player.pause_with_fade(true);
                } else {
                    player.pause();
                    update_state_from_player(&player, &state);
                    let pos = player.get_info().position;
                    let _ = event_tx.send(AudioEvent::Paused {
                        context,
                        request_id: None,
                        position: pos,
                    });
                }
            }

            AudioCommand::Resume { context, fade_in } => {
                if !generation.accepts(&context) {
                    continue;
                }
                if pause_after_preparation.as_ref() == Some(&context) {
                    pause_after_preparation = None;
                }
                if let Some(ref buf) = current_buffer {
                    let remaining_bytes = buf.buffered_ahead();
                    let source_has_all_required_bytes =
                        buf.is_complete() || buf.remote_eof_reached();
                    if buf
                        .policy()
                        .should_enter_buffering(remaining_bytes, source_has_all_required_bytes)
                    {
                        tracing::info!(
                            "Resume: remaining {} bytes < {} (low water mark), entering Buffering",
                            remaining_bytes,
                            buf.policy().low_water_mark_bytes()
                        );
                        let position = player.get_info().position;
                        enter_buffering(&mut player, &state, &event_tx, position, context.clone());
                        continue;
                    }
                }

                if fade_in {
                    player.resume_with_fade(true);
                } else {
                    player.resume();
                }
                update_state_from_player(&player, &state);
                let _ = event_tx.send(AudioEvent::Resumed { context });
            }

            AudioCommand::Stop { context } => {
                if !generation.accepts(&context) {
                    continue;
                }
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = None;
                pause_after_preparation = None;
                let _ = finished_guard.try_mark(context.generation, FinishReason::Stop);
                player.stop();
                update_state_from_player(&player, &state);
                state.set_current_path(None);
                let _ = event_tx.send(AudioEvent::Stopped {
                    context: context.clone(),
                });
            }

            AudioCommand::Seek {
                context,
                nonce,
                position,
            } => {
                if context.cancellation.is_cancelled() {
                    continue;
                }
                let _ = finished_guard.try_mark(context.generation, FinishReason::SeekRebuild);
                if current_context.as_ref() == Some(&context)
                    && let Some(shared_buffer) = current_buffer.as_ref().cloned()
                {
                    if !generation.accepts_seek(&context, nonce) {
                        continue;
                    }
                    let _ = event_tx.send(AudioEvent::SeekStarted {
                        context: context.clone(),
                        nonce,
                        target_position: position,
                    });
                    match player.take_streaming_sink_for_seek(position) {
                        Ok(DetachedStreamingPlayback {
                            sink,
                            duration,
                            cache_path,
                            track_gain,
                            was_paused,
                        }) => {
                            // The old decoder and the replacement decoder share
                            // one retained window. Cancel the old reader before
                            // starting the replacement so they cannot race the
                            // authoritative reader position/window epoch.
                            player.cancel_current_streaming_reader();
                            sink.stop();
                            let buffer = StreamingBuffer::new(shared_buffer.clone());
                            let reader_cancellation = buffer.reader_cancellation();
                            submit_streaming_seek(
                                &seek_worker,
                                &mut player,
                                &event_tx,
                                &state,
                                &generation,
                                &mut current_buffer,
                                &mut current_context,
                                StreamingSeekRequest {
                                    context,
                                    nonce,
                                    position,
                                    sink,
                                    buffer,
                                    reader_cancellation,
                                    shared_buffer,
                                    duration,
                                    cache_path,
                                    track_gain,
                                    was_paused,
                                },
                            )
                        }
                        Err(PlaybackError::Cancelled(_)) => {
                            seek_worker.cancel_active();
                            deferred_streaming_seeks.insert(
                                context.generation,
                                DeferredStreamingSeek {
                                    context,
                                    nonce,
                                    position,
                                },
                            );
                        }
                        Err(error) => {
                            let _ = event_tx.send(AudioEvent::SeekFailed {
                                context,
                                nonce,
                                error,
                            });
                        }
                    }
                } else if current_context.as_ref() == Some(&context) && current_buffer.is_none() {
                    handle_seek(
                        &mut player,
                        &event_tx,
                        &state,
                        &generation,
                        context,
                        nonce,
                        position,
                    );
                } else if generation.accepts_seek(&context, nonce) {
                    let _ = event_tx.send(AudioEvent::SeekFailed {
                        context,
                        nonce,
                        error: PlaybackError::ControlUnavailable(
                            "audio source is still preparing".to_string(),
                        ),
                    });
                }
                finished_guard.reset();
            }

            AudioCommand::CreatePreloadSink {
                identity,
                path,
                track_gain,
            } => {
                if !generation.accepts_preload(&identity) {
                    continue;
                }
                handle_create_preload_sink(
                    &player,
                    &event_tx,
                    &mut preloaded_sinks,
                    identity,
                    path,
                    track_gain,
                );
            }

            AudioCommand::CreatePreloadSinkStreaming {
                identity,
                buffer,
                duration,
                track_gain,
            } => {
                if !generation.accepts_preload(&identity) {
                    continue;
                }
                let error_identity = identity.clone();
                if let Err(error) = preparation_pool.prepare_preload(PreloadPreparationRequest {
                    identity,
                    buffer,
                    duration,
                    track_gain,
                }) {
                    tracing::warn!(
                        request_id = error_identity.request_id,
                        %error,
                        "Streaming preload preparation rejected"
                    );
                    let _ = event_tx.send(AudioEvent::PreloadFailed {
                        identity: error_identity,
                        error,
                    });
                }
            }

            AudioCommand::PlayPreloaded {
                context: owner,
                identity,
                playback_request_id,
                fade_in,
                transition,
            } => {
                if !generation.accepts(&owner) {
                    continue;
                }
                let preloaded = match take_healthy_preloaded_sink(&mut preloaded_sinks, &identity) {
                    Ok(preloaded) => preloaded,
                    Err(failure) => {
                        tracing::warn!(
                            request_id = identity.request_id,
                            generation = identity.generation.0,
                            error = %failure.error,
                            "Rejected preload before immediate promotion"
                        );
                        let _ = event_tx.send(AudioEvent::Error {
                            context: owner,
                            request_id: Some(playback_request_id),
                            error: failure.error,
                        });
                        continue;
                    }
                };
                let will_overlap = match player.preloaded_will_overlap(&transition) {
                    Ok(will_overlap) => will_overlap,
                    Err(error) => {
                        clear_preloaded_buffer_callback(&preloaded);
                        let _ = event_tx.send(AudioEvent::Error {
                            context: owner,
                            request_id: Some(playback_request_id),
                            error,
                        });
                        continue;
                    }
                };
                let Some(context) = generation.activate_preloaded_generation(&identity) else {
                    clear_preloaded_buffer_callback(&preloaded);
                    continue;
                };
                let _ =
                    finished_guard.try_mark(context.generation, FinishReason::TransitionDisposed);
                finished_guard.reset();
                let outgoing_shared_buffer = if will_overlap {
                    current_buffer
                        .take()
                        .inspect(SharedBuffer::clear_buffer_callback)
                } else {
                    cancel_current_streaming_buffer(&mut current_buffer);
                    None
                };
                // Keep the preload's source identity separate from the new playback generation.
                current_buffer = preloaded.shared_buffer.clone();
                current_context = Some(context.clone());
                handle_play_preloaded(
                    &mut player,
                    &buffer_mailbox,
                    &event_tx,
                    &state,
                    context,
                    playback_request_id,
                    preloaded,
                    outgoing_shared_buffer,
                    fade_in,
                    transition,
                );
            }

            AudioCommand::SchedulePreloadedTransition {
                owner,
                identity,
                playback_request_id,
                trigger_at,
                fade_in,
                transition,
            } => {
                let owner_current = generation.accepts(&owner);
                let preload_ready = preloaded_sinks
                    .get(&identity.request_id)
                    .is_some_and(|preloaded| preloaded.identity == identity);
                if !owner_current {
                    continue;
                }
                if !generation.accepts_preload(&identity)
                    || owner.generation != identity.generation
                    || owner.cancellation != identity.cancellation
                    || !preload_ready
                {
                    release_preloaded_identity(&mut preloaded_sinks, &identity);
                    let _ = event_tx.send(AudioEvent::Error {
                        context: owner,
                        request_id: Some(playback_request_id),
                        error: PlaybackError::Cancelled(
                            "scheduled preload is no longer ready".to_string(),
                        ),
                    });
                    continue;
                }
                if let Some(error) = preload_promotion_error(&preloaded_sinks, &identity) {
                    tracing::warn!(
                        request_id = identity.request_id,
                        generation = identity.generation.0,
                        %error,
                        "Rejected unhealthy streaming preload before scheduling promotion"
                    );
                    release_preloaded_identity(&mut preloaded_sinks, &identity);
                    let _ = event_tx.send(AudioEvent::Error {
                        context: owner,
                        request_id: Some(playback_request_id),
                        error,
                    });
                    continue;
                }
                if let Some(existing) = scheduled_transition.as_ref() {
                    if existing.identity == identity
                        && existing.transition.group == transition.group
                    {
                        continue;
                    }
                    if existing.identity == identity {
                        let existing = scheduled_transition
                            .take()
                            .expect("scheduled transition exists");
                        transition_scheduler.cancel(existing.transition.group);
                    } else {
                        cancel_scheduled_transition(
                            &mut scheduled_transition,
                            &mut transition_scheduler,
                            &mut preloaded_sinks,
                        );
                    }
                }
                transition_scheduler.schedule(transition.group, trigger_at);
                scheduled_transition = Some(ScheduledPreloadedTransition {
                    owner,
                    identity,
                    playback_request_id,
                    fade_in,
                    transition,
                });
            }

            AudioCommand::CancelScheduledTransition { owner } => {
                if scheduled_transition
                    .as_ref()
                    .is_some_and(|scheduled| scheduled.owner == owner)
                {
                    cancel_scheduled_transition(
                        &mut scheduled_transition,
                        &mut transition_scheduler,
                        &mut preloaded_sinks,
                    );
                }
            }

            AudioCommand::ReleasePreload { identity } => {
                if scheduled_transition
                    .as_ref()
                    .is_some_and(|scheduled| scheduled.identity == identity)
                {
                    cancel_scheduled_transition(
                        &mut scheduled_transition,
                        &mut transition_scheduler,
                        &mut preloaded_sinks,
                    );
                    continue;
                }
                // Exact-identity cleanup is allowed for stale generations; unlike publication,
                // releasing an old sink must not leak resources after generation rollover.
                if preloaded_sinks
                    .get(&identity.request_id)
                    .is_some_and(|preloaded| preloaded.identity == identity)
                {
                    let preloaded = preloaded_sinks
                        .remove(&identity.request_id)
                        .expect("preload exists");
                    cancel_preloaded_streaming_buffer(&preloaded);
                    tracing::debug!("Released stale preload sink: identity={:?}", identity);
                }
            }

            AudioCommand::SwitchDevice { device_name } => {
                // Clear all preloaded sinks when switching device (they use old mixer)
                for preloaded in preloaded_sinks.values() {
                    cancel_preloaded_streaming_buffer(preloaded);
                }
                preloaded_sinks.clear();
                cancel_current_streaming_buffer(&mut current_buffer);
                current_context = None;
                handle_switch_device(&mut player, &event_tx, &state, device_name);
            }

            AudioCommand::LatestMailboxWake => {}

            AudioCommand::BufferDataAvailable => {
                while let Some(update) = buffer_mailbox.take_latest() {
                    handle_buffer_data_update(
                        &mut player,
                        &event_tx,
                        &state,
                        &generation,
                        current_buffer.as_ref(),
                        update,
                    );
                }
            }
        }

        if run_control_maintenance(
            &mut player,
            &buffer_mailbox,
            &latest_controls,
            &event_tx,
            &state,
            &generation,
            &preparation_pool,
            &mut current_buffer,
            &mut current_context,
            &mut finished_guard,
            &mut preloaded_sinks,
            &mut scheduled_transition,
            &mut transition_scheduler,
            maintenance_due,
        ) {
            break;
        }
    }

    tracing::info!("Audio thread exiting (command channel closed)");
}

#[expect(
    clippy::too_many_arguments,
    reason = "maintenance operates on actor-owned clocks and mailboxes; remove when ControlMaintenanceContext is introduced"
)]
fn run_control_maintenance(
    player: &mut AudioPlayer,
    buffer_mailbox: &BufferDataMailbox,
    latest_controls: &LatestControlMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    preparation_pool: &StreamingPreparationPool,
    current_buffer: &mut Option<SharedBuffer>,
    current_context: &mut Option<super::identity::PlaybackContext>,
    finished_guard: &mut FinishedGuard,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    scheduled_transition: &mut Option<ScheduledPreloadedTransition>,
    transition_scheduler: &mut super::automix::AudioClockScheduler,
    force_tick: bool,
) -> bool {
    let (latest_volume, tick_pending, shutdown) = latest_controls.take();
    if let Some(volume) = latest_volume {
        player.set_volume(volume);
        state.set_volume(volume);
    }
    while let Some(update) = buffer_mailbox.take_latest() {
        handle_buffer_data_update(
            player,
            event_tx,
            state,
            generation,
            current_buffer.as_ref(),
            update,
        );
    }
    let stream_terminated = terminate_terminal_stream(
        player,
        event_tx,
        state,
        generation,
        current_buffer,
        current_context,
    );
    if (tick_pending || force_tick) && !stream_terminated {
        process_tick(
            player,
            state,
            event_tx,
            current_buffer.as_ref(),
            current_context.as_ref(),
        );
    }
    if let Some(context) = shutdown {
        preparation_pool.cancel_active_playback();
        cancel_scheduled_transition(scheduled_transition, transition_scheduler, preloaded_sinks);
        handle_shutdown(
            player,
            event_tx,
            state,
            current_buffer,
            preloaded_sinks,
            context,
        );
        return true;
    }

    if let Some(action) = transition_scheduler.poll(player.get_info().position)
        && scheduled_transition
            .as_ref()
            .is_some_and(|scheduled| scheduled.transition.group == action.group)
    {
        let scheduled = scheduled_transition
            .take()
            .expect("matching scheduled transition exists");
        let owner_current = generation.accepts(&scheduled.owner);
        if owner_current
            && generation.accepts_preload(&scheduled.identity)
            && preloaded_sinks
                .get(&scheduled.identity.request_id)
                .is_some_and(|preloaded| preloaded.identity == scheduled.identity)
        {
            match take_healthy_preloaded_sink(preloaded_sinks, &scheduled.identity) {
                Err(failure) => {
                    tracing::warn!(
                        request_id = scheduled.identity.request_id,
                        generation = scheduled.identity.generation.0,
                        error = %failure.error,
                        "Rejected preload at scheduled promotion deadline"
                    );
                    let _ = event_tx.send(AudioEvent::Error {
                        context: scheduled.owner,
                        request_id: Some(scheduled.playback_request_id),
                        error: failure.error,
                    });
                }
                Ok(preloaded) => {
                    let will_overlap = match player.preloaded_will_overlap(&scheduled.transition) {
                        Ok(will_overlap) => will_overlap,
                        Err(error) => {
                            clear_preloaded_buffer_callback(&preloaded);
                            let _ = event_tx.send(AudioEvent::Error {
                                context: scheduled.owner,
                                request_id: Some(scheduled.playback_request_id),
                                error,
                            });
                            return false;
                        }
                    };
                    if let Some(context) =
                        generation.activate_preloaded_generation(&scheduled.identity)
                    {
                        let mut transition = scheduled.transition;
                        if action.underrun
                            && transition.kind == super::automix::TransitionKind::Automix
                        {
                            transition = super::automix::TransitionDirective::baseline_natural(
                                transition.group,
                            );
                        }
                        let _ = finished_guard
                            .try_mark(context.generation, FinishReason::TransitionDisposed);
                        finished_guard.reset();
                        let outgoing_shared_buffer = if will_overlap {
                            current_buffer
                                .take()
                                .inspect(SharedBuffer::clear_buffer_callback)
                        } else {
                            cancel_current_streaming_buffer(current_buffer);
                            None
                        };
                        *current_buffer = preloaded.shared_buffer.clone();
                        *current_context = Some(context.clone());
                        handle_play_preloaded(
                            player,
                            buffer_mailbox,
                            event_tx,
                            state,
                            context,
                            scheduled.playback_request_id,
                            preloaded,
                            outgoing_shared_buffer,
                            scheduled.fade_in,
                            transition,
                        );
                    } else {
                        clear_preloaded_buffer_callback(&preloaded);
                    }
                }
            }
        } else {
            release_preloaded_identity(preloaded_sinks, &scheduled.identity);
            if owner_current {
                let _ = event_tx.send(AudioEvent::Error {
                    context: scheduled.owner,
                    request_id: Some(scheduled.playback_request_id),
                    error: PlaybackError::Cancelled(
                        "scheduled preload became unavailable before its deadline".to_string(),
                    ),
                });
            }
        }
    }

    let _ = player.poll_transition();

    if player.poll_pending_pause() {
        update_state_from_player(player, state);
        if let Some(context) = current_context.as_ref()
            && generation.accepts(context)
        {
            let _ = event_tx.send(AudioEvent::Paused {
                context: context.clone(),
                request_id: None,
                position: player.get_info().position,
            });
        }
    }

    check_playback_finished(
        player,
        event_tx,
        state,
        current_buffer.as_ref(),
        current_context.as_ref(),
        generation,
        finished_guard,
    );
    false
}

fn terminate_terminal_stream(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    current_buffer: &mut Option<SharedBuffer>,
    current_context: &mut Option<super::identity::PlaybackContext>,
) -> bool {
    let Some(buffer) = current_buffer.as_ref() else {
        return false;
    };
    let Some(context) = current_context.as_ref() else {
        return false;
    };
    if !generation.accepts(context) {
        return false;
    }

    let terminal_error = match terminal_stream_state(buffer.health()) {
        TerminalStreamState::Healthy => return false,
        TerminalStreamState::Cancelled => None,
        TerminalStreamState::Failed(error) => Some(error),
    };
    let context = context.clone();

    cancel_current_streaming_buffer(current_buffer);
    *current_context = None;
    player.stop();
    update_state_from_player(player, state);
    state.set_current_path(None);

    tracing::warn!(
        generation = context.generation.0,
        cancelled = terminal_error.is_none(),
        error = terminal_error
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            .unwrap_or("cancelled"),
        "Terminated non-refillable streaming playback"
    );
    let _ = event_tx.send(AudioEvent::Stopped {
        context: context.clone(),
    });
    if let Some(error) = terminal_error {
        let _ = event_tx.send(AudioEvent::Error {
            context,
            request_id: None,
            error,
        });
    }
    true
}

#[derive(Debug, PartialEq, Eq)]
enum TerminalStreamState {
    Healthy,
    Cancelled,
    Failed(PlaybackError),
}

fn terminal_stream_state(health: SharedBufferHealth) -> TerminalStreamState {
    match health {
        SharedBufferHealth::Refillable | SharedBufferHealth::Complete => {
            TerminalStreamState::Healthy
        }
        SharedBufferHealth::Cancelled => TerminalStreamState::Cancelled,
        SharedBufferHealth::Failed(error) => TerminalStreamState::Failed(error),
        SharedBufferHealth::CoordinatorStopped => {
            TerminalStreamState::Failed(PlaybackError::StreamingFailed(
                "streaming coordinator stopped before completion".to_string(),
            ))
        }
    }
}

fn preload_promotion_error(
    preloaded_sinks: &HashMap<u64, PreloadedSink>,
    identity: &super::identity::PreloadIdentity,
) -> Option<PlaybackError> {
    let preloaded = preloaded_sinks.get(&identity.request_id)?;
    if preloaded.identity != *identity {
        return None;
    }
    streaming_preload_promotion_error(preloaded.shared_buffer.as_ref())
}

fn streaming_preload_promotion_error(
    shared_buffer: Option<&SharedBuffer>,
) -> Option<PlaybackError> {
    shared_buffer.and_then(|buffer| buffer.health().promotion_error())
}

struct PreloadedPromotionFailure {
    error: PlaybackError,
}

fn clear_preloaded_buffer_callback(preloaded: &PreloadedSink) {
    if let Some(reader_cancellation) = &preloaded.reader_cancellation {
        reader_cancellation.cancel();
    }
    if let Some(shared_buffer) = &preloaded.shared_buffer {
        shared_buffer.clear_buffer_callback();
    }
}

fn cancel_preloaded_streaming_buffer(preloaded: &PreloadedSink) {
    clear_preloaded_buffer_callback(preloaded);
}

/// Cancel first so a mixer blocked in `Source::next` / `SharedBuffer::read_at`
/// is notified before the owning Sink is stopped or replaced.
fn cancel_current_streaming_buffer(current_buffer: &mut Option<SharedBuffer>) {
    if let Some(buffer) = current_buffer.take() {
        buffer.clear_buffer_callback();
    }
}

/// Remove and validate a preload while the outgoing generation is still
/// authoritative. Generation promotion happens only after this succeeds, so
/// a Ready-then-failed stream cannot relabel the outgoing Sink or publish
/// `Started` before the app receives its recoverable preload error.
fn take_healthy_preloaded_sink(
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    identity: &super::identity::PreloadIdentity,
) -> Result<PreloadedSink, PreloadedPromotionFailure> {
    let Some(preloaded) = preloaded_sinks.remove(&identity.request_id) else {
        return Err(PreloadedPromotionFailure {
            error: PlaybackError::SourceUnavailable(format!(
                "preloaded sink {} was not found",
                identity.request_id
            )),
        });
    };

    if preloaded.identity != *identity {
        clear_preloaded_buffer_callback(&preloaded);
        return Err(PreloadedPromotionFailure {
            error: PlaybackError::InvariantViolation(format!(
                "preloaded sink identity mismatch: {}",
                identity.request_id
            )),
        });
    }

    if let Some(error) = streaming_preload_promotion_error(preloaded.shared_buffer.as_ref()) {
        clear_preloaded_buffer_callback(&preloaded);
        return Err(PreloadedPromotionFailure { error });
    }

    Ok(preloaded)
}

fn streaming_playback_preparation_error(shared_buffer: &SharedBuffer) -> Option<PlaybackError> {
    match shared_buffer.health() {
        SharedBufferHealth::Refillable | SharedBufferHealth::Complete => None,
        SharedBufferHealth::Cancelled => Some(PlaybackError::Cancelled(
            "streaming playback preparation was cancelled".to_string(),
        )),
        SharedBufferHealth::Failed(error) => Some(error),
        SharedBufferHealth::CoordinatorStopped => Some(PlaybackError::StreamingFailed(
            "streaming coordinator stopped before playback preparation".to_string(),
        )),
    }
}

fn release_preloaded_identity(
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    identity: &super::identity::PreloadIdentity,
) {
    if preloaded_sinks
        .get(&identity.request_id)
        .is_some_and(|preloaded| preloaded.identity == *identity)
        && let Some(preloaded) = preloaded_sinks.remove(&identity.request_id)
    {
        clear_preloaded_buffer_callback(&preloaded);
    }
}

fn cancel_scheduled_transition(
    scheduled: &mut Option<ScheduledPreloadedTransition>,
    scheduler: &mut super::automix::AudioClockScheduler,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
) {
    if let Some(cancelled) = scheduled.take() {
        scheduler.cancel(cancelled.transition.group);
        release_preloaded_identity(preloaded_sinks, &cancelled.identity);
    }
}

fn process_tick(
    player: &mut AudioPlayer,
    state: &SharedPlaybackState,
    event_tx: &AudioEventSender,
    current_buffer: Option<&SharedBuffer>,
    current_context: Option<&super::identity::PlaybackContext>,
) {
    if let Some(buf) = current_buffer
        && let Some(context) = current_context.cloned()
    {
        check_buffer_status(player, state, event_tx, buf, context);
    }

    let info = player.get_info();
    state.set_position(info.position);
    let current_status = state.get_info().status;
    if !matches!(
        current_status,
        PlaybackStatus::Buffering { .. } | PlaybackStatus::Paused
    ) {
        state.set_status(info.status);
    }
}

fn handle_shutdown(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    current_buffer: &mut Option<SharedBuffer>,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    context: super::identity::PlaybackContext,
) {
    cancel_current_streaming_buffer(current_buffer);
    for (_, preloaded) in preloaded_sinks.drain() {
        cancel_preloaded_streaming_buffer(&preloaded);
    }
    player.stop();
    update_state_from_player(player, state);
    state.set_current_path(None);
    let _ = event_tx.send(AudioEvent::Stopped { context });
}

// ============ Command Handlers ============

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the AudioCommand::Play protocol payload; remove when that variant carries a named request"
)]
fn handle_play(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    request_id: u64,
    path: PathBuf,
    fade_in: bool,
    track_gain: f32,
) {
    state.set_cache_bytes(0, 0);
    state.set_buffered_ahead_bytes(0);

    match player.play_with_fade(path.clone(), fade_in, track_gain) {
        Ok(_) => {
            update_state_from_player(player, state);
            state.set_current_path(Some(path.clone()));
            let _ = event_tx.send(AudioEvent::Started {
                context: context.clone(),
                request_id,
                path: Some(path),
            });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(request_id),
                error: e,
            });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the AudioCommand::LoadPaused payload; remove when that variant carries a named request"
)]
fn handle_load_paused(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    request_id: u64,
    path: PathBuf,
    position: Duration,
    track_gain: f32,
) {
    state.set_cache_bytes(0, 0);
    state.set_buffered_ahead_bytes(0);

    match player.load_paused(path.clone(), position, track_gain) {
        Ok(_) => {
            update_state_from_player(player, state);
            state.set_current_path(Some(path));
            let _ = event_tx.send(AudioEvent::Paused {
                context: context.clone(),
                request_id: Some(request_id),
                position: player.get_info().position,
            });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(request_id),
                error: e,
            });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the AudioCommand::PlayAt payload; remove when that variant carries a named request"
)]
fn handle_play_at(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    request_id: u64,
    path: PathBuf,
    position: Duration,
    fade_in: bool,
    track_gain: f32,
) {
    state.set_cache_bytes(0, 0);
    state.set_buffered_ahead_bytes(0);

    match player.play_from_position_with_fade(path.clone(), position, fade_in, track_gain) {
        Ok(seek_error) => {
            update_state_from_player(player, state);
            state.set_current_path(Some(path.clone()));
            let _ = event_tx.send(AudioEvent::Started {
                context: context.clone(),
                request_id,
                path: Some(path),
            });
            if let Some(error) = seek_error {
                let _ = event_tx.send(AudioEvent::Error {
                    context: context.clone(),
                    request_id: Some(request_id),
                    error,
                });
            }
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(request_id),
                error: e,
            });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "stream preparation completion joins actor state with an immutable result; remove when a preparation commit context owns the actor references"
)]
fn handle_streaming_preparation_result(
    prepared: StreamingPreparationResult,
    player: &mut AudioPlayer,
    buffer_mailbox: &BufferDataMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    preparation_pool: &StreamingPreparationPool,
    current_buffer: &mut Option<SharedBuffer>,
    current_context: &mut Option<super::identity::PlaybackContext>,
    pause_after_preparation: &mut Option<super::identity::PlaybackContext>,
    finished_guard: &mut FinishedGuard,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    scheduled_transition: &mut Option<ScheduledPreloadedTransition>,
    transition_scheduler: &mut super::automix::AudioClockScheduler,
) {
    match prepared {
        StreamingPreparationResult::Playback(prepared) => {
            let PreparedPlayback {
                context,
                request_id,
                shared_buffer,
                reader_cancellation,
                source,
                duration,
                cache_path,
                track_gain,
                mode,
            } = prepared;
            // Once this exact preparation result reaches the actor, its
            // reader token is no longer worker-owned. Do not let a later
            // playback command cancel a successfully installed current reader
            // through stale preparation-pool state.
            preparation_pool.clear_active_playback(&reader_cancellation);
            if !generation.accepts(&context) || context.cancellation.is_cancelled() {
                tracing::debug!(
                    request_id,
                    generation = context.generation.0,
                    "Discarded stale streaming playback preparation"
                );
                take_pending_preparation_pause(pause_after_preparation, &context);
                return;
            }

            let pause_requested = take_pending_preparation_pause(pause_after_preparation, &context);

            if let Some(error) = streaming_playback_preparation_error(&shared_buffer) {
                tracing::warn!(
                    request_id,
                    generation = context.generation.0,
                    %error,
                    "Rejected terminal streaming playback preparation"
                );
                let _ = event_tx.send(AudioEvent::Error {
                    context,
                    request_id: Some(request_id),
                    error,
                });
                return;
            }

            let source = match source {
                Ok(source) => source,
                Err(error) => {
                    let _ = event_tx.send(AudioEvent::Error {
                        context,
                        request_id: Some(request_id),
                        error,
                    });
                    return;
                }
            };

            cancel_scheduled_transition(
                scheduled_transition,
                transition_scheduler,
                preloaded_sinks,
            );
            finished_guard.reset();
            cancel_current_streaming_buffer(current_buffer);

            let loaded = match mode {
                PlaybackPreparationMode::Play { fade_in } => {
                    if pause_requested {
                        handle_load_paused_streaming(
                            player,
                            buffer_mailbox,
                            event_tx,
                            state,
                            context.clone(),
                            request_id,
                            source,
                            reader_cancellation.clone(),
                            shared_buffer.clone(),
                            duration,
                            cache_path,
                            Duration::ZERO,
                            track_gain,
                        )
                    } else {
                        handle_play_streaming(
                            player,
                            buffer_mailbox,
                            event_tx,
                            state,
                            context.clone(),
                            request_id,
                            source,
                            reader_cancellation.clone(),
                            shared_buffer.clone(),
                            duration,
                            cache_path,
                            fade_in,
                            track_gain,
                        )
                    }
                }
                PlaybackPreparationMode::LoadPaused { position } => handle_load_paused_streaming(
                    player,
                    buffer_mailbox,
                    event_tx,
                    state,
                    context.clone(),
                    request_id,
                    source,
                    reader_cancellation.clone(),
                    shared_buffer.clone(),
                    duration,
                    cache_path,
                    position,
                    track_gain,
                ),
            };

            if loaded {
                *current_buffer = Some(shared_buffer);
                *current_context = Some(context);
            } else {
                *current_context = None;
            }
        }
        StreamingPreparationResult::Preload(prepared) => {
            let PreparedPreload {
                identity,
                shared_buffer,
                reader_cancellation,
                source,
                duration,
                track_gain,
            } = prepared;
            if !generation.accepts_preload(&identity) || identity.cancellation.is_cancelled() {
                tracing::debug!(
                    request_id = identity.request_id,
                    generation = identity.generation.0,
                    "Discarded stale streaming preload preparation"
                );
                return;
            }
            if let Some(error) = shared_buffer.health().promotion_error() {
                tracing::warn!(
                    request_id = identity.request_id,
                    generation = identity.generation.0,
                    %error,
                    "Discarded unhealthy streaming preload preparation"
                );
                let _ = event_tx.send(AudioEvent::PreloadFailed { identity, error });
                return;
            }
            match source {
                Ok(source) => handle_create_preload_sink_streaming(
                    player,
                    event_tx,
                    preloaded_sinks,
                    identity,
                    source,
                    shared_buffer,
                    reader_cancellation,
                    duration,
                    track_gain,
                ),
                Err(error) => {
                    let _ = event_tx.send(AudioEvent::PreloadFailed { identity, error });
                }
            }
        }
    }
}

fn take_pending_preparation_pause(
    pending: &mut Option<super::identity::PlaybackContext>,
    context: &super::identity::PlaybackContext,
) -> bool {
    if pending.as_ref() == Some(context) {
        *pending = None;
        true
    } else {
        false
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the streaming play protocol payload; remove when the command carries a named streaming request"
)]
fn handle_play_streaming(
    player: &mut AudioPlayer,
    buffer_mailbox: &BufferDataMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    request_id: u64,
    source: PreparedStreamingSource,
    reader_cancellation: StreamingReaderCancellation,
    shared_buffer: SharedBuffer,
    duration: Duration,
    cache_path: Option<PathBuf>,
    fade_in: bool,
    track_gain: f32,
) -> bool {
    // Clear pending seek from previous track (important for correct display_position)

    // Reset buffer state from previous track before setting up new callback
    // This ensures UI shows fresh progress for the new track
    state.set_cache_bytes(0, 0);

    // Set up buffer callback to send BufferDataAvailable command
    setup_buffer_callback(&shared_buffer, buffer_mailbox, &context);

    // Initialize buffer progress (may be 0 if HTTP response not yet received)
    let cached = shared_buffer.cached_bytes();
    let total = shared_buffer.total_size();
    state.set_cache_bytes(cached, total);
    state.set_buffered_ahead_bytes(shared_buffer.buffered_ahead());

    // Send initial progress event if we have data
    if total > 0 {
        let progress = cached as f32 / total as f32;
        let _ = event_tx.send(AudioEvent::CacheProgress {
            context: context.clone(),
            cached,
            total,
            progress,
        });
    }

    match player.play_prepared_streaming(
        source,
        reader_cancellation,
        duration,
        cache_path.clone(),
        fade_in,
        track_gain,
    ) {
        Ok(_) => {
            update_state_from_player(player, state);
            state.set_current_path(cache_path);
            let _ = event_tx.send(AudioEvent::Started {
                context: context.clone(),
                request_id,
                path: None,
            });
            true
        }
        Err(e) => {
            shared_buffer.clear_buffer_callback();
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(request_id),
                error: e,
            });
            false
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the paused-stream protocol payload; remove when the command carries a named streaming request"
)]
fn handle_load_paused_streaming(
    player: &mut AudioPlayer,
    buffer_mailbox: &BufferDataMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    request_id: u64,
    source: PreparedStreamingSource,
    reader_cancellation: StreamingReaderCancellation,
    shared_buffer: SharedBuffer,
    duration: Duration,
    cache_path: Option<PathBuf>,
    position: Duration,
    track_gain: f32,
) -> bool {
    state.set_cache_bytes(0, 0);

    setup_buffer_callback(&shared_buffer, buffer_mailbox, &context);

    let cached = shared_buffer.cached_bytes();
    let total = shared_buffer.total_size();
    state.set_cache_bytes(cached, total);
    state.set_buffered_ahead_bytes(shared_buffer.buffered_ahead());

    if total > 0 {
        let progress = cached as f32 / total as f32;
        let _ = event_tx.send(AudioEvent::CacheProgress {
            context: context.clone(),
            cached,
            total,
            progress,
        });
    }

    match player.load_prepared_streaming_paused(
        source,
        reader_cancellation,
        duration,
        cache_path.clone(),
        position,
        track_gain,
    ) {
        Ok(seek_error) => {
            update_state_from_player(player, state);
            state.set_current_path(cache_path);
            let _ = event_tx.send(AudioEvent::Paused {
                context: context.clone(),
                request_id: Some(request_id),
                position: player.get_info().position,
            });
            if let Some(error) = seek_error {
                let _ = event_tx.send(AudioEvent::Error {
                    context: context.clone(),
                    request_id: Some(request_id),
                    error,
                });
            }
            true
        }
        Err(e) => {
            shared_buffer.clear_buffer_callback();
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(request_id),
                error: e,
            });
            false
        }
    }
}

fn handle_seek(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    context: super::identity::PlaybackContext,
    nonce: super::identity::SeekNonce,
    position: Duration,
) {
    if !generation.accepts_seek(&context, nonce) {
        return;
    }
    let _ = event_tx.send(AudioEvent::SeekStarted {
        context: context.clone(),
        nonce,
        target_position: position,
    });

    match player.seek(position) {
        Ok(_) => {
            if !generation.accepts_seek(&context, nonce) {
                return;
            }
            state.set_position(position);
            let _ = event_tx.send(AudioEvent::SeekComplete {
                context: context.clone(),
                nonce,
                position,
            });
        }
        Err(e) => {
            if !generation.accepts_seek(&context, nonce) {
                return;
            }
            let _ = event_tx.send(AudioEvent::SeekFailed {
                context: context.clone(),
                nonce,
                error: e,
            });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "seek submission captures one immutable worker request plus routing handles; remove when StreamingSeekRequest owns the remaining routing data"
)]
fn submit_streaming_seek(
    worker: &StreamingSeekWorker,
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    current_buffer: &mut Option<SharedBuffer>,
    current_context: &mut Option<super::identity::PlaybackContext>,
    request: StreamingSeekRequest,
) {
    if let Err(error) = worker.try_submit(request) {
        let (error, request) = *error;
        cancel_streaming_seek_runtime(&request.reader_cancellation, &request.shared_buffer);
        if generation.accepts_seek(&request.context, request.nonce)
            && current_context.as_ref() == Some(&request.context)
        {
            cancel_current_streaming_buffer(current_buffer);
            *current_context = None;
            player.stop();
            update_state_from_player(player, state);
            state.set_current_path(None);
            let _ = event_tx.send(AudioEvent::SeekFailed {
                context: request.context.clone(),
                nonce: request.nonce,
                error,
            });
            let _ = event_tx.send(AudioEvent::Stopped {
                context: request.context,
            });
        }
        request.sink.stop();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "seek completion atomically validates actor generation and installs the result; remove when a seek commit context owns the actor references"
)]
fn handle_streaming_seek_result(
    result: StreamingSeekResult,
    player: &mut AudioPlayer,
    worker: &StreamingSeekWorker,
    buffer_mailbox: &BufferDataMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    current_buffer: &mut Option<SharedBuffer>,
    current_context: &mut Option<super::identity::PlaybackContext>,
    deferred_streaming_seeks: &mut HashMap<
        super::identity::PlaybackGeneration,
        DeferredStreamingSeek,
    >,
) {
    let StreamingSeekResult {
        context,
        nonce,
        target_position,
        source,
        sink,
        reader_cancellation,
        shared_buffer,
        duration,
        cache_path,
        track_gain,
        was_paused,
    } = result;
    worker.clear_active();

    if let Some(deferred) = deferred_streaming_seeks.remove(&context.generation)
        && generation.accepts_seek(&deferred.context, deferred.nonce)
        && current_context.as_ref() == Some(&deferred.context)
        && current_buffer.as_ref().is_some_and(|buffer| {
            matches!(
                buffer.health(),
                SharedBufferHealth::Refillable | SharedBufferHealth::Complete
            )
        })
    {
        reader_cancellation.cancel();
        let buffer = StreamingBuffer::new(shared_buffer.clone());
        let next_reader_cancellation = buffer.reader_cancellation();
        submit_streaming_seek(
            worker,
            player,
            event_tx,
            state,
            generation,
            current_buffer,
            current_context,
            StreamingSeekRequest {
                context: deferred.context,
                nonce: deferred.nonce,
                position: deferred.position,
                sink,
                buffer,
                reader_cancellation: next_reader_cancellation,
                shared_buffer,
                duration,
                cache_path,
                track_gain,
                was_paused,
            },
        );
        return;
    }

    let owner_current = generation.accepts_seek(&context, nonce)
        && current_context.as_ref() == Some(&context)
        && current_buffer.as_ref().is_some_and(|buffer| {
            matches!(
                buffer.health(),
                SharedBufferHealth::Refillable | SharedBufferHealth::Complete
            )
        });
    if !owner_current {
        cancel_streaming_seek_runtime(&reader_cancellation, &shared_buffer);
        if current_context.as_ref() == Some(&context) {
            cancel_current_streaming_buffer(current_buffer);
            player.cancel_current_streaming_reader();
            sink.stop();
            *current_context = None;
            player.stop();
            update_state_from_player(player, state);
            state.set_current_path(None);
        } else {
            sink.stop();
        }
        return;
    }

    match source {
        Ok(source) => {
            player.cancel_current_streaming_reader();
            sink.stop();
            if let Err(error) = player.install_preseeked_streaming_source(
                source,
                reader_cancellation.clone(),
                duration,
                cache_path,
                target_position,
                track_gain,
                was_paused,
            ) {
                tracing::warn!(
                    generation = context.generation.0,
                    nonce = nonce.0,
                    target_ms = target_position.as_millis(),
                    %error,
                    "Streaming seek handoff failed"
                );
                reader_cancellation.cancel();
                cancel_current_streaming_buffer(current_buffer);
                *current_context = None;
                player.stop();
                update_state_from_player(player, state);
                state.set_current_path(None);
                let _ = event_tx.send(AudioEvent::SeekFailed {
                    context: context.clone(),
                    nonce,
                    error,
                });
                let _ = event_tx.send(AudioEvent::Stopped { context });
                return;
            }
            setup_buffer_callback(&shared_buffer, buffer_mailbox, &context);
            update_state_from_player(player, state);
            let _ = event_tx.send(AudioEvent::SeekComplete {
                context,
                nonce,
                position: target_position,
            });
        }
        Err(error) => {
            tracing::warn!(
                generation = context.generation.0,
                nonce = nonce.0,
                target_ms = target_position.as_millis(),
                %error,
                "Streaming seek failed; stopping the detached runtime"
            );
            reader_cancellation.cancel();
            cancel_current_streaming_buffer(current_buffer);
            player.cancel_current_streaming_reader();
            sink.stop();
            *current_context = None;
            player.stop();
            update_state_from_player(player, state);
            state.set_current_path(None);
            let _ = event_tx.send(AudioEvent::SeekFailed {
                context: context.clone(),
                nonce,
                error,
            });
            let _ = event_tx.send(AudioEvent::Stopped { context });
        }
    }
}

fn handle_create_preload_sink(
    player: &AudioPlayer,
    event_tx: &AudioEventSender,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    identity: super::identity::PreloadIdentity,
    path: PathBuf,
    track_gain: f32,
) {
    let request_id = identity.request_id;
    match player.create_preload_sink(&path, track_gain) {
        Ok((sink, duration, runtime)) => {
            // Store the sink for later playback
            preloaded_sinks.insert(
                request_id,
                PreloadedSink {
                    identity: identity.clone(),
                    sink,
                    duration,
                    path: path.clone(),
                    track_gain,
                    runtime,
                    is_streaming: false,
                    shared_buffer: None, // Local files don't have shared buffer
                    reader_cancellation: None,
                },
            );
            tracing::debug!(
                "Preload sink created: request_id={}, path={:?}",
                request_id,
                path
            );
            let _ = event_tx.send(AudioEvent::PreloadReady {
                identity,
                duration,
                path,
            });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::PreloadFailed { identity, error: e });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "preload sink creation mirrors the audio command contract; remove when the command carries a named preload request"
)]
fn handle_create_preload_sink_streaming(
    player: &AudioPlayer,
    event_tx: &AudioEventSender,
    preloaded_sinks: &mut HashMap<u64, PreloadedSink>,
    identity: super::identity::PreloadIdentity,
    source: PreparedStreamingSource,
    shared_buffer: SharedBuffer,
    reader_cancellation: StreamingReaderCancellation,
    duration: Duration,
    track_gain: f32,
) {
    let request_id = identity.request_id;
    match player.create_preload_sink_prepared_streaming(source, duration, track_gain) {
        Ok((sink, actual_duration, runtime)) => {
            // For streaming, we don't have a real path, use a placeholder
            let path = PathBuf::from(format!("streaming://{}", request_id));
            preloaded_sinks.insert(
                request_id,
                PreloadedSink {
                    identity: identity.clone(),
                    sink,
                    duration: actual_duration,
                    path: path.clone(),
                    track_gain,
                    runtime,
                    is_streaming: true,
                    shared_buffer: Some(shared_buffer), // Save for callback setup on play
                    reader_cancellation: Some(reader_cancellation),
                },
            );
            tracing::debug!("Preload streaming sink created: request_id={}", request_id);
            let _ = event_tx.send(AudioEvent::PreloadReady {
                identity,
                duration: actual_duration,
                path,
            });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::PreloadFailed { identity, error: e });
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "handler mirrors the preloaded-play command payload; remove when the command carries a named handoff request"
)]
fn handle_play_preloaded(
    player: &mut AudioPlayer,
    buffer_mailbox: &BufferDataMailbox,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    context: super::identity::PlaybackContext,
    playback_request_id: u64,
    preloaded: PreloadedSink,
    outgoing_shared_buffer: Option<SharedBuffer>,
    fade_in: bool,
    transition: super::automix::TransitionDirective,
) {
    let PreloadedSink {
        identity,
        sink,
        duration,
        path,
        track_gain,
        runtime,
        is_streaming,
        shared_buffer,
        reader_cancellation,
    } = preloaded;
    let request_id = identity.request_id;

    // Clear pending seek from previous track (important for correct display_position)

    // Set up buffer callback for streaming preloads
    if let Some(shared_buffer) = &shared_buffer {
        // Reset buffer progress for new track
        let cached = shared_buffer.cached_bytes();
        let total = shared_buffer.total_size();
        state.set_cache_bytes(cached, total);
        state.set_buffered_ahead_bytes(shared_buffer.buffered_ahead());

        // Set up callback to send BufferDataAvailable command (DRY: single callback setup point)
        setup_buffer_callback(shared_buffer, buffer_mailbox, &context);

        tracing::info!(
            request_id,
            cached,
            total,
            "Set up promoted streaming preload callback"
        );
    } else {
        state.set_cache_bytes(0, 0);
        state.set_buffered_ahead_bytes(0);
    }

    match player.play_preloaded_sink(
        sink,
        duration,
        path.clone(),
        is_streaming,
        fade_in,
        track_gain,
        runtime,
        reader_cancellation,
        outgoing_shared_buffer,
        transition,
    ) {
        Ok(_) => {
            update_state_from_player(player, state);
            state.set_current_path(Some(path.clone()));
            let _ = event_tx.send(AudioEvent::Started {
                context: context.clone(),
                request_id: playback_request_id,
                path: Some(path),
            });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::Error {
                context: context.clone(),
                request_id: Some(playback_request_id),
                error: e,
            });
        }
    }
}

fn handle_switch_device(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    device_name: Option<String>,
) {
    match player.switch_device(device_name.as_deref()) {
        Ok(restore_state) => {
            update_state_from_player(player, state);
            let _ = event_tx.send(AudioEvent::DeviceSwitched { restore_state });
        }
        Err(e) => {
            let _ = event_tx.send(AudioEvent::DeviceSwitchFailed { error: e });
        }
    }
}

// ============ Helpers ============

/// Set up buffer callback to send BufferDataAvailable command
///
/// This is the single callback setup point (DRY principle).
/// The callback only sends commands to Audio Thread, it does not modify state directly.
fn setup_buffer_callback(
    shared_buffer: &SharedBuffer,
    buffer_mailbox: &BufferDataMailbox,
    context: &super::identity::PlaybackContext,
) {
    let mailbox = buffer_mailbox.clone();
    let context = context.clone();
    shared_buffer.set_buffer_callback(move |event| {
        use super::streaming::BufferEvent;
        if let BufferEvent::CacheAdvanced { cached, total } = event {
            mailbox.publish(BufferDataUpdate {
                context: context.clone(),
                cached,
                total,
            });
        }
    });
}

fn handle_buffer_data_update(
    player: &mut AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    generation: &super::identity::PlaybackGenerationController,
    current_buffer: Option<&SharedBuffer>,
    update: BufferDataUpdate,
) {
    let context = update.context;
    if context.cancellation.is_cancelled() || !generation.accepts(&context) {
        return;
    }

    let cached = update.cached;
    let total = update.total;
    state.set_cache_bytes(cached, total);

    let progress = if total > 0 {
        cached as f32 / total as f32
    } else {
        0.0
    };
    let _ = event_tx.send(AudioEvent::CacheProgress {
        context: context.clone(),
        cached,
        total,
        progress,
    });

    if let Some(buf) = current_buffer
        && let PlaybackStatus::Buffering { .. } = state.get_info().status
    {
        let remaining_bytes = buf.buffered_ahead();
        state.set_buffered_ahead_bytes(remaining_bytes);
        if buf.policy().can_start_or_resume(
            remaining_bytes,
            buf.is_complete() || buf.remote_eof_reached(),
        ) {
            exit_buffering(player, state, event_tx, context);
        }
    }
}

/// Check buffer status and enter/exit Buffering state as needed
///
/// Called periodically from Tick handler for streaming playback.
///
/// Uses hysteresis (watermark) mechanism to prevent rapid state oscillation:
/// - Enter Buffering below the policy low watermark
/// - Exit Buffering at the policy high watermark
///
/// IMPORTANT: Uses state.get_info().status (SharedPlaybackState) instead of
/// player.get_info().status because enter_buffering/exit_buffering only update
/// SharedPlaybackState, not player's internal status. Using player's status
/// would cause repeated enter_buffering calls.
fn check_buffer_status(
    player: &mut AudioPlayer,
    state: &SharedPlaybackState,
    event_tx: &AudioEventSender,
    buffer: &SharedBuffer,
    context: super::identity::PlaybackContext,
) {
    // Use SharedPlaybackState for status check (single source of truth)
    let current_status = state.get_info().status;
    // Use player for the user-visible position only.
    let player_info = player.get_info();
    let remaining_bytes = buffer.buffered_ahead();
    state.set_buffered_ahead_bytes(remaining_bytes);
    let source_has_all_required_bytes = buffer.is_complete() || buffer.remote_eof_reached();
    let policy = buffer.policy();

    match &current_status {
        PlaybackStatus::Playing
            if policy.should_enter_buffering(remaining_bytes, source_has_all_required_bytes) =>
        {
            tracing::info!(
                "Buffer low: remaining {} bytes < {} (low water mark), entering Buffering",
                remaining_bytes,
                policy.low_water_mark_bytes()
            );
            enter_buffering(
                player,
                state,
                event_tx,
                player_info.position,
                context.clone(),
            );
        }
        PlaybackStatus::Buffering { .. } => {
            // Check if we can exit Buffering (HIGH water mark)
            // Exit Buffering if:
            // 1. Remaining data reaches the policy high watermark, OR
            // 2. Download is complete (no more data coming)
            if source_has_all_required_bytes {
                tracing::info!("Streaming window reached EOF, exiting Buffering");
                exit_buffering(player, state, event_tx, context.clone());
            } else if policy.can_start_or_resume(remaining_bytes, false) {
                tracing::info!(
                    "Buffer sufficient: remaining {} bytes > {} (high water mark), exiting Buffering",
                    remaining_bytes,
                    policy.high_water_mark_bytes()
                );
                exit_buffering(player, state, event_tx, context.clone());
            }
            // Otherwise, stay in Buffering state and wait for more data
        }
        _ => {}
    }
}

/// Enter Buffering state
///
/// Pauses the Sink and sets status to Buffering.
fn enter_buffering(
    player: &mut AudioPlayer,
    state: &SharedPlaybackState,
    event_tx: &AudioEventSender,
    position: Duration,
    context: super::identity::PlaybackContext,
) {
    // Pause sink without changing player's internal status
    player.pause_sink();

    let old_status = state.get_info().status;
    let new_status = PlaybackStatus::Buffering { position };
    state.set_status(new_status.clone());

    let _ = event_tx.send(AudioEvent::BufferingStarted {
        context: context.clone(),
        position,
    });
    let _ = event_tx.send(AudioEvent::StateChanged {
        context: context.clone(),
        old_status,
        new_status,
    });

    tracing::info!("Entered Buffering state at position {:?}", position);
}

/// Exit Buffering state
///
/// Resumes the Sink and sets status to Playing. Streaming seek is owned by the
/// dedicated worker and must never be executed from this actor callback.
fn exit_buffering(
    player: &mut AudioPlayer,
    state: &SharedPlaybackState,
    event_tx: &AudioEventSender,
    context: super::identity::PlaybackContext,
) {
    let old_status = state.get_info().status;

    // Resume sink
    player.play_sink();

    state.set_status(PlaybackStatus::Playing);

    let _ = event_tx.send(AudioEvent::BufferingEnded {
        context: context.clone(),
    });
    let _ = event_tx.send(AudioEvent::StateChanged {
        context: context.clone(),
        old_status,
        new_status: PlaybackStatus::Playing,
    });

    tracing::info!("Exited Buffering state, resumed Playing");
}

/// Update shared state from player's current info
fn update_state_from_player(player: &AudioPlayer, state: &SharedPlaybackState) {
    let info = player.get_info();
    state.update_from_info(&info);
}

/// Check if playback finished and send event
///
/// For streaming playback, if sink is empty but download is not complete,
/// this indicates we've caught up with the download
fn check_playback_finished(
    player: &AudioPlayer,
    event_tx: &AudioEventSender,
    state: &SharedPlaybackState,
    current_buffer: Option<&SharedBuffer>,
    context: Option<&super::identity::PlaybackContext>,
    generation: &super::identity::PlaybackGenerationController,
    finished_guard: &mut FinishedGuard,
) {
    // Delay Finished only while the coordinator can still refill the stream.
    // Terminal buffers are handled by `terminate_terminal_stream` before this
    // check and must never be treated as indefinitely recoverable buffering.
    if current_buffer.is_some_and(|buffer| buffer.health() == SharedBufferHealth::Refillable) {
        return;
    }

    if player.is_finished()
        && let Some(context) = context
        && generation.accepts(context)
        && finished_guard.try_mark(context.generation, FinishReason::Natural)
    {
        state.set_status(PlaybackStatus::Stopped);
        let _ = event_tx.send(AudioEvent::Finished {
            context: context.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_worker_spawner(
        thread_name: &'static str,
        _operation: &'static str,
        worker: WorkerTask,
    ) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(worker)
    }

    #[test]
    fn maintenance_deadline_wakes_without_a_critical_command() {
        let (critical_tx, mut critical_rx) = audio_command_channel();
        let (result_tx, mut result_rx) =
            tokio::sync::mpsc::channel(STREAMING_PREPARATION_RESULT_CAPACITY);
        let (seek_result_tx, mut seek_result_rx) =
            tokio::sync::mpsc::channel(STREAMING_SEEK_RESULT_CAPACITY);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        let event = runtime.block_on(next_control_event(
            &mut critical_rx,
            &mut result_rx,
            &mut seek_result_rx,
            tokio::time::Instant::now() + Duration::from_millis(5),
        ));

        assert!(matches!(event, ControlEvent::Maintenance));
        assert!(critical_rx.try_recv().is_err());
        drop(critical_tx);
        drop(result_tx);
        drop(seek_result_tx);
    }

    #[test]
    fn blocked_streaming_seek_does_not_block_stop_and_reader_cancel_wakes_the_worker() {
        let buffer = SharedBuffer::new(100);
        buffer.set_coordinator_active_for_test(true);
        let (mixer, _mixer_source) = rodio::mixer::mixer(
            rodio::ChannelCount::new(1).unwrap(),
            rodio::SampleRate::new(48_000).unwrap(),
        );
        let sink = Sink::connect_new(&mixer);

        let (seek_result_tx, mut seek_result_rx) =
            tokio::sync::mpsc::channel(STREAMING_SEEK_RESULT_CAPACITY);
        let worker = StreamingSeekWorker::new(seek_result_tx, test_worker_spawner).unwrap();
        let generation = crate::identity::PlaybackGenerationController::new();
        let context = generation.activate_generation();
        let (_, nonce) = generation.seek_context().unwrap();
        let streaming_buffer = StreamingBuffer::new(buffer.clone());
        let reader_cancellation = streaming_buffer.reader_cancellation();
        assert!(
            worker
                .try_submit(StreamingSeekRequest {
                    context: context.clone(),
                    nonce,
                    position: Duration::from_secs(1),
                    sink,
                    buffer: streaming_buffer,
                    reader_cancellation,
                    shared_buffer: buffer.clone(),
                    duration: Duration::from_secs(60),
                    cache_path: None,
                    track_gain: 1.0,
                    was_paused: false,
                })
                .is_ok()
        );

        let (critical_tx, mut critical_rx) = audio_command_channel();
        let (_preparation_tx, mut preparation_rx) =
            tokio::sync::mpsc::channel(STREAMING_PREPARATION_RESULT_CAPACITY);
        let (_other_seek_tx, mut other_seek_rx) =
            tokio::sync::mpsc::channel(STREAMING_SEEK_RESULT_CAPACITY);
        let stop_context = generation.activate_generation();
        critical_tx
            .try_send(AudioCommand::Stop {
                context: stop_context.clone(),
            })
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let event = runtime.block_on(next_control_event(
            &mut critical_rx,
            &mut preparation_rx,
            &mut other_seek_rx,
            tokio::time::Instant::now() + Duration::from_millis(300),
        ));
        assert!(matches!(
            event,
            ControlEvent::Command(AudioCommand::Stop { context }) if context == stop_context
        ));

        let started = Instant::now();
        worker.cancel_active();
        let mut current_buffer = Some(buffer.clone());
        cancel_current_streaming_buffer(&mut current_buffer);
        let result = seek_result_rx
            .blocking_recv()
            .expect("cancelled streaming seek should return its Sink");
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!generation.accepts_seek(&result.context, result.nonce));
        assert!(current_buffer.is_none());
        assert_eq!(buffer.health(), SharedBufferHealth::Refillable);
        result.sink.stop();
    }

    #[test]
    fn pause_intent_is_consumed_only_by_its_preparation_context() {
        let generation = crate::identity::PlaybackGenerationController::new();
        let first = generation.activate_generation();
        let second = generation.activate_generation();
        let mut pending = Some(first.clone());

        assert!(!take_pending_preparation_pause(&mut pending, &second));
        assert_eq!(pending, Some(first.clone()));
        assert!(take_pending_preparation_pause(&mut pending, &first));
        assert!(pending.is_none());
    }

    #[test]
    fn completed_preparation_token_cannot_cancel_an_installed_reader() {
        let (result_tx, _result_rx) =
            tokio::sync::mpsc::channel(STREAMING_PREPARATION_RESULT_CAPACITY);
        let pool = StreamingPreparationPool::new(result_tx, test_worker_spawner).unwrap();
        let shared = SharedBuffer::new(16);
        let reader = StreamingBuffer::new(shared);
        let cancellation = reader.reader_cancellation();
        *pool.active_playback_cancellation.lock() = Some(cancellation.clone());

        pool.clear_active_playback(&cancellation);
        pool.cancel_active_playback();

        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn discarded_streaming_seek_cancels_only_its_reader() {
        let shared = SharedBuffer::new(16);
        shared.set_coordinator_active_for_test(true);
        let reader = StreamingBuffer::new(shared.clone());
        let cancellation = reader.reader_cancellation();

        cancel_streaming_seek_runtime(&cancellation, &shared);

        assert!(cancellation.is_cancelled());
        assert_eq!(shared.health(), SharedBufferHealth::Refillable);
    }

    #[test]
    fn preload_promotion_health_accepts_local_and_complete_streams_but_rejects_terminal_streams() {
        assert!(streaming_preload_promotion_error(None).is_none());

        let complete = SharedBuffer::new(4);
        complete.append(&[0; 4]);
        complete.mark_complete();
        assert!(streaming_preload_promotion_error(Some(&complete)).is_none());

        let failed = SharedBuffer::new(100);
        failed.set_error(PlaybackError::NetworkError("connection reset".to_string()));
        assert!(
            streaming_preload_promotion_error(Some(&failed))
                .is_some_and(|error| matches!(error, PlaybackError::UnhealthyPreload(_)))
        );

        let stopped = SharedBuffer::new(100);
        assert!(
            streaming_preload_promotion_error(Some(&stopped))
                .is_some_and(|error| matches!(error, PlaybackError::UnhealthyPreload(_)))
        );
    }

    #[test]
    fn terminal_stream_state_never_treats_failed_or_stopped_coordinators_as_buffering() {
        assert_eq!(
            terminal_stream_state(SharedBufferHealth::Refillable),
            TerminalStreamState::Healthy
        );
        assert_eq!(
            terminal_stream_state(SharedBufferHealth::Complete),
            TerminalStreamState::Healthy
        );
        assert_eq!(
            terminal_stream_state(SharedBufferHealth::Cancelled),
            TerminalStreamState::Cancelled
        );
        assert!(matches!(
            terminal_stream_state(SharedBufferHealth::Failed(PlaybackError::NetworkError(
                "network".to_string()
            ))),
            TerminalStreamState::Failed(PlaybackError::NetworkError(_))
        ));
        assert!(matches!(
            terminal_stream_state(SharedBufferHealth::CoordinatorStopped),
            TerminalStreamState::Failed(PlaybackError::StreamingFailed(_))
        ));
    }

    #[test]
    fn stalled_streaming_preparation_does_not_consume_critical_capacity() {
        let (result_tx, mut result_rx) =
            tokio::sync::mpsc::channel(STREAMING_PREPARATION_RESULT_CAPACITY);
        let pool = StreamingPreparationPool::new(result_tx, test_worker_spawner).unwrap();
        let generation = crate::identity::PlaybackGenerationController::new();
        let context = generation.activate_generation();
        let shared = SharedBuffer::new(1024);
        shared.set_coordinator_active_for_test(true);

        let started = std::time::Instant::now();
        pool.prepare_playback(PlaybackPreparationRequest {
            context: context.clone(),
            request_id: 1,
            buffer: StreamingBuffer::new(shared.clone()),
            duration: Duration::from_secs(1),
            cache_path: None,
            track_gain: 1.0,
            mode: PlaybackPreparationMode::Play { fade_in: false },
        })
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));

        let (critical_tx, mut critical_rx) = audio_command_channel();
        critical_tx
            .try_send(AudioCommand::Pause {
                context,
                fade_out: false,
            })
            .unwrap();
        assert!(matches!(
            critical_rx.try_recv().unwrap(),
            AudioCommand::Pause { .. }
        ));

        pool.cancel_active_playback();
        let wait_started = Instant::now();
        let prepared = loop {
            match result_rx.try_recv() {
                Ok(prepared) => break prepared,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                    if wait_started.elapsed() < Duration::from_millis(500) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => panic!("cancelled preparation did not return in time: {error}"),
            }
        };
        assert!(matches!(
            prepared,
            StreamingPreparationResult::Playback(PreparedPlayback { source: Err(_), .. })
        ));
        assert_eq!(shared.health(), SharedBufferHealth::Refillable);
        drop(pool);
    }

    #[test]
    fn finished_guard_is_exactly_once_per_generation() {
        let mut guard = FinishedGuard::default();
        assert!(!guard.try_mark(crate::identity::PlaybackGeneration(1), FinishReason::Stop));
        assert!(!guard.try_mark(
            crate::identity::PlaybackGeneration(1),
            FinishReason::SeekRebuild
        ));
        assert!(!guard.try_mark(
            crate::identity::PlaybackGeneration(1),
            FinishReason::TransitionDisposed
        ));
        assert!(!guard.try_mark(crate::identity::PlaybackGeneration(1), FinishReason::Stale));
        assert!(guard.try_mark(
            crate::identity::PlaybackGeneration(1),
            FinishReason::Natural
        ));
        assert!(!guard.try_mark(
            crate::identity::PlaybackGeneration(1),
            FinishReason::Natural
        ));
        assert!(guard.try_mark(
            crate::identity::PlaybackGeneration(2),
            FinishReason::Natural
        ));
        guard.reset();
        assert!(guard.try_mark(
            crate::identity::PlaybackGeneration(2),
            FinishReason::Natural
        ));
    }

    #[test]
    fn resume_buffering_uses_low_water_mark_without_hysteresis_deadlock() {
        let policy = super::super::streaming::StreamingBufferPolicy::from_bitrate(Some(320_000));
        assert!(policy.should_enter_buffering(policy.low_water_mark_bytes() - 1, false));
        assert!(!policy.should_enter_buffering(policy.low_water_mark_bytes(), false));
        assert!(!policy.should_enter_buffering(policy.high_water_mark_bytes() - 1, false));
        assert!(!policy.should_enter_buffering(0, true));
    }
}
