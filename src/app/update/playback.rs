// src/app/update/playback.rs
//! Playback control message handlers

use iced::Task;
use std::time::Duration;

use crate::app::helpers::update_tray_state_with_favorite;
use crate::app::message::Message;
use crate::app::state::App;
use crate::audio::AudioEvent;

impl App {
    /// Handle playback-related messages
    pub fn handle_playback(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::PlaySong(id) => {
                tracing::info!("Playing song id: {}", id);
                // Find song in queue or add it
                if let Some(idx) = self.playback.queue.iter().position(|s| s.id == *id) {
                    return Some(self.play_song_at_index(idx));
                }

                // Try to find in DB songs
                if let Some(song) = self.library.db_songs.iter().find(|s| s.id == *id).cloned() {
                    self.clear_shuffle_cache();
                    self.playback.queue.push(song);
                    self.persist_queue_snapshot();
                    let idx = self.playback.queue.len() - 1;
                    return Some(self.play_song_at_index(idx));
                }

                // Try NCM playlist songs
                if *id < 0 {
                    let ncm_id = (-*id) as u64;
                    if let Some(song_info) = self
                        .ui
                        .home
                        .current_ncm_playlist_songs
                        .iter()
                        .find(|s| s.id == ncm_id)
                        .cloned()
                    {
                        self.set_ncm_scrobble_source(self.current_route_ncm_scrobble_source());
                        self.extend_queue_artist_metadata(std::slice::from_ref(&song_info));
                        let db_song = Self::ncm_track_to_db_song(&song_info);
                        self.clear_shuffle_cache();
                        self.playback.queue.push(db_song);
                        self.persist_queue_snapshot();
                        let idx = self.playback.queue.len() - 1;
                        return Some(self.play_song_at_index(idx));
                    }
                }

                Some(Task::none())
            }

            Message::TogglePlayback => {
                tracing::info!("TogglePlayback message received");
                Some(self.toggle_current_playback())
            }

            Message::NextSong => Some(self.play_next_song()),

            Message::PrevSong => Some(self.play_prev_song()),

            Message::SeekPreview(position) => {
                self.ui.seek_preview_position = Some(*position);
                Some(Task::none())
            }

            Message::SeekRelease => Some(self.apply_seek()),

            Message::SetVolume(volume) => {
                self.set_output_volume(*volume, true);
                Some(Task::none())
            }

            Message::PlaybackTick => Some(self.handle_playback_tick()),

            Message::CyclePlayMode => {
                if self.is_fm_mode() {
                    return Some(Self::toast_error(
                        "私人FM模式下无法更改播放模式".to_string(),
                    ));
                }

                self.core.settings.play_mode = self.core.settings.play_mode.next();
                let _ = self.core.settings.save();
                tracing::info!(
                    "Play mode changed to: {}",
                    self.core.settings.play_mode.display_name()
                );

                let is_playing = self.playback_is_playing();
                self.update_tray_and_mpris_current(is_playing);

                // Clear shuffle cache and re-calculate for new mode
                self.clear_shuffle_cache();
                self.cache_shuffle_indices();

                // Refresh coordinator window (adjacent indices may have changed)
                self.refresh_preload_window();

                Some(self.preload_adjacent_tracks_with_ncm())
            }

            // Streaming playback messages
            Message::StreamingEvent(event) => Some(self.handle_streaming_event(event.clone())),

            Message::AudioEvent(event) => {
                let task = self.handle_audio_event(event.clone());
                if let Some(discord_task) = self.maybe_update_discord(event) {
                    Some(Task::batch(vec![task, discord_task]))
                } else {
                    Some(task)
                }
            }

            _ => None,
        }
    }

    pub(super) fn update_tray_and_mpris_current(&mut self, is_playing: bool) {
        let (title, artist, ncm_song_id, is_favorited) = self
            .playback
            .current_song
            .as_ref()
            .map(|song| {
                let ncm_song_id = (song.id < 0).then(|| (-song.id) as u64);
                let is_favorited = ncm_song_id
                    .and_then(|ncm_id| {
                        self.core
                            .user_info
                            .as_ref()
                            .map(|user| user.like_songs.contains(&ncm_id))
                    })
                    .unwrap_or(false);

                (
                    Some(song.title.clone()),
                    Some(song.artist.clone()),
                    ncm_song_id,
                    is_favorited,
                )
            })
            .unwrap_or((None, None, None, false));
        update_tray_state_with_favorite(
            is_playing,
            title,
            artist,
            self.core.settings.play_mode,
            ncm_song_id,
            is_favorited,
            self.core.locale.language,
        );
        self.update_mpris_state();
    }

    fn apply_seek(&mut self) -> Task<Message> {
        if let Some(preview_pos) = self.ui.seek_preview_position.take() {
            self.refresh_playback_runtime();
            let info = self.playback_info().clone();

            if info.duration.as_secs_f32() > 0.0 {
                let seek_pos =
                    std::time::Duration::from_secs_f32(preview_pos * info.duration.as_secs_f32());
                self.seek_to_position(seek_pos);
            } else if let Some(song) = self.playback.current_song.clone() {
                let path = std::path::PathBuf::from(&song.file_path);
                if !song.file_path.is_empty() && path.exists() {
                    let duration = song.duration_secs as f32;
                    let seek_pos = std::time::Duration::from_secs_f32(preview_pos * duration);
                    if self
                        .restart_audio_path_for_song_at_position(&song, seek_pos)
                        .is_ok()
                    {
                        return Task::none();
                    }
                }
            }
        }
        Task::none()
    }

    pub fn update_audio_tick(&mut self) {
        self.tick_audio_output();
    }

    fn handle_playback_tick(&mut self) -> Task<Message> {
        self.update_audio_tick();
        let analysis_now = std::time::Instant::now();
        if self.core.settings.playback.automix_enabled
            && self
                .playback
                .automix_analysis_poll_at
                .is_none_or(|last| analysis_now.duration_since(last) >= Duration::from_secs(2))
        {
            self.playback.automix_analysis_poll_at = Some(analysis_now);
            if let Some(song) = self.playback.current_song.clone() {
                self.schedule_automix_analysis_window(&song);
            }
        }
        let crossfade_task = self
            .try_start_natural_crossfade()
            .unwrap_or_else(Task::none);
        let now = iced::time::Instant::now();
        let should_sync_mpris = self
            .ui
            .last_mpris_sync
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1));
        if should_sync_mpris {
            self.ui.last_mpris_sync = Some(now);
            self.update_mpris_state();
        }

        let lyrics_scroll_task = if self.ui.lyrics.is_open {
            self.update_lyrics_animations()
        } else {
            Task::none()
        };

        self.check_lyrics_page_close();
        let lyrics_viewport_task = self.flush_pending_lyrics_viewport_after_animation();

        // Auto-save position every 5 seconds
        self.ui.save_position_counter += 1;
        if self.ui.save_position_counter >= 50 {
            self.ui.save_position_counter = 0;
            if let (Some(db), Some(song)) = (&self.core.db, &self.playback.current_song)
                && self.playback_is_playing()
            {
                let info = self.playback_info().clone();
                let position_secs = info.position.as_secs_f64();
                let db = db.clone();
                let song_id = song.id;
                let queue_pos = self.playback.current_index.unwrap_or(0) as i64;
                tokio::spawn(async move {
                    let _ = db
                        .update_playback_position(Some(song_id), queue_pos, position_secs)
                        .await;
                });
            }
        }

        Task::batch([lyrics_scroll_task, lyrics_viewport_task, crossfade_task])
    }

    pub fn handle_audio_event(&mut self, event: AudioEvent) -> Task<Message> {
        if let AudioEvent::PreloadFailed { identity, error } = &event {
            tracing::warn!("Preload failed: identity={:?}, error={}", identity, error);
            self.playback
                .audio_preload_manager
                .mark_failed_by_identity(identity);
            self.release_preload_request(identity.clone());
            return Task::none();
        }

        if !self.audio_event_is_current(&event) {
            return Task::none();
        }
        self.refresh_playback_runtime();

        let should_sync_mpris = matches!(
            &event,
            AudioEvent::SeekComplete { .. }
                | AudioEvent::SeekStarted { .. }
                | AudioEvent::StateChanged { .. }
                | AudioEvent::BufferingStarted { .. }
                | AudioEvent::BufferingEnded { .. }
        );

        match event {
            AudioEvent::Started {
                request_id, path, ..
            } => {
                return self.handle_audio_started_event(request_id, path);
            }
            AudioEvent::Paused {
                request_id,
                position,
                ..
            } => {
                return self.handle_audio_paused_event(request_id, position);
            }
            AudioEvent::Resumed { .. } => return self.handle_audio_resumed_event(),
            AudioEvent::Stopped { .. } => return self.handle_audio_stopped_event(),
            AudioEvent::SeekComplete { position, .. } => {
                tracing::debug!("AudioEvent::SeekComplete at {:?}", position);
            }
            AudioEvent::SeekFailed { error, .. } => {
                tracing::warn!("Seek failed: {}", error);
                if matches!(error, crate::audio::PlaybackError::SeekUnsupported(_)) {
                    return Self::toast_warning("该格式不支持拖动进度条".to_string());
                }
                if matches!(
                    error,
                    crate::audio::PlaybackError::StreamingFailed(_)
                        | crate::audio::PlaybackError::UnsupportedStreaming(_)
                        | crate::audio::PlaybackError::NetworkError(_)
                ) {
                    let progress = self
                        .playback
                        .active_streaming_buffer
                        .as_ref()
                        .map(|buffer| {
                            let target = buffer.policy().high_water_mark_bytes().max(1);
                            (buffer.buffered_ahead().min(target) * 100 / target) as u32
                        })
                        .unwrap_or(0);
                    return Self::toast_info(format!(
                        "正在缓冲中 ({}%)，请稍候再拖动进度",
                        progress
                    ));
                }
            }
            AudioEvent::SeekStarted {
                target_position, ..
            } => {
                tracing::debug!("AudioEvent::SeekStarted: target={:?}", target_position);
            }
            AudioEvent::StateChanged {
                old_status,
                new_status,
                ..
            } => {
                tracing::debug!(
                    "AudioEvent::StateChanged: {:?} -> {:?}",
                    old_status,
                    new_status
                );
            }
            AudioEvent::CacheProgress {
                cached,
                total,
                progress,
                ..
            } => {
                tracing::trace!(
                    "AudioEvent::CacheProgress: {}/{} ({:.1}%)",
                    cached,
                    total,
                    progress * 100.0
                );
            }
            AudioEvent::BufferingStarted { position, .. } => {
                tracing::info!("AudioEvent::BufferingStarted at {:?}", position);
            }
            AudioEvent::BufferingEnded { .. } => {
                tracing::info!("AudioEvent::BufferingEnded");
            }
            AudioEvent::PreloadReady {
                identity,
                duration,
                path,
            } => {
                tracing::debug!(
                    "AudioEvent::PreloadReady: identity={:?}, path={:?}",
                    identity,
                    path
                );
                self.handle_audio_preload_ready(identity, duration, path);
            }
            AudioEvent::PreloadFailed { .. } => {
                unreachable!("preload failures are handled before current-event filtering")
            }
            AudioEvent::DeviceSwitched { restore_state } => {
                tracing::info!("Audio device switched: {:?}", restore_state);
            }
            AudioEvent::DeviceSwitchFailed { error } => {
                tracing::error!("Device switch failed: {}", error);
                return Self::toast_error(format!("切换音频设备失败: {}", error));
            }
            AudioEvent::Finished { .. } => {
                tracing::info!("Song finished (AudioEvent::Finished)");
                if self.playback.pending_resolution_index.is_none()
                    && self.playback.current_song.is_some()
                {
                    return self.handle_song_finished();
                }
            }
            AudioEvent::Error {
                request_id, error, ..
            } => return self.handle_audio_error_event(request_id, error),
        }

        if should_sync_mpris {
            self.refresh_playback_runtime();
            self.update_mpris_state();
        }

        Task::none()
    }

    fn accepts_preload_identity(&self, identity: &crate::audio::identity::PreloadIdentity) -> bool {
        self.accepts_audio_preload_identity(identity)
    }
    pub(super) fn audio_event_is_current(&self, event: &AudioEvent) -> bool {
        match event {
            AudioEvent::DeviceSwitched { .. } | AudioEvent::DeviceSwitchFailed { .. } => true,
            AudioEvent::PreloadReady { identity, .. }
            | AudioEvent::PreloadFailed { identity, .. } => self.accepts_preload_identity(identity),
            AudioEvent::SeekComplete { context, nonce, .. }
            | AudioEvent::SeekFailed { context, nonce, .. }
            | AudioEvent::SeekStarted { context, nonce, .. } => {
                self.accepts_audio_seek(context, *nonce)
            }
            AudioEvent::Started { context, .. }
            | AudioEvent::Paused { context, .. }
            | AudioEvent::Resumed { context }
            | AudioEvent::Stopped { context }
            | AudioEvent::StateChanged { context, .. }
            | AudioEvent::CacheProgress { context, .. }
            | AudioEvent::BufferingStarted { context, .. }
            | AudioEvent::BufferingEnded { context }
            | AudioEvent::Finished { context }
            | AudioEvent::Error { context, .. } => self.accepts_audio_context(context),
        }
    }
    fn handle_streaming_event(
        &mut self,
        event: crate::audio::streaming::StreamingEvent,
    ) -> Task<Message> {
        use crate::audio::streaming::{StreamingEventKind, StreamingIdentity};

        let is_current = match &event.identity {
            StreamingIdentity::Playback(context) => self.accepts_audio_context(context),
            StreamingIdentity::Preload(identity) => self.accepts_preload_identity(identity),
        };
        if !is_current {
            return Task::none();
        }

        match event.kind {
            StreamingEventKind::Playable => tracing::info!("Streaming track is now playable"),
            StreamingEventKind::CacheProgress(cached, total) => {
                tracing::trace!("Streaming cache progress: {}/{} bytes", cached, total)
            }
            StreamingEventKind::DownloadComplete => {
                tracing::info!("Streaming download complete; final cache publication pending")
            }
            StreamingEventKind::Complete => {
                tracing::info!("Streaming download and cache publication complete")
            }
            StreamingEventKind::CacheFinalized(path) => {
                tracing::info!("Streaming cache finalized at {:?}", path)
            }
            StreamingEventKind::CacheFinalizationFailed(err) => {
                tracing::warn!(
                    "Streaming cache finalization failed; continuing with ring: {}",
                    err
                )
            }
            StreamingEventKind::Error(err) => {
                tracing::error!(identity = ?event.identity, error = %err, "Streaming coordinator failed");
                // SharedBuffer health is authoritative after startup. The audio
                // thread stops a terminal current stream and emits the single
                // user-visible AudioEvent::Error; removing the app reference
                // here would duplicate the toast and leave the Sink unmanaged.
            }
        }
        Task::none()
    }
}
