// src/app/update/queue.rs
//! Queue management message handlers

use iced::Task;

use crate::app::message::Message;
use crate::app::state::App;

impl App {
    /// Handle queue-related messages
    pub fn handle_queue(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::ToggleQueue => {
                self.ui.queue_visible = !self.ui.queue_visible;

                // When opening the queue, scroll to center the current song
                if self.ui.queue_visible {
                    let context = crate::ui::responsive::ResponsiveContext::from_viewport(
                        iced::Size::new(self.core.window_width, self.core.window_height),
                    );
                    let offset = crate::ui::components::queue_panel::calculate_scroll_offset(
                        self.playback.queue.len(),
                        self.playback.current_index,
                        context,
                    );
                    return Some(iced::widget::operation::snap_to(
                        iced::widget::Id::new(
                            crate::ui::components::queue_panel::QUEUE_SCROLLABLE_ID,
                        ),
                        iced::widget::scrollable::RelativeOffset { x: 0.0, y: offset },
                    ));
                }
                Some(Task::none())
            }

            Message::PlayPlaylist(playlist_id) => {
                self.exit_fm_mode();
                let id = *playlist_id;

                // For recently played (id = -1), use the recently_played list
                if id == -1 {
                    if !self.library.recently_played.is_empty() {
                        let db_songs = self.library.recently_played.clone();
                        self.clear_shuffle_cache();
                        self.playback.queue_artists_by_song_id.clear();
                        self.playback.queue = db_songs.clone();
                        self.persist_queue_snapshot();

                        return Some(self.play_song_at_index(0));
                    }
                    return Some(Task::none());
                }

                // For NCM playlists (negative ID), use the cached NCM playlist songs
                if id <= 0 {
                    let ncm_songs = self
                        .ui
                        .playlist_page
                        .online_tracks_for(id, self.ui.playlist_page.ncm_load_generation)
                        .filter(|tracks| !tracks.is_empty())
                        .unwrap_or(&self.ui.home.current_ncm_playlist_songs);
                    if !ncm_songs.is_empty() {
                        let source_id = if self.is_fm_mode() {
                            None
                        } else {
                            self.current_route_ncm_scrobble_source()
                        };
                        return Some(Task::done(Message::AddNcmPlaylistWithSource(
                            ncm_songs.to_vec(),
                            true,
                            source_id,
                        )));
                    }
                    tracing::warn!(
                        page_id = id,
                        generation = self.ui.playlist_page.ncm_load_generation,
                        "playlist_queue_source_unavailable"
                    );
                    return Some(Self::toast_warning(
                        "当前页面的歌曲数据尚未就绪，请稍后重试".to_string(),
                    ));
                }

                // For local playlists, load from database
                if let Some(db) = &self.core.db {
                    let db = db.clone();
                    return Some(Task::perform(
                        async move { db.get_playlist_songs(id).await.unwrap_or_default() },
                        Message::QueueLoaded,
                    ));
                }
                Some(Task::none())
            }

            Message::QueueLoaded(songs) => {
                self.exit_fm_mode();
                self.store_db_song_cover_paths(songs);
                if !songs.is_empty() {
                    self.clear_shuffle_cache();
                    self.playback.queue_artists_by_song_id.clear();
                    self.playback.queue = songs.clone();
                    self.persist_queue_snapshot();
                    return Some(self.play_song_at_index(0));
                }
                Some(Task::none())
            }

            Message::PlayQueueIndex(idx) => Some(self.play_song_at_index(*idx)),

            Message::SongResolvedStreaming(
                idx,
                finalized_cache_path,
                cover_path,
                shared_buffer,
                duration_secs,
                quality,
                context,
            ) => Some(self.handle_song_resolved_streaming(
                *idx,
                super::song_resolver::ResolvedSong {
                    finalized_cache_path: finalized_cache_path.clone(),
                    cover_path: cover_path.clone(),
                    shared_buffer: shared_buffer.clone(),
                    duration_secs: *duration_secs,
                    quality: quality.clone(),
                },
                context.clone(),
            )),

            Message::SongResolveFailed(context, reason) => {
                if !self.accepts_audio_context(context) {
                    tracing::debug!(
                        generation = context.generation.0,
                        "Ignoring stale song resolution failure"
                    );
                    return Some(Task::none());
                }

                tracing::error!("Failed to resolve song: {}", reason);
                // Use handle_playback_failure for consistent failure tracking
                if let Some(idx) = self.playback.current_index {
                    return Some(Task::batch([
                        Self::toast_error(format!("无法播放：{reason}")),
                        self.handle_playback_failure(idx, reason.user_summary()),
                    ]));
                }
                Some(Self::toast_error(format!("无法加载歌曲：{reason}")))
            }

            Message::RemoveFromQueue(idx) => {
                if *idx < self.playback.queue.len() {
                    self.playback.queue.remove(*idx);
                    self.prune_queue_artist_metadata();
                    if let Some(current_idx) = self.playback.current_index {
                        if *idx < current_idx {
                            self.playback.current_index = Some(current_idx - 1);
                        } else if *idx == current_idx {
                            if self.playback.queue.is_empty() {
                                self.playback.current_index = None;
                            } else if current_idx >= self.playback.queue.len() {
                                self.playback.current_index = Some(self.playback.queue.len() - 1);
                            }
                        }
                    }

                    if let Some(db) = &self.core.db {
                        let db = db.clone();
                        let position = *idx as i64;
                        tokio::spawn(async move {
                            let _ = db.remove_from_queue(position).await;
                        });
                    }

                    // Refresh coordinator window and re-preload adjacent tracks
                    self.clear_shuffle_cache();
                    self.cache_shuffle_indices();
                    self.refresh_preload_window();
                    return Some(self.preload_adjacent_tracks_with_ncm());
                }
                Some(Task::none())
            }

            Message::ClearQueue => {
                self.playback.queue.clear();
                self.playback.queue_artists_by_song_id.clear();
                self.playback.current_index = None;
                self.playback.shuffle_cache.clear();
                self.playback.preload_coordinator.clear_window();
                // Release audio preload sinks
                let released = self.playback.audio_preload_manager.reset();
                self.release_preload_requests(released);

                if let Some(db) = &self.core.db {
                    let db = db.clone();
                    tokio::spawn(async move {
                        let _ = db.clear_queue().await;
                    });
                }
                Some(Task::none())
            }

            _ => None,
        }
    }
}
