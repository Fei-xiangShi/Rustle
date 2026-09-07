// src/app/update/discord.rs
//! Discord Rich Presence message handlers

use iced::Task;
use std::time::Duration;

use crate::api::NcmClient;
use crate::app::message::Message;
use crate::app::state::App;
use crate::audio::AudioEvent;
use crate::database::DbSong;
use crate::platform::discord as discord_mod;

struct DiscordActivitySnapshot {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    cover_url: Option<String>,
    filename: Option<String>,
    position: Duration,
    duration: Duration,
    ncm_id: Option<u64>,
    client: Option<NcmClient>,
}

impl App {
    /// Handle Discord Rich Presence messages
    pub fn handle_discord(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::DiscordUpdatePresence => {
                let enabled = self.core.settings.system.discord_enabled;
                let snapshot = self.discord_activity_snapshot();
                if let Some(snapshot) = snapshot {
                    Some(Task::run(
                        futures_util::stream::once(async move {
                            if !enabled {
                                return;
                            }

                            let art_url = resolve_discord_cover_url(
                                snapshot.cover_url,
                                snapshot.ncm_id,
                                snapshot.client,
                            )
                            .await;

                            let activity = discord_mod::build_activity_safe(
                                snapshot.title.as_deref(),
                                snapshot.artist.as_deref(),
                                snapshot.album.as_deref(),
                                art_url.as_deref(),
                                snapshot.filename.as_deref(),
                                snapshot.position,
                                snapshot.duration,
                            );

                            discord_mod::send_activity_oneshot(activity).await;
                        }),
                        |_| Message::Noop,
                    ))
                } else {
                    Some(Task::none())
                }
            }

            Message::DiscordClearPresence => Some(Task::run(
                futures_util::stream::once(async move {
                    discord_mod::clear_activity_oneshot().await;
                }),
                |_| Message::Noop,
            )),

            _ => None,
        }
    }

    /// Trigger a Discord presence update from an AudioEvent.
    pub fn maybe_update_discord(&mut self, event: &AudioEvent) -> Option<Task<Message>> {
        if !self.audio_event_is_current(event) {
            return None;
        }
        if !self.core.settings.system.discord_enabled {
            return None;
        }

        match event {
            AudioEvent::Started { .. } | AudioEvent::Resumed { .. } => {
                Some(Task::done(Message::DiscordUpdatePresence))
            }
            AudioEvent::Paused { .. }
            | AudioEvent::Stopped { .. }
            | AudioEvent::Finished { .. } => Some(Task::done(Message::DiscordClearPresence)),
            AudioEvent::SeekComplete { .. } => Some(Task::done(Message::DiscordUpdatePresence)),
            _ => None,
        }
    }

    /// Capture the current playback state needed to build a Discord activity.
    fn discord_activity_snapshot(&self) -> Option<DiscordActivitySnapshot> {
        let song = self.playback.current_song.as_ref()?;
        let runtime = &self.playback.runtime;

        let ncm_id = ncm_id_from_song(song);
        let cover_url = song
            .cover_path
            .as_deref()
            .filter(|path| crate::image::is_remote_url(path))
            .map(str::to_string)
            .or_else(|| ncm_id.and_then(|id| self.loaded_ncm_cover_url(id)));
        let filename = if song.file_path.is_empty() {
            None
        } else {
            std::path::Path::new(&song.file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        };

        Some(DiscordActivitySnapshot {
            title: non_empty_string(&song.title),
            artist: non_empty_string(&song.artist),
            album: non_empty_string(&song.album),
            cover_url,
            filename,
            position: runtime.display_position,
            duration: runtime.info.duration,
            ncm_id,
            client: self.core.ncm_client.clone(),
        })
    }

    fn loaded_ncm_cover_url(&self, ncm_id: u64) -> Option<String> {
        self.ui
            .home
            .current_ncm_playlist_songs
            .iter()
            .chain(self.ui.search.tracks.iter())
            .find(|song| song.id == ncm_id && crate::image::is_remote_url(song.cover_url()))
            .map(|song| song.cover_url().to_string())
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn ncm_id_from_song(song: &DbSong) -> Option<u64> {
    if song.id < 0 {
        return Some((-song.id) as u64);
    }

    song.file_path
        .strip_prefix("ncm://")
        .and_then(|id| id.parse().ok())
}

async fn resolve_discord_cover_url(
    cover_url: Option<String>,
    ncm_id: Option<u64>,
    client: Option<NcmClient>,
) -> Option<String> {
    if cover_url
        .as_deref()
        .is_some_and(crate::image::is_remote_url)
    {
        return cover_url;
    }

    let (Some(ncm_id), Some(client)) = (ncm_id, client) else {
        return None;
    };

    match client.track_detail(&[ncm_id]).await {
        Ok(tracks) => tracks
            .into_iter()
            .find(|song| song.id == ncm_id && crate::image::is_remote_url(song.cover_url()))
            .map(|song| song.cover_url().to_string()),
        Err(err) => {
            tracing::debug!("Discord RPC: failed to fetch cover URL for {ncm_id}: {err}");
            None
        }
    }
}
