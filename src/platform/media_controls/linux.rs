//! Linux MPRIS D-Bus integration using LocalPlayerInterface trait

use crate::platform::{APP_BINARY_NAME, APP_DISPLAY_NAME, APP_ID};
use mpris_server::{
    LocalPlayerInterface, LocalRootInterface, LocalServer, LoopStatus, Metadata, PlaybackRate,
    PlaybackStatus, Property, Time, TrackId, Volume,
    zbus::{Result, fdo},
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::{MediaCommand, MediaMetadata, MediaPlaybackStatus, MediaState};

/// Convert MediaMetadata to mpris-server Metadata
fn to_mpris_metadata(meta: &MediaMetadata) -> Metadata {
    let mut builder = Metadata::builder();

    if let Some(ref track_id) = meta.track_id {
        builder = builder.trackid(
            TrackId::try_from(format!("/org/{}/track/{}", APP_BINARY_NAME, track_id))
                .unwrap_or(TrackId::NO_TRACK),
        );
    }

    if let Some(ref title) = meta.title {
        builder = builder.title(title);
    }

    if !meta.artists.is_empty() {
        builder = builder.artist(meta.artists.clone());
    }

    if let Some(ref album) = meta.album {
        builder = builder.album(album);
    }

    if !meta.album_artists.is_empty() {
        builder = builder.album_artist(meta.album_artists.clone());
    }

    if let Some(length) = meta.length_us {
        builder = builder.length(Time::from_micros(length));
    }

    if let Some(ref art_url) = meta.art_url {
        builder = builder.art_url(art_url);
    }

    builder.build()
}

impl From<MediaPlaybackStatus> for PlaybackStatus {
    fn from(status: MediaPlaybackStatus) -> Self {
        match status {
            MediaPlaybackStatus::Playing => PlaybackStatus::Playing,
            MediaPlaybackStatus::Paused => PlaybackStatus::Paused,
            MediaPlaybackStatus::Stopped => PlaybackStatus::Stopped,
        }
    }
}

// Shared state between the app and MPRIS
#[derive(Debug, Clone)]
struct MprisSharedState {
    status: Arc<Mutex<PlaybackStatus>>,
    metadata: Arc<Mutex<Metadata>>,
    volume: Arc<Mutex<f64>>,
    position: Arc<Mutex<Time>>,
    can_go_next: Arc<Mutex<bool>>,
    can_go_previous: Arc<Mutex<bool>>,
    can_play: Arc<Mutex<bool>>,
    can_pause: Arc<Mutex<bool>>,
    can_seek: Arc<Mutex<bool>>,
}

impl Default for MprisSharedState {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(PlaybackStatus::Stopped)),
            metadata: Arc::new(Mutex::new(Metadata::default())),
            volume: Arc::new(Mutex::new(1.0)),
            position: Arc::new(Mutex::new(Time::ZERO)),
            can_go_next: Arc::new(Mutex::new(false)),
            can_go_previous: Arc::new(Mutex::new(false)),
            can_play: Arc::new(Mutex::new(false)),
            can_pause: Arc::new(Mutex::new(false)),
            can_seek: Arc::new(Mutex::new(false)),
        }
    }
}

struct MprisPlayer {
    state: MprisSharedState,
    cmd_tx: mpsc::UnboundedSender<MediaCommand>,
}

impl LocalRootInterface for MprisPlayer {
    async fn raise(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Raise);
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Quit);
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok(APP_DISPLAY_NAME.to_string())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(APP_ID.to_string())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["file".to_string(), "rustle".to_string()])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![
            "audio/mpeg".to_string(),
            "audio/ogg".to_string(),
            "audio/flac".to_string(),
        ])
    }
}

impl LocalPlayerInterface for MprisPlayer {
    async fn next(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Pause);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Play);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::PlayPause);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Stop);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::Seek(offset.as_micros()));
        Ok(())
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::SetPosition(
            track_id.to_string(),
            position.as_micros(),
        ));
        Ok(())
    }

    async fn open_uri(&self, uri: String) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::OpenUri(uri));
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        let status = self.state.status.lock().unwrap();
        Ok(*status)
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(LoopStatus::None)
    }

    async fn set_loop_status(&self, _loop_status: LoopStatus) -> Result<()> {
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> Result<()> {
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        let metadata = self.state.metadata.lock().unwrap();
        Ok(metadata.clone())
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        let volume = self.state.volume.lock().unwrap();
        Ok(*volume)
    }

    async fn set_volume(&self, volume: Volume) -> Result<()> {
        let _ = self.cmd_tx.send(MediaCommand::SetVolume(volume));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        let position = self.state.position.lock().unwrap();
        Ok(*position)
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        let can_go_next = self.state.can_go_next.lock().unwrap();
        Ok(*can_go_next)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        let can_go_previous = self.state.can_go_previous.lock().unwrap();
        Ok(*can_go_previous)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        let can_play = self.state.can_play.lock().unwrap();
        Ok(*can_play)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        let can_pause = self.state.can_pause.lock().unwrap();
        Ok(*can_pause)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        let can_seek = self.state.can_seek.lock().unwrap();
        Ok(*can_seek)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// Handle to control media controls from the application (Linux implementation)
#[derive(Debug, Clone)]
pub struct LinuxMediaHandle {
    state: MprisSharedState,
    state_tx: mpsc::UnboundedSender<MediaState>,
}

impl LinuxMediaHandle {
    /// Update MPRIS state and notify D-Bus clients
    pub fn update(&self, state: MediaState) {
        // Update shared state for on-demand queries
        *self.state.status.lock().unwrap() = state.status.into();
        *self.state.metadata.lock().unwrap() = to_mpris_metadata(&state.metadata);
        *self.state.volume.lock().unwrap() = state.volume;
        *self.state.position.lock().unwrap() = Time::from_micros(state.position_us);
        *self.state.can_go_next.lock().unwrap() = state.can_go_next;
        *self.state.can_go_previous.lock().unwrap() = state.can_go_previous;
        *self.state.can_play.lock().unwrap() = state.can_play;
        *self.state.can_pause.lock().unwrap() = state.can_pause;
        *self.state.can_seek.lock().unwrap() = state.can_seek;

        // Send state to MPRIS thread for PropertiesChanged signal
        let _ = self.state_tx.send(state);
    }
}

/// Start MPRIS service
pub fn start() -> (LinuxMediaHandle, mpsc::UnboundedReceiver<MediaCommand>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<MediaState>();
    let state = MprisSharedState::default();
    let player = MprisPlayer {
        state: state.clone(),
        cmd_tx: cmd_tx.clone(),
    };

    // Spawn MPRIS on a dedicated panic-contained thread with its own runtime.
    let spawn_result = crate::runtime::spawn_guarded("linux-mpris", "mpris_service", move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::error!(%error, "Failed to create the MPRIS runtime");
                return;
            }
        };

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            // Start the MPRIS server
            let server = match LocalServer::new(APP_ID, player).await {
                Ok(server) => server,
                Err(error) => {
                    tracing::error!(%error, "Failed to create the MPRIS server");
                    return;
                }
            };

            // Run server and handle state updates concurrently
            tokio::select! {
                _ = server.run() => {}
                _ = async {
                    while let Some(state) = state_rx.recv().await {
                        // Send PropertiesChanged signal to notify clients like Waybar
                        let _ = server.properties_changed([
                            Property::PlaybackStatus(state.status.into()),
                            Property::Metadata(to_mpris_metadata(&state.metadata)),
                            Property::CanGoNext(state.can_go_next),
                            Property::CanGoPrevious(state.can_go_previous),
                            Property::CanPlay(state.can_play),
                            Property::CanPause(state.can_pause),
                            Property::CanSeek(state.can_seek),
                        ]).await;
                    }
                } => {}
            }
        });
    });
    if let Err(error) = spawn_result {
        tracing::error!(
            error_kind = ?error.kind(),
            "Failed to spawn the MPRIS worker"
        );
    }

    (LinuxMediaHandle { state, state_tx }, cmd_rx)
}
