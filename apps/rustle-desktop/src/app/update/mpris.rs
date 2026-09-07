// src/app/update/mpris.rs
//! Media controls message handlers

use iced::Task;
use std::time::Duration;

use crate::app::message::Message;
use crate::app::state::App;
use crate::platform::media_controls::{
    MediaCommand, MediaMetadata, MediaPlaybackStatus, MediaState, is_available,
};

impl App {
    /// Handle media controls related messages
    pub fn handle_mpris(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::MprisStartedWithHandle(handle, rx) => {
                // Only process if media controls are available on this platform
                if !is_available() {
                    return Some(Task::none());
                }

                tracing::info!("Media controls service started");
                self.core.mpris_rx = Some(rx.clone());

                // Store the handle globally for updates
                let handle_clone = handle.clone();
                crate::app::helpers::set_mpris_handle(handle.clone());
                self.core.mpris_handle = Some(handle_clone);
                self.update_mpris_state();

                // Start listening for media control commands
                let rx = rx.clone();
                Some(Task::run(
                    async_stream::stream! {
                        loop {
                            let cmd = rx.lock().await.recv().await;
                            if let Some(cmd) = cmd {
                                yield cmd;
                            } else {
                                break;
                            }
                        }
                    },
                    Message::MprisCommand,
                ))
            }

            Message::MprisCommand(cmd) => self.handle_mpris_command(cmd),

            _ => None,
        }
    }

    /// Handle a specific media control command
    fn handle_mpris_command(&mut self, cmd: &MediaCommand) -> Option<Task<Message>> {
        match cmd {
            MediaCommand::Play => {
                if self.playback_output_available() && !self.playback_is_playing() {
                    Some(self.update(Message::TogglePlayback))
                } else {
                    Some(Task::none())
                }
            }

            MediaCommand::Pause => {
                if self.playback_is_playing() {
                    Some(self.update(Message::TogglePlayback))
                } else {
                    Some(Task::none())
                }
            }

            MediaCommand::PlayPause => Some(self.update(Message::TogglePlayback)),

            MediaCommand::Stop => {
                self.stop_and_clear_current_playback();
                Some(Task::none())
            }

            MediaCommand::Next => Some(self.update(Message::NextSong)),

            MediaCommand::Previous => Some(self.update(Message::PrevSong)),

            MediaCommand::Seek(offset_us) => {
                if self.playback_can_seek() {
                    let info = self.playback_info();
                    let current_us = info.position.as_micros() as i128;
                    let duration_us = info.duration.as_micros() as i128;
                    let max_us = if duration_us > 0 {
                        duration_us
                    } else {
                        i128::MAX
                    };
                    let new_pos = (current_us + i128::from(*offset_us)).clamp(0, max_us) as u64;
                    let new_pos = Duration::from_micros(new_pos);
                    self.seek_to_position(new_pos);
                }
                Some(Task::none())
            }

            MediaCommand::SetPosition(_track_id, position_us) => {
                if self.playback_can_seek() {
                    let duration_us = self.playback_info().duration.as_micros() as i128;
                    let max_us = if duration_us > 0 {
                        duration_us
                    } else {
                        i128::MAX
                    };
                    let new_pos = (*position_us as i128).clamp(0, max_us) as u64;
                    let new_pos = Duration::from_micros(new_pos);
                    self.seek_to_position(new_pos);
                }
                Some(Task::none())
            }

            MediaCommand::SetVolume(volume) => {
                Some(self.update(Message::SetVolume(*volume as f32)))
            }

            MediaCommand::Raise => {
                if self.core.is_window_hidden() {
                    Some(self.update(Message::ShowWindow))
                } else {
                    Some(Task::none())
                }
            }

            MediaCommand::Quit => Some(iced::exit()),

            MediaCommand::OpenUri(uri) => Some(Task::done(Message::UriReceived(uri.clone()))),
        }
    }

    /// Update media controls state when playback changes
    pub fn update_mpris_state(&mut self) {
        if let Some(handle) = &self.core.mpris_handle {
            let runtime = self.playback_runtime();
            let status = if !runtime.has_loaded_audio {
                MediaPlaybackStatus::Stopped
            } else if runtime.is_playing() {
                MediaPlaybackStatus::Playing
            } else {
                MediaPlaybackStatus::Paused
            };

            let metadata = if let Some(song) = &self.playback.current_song {
                let art_url = song.cover_path.as_ref().map(|path| {
                    if path.starts_with("http") {
                        path.clone()
                    } else {
                        format!("file://{}", path)
                    }
                });

                MediaMetadata {
                    track_id: Some(song.id.to_string()),
                    title: Some(song.title.clone()),
                    artists: vec![song.artist.clone()],
                    album: Some(song.album.clone()),
                    album_artists: vec![],
                    length_us: Some(song.duration_secs * 1_000_000),
                    art_url,
                }
            } else {
                MediaMetadata::default()
            };

            let position = runtime.info.position.as_micros() as i64;
            #[cfg(target_os = "linux")]
            let volume = runtime.info.volume;
            #[cfg(target_os = "linux")]
            let can_go_next = self
                .playback
                .current_index
                .is_some_and(|i| i + 1 < self.playback.queue.len());
            #[cfg(target_os = "linux")]
            let can_go_previous = self.playback.current_index.is_some_and(|i| i > 0);
            #[cfg(target_os = "linux")]
            let can_play = self.playback_output_available();
            #[cfg(target_os = "linux")]
            let can_pause = can_play;
            #[cfg(target_os = "linux")]
            let can_seek = can_play;

            let state = MediaState {
                status,
                metadata,
                position_us: position,
                #[cfg(target_os = "linux")]
                volume: volume as f64,
                #[cfg(target_os = "linux")]
                can_go_next,
                #[cfg(target_os = "linux")]
                can_go_previous,
                #[cfg(target_os = "linux")]
                can_play,
                #[cfg(target_os = "linux")]
                can_pause,
                #[cfg(target_os = "linux")]
                can_seek,
            };

            // Update media controls state directly via handle
            handle.update(state);
        }
    }
}
