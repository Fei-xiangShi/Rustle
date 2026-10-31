// src/app/update/queue.rs
//! Queue management message handlers

use crate::app::playback_machine::{PlaybackOrigin, TransportIntent};
use iced::Task;

use crate::app::message::Message;
use crate::app::state::App;

impl App {
    /// Replace the queue and start local playback in the current update turn.
    pub(super) fn play_loaded_local_queue(
        &mut self,
        songs: &[crate::database::DbSong],
        intent: TransportIntent,
    ) -> Task<Message> {
        self.store_db_song_cover_paths(songs);
        if songs.is_empty() {
            return Task::none();
        }
        self.clear_shuffle_cache();
        self.playback.queue_artists_by_song_id.clear();
        if !self.cancel_playback_transaction() {
            return Task::none();
        }
        self.playback
            .queue
            .replace(songs.to_vec(), PlaybackOrigin::Normal);
        self.sync_current_queue_index();
        self.play_song_at_index_with_intent(0, intent)
    }

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
                let id = *playlist_id;

                // For recently played (id = -1), use the recently_played list
                if id == -1 {
                    if !self.library.recently_played.is_empty() {
                        let db_songs = self.library.recently_played.clone();
                        self.clear_shuffle_cache();
                        self.playback.queue_artists_by_song_id.clear();
                        if !self.cancel_playback_transaction() {
                            return Some(Task::none());
                        }
                        self.playback
                            .queue
                            .replace(db_songs, PlaybackOrigin::Normal);
                        self.sync_current_queue_index();
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
                        let tracks = ncm_songs.to_vec();
                        return self.handle_ncm_playlist_queue(
                            &tracks,
                            true,
                            self.current_route_ncm_scrobble_source(),
                        );
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
                    let Some(selection) = self.begin_playback_selection(false) else {
                        return Some(Task::none());
                    };
                    return Some(Task::perform(
                        async move { db.get_playlist_songs(id).await.unwrap_or_default() },
                        move |songs| Message::QueueLoaded(selection, songs),
                    ));
                }
                Some(Task::none())
            }

            Message::QueueLoaded(id, songs) => {
                if self.playback.machine.selection(*id).is_none() {
                    return Some(Task::none());
                }
                if songs.is_empty() {
                    return Some(self.recover_failed_playback_selection(*id));
                }
                let intent = self.playback.machine.intent;
                self.playback.machine.pending = None;
                let task = self.play_loaded_local_queue(songs, intent);
                Some(task)
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
                if let Some(idx) = self
                    .playback
                    .machine
                    .preparing(context)
                    .and_then(|target| target.entry)
                    .and_then(|id| self.playback.queue.index_of(id))
                {
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
                    self.sync_current_queue_index();
                    self.cancel_removed_playback_target();

                    self.persist_queue_snapshot();

                    // Refresh coordinator window and re-preload adjacent tracks
                    self.clear_shuffle_cache();
                    self.cache_shuffle_indices();
                    self.refresh_preload_window();
                    return Some(self.preload_adjacent_tracks_with_ncm());
                }
                Some(Task::none())
            }

            Message::ClearQueue => {
                if !self.cancel_playback_transaction() {
                    return Some(Task::none());
                }
                self.playback.machine.discard_deferred_finish();
                self.playback.queue.clear();
                self.playback.queue_artists_by_song_id.clear();
                self.playback.current_index = None;
                self.playback.shuffle_cache.clear();
                self.playback.preload_coordinator.clear_window();
                // Release audio preload sinks
                let released = self.playback.audio_preload_manager.reset();
                self.release_preload_requests(released);

                self.persist_queue_snapshot();
                Some(Task::none())
            }

            _ => None,
        }
    }
}
