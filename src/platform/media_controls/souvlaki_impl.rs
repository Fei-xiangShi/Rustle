//! Windows/macOS media controls using souvlaki
//!
//! Provides system media controls integration for Windows (SMTC) and macOS (MPNowPlayingInfoCenter)

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata as SouvlakiMetadata, MediaPlayback,
    MediaPosition, PlatformConfig, SeekDirection,
};
#[cfg(target_os = "windows")]
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::{MediaCommand, MediaMetadata, MediaPlaybackStatus, MediaState};

const PLAYBACK_SYNC_INTERVAL: Duration = Duration::from_secs(2);
const POSITION_SYNC_TOLERANCE_US: i64 = 750_000;

/// Convert our MediaPlaybackStatus to souvlaki's MediaPlayback
fn to_souvlaki_playback(status: MediaPlaybackStatus, position_us: i64) -> MediaPlayback {
    match status {
        MediaPlaybackStatus::Playing => MediaPlayback::Playing {
            progress: Some(MediaPosition(Duration::from_micros(position_us as u64))),
        },
        MediaPlaybackStatus::Paused => MediaPlayback::Paused {
            progress: Some(MediaPosition(Duration::from_micros(position_us as u64))),
        },
        MediaPlaybackStatus::Stopped => MediaPlayback::Stopped,
    }
}

/// Shared state for thread-safe access
struct SharedState {
    controls: Mutex<Option<MediaControls>>,
    published: Mutex<PublishedState>,
}

/// Handle to control media controls from the application (Windows/macOS implementation)
#[derive(Clone)]
pub struct SouvlakiMediaHandle {
    state: Arc<SharedState>,
    // Keep owned strings for metadata lifetime
    metadata_cache: Arc<Mutex<MetadataCache>>,
}

/// Cache for metadata strings (souvlaki needs references with specific lifetimes)
#[derive(Default)]
struct MetadataCache {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    cover_url: Option<String>,
    duration: Option<Duration>,
}

#[derive(Default)]
struct PublishedState {
    metadata: Option<MediaMetadata>,
    playback_status: Option<MediaPlaybackStatus>,
    position_us: i64,
    last_playback_sync: Option<Instant>,
}

impl std::fmt::Debug for SouvlakiMediaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SouvlakiMediaHandle").finish()
    }
}

impl SouvlakiMediaHandle {
    /// Update media controls state
    pub fn update(&self, state: MediaState) {
        // Update metadata cache
        {
            let mut cache = self.metadata_cache.lock().unwrap();
            cache.title = state.metadata.title.clone();
            cache.artist = if !state.metadata.artists.is_empty() {
                Some(state.metadata.artists.join(", "))
            } else {
                None
            };
            cache.album = state.metadata.album.clone();
            cache.cover_url = state.metadata.art_url.clone();
            cache.duration = state
                .metadata
                .length_us
                .map(|us| Duration::from_micros(us as u64));
        }

        // Update controls
        if let Ok(mut controls_guard) = self.state.controls.lock()
            && let Some(ref mut controls) = *controls_guard
        {
            let mut published = self.state.published.lock().unwrap();
            let metadata_changed = published.metadata.as_ref() != Some(&state.metadata);

            if metadata_changed {
                let cache = self.metadata_cache.lock().unwrap();
                let metadata = SouvlakiMetadata {
                    title: Some(cache.title.as_deref().unwrap_or("")),
                    artist: Some(cache.artist.as_deref().unwrap_or("")),
                    album: Some(cache.album.as_deref().unwrap_or("")),
                    cover_url: cache.cover_url.as_deref(),
                    duration: cache.duration,
                };

                if let Err(err) = controls.set_metadata(metadata) {
                    tracing::debug!("Failed to update media metadata: {:?}", err);
                } else {
                    published.metadata = Some(state.metadata.clone());
                }
            }

            if should_sync_playback(&published, &state, metadata_changed) {
                let playback = to_souvlaki_playback(state.status, state.position_us);
                if let Err(err) = controls.set_playback(playback) {
                    tracing::debug!("Failed to update media playback: {:?}", err);
                } else {
                    published.playback_status = Some(state.status);
                    published.position_us = state.position_us;
                    published.last_playback_sync = Some(Instant::now());
                }
            }
        }
    }
}

fn should_sync_playback(
    published: &PublishedState,
    state: &MediaState,
    metadata_changed: bool,
) -> bool {
    if metadata_changed || published.playback_status != Some(state.status) {
        return true;
    }

    let Some(last_sync) = published.last_playback_sync else {
        return true;
    };

    let expected_position_us = match published.playback_status {
        Some(MediaPlaybackStatus::Playing) => published
            .position_us
            .saturating_add(last_sync.elapsed().as_micros().min(i64::MAX as u128) as i64),
        _ => published.position_us,
    };

    let drift_us = state.position_us.saturating_sub(expected_position_us).abs();

    match state.status {
        MediaPlaybackStatus::Playing => {
            drift_us >= POSITION_SYNC_TOLERANCE_US || last_sync.elapsed() >= PLAYBACK_SYNC_INTERVAL
        }
        MediaPlaybackStatus::Paused | MediaPlaybackStatus::Stopped => {
            drift_us >= POSITION_SYNC_TOLERANCE_US
        }
    }
}

/// Start media controls service using souvlaki
pub fn start(
    window_handle: Option<usize>,
) -> (SouvlakiMediaHandle, mpsc::UnboundedReceiver<MediaCommand>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let state = Arc::new(SharedState {
        controls: Mutex::new(None),
        published: Mutex::new(PublishedState::default()),
    });
    let metadata_cache = Arc::new(Mutex::new(MetadataCache::default()));

    #[cfg(target_os = "windows")]
    let hwnd = match window_handle {
        Some(handle) => Some(handle as *mut c_void),
        None => {
            tracing::warn!(
                "Skipping Windows media controls initialization because no HWND was available"
            );
            return (
                SouvlakiMediaHandle {
                    state,
                    metadata_cache,
                },
                cmd_rx,
            );
        }
    };

    #[cfg(target_os = "macos")]
    let hwnd = {
        let _ = window_handle;
        None
    };

    let config = PlatformConfig {
        dbus_name: "rustle", // Not used on Windows/macOS
        display_name: "Rustle",
        hwnd,
    };

    // Try to create media controls
    match catch_unwind(AssertUnwindSafe(|| MediaControls::new(config))) {
        Ok(Ok(mut controls)) => {
            // Attach event handler
            let tx = cmd_tx.clone();
            if let Err(e) = controls.attach(move |event: MediaControlEvent| {
                let cmd = match event {
                    MediaControlEvent::Play => Some(MediaCommand::Play),
                    MediaControlEvent::Pause => Some(MediaCommand::Pause),
                    MediaControlEvent::Toggle => Some(MediaCommand::PlayPause),
                    MediaControlEvent::Next => Some(MediaCommand::Next),
                    MediaControlEvent::Previous => Some(MediaCommand::Previous),
                    MediaControlEvent::Stop => Some(MediaCommand::Stop),
                    MediaControlEvent::Seek(direction) => {
                        // Seek by 10 seconds
                        let offset = match direction {
                            SeekDirection::Forward => 10_000_000i64, // 10 seconds in microseconds
                            SeekDirection::Backward => -10_000_000i64,
                        };
                        Some(MediaCommand::Seek(offset))
                    }
                    MediaControlEvent::SeekBy(direction, duration) => {
                        let micros = duration.as_micros() as i64;
                        let offset = match direction {
                            SeekDirection::Forward => micros,
                            SeekDirection::Backward => -micros,
                        };
                        Some(MediaCommand::Seek(offset))
                    }
                    MediaControlEvent::SetPosition(pos) => Some(MediaCommand::SetPosition(
                        String::new(),
                        pos.0.as_micros() as i64,
                    )),
                    MediaControlEvent::SetVolume(volume) => Some(MediaCommand::SetVolume(volume)),
                    MediaControlEvent::Raise => Some(MediaCommand::Raise),
                    MediaControlEvent::Quit => Some(MediaCommand::Quit),
                    MediaControlEvent::OpenUri(uri) => Some(MediaCommand::OpenUri(uri)),
                };

                if let Some(cmd) = cmd {
                    let _ = tx.send(cmd);
                }
            }) {
                tracing::warn!("Failed to attach media controls event handler: {:?}", e);
            }

            *state.controls.lock().unwrap() = Some(controls);
            tracing::info!("Media controls (souvlaki) initialized successfully");
        }
        Ok(Err(e)) => {
            tracing::warn!("Failed to create media controls: {:?}", e);
        }
        Err(_) => {
            tracing::warn!("Media controls initialization panicked; disabling media controls");
        }
    }

    (
        SouvlakiMediaHandle {
            state,
            metadata_cache,
        },
        cmd_rx,
    )
}
