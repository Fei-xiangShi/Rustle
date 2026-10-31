// src/app/update/database.rs
//! Database message handlers

use crate::app::playback_machine::{PlaybackOrigin, PlaybackTransaction, TransportIntent};

use iced::Task;

use crate::app::helpers::{
    load_download_history, load_playback_state, load_playlists, load_queue, load_songs,
    load_watched_folders, validate_songs,
};
use crate::app::message::Message;
use crate::app::state::App;
use crate::ui::pages;

enum StartupRestoreDecision {
    WaitForSongs,
    Skip,
    Restore(usize),
}

impl App {
    fn resolve_startup_restore_index(
        &self,
        state: &crate::database::DbPlaybackState,
    ) -> StartupRestoreDecision {
        if state.playback_origin.is_some() && state.queue_position < 0 {
            // A current track removed from the queue must not bind to another duplicate.
            return StartupRestoreDecision::Skip;
        }
        let queue_len = self.playback.queue.len();
        if queue_len == 0 {
            tracing::info!("Startup restore skipped: queue is empty");
            return StartupRestoreDecision::Skip;
        }

        if state.queue_position >= 0 {
            let idx = state.queue_position as usize;
            if idx < queue_len {
                tracing::info!(
                    "Startup restore selected queue index {} from saved queue_position {}",
                    idx,
                    state.queue_position
                );
                return StartupRestoreDecision::Restore(idx);
            }

            tracing::warn!(
                "Startup restore queue_position {} is out of range for queue len {}",
                state.queue_position,
                queue_len
            );
        } else {
            tracing::warn!(
                "Startup restore queue_position {} is invalid",
                state.queue_position
            );
        }

        let Some(current_song_id) = state.current_song_id else {
            tracing::warn!(
                "Startup restore skipped: queue_position invalid and current_song_id is missing"
            );
            return StartupRestoreDecision::Skip;
        };

        if let Some(idx) = self
            .playback
            .queue
            .iter()
            .position(|song| song.id == current_song_id)
        {
            tracing::info!(
                "Startup restore selected queue index {} from direct current_song_id match {}",
                idx,
                current_song_id
            );
            return StartupRestoreDecision::Restore(idx);
        }

        if !self.playback.startup_restore.songs_loaded {
            tracing::info!(
                "Startup restore deferred: waiting for songs list to resolve current_song_id {}",
                current_song_id
            );
            return StartupRestoreDecision::WaitForSongs;
        }

        let Some(saved_song_path) = self
            .library
            .db_songs
            .iter()
            .find(|song| song.id == current_song_id)
            .map(|song| song.file_path.clone())
        else {
            tracing::warn!(
                "Startup restore could not find saved song metadata for current_song_id {}",
                current_song_id
            );
            return StartupRestoreDecision::Skip;
        };

        if let Some(idx) = self
            .playback
            .queue
            .iter()
            .position(|song| song.file_path == saved_song_path)
        {
            tracing::info!(
                "Startup restore selected queue index {} from current_song_id {} via path {}",
                idx,
                current_song_id,
                saved_song_path
            );
            return StartupRestoreDecision::Restore(idx);
        }

        tracing::warn!(
            "Startup restore could not find current_song_id {} with path {} in queue",
            current_song_id,
            saved_song_path
        );
        StartupRestoreDecision::Skip
    }

    fn startup_restore_ready(&self) -> bool {
        let restore = &self.playback.startup_restore;
        restore.playback_state_loaded && restore.queue_loaded
    }

    pub(super) fn finish_startup_restore(&mut self) {
        self.playback.startup_restore.in_progress = false;
        self.playback.startup_restore.completed = true;
    }

    fn try_restore_startup_playback(&mut self) -> Task<Message> {
        if self.playback.startup_restore.completed || self.playback.startup_restore.in_progress {
            return Task::none();
        }

        // Files opened at launch take precedence over resuming the previous
        // session, even while they are still debouncing or importing.
        if self.open_files_requested() {
            self.finish_startup_restore();
            return Task::none();
        }

        if !self.startup_restore_ready() {
            tracing::debug!(
                "Startup restore deferred: playback_state_loaded={}, queue_loaded={}, songs_loaded={}",
                self.playback.startup_restore.playback_state_loaded,
                self.playback.startup_restore.queue_loaded,
                self.playback.startup_restore.songs_loaded
            );
            return Task::none();
        }

        let Some(state) = self.playback.saved_state.clone() else {
            tracing::debug!("Startup restore skipped: playback state not loaded yet");
            return Task::none();
        };

        let idx = match self.resolve_startup_restore_index(&state) {
            StartupRestoreDecision::WaitForSongs => return Task::none(),
            StartupRestoreDecision::Skip => {
                self.finish_startup_restore();
                return Task::none();
            }
            StartupRestoreDecision::Restore(idx) => idx,
        };

        let Some(song) = self.playback.queue.get(idx).cloned() else {
            tracing::warn!(
                "Startup restore selected queue index {} but queue len is {}",
                idx,
                self.playback.queue.len()
            );
            self.finish_startup_restore();
            return Task::none();
        };

        if self.playback.machine.pending.is_some() || self.playback.machine.current.is_some() {
            self.finish_startup_restore();
            return Task::none();
        }
        let origin = if state.personal_fm_mode {
            PlaybackOrigin::PersonalFm { session: 0 }
        } else {
            PlaybackOrigin::Normal
        };
        let origin = state
            .playback_origin
            .as_deref()
            .and_then(|value| serde_json::from_str::<PlaybackOrigin>(value).ok())
            .unwrap_or(origin);
        let origins = state
            .queue_origins
            .as_deref()
            .and_then(|value| serde_json::from_str::<Vec<PlaybackOrigin>>(value).ok());
        for index in 0..self.playback.queue.len() {
            let entry_origin = origins
                .as_ref()
                .and_then(|origins| origins.get(index))
                .copied()
                .unwrap_or(origin);
            self.playback.machine.observe_restored_origin(entry_origin);
            self.playback.queue.set_origin(index, entry_origin);
        }
        // Legacy snapshots carry only a queue-wide FM bit.
        self.playback.queue.set_origin(idx, origin);
        self.playback.machine.intent = TransportIntent::Pause;
        let background_task = Task::none();
        self.playback.startup_restore.in_progress = true;

        if crate::app::update::song_resolver::needs_resolution(&song) {
            tracing::info!("Restoring NCM song: {} - {}", song.title, song.artist);

            if let Some(client) = self.core.ncm_client.clone() {
                let song_clone = song.clone();
                let saved_position = state.position_secs;
                let client = std::sync::Arc::new(client);
                let context = match self.begin_audio_resolution_context() {
                    Ok(context) => context,
                    Err(error) => {
                        tracing::warn!("Failed to begin NCM startup resolution: {error}");
                        self.finish_startup_restore();
                        return background_task;
                    }
                };

                let Some(target) = self.queue_target(idx) else {
                    return Task::none();
                };
                self.playback.machine.pending = Some(PlaybackTransaction::Preparing {
                    target,
                    context: context.clone(),
                });

                let context_for_task = context.clone();

                let resolve_task = Task::perform(
                    async move {
                        // Startup restore does not surface streaming progress. Drain
                        // the resolver channel so strict Range startup can reach its
                        // high watermark without progress backpressure.
                        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
                        tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
                        crate::app::update::song_resolver::resolve_song(
                            client,
                            &song_clone,
                            context_for_task,
                            event_tx,
                        )
                        .await
                    },
                    move |result| {
                        Message::SongResolvedForRestore(idx, result, saved_position, context)
                    },
                );

                return Task::batch([resolve_task, background_task]);
            }

            tracing::warn!("NCM client not available for song restoration");
            self.finish_startup_restore();
            return background_task;
        }

        let path_buf = std::path::PathBuf::from(&song.file_path);
        if !path_buf.exists() {
            tracing::warn!(
                "Saved song path no longer exists, cannot restore: {}",
                song.file_path
            );
            self.finish_startup_restore();
            return background_task;
        }

        let position = std::time::Duration::from_secs_f64(state.position_secs);
        if let Err(err) = self.load_audio_path_paused_for_song(idx, &song, path_buf, position) {
            tracing::warn!("Failed to restore local song {}: {}", song.title, err);
            self.finish_startup_restore();
        } else {
            tracing::info!("Loaded song and seeked to {:?}", position);
            self.finish_startup_restore();
        }

        background_task
    }

    /// Handle database-related messages
    pub fn handle_database(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::DatabaseReady(db) => {
                tracing::info!("Database initialized successfully");
                self.core.db = Some(db.clone());

                // First validate songs in background, then load data
                Some(Task::batch([
                    // Background validation - runs first to clean up invalid entries
                    Task::perform(validate_songs(db.clone()), Message::SongsValidated),
                    // Load data (will run in parallel, but validation is fast)
                    Task::perform(load_songs(db.clone()), Message::SongsLoaded),
                    Task::perform(load_playlists(db.clone()), Message::PlaylistsLoaded),
                    Task::perform(
                        load_watched_folders(db.clone()),
                        Message::WatchedFoldersLoaded,
                    ),
                    Task::perform(load_playback_state(db.clone()), |state| match state {
                        Some(state) => Message::PlaybackStateLoaded(state),
                        None => Message::DatabaseError(crate::error::AppError::new(
                            crate::error::ErrorCode::StorageNotFound,
                            "No saved playback state was found",
                        )),
                    }),
                    Task::perform(load_queue(db.clone()), Message::QueueRestored),
                    Task::perform(load_download_history(db.clone()), Message::DownloadsLoaded),
                ]))
            }

            Message::DatabaseError(err) => {
                tracing::error!("Database error: {}", err);
                self.core.db_error = Some(err.user_summary().to_owned());
                Some(Task::none())
            }

            Message::SongsValidated(removed_count) => {
                if *removed_count > 0 {
                    tracing::info!("Validated songs: {} invalid entries removed", removed_count);
                    // Reload songs after validation to get clean list
                    if let Some(db) = &self.core.db {
                        return Some(Task::perform(load_songs(db.clone()), Message::SongsLoaded));
                    }
                }
                Some(Task::none())
            }

            Message::SongsLoaded(songs) => {
                tracing::info!("Loaded {} songs from database", songs.len());
                self.store_db_song_cover_paths(songs);
                self.library.db_songs = songs.clone();
                self.playback.startup_restore.songs_loaded = true;
                if self.playback.current_song.is_none() || !self.playback.startup_restore.completed
                {
                    return Some(self.try_restore_startup_playback());
                }
                Some(Task::none())
            }

            Message::PlaylistsLoaded(playlists) => {
                tracing::info!("Loaded {} playlists from database", playlists.len());
                self.store_local_playlist_cover_paths(playlists);
                self.library.playlists = playlists.clone();
                if let Some(importing) = &mut self.ui.importing_playlist
                    && let Some(id) = importing.playlist_id
                    && playlists.iter().any(|playlist| playlist.id == id)
                {
                    importing.ready();
                }
                Some(Task::none())
            }

            Message::PlaybackStateLoaded(state) => {
                tracing::info!(
                    "Loaded playback state: position={}, volume={}",
                    state.position_secs,
                    state.volume
                );
                self.set_output_volume(state.volume as f32, false);
                self.playback.saved_state = Some(state.clone());
                self.playback.startup_restore.playback_state_loaded = true;
                self.refresh_playback_runtime();
                Some(self.try_restore_startup_playback())
            }

            Message::QueueRestored(queue) => {
                tracing::info!("Restored {} songs in queue", queue.len());
                self.store_db_song_cover_paths(queue);
                self.clear_shuffle_cache();
                self.playback.queue_artists_by_song_id.clear();
                if self.playback.machine.current.is_none()
                    && self.playback.machine.pending.is_none()
                {
                    self.playback
                        .queue
                        .replace(queue.clone(), PlaybackOrigin::Normal);
                }
                self.playback.startup_restore.queue_loaded = true;

                // Initialize shuffle cache for preloading (must be done before preload)
                self.cache_shuffle_indices();

                // Restore yields to opened files first, then the flush that was
                // waiting for the queue can run.
                Some(Task::batch([
                    self.try_restore_startup_playback(),
                    self.flush_open_local_files(),
                ]))
            }

            Message::SongResolvedForRestore(_idx, result, saved_position, context) => {
                if !self.accepts_audio_context(context) {
                    tracing::debug!(
                        generation = context.generation.0,
                        index = _idx,
                        "Ignoring stale startup restore resolution"
                    );
                    return Some(Task::none());
                }

                let Some(target) = self.playback.machine.preparing(context).cloned() else {
                    return Some(Task::none());
                };
                let Some(idx) = target.entry.and_then(|id| self.playback.queue.index_of(id)) else {
                    self.cancel_playback_transaction();
                    self.finish_startup_restore();
                    return Some(Task::none());
                };
                self.playback.startup_restore.in_progress = false;

                let resolved = match result {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        tracing::warn!(
                            "Failed to resolve NCM song for restore at index {}: {}",
                            idx,
                            error
                        );
                        self.cancel_playback_transaction();
                        self.finish_startup_restore();
                        return Some(Task::none());
                    }
                };
                let Some(song) = self.apply_resolved_song_to_queue(idx, resolved) else {
                    self.cancel_playback_transaction();
                    self.finish_startup_restore();
                    return Some(Task::none());
                };
                if let Some(PlaybackTransaction::Preparing { target, .. }) =
                    &mut self.playback.machine.pending
                {
                    target.quality = resolved.quality.clone();
                }
                let position = std::time::Duration::from_secs_f64(*saved_position);
                let restore_result = if let Some(buffer) = resolved.shared_buffer.clone() {
                    self.load_streaming_buffer_paused_for_song_in_context(
                        &song,
                        buffer,
                        resolved.duration_secs,
                        resolved.finalized_cache_path.clone(),
                        position,
                        context,
                    )
                } else if let Some(path) = resolved.finalized_cache_path.clone() {
                    self.load_audio_path_paused_for_song_in_context(
                        &song,
                        path.into(),
                        position,
                        context,
                    )
                } else {
                    Err(crate::audio::PlaybackError::SourceUnavailable(
                        "resolved restore has no audio source".to_string(),
                    ))
                };
                if let Err(error) = restore_result {
                    tracing::warn!("Failed to restore playback: {error}");
                    self.cancel_playback_transaction();
                }
                self.finish_startup_restore();
                Some(Task::none())
            }

            Message::RecentlyPlayedLoaded(songs) => {
                tracing::info!("Loaded {} recently played songs", songs.len());
                self.store_db_song_cover_paths(songs);
                self.library.recently_played = songs.clone();

                // Create a playlist view for recently played
                let song_views: Vec<pages::PlaylistSongView> = songs
                    .iter()
                    .enumerate()
                    .map(|(i, song)| {
                        let meta = crate::metadata::SongMetadata::from(song);

                        pages::PlaylistSongView::new(
                            crate::ui::components::playlist_view::SongItemData {
                                id: song.id,
                                cover_key: crate::image::song_cover_key_for_source(
                                    song.id,
                                    &song.file_path,
                                ),
                                // Recently played rows may be online NCM entries. Keep
                                // the persisted remote cover URL on the virtual-list
                                // item so its viewport loader can populate the cache.
                                cover_url: song
                                    .cover_path
                                    .as_deref()
                                    .filter(|path| crate::image::is_remote_url(path))
                                    .map(str::to_owned)
                                    .or_else(|| {
                                        std::path::Path::new(&song.file_path)
                                            .is_absolute()
                                            .then(|| song.file_path.clone())
                                    }),
                                index: i + 1,
                                title: meta.title.clone(),
                                artist: meta.artist.clone(),
                                album: meta.album.clone(),
                                duration: meta.duration_display(),
                                added_date: self
                                    .core
                                    .locale
                                    .get(crate::i18n::Key::RecentlyPlayedList)
                                    .to_string(),
                                source: crate::utils::compute_source(
                                    &song.file_path,
                                    song.id,
                                    None,
                                    None,
                                ),
                            },
                        )
                    })
                    .collect();

                // Calculate total duration
                let total_secs: u64 = songs.iter().map(|s| s.duration_secs as u64).sum();
                let total_mins = total_secs / 60;
                let total_hours = total_mins / 60;
                let remaining_mins = total_mins % 60;
                let total_duration = if total_hours > 0 {
                    format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
                } else {
                    format!("{} 分钟", total_mins)
                };

                // Recently played has no playlist-owned cover. Reuse the
                // first available song cover so the detail header can derive
                // the same gradient as ordinary playlist pages.
                let cover_path = songs.iter().find_map(|song| {
                    song.cover_path
                        .as_deref()
                        .filter(|path| crate::image::is_valid_local_path(path))
                        .map(str::to_owned)
                });
                let palette = None;

                // Create playlist view with special ID for recently played
                let playlist_view = pages::PlaylistView {
                    kind: pages::playlist::DetailPageKind::Playlist,
                    id: -1, // Special ID for recently played
                    name: self
                        .core
                        .locale
                        .get(crate::i18n::Key::RecentlyPlayed)
                        .to_string(),
                    description: Some(
                        self.core
                            .locale
                            .get(crate::i18n::Key::RecentlyPlayedDescription)
                            .to_string(),
                    ),
                    profile_stats: None,
                    artist_tab: pages::playlist::ArtistPageTab::TopSongs,
                    artist_albums: Vec::new(),
                    user_playlists: Vec::new(),
                    cover_path,
                    owner: "本地".to_string(),
                    owner_artist_id: None,
                    owner_avatar_path: None,
                    creator_id: 0,
                    song_count: songs.len() as u32,
                    total_duration,
                    like_count: String::new(),
                    songs: song_views,
                    palette,
                    is_local: true,
                    is_subscribed: false,
                    watched_folder_path: None,
                    watch_enabled: false,
                };

                self.ui.playlist_page.current = Some(playlist_view);
                self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);

                Some(Task::none())
            }

            _ => None,
        }
    }
}
