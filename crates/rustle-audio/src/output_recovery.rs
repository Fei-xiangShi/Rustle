//! Serialized output-device reconstruction primitives.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rodio::Source;

use super::chain::AudioProcessingChain;
use super::device::{OutputFailureCallback, OutputRecoveryReason};
use super::events::DeviceSignalMailbox;
use super::identity::PlaybackContext;
use super::player::{AudioPlayer, PlaybackError, PlaybackResult, prepare_streaming_source};
use super::streaming::{SharedBuffer, StreamingBuffer};
use super::thread::WorkerSpawner;

pub(crate) const OUTPUT_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(300);
pub(crate) const OUTPUT_STALL_THRESHOLD: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputGeneration(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputRecoveryId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRecoveryIntent {
    Playing,
    Paused,
}

#[derive(Clone)]
pub(crate) enum OutputRecoverySource {
    Idle,
    Local {
        path: PathBuf,
    },
    Streaming {
        shared_buffer: SharedBuffer,
        cache_path: Option<PathBuf>,
    },
}

#[derive(Clone)]
pub(crate) struct OutputRecoverySnapshot {
    pub recovery_id: OutputRecoveryId,
    pub output_generation: OutputGeneration,
    pub requested_selection: Option<String>,
    pub reason: OutputRecoveryReason,
    pub context: Option<PlaybackContext>,
    pub source_revision: u64,
    pub source: OutputRecoverySource,
    pub position: Duration,
    pub duration: Duration,
    pub intent: OutputRecoveryIntent,
    pub volume: f32,
    pub track_gain: f32,
}

pub(crate) struct OutputRecoveryRequest {
    pub snapshot: OutputRecoverySnapshot,
    pub chain: AudioProcessingChain,
    pub device_signals: DeviceSignalMailbox,
}

pub(crate) struct PreparedOutputRecovery {
    pub snapshot: OutputRecoverySnapshot,
    pub candidate: PlaybackResult<AudioPlayer>,
}

pub(crate) struct OutputRecoveryWorker {
    request_tx: std::sync::mpsc::SyncSender<OutputRecoveryRequest>,
}

impl OutputRecoveryWorker {
    pub(crate) fn new(
        result_tx: tokio::sync::mpsc::Sender<PreparedOutputRecovery>,
        spawn_worker: WorkerSpawner,
    ) -> PlaybackResult<Self> {
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<OutputRecoveryRequest>(1);
        spawn_worker(
            "audio-output-recovery",
            "prepare_output_recovery",
            Box::new(move || {
                while let Ok(request) = request_rx.recv() {
                    let snapshot = request.snapshot.clone();
                    let started = Instant::now();
                    let candidate = prepare_candidate(request);
                    tracing::debug!(
                        recovery_id = snapshot.recovery_id.0,
                        output_generation = snapshot.output_generation.0,
                        elapsed_ms = started.elapsed().as_millis(),
                        success = candidate.is_ok(),
                        "Output recovery preparation completed"
                    );
                    if result_tx
                        .blocking_send(PreparedOutputRecovery {
                            snapshot,
                            candidate,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        )
        .map_err(|error| PlaybackError::ControlUnavailable(error.to_string()))?;
        Ok(Self { request_tx })
    }

    pub(crate) fn try_submit(
        &self,
        request: OutputRecoveryRequest,
    ) -> Result<(), Box<(PlaybackError, OutputRecoveryRequest)>> {
        self.request_tx.try_send(request).map_err(|error| {
            let (message, request) = match error {
                std::sync::mpsc::TrySendError::Full(request) => {
                    ("output recovery worker queue is full", request)
                }
                std::sync::mpsc::TrySendError::Disconnected(request) => {
                    ("output recovery worker is unavailable", request)
                }
            };
            Box::new((
                PlaybackError::ControlUnavailable(message.to_string()),
                request,
            ))
        })
    }
}

fn prepare_candidate(request: OutputRecoveryRequest) -> PlaybackResult<AudioPlayer> {
    let output_generation = request.snapshot.output_generation;
    let failure_signals = request.device_signals;
    let output_failure_callback: OutputFailureCallback = Arc::new(move |error| {
        failure_signals.publish_output_failure(output_generation.0, error);
    });
    let mut candidate = AudioPlayer::with_device_observer(
        request.snapshot.requested_selection.as_deref(),
        request.chain,
        output_failure_callback,
    )?;
    candidate.set_volume(request.snapshot.volume);

    match &request.snapshot.source {
        OutputRecoverySource::Idle => {}
        OutputRecoverySource::Local { path } => candidate.prepare_local_recovery_paused(
            path.clone(),
            request.snapshot.position,
            request.snapshot.track_gain,
        )?,
        OutputRecoverySource::Streaming {
            shared_buffer,
            cache_path,
        } => {
            let buffer = StreamingBuffer::new(shared_buffer.clone());
            let reader_cancellation = buffer.reader_cancellation();
            let mut source = prepare_streaming_source(buffer)?;
            source
                .try_seek(request.snapshot.position)
                .map_err(|error| PlaybackError::SeekUnsupported(error.to_string()))?;
            candidate.prepare_streaming_recovery_paused(
                source,
                reader_cancellation,
                request.snapshot.duration,
                cache_path.clone(),
                request.snapshot.position,
                request.snapshot.track_gain,
            )?;
        }
    }

    Ok(candidate)
}

pub(crate) fn is_transient_output_error(error: &PlaybackError) -> bool {
    matches!(
        error,
        PlaybackError::DeviceUnavailable(_) | PlaybackError::ControlUnavailable(_)
    )
}

pub(crate) fn should_retry_output(retry_count: u8, error: &PlaybackError) -> bool {
    retry_count == 0 && is_transient_output_error(error)
}

#[derive(Debug, Default)]
pub(crate) struct OutputStallDetector {
    last_position: Option<Duration>,
    stalled_since: Option<Instant>,
    fired: bool,
}

impl OutputStallDetector {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn sample(&mut self, now: Instant, position: Duration, eligible: bool) -> bool {
        if !eligible {
            self.reset();
            return false;
        }

        let progressed = self
            .last_position
            .is_none_or(|last| position > last.saturating_add(Duration::from_millis(10)));
        self.last_position = Some(position);
        if progressed {
            self.stalled_since = Some(now);
            self.fired = false;
            return false;
        }

        let stalled_since = *self.stalled_since.get_or_insert(now);
        if !self.fired && now.saturating_duration_since(stalled_since) >= OUTPUT_STALL_THRESHOLD {
            self.fired = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_detector_fires_once_until_progress_resumes() {
        let start = Instant::now();
        let mut detector = OutputStallDetector::default();
        assert!(!detector.sample(start, Duration::from_secs(3), true));
        assert!(!detector.sample(
            start + OUTPUT_STALL_THRESHOLD - Duration::from_millis(1),
            Duration::from_secs(3),
            true,
        ));
        assert!(detector.sample(start + OUTPUT_STALL_THRESHOLD, Duration::from_secs(3), true,));
        assert!(!detector.sample(
            start + OUTPUT_STALL_THRESHOLD + Duration::from_secs(1),
            Duration::from_secs(3),
            true,
        ));
        assert!(!detector.sample(
            start + OUTPUT_STALL_THRESHOLD + Duration::from_secs(2),
            Duration::from_secs(4),
            true,
        ));
    }

    #[test]
    fn ineligible_state_resets_stall_window() {
        let start = Instant::now();
        let mut detector = OutputStallDetector::default();
        assert!(!detector.sample(start, Duration::from_secs(1), true));
        assert!(!detector.sample(
            start + OUTPUT_STALL_THRESHOLD,
            Duration::from_secs(1),
            false,
        ));
        assert!(!detector.sample(
            start + OUTPUT_STALL_THRESHOLD + Duration::from_millis(1),
            Duration::from_secs(1),
            true,
        ));
    }

    #[test]
    fn retry_is_bounded_to_one_transient_attempt() {
        let transient = PlaybackError::DeviceUnavailable("endpoint not ready".to_string());
        let permanent = PlaybackError::DecodeError("invalid source".to_string());
        assert!(should_retry_output(0, &transient));
        assert!(!should_retry_output(1, &transient));
        assert!(!should_retry_output(0, &permanent));
    }

    #[test]
    fn replacing_a_streaming_reader_does_not_cancel_the_shared_coordinator() {
        let shared = SharedBuffer::new(1024);
        shared.set_coordinator_active_for_test(true);
        let reader = StreamingBuffer::new(shared.clone());
        reader.reader_cancellation().cancel();
        assert_eq!(
            shared.health(),
            super::super::streaming::SharedBufferHealth::Refillable
        );
    }
}
