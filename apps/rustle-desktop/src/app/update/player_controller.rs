// src/app/update/player_controller.rs
//! Unified player controller for all playback operations
//!
//! Uses QueueNavigator as Single Source of Truth for index calculations.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use iced::Task;

use crate::api::{ArtistSummary, Track};
use crate::app::message::Message;
use crate::app::state::{App, PendingPlaybackKind, PendingPlaybackRequest};
use crate::database::DbSong;
use crate::features::PlayMode;
use crate::i18n::Key;

use super::audio_preload_manager::PreloadDirection;
use super::preload_coordinator::WindowChange;
use super::queue_navigator::QueueNavigator;
use super::song_resolver::ResolvedSong;

#[derive(Clone, Copy)]
pub(super) enum TrackGainMode {
    MetadataOnly,
    AnalyzeIfMissing,
}

pub(super) enum PlaybackSource {
    AudioPath {
        path: PathBuf,
        gain_mode: TrackGainMode,
        start_position: Option<std::time::Duration>,
    },
    StreamingBuffer {
        buffer: crate::audio::SharedBuffer,
        duration_secs: Option<u64>,
        finalized_cache_path: Option<String>,
    },
    Preloaded {
        identity: crate::audio::identity::PreloadIdentity,
        buffer: Option<crate::audio::SharedBuffer>,
    },
}

/// Maximum consecutive failures before stopping playback
const MAX_CONSECUTIVE_FAILURES: u8 = 3;

/// Transfer the app-side streaming reference after the audio thread has
/// committed a preloaded handoff. The audio thread may still own the outgoing
/// buffer for Crossfade/Automix, so dropping this clone must not cancel it.
fn handoff_active_streaming_buffer(
    active: &mut Option<crate::audio::SharedBuffer>,
    incoming: Option<crate::audio::SharedBuffer>,
) {
    *active = incoming;
}

fn extend_ncm_artist_metadata(cache: &mut HashMap<u64, Vec<ArtistSummary>>, tracks: &[Track]) {
    cache.extend(tracks.iter().map(|track| (track.id, track.artists.clone())));
}

fn apply_resolved_song_metadata(song: &mut DbSong, resolved: &ResolvedSong) {
    // `finalized_cache_path` is a playback transport, not song identity.
    // Keep `ncm://<id>` (or the negative application ID) intact so cover,
    // lyrics, favorites, persistence, and later quality negotiation still
    // resolve the same NCM resource after the cache is published.
    if let Some(cover_path) = &resolved.cover_path {
        song.cover_path = Some(cover_path.clone());
    }
}

impl App {
    pub(super) fn replace_queue_artist_metadata(&mut self, tracks: &[Track]) {
        self.playback.queue_artists_by_song_id.clear();
        self.extend_queue_artist_metadata(tracks);
    }

    pub(super) fn extend_queue_artist_metadata(&mut self, tracks: &[Track]) {
        extend_ncm_artist_metadata(&mut self.playback.queue_artists_by_song_id, tracks);
    }

    pub(super) fn prune_queue_artist_metadata(&mut self) {
        let queued_ncm_ids: HashSet<u64> = self
            .playback
            .queue
            .iter()
            .filter_map(|song| (song.id < 0).then_some((-song.id) as u64))
            .collect();
        self.playback
            .queue_artists_by_song_id
            .retain(|song_id, _| queued_ncm_ids.contains(song_id));
    }

    fn resolve_artists_for_song(&self, song: &DbSong) -> Vec<crate::api::ArtistSummary> {
        let ncm_id = if song.id < 0 {
            Some((-song.id) as u64)
        } else {
            None
        };

        if let Some(artists) = ncm_id
            .and_then(|id| self.playback.queue_artists_by_song_id.get(&id))
            .filter(|artists| !artists.is_empty())
        {
            return artists.clone();
        }

        self.ui
            .playlist_page
            .current_online_tracks()
            .into_iter()
            .flatten()
            .chain(self.ui.home.current_ncm_playlist_songs.iter())
            .chain(self.ui.search.tracks.iter())
            .find(|candidate| {
                if let Some(id) = ncm_id {
                    candidate.id == id
                } else {
                    candidate.title == song.title
                        && candidate.artist_names() == song.artist
                        && candidate.album.name == song.album
                }
            })
            .map(|song| song.artists.clone())
            .unwrap_or_default()
    }

    pub(super) fn effective_queue_play_mode(&self) -> PlayMode {
        if self.is_fm_mode() {
            PlayMode::Sequential
        } else {
            self.core.settings.play_mode
        }
    }

    pub(super) fn replace_active_streaming_buffer(
        &mut self,
        next_buffer: Option<crate::audio::SharedBuffer>,
    ) {
        self.playback.active_streaming_buffer.take();
        self.playback.active_streaming_buffer = next_buffer;
    }

    fn cache_track_gain_in_memory(&mut self, song: &DbSong, normalization_gain: f64) {
        let update_song = |candidate: &mut DbSong| {
            candidate.normalization_gain = Some(normalization_gain);
        };

        for candidate in &mut self.playback.queue {
            if candidate.id == song.id || candidate.file_path == song.file_path {
                update_song(candidate);
            }
        }

        for candidate in &mut self.library.db_songs {
            if candidate.id == song.id || candidate.file_path == song.file_path {
                update_song(candidate);
            }
        }

        for candidate in &mut self.library.recently_played {
            if candidate.id == song.id || candidate.file_path == song.file_path {
                update_song(candidate);
            }
        }

        if let Some(current_song) = &mut self.playback.current_song
            && (current_song.id == song.id || current_song.file_path == song.file_path)
        {
            update_song(current_song);
        }
    }

    fn fade_in_enabled(&self) -> bool {
        self.core.settings.playback.fade_in_out
    }

    fn resolve_queue_index_for_song(&self, song: &DbSong) -> Option<usize> {
        self.playback
            .current_index
            .filter(|&idx| {
                self.playback
                    .queue
                    .get(idx)
                    .map(|candidate| {
                        candidate.id == song.id || candidate.file_path == song.file_path
                    })
                    .unwrap_or(false)
            })
            .or_else(|| {
                self.playback.queue.iter().position(|candidate| {
                    candidate.id == song.id || candidate.file_path == song.file_path
                })
            })
    }

    fn queue_pending_playback_request(
        &mut self,
        request_id: u64,
        queue_index: Option<usize>,
        song: DbSong,
        kind: PendingPlaybackKind,
    ) {
        self.playback.pause_requested = false;
        self.playback.pending_playback_request = Some(PendingPlaybackRequest {
            request_id,
            queue_index,
            song,
            kind,
        });
    }

    fn take_pending_playback_request(&mut self, request_id: u64) -> Option<PendingPlaybackRequest> {
        if self
            .playback
            .pending_playback_request
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            self.playback.pending_playback_request.take()
        } else {
            None
        }
    }

    fn promote_scheduled_transition_buffer(&mut self, request_id: u64) {
        if self.playback.scheduled_transition_request_id == Some(request_id) {
            let incoming = self.playback.scheduled_transition_buffer.take();
            self.playback.scheduled_transition_request_id = None;
            self.playback.pending_transition = None;
            self.playback.pending_transition_trigger = None;
            handoff_active_streaming_buffer(&mut self.playback.active_streaming_buffer, incoming);
        }
    }

    pub(super) fn apply_resolved_song_to_queue(
        &mut self,
        idx: usize,
        resolved: &ResolvedSong,
    ) -> Option<DbSong> {
        let song = self.playback.queue.get_mut(idx)?;
        apply_resolved_song_metadata(song, resolved);

        if let Some(db) = &self.core.db {
            let db = db.clone();
            let song_clone = song.clone();
            tokio::spawn(async move {
                let _ = db.upsert_ncm_song(&song_clone).await;
            });
        }

        Some(song.clone())
    }

    fn playback_source_from_resolved_song(
        song: &DbSong,
        resolved: &ResolvedSong,
    ) -> crate::audio::PlaybackResult<PlaybackSource> {
        if let Some(buffer) = resolved.shared_buffer.clone() {
            return Ok(PlaybackSource::StreamingBuffer {
                buffer,
                duration_secs: resolved.duration_secs,
                finalized_cache_path: resolved.finalized_cache_path.clone(),
            });
        }

        if let Some(path) = &resolved.finalized_cache_path {
            let path = PathBuf::from(path);
            if !path.is_file() {
                return Err(crate::audio::PlaybackError::FileNotFound(
                    path.to_string_lossy().into_owned(),
                ));
            }
            return Ok(PlaybackSource::AudioPath {
                path,
                gain_mode: TrackGainMode::MetadataOnly,
                start_position: None,
            });
        }

        Self::audio_path_source_for_song(song)
    }

    pub(super) fn resolve_track_gain_for_song(
        &mut self,
        song: &DbSong,
        gain_mode: TrackGainMode,
    ) -> f32 {
        if !self.core.settings.playback.volume_normalization {
            return 1.0;
        }

        if let Some(cached_gain) = song.normalization_gain {
            return cached_gain as f32;
        }

        let path = Path::new(&song.file_path);
        if !path.exists() {
            return 1.0;
        }

        // Try metadata tags first (fast, synchronous)
        if let Some(tagged_gain) = crate::features::extract_track_gain(path) {
            let gain = tagged_gain as f64;
            self.cache_track_gain_in_memory(song, gain);
            if let Some(db) = &self.core.db {
                let db = db.clone();
                let song_id = song.id;
                let file_path = song.file_path.clone();
                tokio::spawn(async move {
                    let _ = db
                        .update_song_normalization(song_id, &file_path, gain)
                        .await;
                });
            }
            return tagged_gain;
        }

        // No tags available — launch waveform analysis in background if mode allows it.
        // Play immediately with unity gain to avoid blocking the UI thread.
        // Result is saved to DB for the next session; no in-memory update needed
        // because the gain won't apply until the next play or restart anyway.
        if matches!(gain_mode, TrackGainMode::AnalyzeIfMissing) {
            let path_buf = path.to_path_buf();
            let db = self.core.db.clone();
            let song_id = song.id;
            let file_path = song.file_path.clone();

            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::features::resolve_track_gain(&path_buf)
                })
                .await;

                if let Ok(Some(gain)) = result
                    && let Some(db) = db
                {
                    let _ = db
                        .update_song_normalization(song_id, &file_path, gain as f64)
                        .await;
                }
            });
        }

        1.0
    }

    pub(super) fn play_audio_path_for_song(
        &mut self,
        song: &DbSong,
        path: PathBuf,
        gain_mode: TrackGainMode,
    ) -> crate::audio::PlaybackResult<u64> {
        let track_gain = self.resolve_track_gain_for_song(song, gain_mode);
        let fade_in = self.fade_in_enabled();
        self.play_audio_file(path, fade_in, track_gain)
    }

    pub(super) fn play_audio_path_at_position_for_song(
        &mut self,
        song: &DbSong,
        path: PathBuf,
        gain_mode: TrackGainMode,
        position: std::time::Duration,
    ) -> crate::audio::PlaybackResult<u64> {
        let track_gain = self.resolve_track_gain_for_song(song, gain_mode);
        let fade_in = self.fade_in_enabled();
        self.play_audio_file_at_position(path, position, fade_in, track_gain)
    }

    pub(super) fn play_streaming_buffer_for_song(
        &mut self,
        song: &DbSong,
        buffer: crate::audio::SharedBuffer,
        duration_secs: Option<u64>,
        finalized_cache_path: Option<String>,
    ) -> crate::audio::PlaybackResult<u64> {
        let track_gain = self.resolve_track_gain_for_song(song, TrackGainMode::MetadataOnly);
        let fade_in = self.fade_in_enabled();
        let duration =
            std::time::Duration::from_secs(duration_secs.unwrap_or(song.duration_secs as u64));
        let cache_path = finalized_cache_path.map(PathBuf::from);
        self.play_streaming_audio(buffer, duration, cache_path, fade_in, track_gain)
    }

    pub(super) fn play_preloaded_request(
        &mut self,
        identity: crate::audio::identity::PreloadIdentity,
        buffer: Option<crate::audio::SharedBuffer>,
    ) -> crate::audio::PlaybackResult<u64> {
        let transition = self.playback.pending_transition.take();
        let trigger_at = self.playback.pending_transition_trigger.take();
        match (trigger_at, transition) {
            (Some(trigger_at), Some(transition)) => {
                self.playback.scheduled_transition_buffer = buffer;
                match self.schedule_preloaded_audio_transition(
                    identity,
                    trigger_at,
                    self.fade_in_enabled(),
                    transition,
                ) {
                    Ok(request_id) => {
                        self.playback.scheduled_transition_request_id = Some(request_id);
                        Ok(request_id)
                    }
                    Err(error) => {
                        self.clear_scheduled_transition_state();
                        Err(error)
                    }
                }
            }
            (_, transition) => {
                self.clear_scheduled_transition_state();
                let request_id =
                    self.play_preloaded_audio(identity, self.fade_in_enabled(), transition)?;
                // Keep the outgoing stream owned until the audio thread has
                // validated and actually started the preload. Started promotes
                // this buffer; Error drops it and can resolve the same track.
                self.playback.scheduled_transition_buffer = buffer;
                self.playback.scheduled_transition_request_id = Some(request_id);
                Ok(request_id)
            }
        }
    }

    pub(super) fn audio_path_source_for_song(
        song: &DbSong,
    ) -> crate::audio::PlaybackResult<PlaybackSource> {
        let path = PathBuf::from(&song.file_path);
        if song.file_path.is_empty() || !path.exists() {
            return Err(crate::audio::PlaybackError::FileNotFound(
                song.file_path.clone(),
            ));
        }

        Ok(PlaybackSource::AudioPath {
            path,
            gain_mode: TrackGainMode::AnalyzeIfMissing,
            start_position: None,
        })
    }

    pub(super) fn audio_path_source_for_song_at_position(
        song: &DbSong,
        position: std::time::Duration,
    ) -> crate::audio::PlaybackResult<PlaybackSource> {
        let path = PathBuf::from(&song.file_path);
        if song.file_path.is_empty() || !path.exists() {
            return Err(crate::audio::PlaybackError::FileNotFound(
                song.file_path.clone(),
            ));
        }

        Ok(PlaybackSource::AudioPath {
            path,
            gain_mode: TrackGainMode::AnalyzeIfMissing,
            start_position: Some(position),
        })
    }

    pub(super) fn start_audio_source_for_song(
        &mut self,
        song: &DbSong,
        source: PlaybackSource,
    ) -> crate::audio::PlaybackResult<u64> {
        match source {
            PlaybackSource::AudioPath {
                path,
                gain_mode,
                start_position,
            } => {
                self.clear_scheduled_transition_state();
                self.replace_active_streaming_buffer(None);
                if let Some(position) = start_position {
                    self.play_audio_path_at_position_for_song(song, path, gain_mode, position)
                } else {
                    self.play_audio_path_for_song(song, path, gain_mode)
                }
            }
            PlaybackSource::StreamingBuffer {
                buffer,
                duration_secs,
                finalized_cache_path,
            } => {
                self.clear_scheduled_transition_state();
                let active_buffer = buffer.clone();
                self.replace_active_streaming_buffer(Some(active_buffer));
                self.play_streaming_buffer_for_song(
                    song,
                    buffer,
                    duration_secs,
                    finalized_cache_path,
                )
            }
            PlaybackSource::Preloaded { identity, buffer } => {
                self.play_preloaded_request(identity, buffer)
            }
        }
    }

    pub(super) fn start_resolved_audio_source_for_song(
        &mut self,
        song: &DbSong,
        source: PlaybackSource,
        context: &crate::audio::identity::PlaybackContext,
    ) -> crate::audio::PlaybackResult<u64> {
        match source {
            PlaybackSource::AudioPath {
                path,
                gain_mode,
                start_position,
            } => {
                self.clear_scheduled_transition_state();
                let track_gain = self.resolve_track_gain_for_song(song, gain_mode);
                let fade_in = self.fade_in_enabled();
                let request_id = if let Some(position) = start_position {
                    self.play_audio_file_at_position_in_context(
                        context, path, position, fade_in, track_gain,
                    )?
                } else {
                    self.play_audio_file_in_context(context, path, fade_in, track_gain)?
                };
                self.replace_active_streaming_buffer(None);
                Ok(request_id)
            }
            PlaybackSource::StreamingBuffer {
                buffer,
                duration_secs,
                finalized_cache_path,
            } => {
                self.clear_scheduled_transition_state();
                let track_gain =
                    self.resolve_track_gain_for_song(song, TrackGainMode::MetadataOnly);
                let fade_in = self.fade_in_enabled();
                let duration = std::time::Duration::from_secs(
                    duration_secs.unwrap_or(song.duration_secs as u64),
                );
                let cache_path = finalized_cache_path.map(PathBuf::from);
                let request_id = self.play_streaming_audio_in_context(
                    context,
                    buffer.clone(),
                    duration,
                    cache_path,
                    fade_in,
                    track_gain,
                )?;
                self.replace_active_streaming_buffer(Some(buffer));
                Ok(request_id)
            }
            PlaybackSource::Preloaded { .. } => {
                Err(crate::audio::PlaybackError::InvariantViolation(
                    "resolved playback cannot promote a preload".to_string(),
                ))
            }
        }
    }

    pub(super) fn load_audio_path_paused_for_song(
        &mut self,
        song: &DbSong,
        path: PathBuf,
        position: std::time::Duration,
    ) -> crate::audio::PlaybackResult<()> {
        let track_gain = self.resolve_track_gain_for_song(song, TrackGainMode::AnalyzeIfMissing);
        let request_id = self.load_audio_file_paused(path, position, track_gain)?;
        let queue_index = self.resolve_queue_index_for_song(song);
        self.queue_pending_playback_request(
            request_id,
            queue_index,
            song.clone(),
            PendingPlaybackKind::LoadPaused,
        );
        self.replace_active_streaming_buffer(None);
        Ok(())
    }

    pub(super) fn load_audio_path_paused_for_song_in_context(
        &mut self,
        song: &DbSong,
        path: PathBuf,
        position: std::time::Duration,
        context: &crate::audio::identity::PlaybackContext,
    ) -> crate::audio::PlaybackResult<()> {
        let track_gain = self.resolve_track_gain_for_song(song, TrackGainMode::AnalyzeIfMissing);
        let request_id =
            self.load_audio_file_paused_in_context(context, path, position, track_gain)?;
        let queue_index = self.resolve_queue_index_for_song(song);
        self.queue_pending_playback_request(
            request_id,
            queue_index,
            song.clone(),
            PendingPlaybackKind::LoadPaused,
        );
        self.replace_active_streaming_buffer(None);
        Ok(())
    }

    pub(super) fn load_streaming_buffer_paused_for_song_in_context(
        &mut self,
        song: &DbSong,
        buffer: crate::audio::SharedBuffer,
        duration_secs: Option<u64>,
        finalized_cache_path: Option<String>,
        position: std::time::Duration,
        context: &crate::audio::identity::PlaybackContext,
    ) -> crate::audio::PlaybackResult<()> {
        let track_gain = self.resolve_track_gain_for_song(song, TrackGainMode::MetadataOnly);
        let duration =
            std::time::Duration::from_secs(duration_secs.unwrap_or(song.duration_secs as u64));
        let cache_path = finalized_cache_path.map(PathBuf::from);
        let request_id = self.load_streaming_audio_paused_in_context(
            context,
            buffer.clone(),
            duration,
            cache_path,
            position,
            track_gain,
        )?;
        let queue_index = self.resolve_queue_index_for_song(song);
        self.queue_pending_playback_request(
            request_id,
            queue_index,
            song.clone(),
            PendingPlaybackKind::LoadPaused,
        );
        self.replace_active_streaming_buffer(Some(buffer));
        Ok(())
    }

    pub(super) fn restart_audio_path_for_song_at_position(
        &mut self,
        song: &DbSong,
        position: std::time::Duration,
    ) -> crate::audio::PlaybackResult<()> {
        let source = Self::audio_path_source_for_song_at_position(song, position)?;
        let request_id = self.start_audio_source_for_song(song, source)?;
        let queue_index = self.resolve_queue_index_for_song(song);
        self.queue_pending_playback_request(
            request_id,
            queue_index,
            song.clone(),
            PendingPlaybackKind::RestartCurrent,
        );
        Ok(())
    }

    pub(super) fn start_queue_song_from_source(
        &mut self,
        idx: usize,
        song: DbSong,
        source: PlaybackSource,
    ) -> crate::audio::PlaybackResult<Task<Message>> {
        let request_id = self.start_audio_source_for_song(&song, source)?;
        self.queue_pending_playback_request(
            request_id,
            Some(idx),
            song,
            PendingPlaybackKind::StartPlaying,
        );
        Ok(Task::none())
    }

    fn start_resolved_queue_song_from_source(
        &mut self,
        idx: usize,
        song: DbSong,
        source: PlaybackSource,
        context: &crate::audio::identity::PlaybackContext,
    ) -> crate::audio::PlaybackResult<Task<Message>> {
        let request_id = self.start_resolved_audio_source_for_song(&song, source, context)?;
        self.queue_pending_playback_request(
            request_id,
            Some(idx),
            song,
            PendingPlaybackKind::StartPlaying,
        );
        Ok(Task::none())
    }

    /// Central method to play a song at a specific queue index
    pub fn play_song_at_index(&mut self, idx: usize) -> Task<Message> {
        if idx >= self.playback.queue.len() {
            tracing::warn!("Invalid queue index: {}", idx);
            return Task::none();
        }

        let song = self.playback.queue[idx].clone();
        // Quality belongs to a single song/request generation. Clear it before
        // resolving a different queue item so stale UI never survives a fast
        // song switch.
        self.playback.current_quality = None;

        if super::song_resolver::needs_resolution(&song) {
            self.playback.current_index = Some(idx);
            tracing::info!("Song {} needs resolution", song.title);
            return self.resolve_and_play(idx, song);
        }

        let source = match Self::audio_path_source_for_song(&song) {
            Ok(source) => source,
            Err(err) => {
                tracing::error!("Failed to play {}: {}", song.title, err);
                return self.handle_playback_failure(idx, &err.to_string());
            }
        };

        match self.start_queue_song_from_source(idx, song.clone(), source) {
            Ok(task) => task,
            Err(err) => {
                tracing::error!("Failed to play {}: {}", song.title, err);
                self.handle_playback_failure(idx, &err.to_string())
            }
        }
    }

    /// Handle playback failure with consecutive failure detection
    /// Design: Skip failed songs and continue to next, show toast after MAX_CONSECUTIVE_FAILURES
    pub fn handle_playback_failure(&mut self, failed_idx: usize, error: &str) -> Task<Message> {
        self.playback.consecutive_failures += 1;

        tracing::warn!(
            "Playback failure {} of {}: {}",
            self.playback.consecutive_failures,
            MAX_CONSECUTIVE_FAILURES,
            error
        );

        // Show warning after too many consecutive failures
        let toast_task = if self.playback.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            tracing::warn!(
                "Consecutive failures reached {}, showing warning and stopping retry",
                self.playback.consecutive_failures
            );

            Self::toast_error(format!(
                "连续 {} 首歌曲播放失败，已停止播放",
                MAX_CONSECUTIVE_FAILURES
            ))
        } else {
            Task::none()
        };

        // Only skip to next if we haven't exceeded max failures
        // This prevents infinite loop when all songs fail
        if self.playback.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            // Stop trying - reset counter for next user-initiated play
            self.playback.consecutive_failures = 0;
            return toast_task;
        }

        // Skip to next playable song
        Task::batch([toast_task, self.skip_to_next_playable(failed_idx)])
    }

    fn save_playback_position_snapshot(&self, song_id: i64, queue_pos: i64, position_secs: f64) {
        if let Some(db) = &self.core.db {
            let db = db.clone();
            tokio::spawn(async move {
                let _ = db
                    .update_playback_position(Some(song_id), queue_pos, position_secs)
                    .await;
            });
        }
    }

    fn commit_current_song_playback_state(
        &mut self,
        queue_index: Option<usize>,
        song: DbSong,
        is_playing: bool,
    ) -> Task<Message> {
        if let Some(idx) = queue_index {
            self.playback.current_index = Some(idx);
        }

        let artists = self.resolve_artists_for_song(&song);
        self.playback.current_artist_id = artists.first().map(|artist| artist.id);
        self.playback.current_artists = artists;
        self.playback.current_song = Some(song.clone());
        self.playback.consecutive_failures = 0;
        self.playback.crossfade_triggered = false;
        self.playback.pause_requested = false;
        self.update_tray_and_mpris_current(is_playing);
        Task::none()
    }

    fn on_song_loaded_paused(&mut self, idx: usize, song: DbSong) -> Task<Message> {
        tracing::info!("Loaded paused song: {} - {}", song.title, song.artist);

        let cover_task = self.commit_current_song_playback_state(Some(idx), song.clone(), false);
        self.cache_shuffle_indices();
        self.refresh_preload_window();
        Task::batch([cover_task, self.schedule_post_switch_side_effects(&song)])
    }

    pub fn handle_audio_started_event(
        &mut self,
        request_id: u64,
        path: Option<PathBuf>,
    ) -> Task<Message> {
        tracing::debug!(
            "AudioEvent::Started: request_id={}, path={:?}",
            request_id,
            path
        );

        self.promote_scheduled_transition_buffer(request_id);
        let Some(pending) = self.take_pending_playback_request(request_id) else {
            tracing::debug!(
                "Ignoring Started for stale/unknown request_id={}",
                request_id
            );
            return Task::none();
        };

        match pending.kind {
            PendingPlaybackKind::StartPlaying => {
                if let Some(idx) = pending.queue_index {
                    self.on_song_started(idx, pending.song)
                } else {
                    self.commit_current_song_playback_state(None, pending.song, true)
                }
            }
            PendingPlaybackKind::RestartCurrent => {
                self.commit_current_song_playback_state(pending.queue_index, pending.song, true)
            }
            PendingPlaybackKind::LoadPaused => {
                tracing::debug!("Ignoring Started for paused-load request_id={}", request_id);
                Task::none()
            }
        }
    }

    pub fn handle_audio_paused_event(
        &mut self,
        request_id: Option<u64>,
        position: std::time::Duration,
    ) -> Task<Message> {
        tracing::debug!(
            "AudioEvent::Paused: request_id={:?}, position={:?}",
            request_id,
            position
        );

        if let Some(request_id) = request_id {
            let Some(pending) = self.take_pending_playback_request(request_id) else {
                tracing::debug!(
                    "Ignoring Paused for stale/unknown request_id={}",
                    request_id
                );
                return Task::none();
            };
            self.playback.pause_requested = false;

            return match pending.kind {
                PendingPlaybackKind::LoadPaused => {
                    if let Some(idx) = pending.queue_index {
                        self.on_song_loaded_paused(idx, pending.song)
                    } else {
                        self.commit_current_song_playback_state(None, pending.song, false)
                    }
                }
                PendingPlaybackKind::RestartCurrent => self.commit_current_song_playback_state(
                    pending.queue_index,
                    pending.song,
                    false,
                ),
                PendingPlaybackKind::StartPlaying => {
                    tracing::debug!(
                        "Ignoring Paused for playing-track request_id={}",
                        request_id
                    );
                    Task::none()
                }
            };
        }

        self.playback.pause_requested = false;
        if let (Some(song), Some(queue_index)) =
            (&self.playback.current_song, self.playback.current_index)
        {
            self.save_playback_position_snapshot(
                song.id,
                queue_index as i64,
                position.as_secs_f64(),
            );
        }
        self.update_tray_and_mpris_current(false);
        Task::none()
    }

    pub fn handle_audio_resumed_event(&mut self) -> Task<Message> {
        tracing::debug!("AudioEvent::Resumed");
        self.playback.pause_requested = false;
        self.update_tray_and_mpris_current(true);
        Task::none()
    }

    pub fn handle_audio_stopped_event(&mut self) -> Task<Message> {
        tracing::debug!("AudioEvent::Stopped");
        self.playback.pause_requested = false;
        if self.playback.pending_playback_request.is_none() {
            self.update_tray_and_mpris_current(false);
        }
        Task::none()
    }

    pub fn handle_audio_error_event(
        &mut self,
        request_id: Option<u64>,
        error: crate::audio::PlaybackError,
    ) -> Task<Message> {
        tracing::error!("Audio error: request_id={:?}, error={}", request_id, error);

        let unhealthy_preload = matches!(&error, crate::audio::PlaybackError::UnhealthyPreload(_));
        let toast_message = match &error {
            crate::audio::PlaybackError::UnsupportedFormat(_)
            | crate::audio::PlaybackError::SeekUnsupported(_) => "不支持的音频格式".to_string(),
            crate::audio::PlaybackError::UnsupportedStreaming(_) => {
                "音频源不支持 Range 流式播放".to_string()
            }
            crate::audio::PlaybackError::NetworkError(_)
            | crate::audio::PlaybackError::StreamingFailed(_) => "网络读取出错".to_string(),
            crate::audio::PlaybackError::FileNotFound(_)
            | crate::audio::PlaybackError::SourceUnavailable(_) => "文件不存在".to_string(),
            crate::audio::PlaybackError::IoError(_) => "文件读取出错".to_string(),
            crate::audio::PlaybackError::UnhealthyPreload(_) => {
                "预加载已失效，正在重新加载".to_string()
            }
            _ => format!("播放错误: {}", error),
        };

        if let Some(request_id) = request_id {
            let was_scheduled = self.playback.scheduled_transition_request_id == Some(request_id);
            let Some(pending) = self.take_pending_playback_request(request_id) else {
                tracing::debug!("Ignoring Error for stale/unknown request_id={}", request_id);
                return Task::none();
            };
            if was_scheduled {
                self.clear_scheduled_transition_state();
            }
            self.playback.pause_requested = false;

            return match pending.kind {
                PendingPlaybackKind::StartPlaying => {
                    if let Some(idx) = pending.queue_index {
                        if unhealthy_preload {
                            tracing::warn!(
                                queue_index = idx,
                                request_id,
                                "Falling back once to normal resolution for unhealthy preload"
                            );
                            let fallback = self.play_song_at_index(idx);
                            self.replace_active_streaming_buffer(None);
                            fallback
                        } else {
                            self.handle_playback_failure(idx, &error.to_string())
                        }
                    } else {
                        Self::toast_error(toast_message)
                    }
                }
                PendingPlaybackKind::LoadPaused | PendingPlaybackKind::RestartCurrent => {
                    Self::toast_error(toast_message)
                }
            };
        }

        self.playback.pause_requested = false;
        self.replace_active_streaming_buffer(None);
        Self::toast_error(toast_message)
    }

    pub fn pause_current_playback(&mut self) {
        self.refresh_playback_runtime();

        if let (Some(song), Some(queue_index)) =
            (&self.playback.current_song, self.playback.current_index)
        {
            let position = self.playback_info().position.as_secs_f64();
            self.save_playback_position_snapshot(song.id, queue_index as i64, position);
        }

        match self.pause_audio_output_with_fade(self.fade_in_enabled()) {
            Ok(()) => {
                self.playback.pause_requested = true;
                self.playback.runtime.info.status = crate::audio::PlaybackStatus::Paused;
                self.update_tray_and_mpris_current(false);
            }
            Err(error) => tracing::warn!("Failed to enqueue pause: {error}"),
        }
    }

    pub fn resume_current_playback(&mut self) {
        match self.resume_audio_output_with_fade(self.fade_in_enabled()) {
            Ok(()) => {
                self.playback.pause_requested = false;
                self.playback.runtime.info.status = crate::audio::PlaybackStatus::Playing;
                self.update_tray_and_mpris_current(true);
            }
            Err(error) => tracing::warn!("Failed to enqueue resume: {error}"),
        }
    }

    pub fn stop_audio_output(&mut self) {
        self.clear_scheduled_transition_state();
        self.playback.pending_playback_request = None;
        self.playback.pause_requested = false;
        self.stop_audio_backend();
    }

    pub fn stop_and_clear_current_playback(&mut self) {
        self.stop_audio_output();
        self.playback.current_song = None;
        self.playback.current_artist_id = None;
        self.playback.current_artists.clear();
        self.playback.current_quality = None;
        self.playback.preload_coordinator.clear_window();
        let released = self.playback.audio_preload_manager.reset();
        self.release_preload_requests(released);
    }

    pub fn seek_to_position(&mut self, position: std::time::Duration) {
        self.clear_scheduled_transition_state();
        self.seek_audio_output(position);
    }

    pub fn seek_by_offset(&mut self, offset: std::time::Duration, forward: bool) {
        if !self.playback_output_available() {
            return;
        }

        self.refresh_playback_runtime();
        let info = self.playback_info().clone();
        let new_pos = if forward {
            if info.duration.is_zero() {
                return;
            }
            (info.position + offset).min(info.duration)
        } else {
            info.position.saturating_sub(offset)
        };
        self.clear_scheduled_transition_state();
        self.seek_audio_output(new_pos);
    }

    pub fn toggle_current_playback(&mut self) -> Task<Message> {
        use crate::audio::PlaybackStatus;

        if !self.playback_output_available() {
            tracing::warn!("toggle_playback: No audio player");
            return Task::none();
        }

        self.refresh_playback_runtime();
        let status = self.playback_status();

        tracing::info!("toggle_playback: current status = {:?}", status);

        match status {
            PlaybackStatus::Stopped => {
                if let Some(idx) = self.playback.current_index {
                    return self.play_song_at_index(idx);
                }

                if let Some(song) = self.playback.current_song.clone() {
                    let is_ncm = song.id < 0
                        || song.file_path.is_empty()
                        || song.file_path.starts_with("ncm://");
                    if is_ncm {
                        tracing::warn!("Cannot play NCM song without queue index");
                        return Task::none();
                    }

                    let playback_pos = self
                        .playback
                        .saved_state
                        .as_ref()
                        .filter(|s| s.position_secs > 0.0)
                        .map(|s| std::time::Duration::from_secs_f64(s.position_secs));
                    let has_saved_position = playback_pos.is_some();

                    let source = if let Some(position) = playback_pos {
                        Self::audio_path_source_for_song_at_position(&song, position)
                    } else {
                        Self::audio_path_source_for_song(&song)
                    };

                    if let Ok(source) = source
                        && let Ok(request_id) = self.start_audio_source_for_song(&song, source)
                    {
                        let queue_index = self.resolve_queue_index_for_song(&song);
                        self.queue_pending_playback_request(
                            request_id,
                            queue_index,
                            song,
                            PendingPlaybackKind::RestartCurrent,
                        );
                        if has_saved_position && let Some(state) = &mut self.playback.saved_state {
                            state.position_secs = 0.0;
                        }
                    }
                }
                Task::none()
            }
            PlaybackStatus::Playing | PlaybackStatus::Buffering { .. } => {
                self.pause_current_playback();
                Task::none()
            }
            PlaybackStatus::Paused => {
                self.resume_current_playback();
                Task::none()
            }
        }
    }

    /// Refresh coordinator preload window from current playback state.
    /// Uses refresh_window_with_indices to avoid QueueNavigator borrow conflicts.
    pub(super) fn refresh_preload_window(&mut self) -> WindowChange {
        let adjacent = {
            let nav = QueueNavigator::new(
                self.playback.queue.len(),
                self.playback.current_index,
                self.effective_queue_play_mode(),
                &self.playback.shuffle_cache,
            );
            (nav.adjacent_indices(), nav.current_index())
        };
        let (adjacent, current_index) = adjacent;
        let current_song_id = self.playback.current_song.as_ref().map(|s| s.id);
        let next_song_id = adjacent
            .next
            .and_then(|idx| self.playback.queue.get(idx))
            .map(|s| s.id);
        let prev_song_id = adjacent
            .prev
            .and_then(|idx| self.playback.queue.get(idx))
            .map(|s| s.id);
        let change = self
            .playback
            .preload_coordinator
            .refresh_window_with_indices(
                current_song_id,
                current_index,
                adjacent.next,
                next_song_id,
                adjacent.prev,
                prev_song_id,
            );
        // Sync render manager: keep only entries for songs in the current window
        let window = self.playback.preload_coordinator.window();
        let keep_ids: Vec<i64> = [
            window.current_song_id,
            window.next_song_id,
            window.prev_song_id,
        ]
        .into_iter()
        .flatten()
        .collect();
        self.playback.lyrics_render_manager.retain(&keep_ids);
        change
    }

    fn on_song_started(&mut self, idx: usize, song: DbSong) -> Task<Message> {
        tracing::info!("Playing: {} - {}", song.title, song.artist);

        let cover_task = self.commit_current_song_playback_state(Some(idx), song.clone(), true);

        if let Some(db) = &self.core.db {
            let db = db.clone();
            let song_id = song.id;
            tokio::spawn(async move {
                let _ = db.record_play(song_id, 0, false).await;
            });
        }

        if let Some(db) = &self.core.db {
            let db = db.clone();
            let song_id = song.id;
            let queue_pos = idx as i64;
            tokio::spawn(async move {
                let _ = db
                    .update_playback_position(Some(song_id), queue_pos, 0.0)
                    .await;
            });
        }

        // Refresh preload coordinator window
        self.cache_shuffle_indices();
        self.refresh_preload_window();
        Task::batch([cover_task, self.schedule_post_switch_side_effects(&song)])
    }

    /// 为当前歌曲加载歌词和背景（歌词页面打开时调用）
    fn load_lyrics_for_current_song(&mut self, song: &DbSong) -> Task<Message> {
        // 使用统一的异步加载方法
        self.load_lyrics_async(song)
    }

    /// Playback-side prefetch coordinator after a track switch completes.
    fn schedule_post_switch_side_effects(&mut self, song: &DbSong) -> Task<Message> {
        self.schedule_automix_analysis_window(song);
        let audio_task = self.preload_adjacent_tracks_with_ncm();
        let lyrics_task = self.schedule_lyrics_prefetches(song);
        let bg_task = self.schedule_background_prep();

        Task::batch([audio_task, lyrics_task, bg_task])
    }

    fn automix_config(&self) -> crate::audio::automix::AnalysisConfig {
        crate::audio::automix::AnalysisConfig {
            max_seconds: self
                .core
                .settings
                .playback
                .automix_analysis_max_seconds
                .max(1),
            ..crate::audio::automix::AnalysisConfig::default()
        }
    }

    pub(super) fn schedule_automix_analysis_window(&self, current_song: &DbSong) {
        if !self.core.settings.playback.automix_enabled {
            return;
        }
        let Some(context) = self.current_audio_context() else {
            return;
        };
        let mut candidates = Vec::new();
        let current_path = self
            .playback
            .active_streaming_buffer
            .as_ref()
            .and_then(crate::audio::SharedBuffer::finalized_cache_path)
            .unwrap_or_else(|| PathBuf::from(&current_song.file_path));
        candidates.push((current_song.clone(), current_path));
        let window = self.playback.preload_coordinator.window();
        for (index, direction) in [
            (window.next_index, PreloadDirection::Next),
            (window.prev_index, PreloadDirection::Previous),
        ]
        .into_iter()
        .filter_map(|(index, direction)| index.map(|index| (index, direction)))
        {
            if let Some(song) = self.playback.queue.get(index) {
                let path = self
                    .playback
                    .audio_preload_manager
                    .slot(direction)
                    .filter(|slot| slot.idx == index)
                    .and_then(|slot| slot.buffer.as_ref())
                    .and_then(crate::audio::SharedBuffer::finalized_cache_path)
                    .unwrap_or_else(|| PathBuf::from(&song.file_path));
                candidates.push((song.clone(), path));
            }
        }
        let config = self.automix_config();
        for (song, path) in candidates {
            if !path.is_file() {
                continue;
            }
            let content_id = crate::audio::automix::content_identity(&path, &song.id.to_string());
            let cache = crate::audio::automix::AnalysisCache::new(
                crate::utils::automix_cache_dir(),
                512,
                crate::cache::cache_publisher(),
            );
            let cancellation = context.cancellation.clone();
            tokio::task::spawn_blocking(move || {
                if cancellation.is_cancelled() {
                    return;
                }
                let _ = cache.analyze_file_if_missing(&path, &content_id, config, || {
                    cancellation.is_cancelled()
                });
            });
        }
    }

    fn natural_transition_for(
        &self,
        current: &DbSong,
        next: &DbSong,
        next_index: usize,
    ) -> (Duration, crate::audio::automix::TransitionDirective) {
        let generation = self
            .current_audio_context()
            .map(|context| context.generation.0)
            .unwrap_or(0);
        let group = crate::audio::automix::ScheduleGroup(
            generation.rotate_left(32) ^ next.id.unsigned_abs(),
        );
        let baseline = crate::audio::automix::TransitionDirective::baseline_natural(group);
        if !self.core.settings.playback.automix_enabled {
            return (
                self.playback_info()
                    .duration
                    .saturating_sub(baseline.duration),
                baseline,
            );
        }
        let current_path = self
            .playback
            .active_streaming_buffer
            .as_ref()
            .and_then(crate::audio::SharedBuffer::finalized_cache_path)
            .unwrap_or_else(|| PathBuf::from(&current.file_path));
        let next_path = self
            .playback
            .audio_preload_manager
            .slot(PreloadDirection::Next)
            .filter(|slot| slot.idx == next_index)
            .and_then(|slot| slot.buffer.as_ref())
            .and_then(crate::audio::SharedBuffer::finalized_cache_path)
            .unwrap_or_else(|| PathBuf::from(&next.file_path));
        let current_id =
            crate::audio::automix::content_identity(&current_path, &current.id.to_string());
        let next_id = crate::audio::automix::content_identity(&next_path, &next.id.to_string());
        let config = self.automix_config();
        let cache = crate::audio::automix::AnalysisCache::new(
            crate::utils::automix_cache_dir(),
            512,
            crate::cache::cache_publisher(),
        );
        let analyses = cache
            .load(&current_id, config)
            .ok()
            .flatten()
            .zip(cache.load(&next_id, config).ok().flatten());
        let Some((current_analysis, next_analysis)) = analyses else {
            return (
                self.playback_info()
                    .duration
                    .saturating_sub(baseline.duration),
                baseline,
            );
        };
        let Ok(plan) = crate::audio::automix::plan_transition(
            &current_analysis,
            &next_analysis,
            Duration::from_millis(crate::audio::automix::BASELINE_CROSSFADE_MS.into()),
        ) else {
            return (
                self.playback_info()
                    .duration
                    .saturating_sub(baseline.duration),
                baseline,
            );
        };
        let automation =
            crate::audio::automix::automation_for_transition(&current_analysis, &next_analysis);
        let trigger = plan.exit.min(self.playback_info().duration);
        (
            trigger,
            crate::audio::automix::TransitionDirective::automix(group, &plan, automation),
        )
    }

    /// Schedule current-song lyrics display loading plus background cache warmup.
    fn schedule_lyrics_prefetches(&mut self, current_song: &DbSong) -> Task<Message> {
        let mut tasks = Vec::new();

        if self.ui.lyrics.is_open {
            // Render-ready source of truth: LyricsRenderManager
            let font_family = self.core.settings.lyrics.lyrics_font_family.as_deref();
            let lyrics_ready = self.current_lyrics_shape_metrics().is_some_and(|(cw, fs)| {
                self.playback.lyrics_render_manager.is_render_ready(
                    current_song.id,
                    cw,
                    fs,
                    font_family,
                )
            });

            if lyrics_ready {
                if self.install_current_lyrics_render_if_ready(current_song.id) {
                    tracing::debug!(
                        "Lyrics render-ready for current song {}, installed without prefetch",
                        current_song.id
                    );
                }
                tasks.push(self.update_lyrics_background(current_song));
            } else {
                tasks.push(self.load_lyrics_for_current_song(current_song));
            }
        } else {
            tasks.push(self.warm_lyrics_cache_for_song(current_song));
        }

        // Use coordinator window for adjacent indices
        let window = self.playback.preload_coordinator.window();
        let mut scheduled_song_ids = HashSet::from([current_song.id]);

        for idx in [window.next_index, window.prev_index].into_iter().flatten() {
            let Some(candidate) = self.playback.queue.get(idx).cloned() else {
                continue;
            };

            if !scheduled_song_ids.insert(candidate.id) {
                continue;
            }

            tasks.push(self.warm_lyrics_cache_for_song(&candidate));
        }

        // When lyrics page is open, also trigger render prep for adjacent songs
        if self.ui.lyrics.is_open {
            tasks.push(self.schedule_adjacent_lyrics_render_prep());
        }

        Task::batch(tasks)
    }

    /// Ensure song has local cover path instead of remote URL
    /// 确保预加载和实际播放使用相同的索引
    pub fn cache_shuffle_indices(&mut self) {
        let queue_len = self.playback.queue.len();
        let current_index = self.playback.current_index;

        if self.effective_queue_play_mode() == PlayMode::Shuffle {
            self.playback
                .shuffle_cache
                .sync_current(queue_len, current_index);
        } else {
            self.playback.shuffle_cache.clear();
        }
    }

    /// Clear cached shuffle indices (call when queue or play mode changes)
    pub fn clear_shuffle_cache(&mut self) {
        self.playback.shuffle_cache.clear();
        let released_request_ids = self.playback.audio_preload_manager.reset();
        self.release_preload_requests(released_request_ids);
    }

    fn resolve_and_play(&mut self, idx: usize, song: DbSong) -> Task<Message> {
        // Mark this index as the one we're waiting for
        // Any other resolution results will only update song info, not trigger playback
        self.playback.pending_resolution_index = Some(idx);

        if let Some(client) = &self.core.ncm_client {
            let client = std::sync::Arc::new(client.clone());
            let context = match self.begin_audio_resolution_context() {
                Ok(context) => context,
                Err(error) => {
                    self.playback.pending_resolution_index = None;
                    return Self::toast_warning(error.to_string());
                }
            };
            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
            let resolve_context = context.clone();
            let message_context = context.clone();

            let resolve_task = Task::perform(
                async move {
                    super::song_resolver::resolve_song(client, &song, resolve_context, event_tx)
                        .await
                        .map(|resolved| (idx, resolved))
                },
                move |result| match result {
                    Ok((idx, resolved)) => Message::SongResolvedStreaming(
                        idx,
                        resolved.finalized_cache_path,
                        resolved.cover_path,
                        resolved.shared_buffer,
                        resolved.duration_secs,
                        resolved.quality,
                        message_context.clone(),
                    ),
                    Err(error) => Message::SongResolveFailed(message_context.clone(), error),
                },
            );

            let event_task = Task::run(
                async_stream::stream! {
                    while let Some(event) = event_rx.recv().await {
                        yield Message::StreamingEvent(event);
                    }
                },
                |message| message,
            );

            Task::batch([resolve_task, event_task])
        } else {
            self.playback.pending_resolution_index = None;
            Self::toast_warning(self.core.locale.get(Key::NotLoggedIn).to_string())
        }
    }

    /// Handle song resolved with streaming support
    pub fn handle_song_resolved_streaming(
        &mut self,
        idx: usize,
        resolved: ResolvedSong,
        context: crate::audio::identity::PlaybackContext,
    ) -> Task<Message> {
        if !self.accepts_audio_context(&context) {
            tracing::debug!("Ignoring stale song resolution at index {}", idx);
            return Task::none();
        }
        tracing::info!(
            "Song at index {} resolved to {:?} (buffer: {})",
            idx,
            resolved.finalized_cache_path,
            resolved.shared_buffer.is_some()
        );
        if self.playback.current_index == Some(idx) {
            self.playback.current_quality = resolved.quality.clone();
        }
        let _ = self.apply_resolved_song_to_queue(idx, &resolved);

        // Only trigger playback if this is the song we're actually waiting for
        let should_play = self.playback.pending_resolution_index == Some(idx);
        if !should_play {
            tracing::debug!(
                "Ignoring resolved song at index {} (pending: {:?})",
                idx,
                self.playback.pending_resolution_index
            );
            return Task::none();
        }

        // Clear pending state
        self.playback.pending_resolution_index = None;

        let Some(song) = self.playback.queue.get(idx).cloned() else {
            return Task::none();
        };

        let source = match Self::playback_source_from_resolved_song(&song, &resolved) {
            Ok(source) => source,
            Err(error) => return self.handle_playback_failure(idx, &error.to_string()),
        };

        match self.start_resolved_queue_song_from_source(idx, song, source, &context) {
            Ok(task) => {
                if resolved.shared_buffer.is_some() {
                    tracing::info!("Playing from streaming buffer");
                }
                task
            }
            Err(error) => self.handle_playback_failure(idx, &error.to_string()),
        }
    }

    fn calculate_next_index(&self) -> Option<usize> {
        let nav = QueueNavigator::new(
            self.playback.queue.len(),
            self.playback.current_index,
            self.effective_queue_play_mode(),
            &self.playback.shuffle_cache,
        );
        nav.next_index()
    }

    fn calculate_prev_index(&self) -> Option<usize> {
        let nav = QueueNavigator::new(
            self.playback.queue.len(),
            self.playback.current_index,
            self.effective_queue_play_mode(),
            &self.playback.shuffle_cache,
        );
        nav.prev_index()
    }

    pub fn play_next_song(&mut self) -> Task<Message> {
        let next_idx = self.calculate_next_index();

        if next_idx.is_none() {
            if self.is_fm_mode() {
                tracing::info!("FM mode: no next song, fetching more songs");
                return self.fetch_more_fm_songs_and_play();
            }
            return self.handle_queue_finished();
        }

        let next_idx = next_idx.unwrap();
        let fetch_task = if self.is_fm_mode() && self.should_fetch_more_fm() {
            tracing::info!(
                "FM mode: fetching more songs (current_idx={}, queue_len={})",
                self.playback.current_index.unwrap_or(0),
                self.playback.queue.len()
            );
            self.fetch_more_fm_songs()
        } else {
            Task::none()
        };

        // Try to use preloaded track from PreloadManager (zero-delay playback)
        if let Some(song) = self.playback.queue.get(next_idx).cloned()
            && let Some(source) = self.take_preloaded_source(next_idx, PreloadDirection::Next)
        {
            tracing::info!("Playing preloaded next (index {}) - zero delay", next_idx);
            return match self.start_queue_song_from_source(next_idx, song, source) {
                Ok(play_task) => Task::batch([fetch_task, play_task]),
                Err(err) => Task::batch([
                    fetch_task,
                    self.handle_playback_failure(next_idx, &err.to_string()),
                ]),
            };
        }

        let play_task = self.play_song_at_index(next_idx);
        Task::batch([fetch_task, play_task])
    }

    pub(super) fn try_start_natural_crossfade(&mut self) -> Option<Task<Message>> {
        if self.playback.crossfade_triggered
            || self.playback.pending_playback_request.is_some()
            || !self.playback_is_playing()
        {
            return None;
        }
        let info = self.playback_info().clone();
        if info.duration.is_zero() {
            return None;
        }
        let next_idx = self.calculate_next_index()?;
        let current = self.playback.current_song.as_ref()?;
        let next = self.playback.queue.get(next_idx)?;
        let (trigger_at, transition) = self.natural_transition_for(current, next, next_idx);
        let planning_window =
            Duration::from_secs(crate::audio::automix::AUTOMIX_PLANNING_WINDOW_SECS);
        if info.position < trigger_at.saturating_sub(planning_window) {
            return None;
        }
        if !self
            .playback
            .audio_preload_manager
            .is_ready(next_idx, PreloadDirection::Next)
        {
            return None;
        }
        self.playback.crossfade_triggered = true;
        self.playback.pending_transition = Some(transition);
        self.playback.pending_transition_trigger = Some(trigger_at);
        Some(self.play_next_song())
    }

    pub fn play_prev_song(&mut self) -> Task<Message> {
        let Some(prev_idx) = self.calculate_prev_index() else {
            return Task::none();
        };

        // Try to use preloaded track from PreloadManager (zero-delay playback)
        if let Some(song) = self.playback.queue.get(prev_idx).cloned()
            && let Some(source) = self.take_preloaded_source(prev_idx, PreloadDirection::Previous)
        {
            tracing::info!("Playing preloaded prev (index {}) - zero delay", prev_idx);
            return match self.start_queue_song_from_source(prev_idx, song, source) {
                Ok(task) => task,
                Err(err) => self.handle_playback_failure(prev_idx, &err.to_string()),
            };
        }

        self.play_song_at_index(prev_idx)
    }

    fn skip_to_next_playable(&mut self, failed_idx: usize) -> Task<Message> {
        // Use QueueNavigator's skip_to_next_playable for consistent behavior
        let next_idx = super::queue_navigator::skip_to_next_playable(
            self.playback.queue.len(),
            failed_idx,
            self.core.settings.play_mode,
            &self.playback.shuffle_cache,
        );

        let Some(next_idx) = next_idx else {
            return Task::none();
        };

        let song = &self.playback.queue[next_idx];
        if super::song_resolver::needs_resolution(song) || PathBuf::from(&song.file_path).exists() {
            return self.play_song_at_index(next_idx);
        }

        tracing::warn!("Skipping unavailable song: {}", song.title);
        Task::none()
    }

    pub fn handle_song_finished(&mut self) -> Task<Message> {
        tracing::info!(
            "handle_song_finished called, play_mode: {:?}, fm_mode: {}",
            self.core.settings.play_mode,
            self.is_fm_mode()
        );

        if let (Some(db), Some(song)) = (&self.core.db, &self.playback.current_song) {
            let db = db.clone();
            let song_id = song.id;
            let duration_secs = song.duration_secs;
            tokio::spawn(async move {
                let _ = db.record_play(song_id, duration_secs, true).await;
            });
        }

        if let (Some(client), Some(song)) = (&self.core.ncm_client, &self.playback.current_song) {
            let ncm_song_id = if song.id < 0 {
                Some((-song.id) as u64)
            } else if let Some(rest) = song.file_path.strip_prefix("ncm://") {
                rest.parse::<u64>().ok()
            } else {
                None
            };

            if let Some(ncm_song_id) = ncm_song_id {
                let client = client.clone();
                let source_id = self.playback.ncm_scrobble_source_id;
                let time_secs = song.duration_secs.max(0) as u64;
                tokio::spawn(async move {
                    if let Err(error) = client
                        .scrobble_song(ncm_song_id, source_id, time_secs)
                        .await
                    {
                        tracing::warn!(
                            ncm_song_id,
                            error_code = %error.code().as_str(),
                            "ncm_scrobble_failed"
                        );
                    }
                });
            }
        }

        // 清除播放完成状态，防止重复触发
        self.stop_audio_output();

        self.play_next_song()
    }

    fn handle_queue_finished(&mut self) -> Task<Message> {
        tracing::info!("Queue finished");
        if self.playback.queue.is_empty() {
            return Task::none();
        }

        let first_song = self.playback.queue[0].clone();
        let cover_task =
            self.commit_current_song_playback_state(Some(0), first_song.clone(), false);

        self.stop_audio_output();

        if let Some(db) = &self.core.db {
            let db = db.clone();
            let song_id = first_song.id;
            tokio::spawn(async move {
                let _ = db.update_playback_position(Some(song_id), 0, 0.0).await;
            });
        }

        if let Some(state) = &mut self.playback.saved_state {
            state.position_secs = 0.0;
        }

        cover_task
    }

    /// Warm remote lyrics cache for an NCM song without touching display state.
    pub fn warm_lyrics_cache_for_song(&mut self, song: &DbSong) -> Task<Message> {
        // Only preload for NCM songs (negative ID)
        if song.id >= 0 {
            return Task::none();
        }

        let ncm_id = (-song.id) as u64;

        if !self
            .playback
            .lyrics_preload_manager
            .should_schedule_warmup(song.id, ncm_id)
        {
            return Task::none();
        }

        self.playback
            .preload_coordinator
            .ensure_lyrics_slot(song.id);
        Task::done(Message::WarmLyricsCache(song.id, ncm_id))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, PlaybackSource, ResolvedSong, apply_resolved_song_metadata,
        extend_ncm_artist_metadata, handoff_active_streaming_buffer,
    };
    use crate::api::{ArtistSummary, Track};
    use crate::audio::SharedBuffer;
    use crate::database::DbSong;
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn completed_stream_replays_from_the_resolved_cache_file() {
        let path = crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-replay.wav"));
        std::fs::write(&path, b"cached audio").unwrap();
        let mut song = App::ncm_track_to_db_song(&Track {
            id: 208567,
            ..Track::default()
        });
        let logical_source = song.file_path.clone();
        let mut resolved = ResolvedSong {
            finalized_cache_path: None,
            cover_path: None,
            shared_buffer: Some(SharedBuffer::new(12)),
            duration_secs: Some(180),
            quality: None,
        };
        assert!(matches!(
            App::playback_source_from_resolved_song(&song, &resolved),
            Ok(PlaybackSource::StreamingBuffer { .. })
        ));

        // The next resolution is a disk cache hit. Metadata deliberately keeps
        // ncm:// identity; the controller must choose the resolved transport.
        resolved.shared_buffer = None;
        resolved.finalized_cache_path = Some(path.to_string_lossy().into_owned());
        apply_resolved_song_metadata(&mut song, &resolved);
        let replay = App::playback_source_from_resolved_song(&song, &resolved);
        assert_eq!(song.file_path, logical_source);
        song.file_path = "ncm://208567".to_string();
        let ncm_replay = App::playback_source_from_resolved_song(&song, &resolved);
        std::fs::remove_file(&path).unwrap();
        for result in [replay, ncm_replay] {
            let source =
                result.unwrap_or_else(|error| panic!("cached replay was rejected: {error}"));
            assert!(
                matches!(source, PlaybackSource::AudioPath { path: actual, start_position: None, .. }
                if actual == path)
            );
        }
        assert_eq!(song.file_path, "ncm://208567");
    }

    #[test]
    fn missing_resolved_cache_reports_that_path_instead_of_the_song_identity() {
        let song = App::ncm_track_to_db_song(&Track {
            id: 208567,
            ..Track::default()
        });
        let path =
            crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-missing-cache.wav"));
        let resolved = ResolvedSong {
            finalized_cache_path: Some(path.to_string_lossy().into_owned()),
            cover_path: None,
            shared_buffer: None,
            duration_secs: None,
            quality: None,
        };
        assert!(
            matches!(App::playback_source_from_resolved_song(&song, &resolved),
            Err(crate::audio::PlaybackError::FileNotFound(actual)) if Path::new(&actual) == path)
        );
    }

    #[test]
    fn finalized_cache_transport_does_not_replace_ncm_source_identity() {
        let mut song = DbSong {
            id: 1476,
            file_path: "ncm://4934355".to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_secs: 180,
            track_number: None,
            year: None,
            genre: None,
            cover_path: None,
            file_hash: None,
            file_size: 0,
            format: Some("ncm".to_string()),
            normalization_gain: None,
            play_count: 0,
            last_played: None,
            last_modified: 0,
            is_missing: false,
            created_at: 0,
        };
        let resolved = ResolvedSong {
            finalized_cache_path: Some("/cache/4934355_lossless.flac".to_string()),
            cover_path: Some("https://example.invalid/cover.jpg".to_string()),
            shared_buffer: None,
            duration_secs: Some(180),
            quality: None,
        };

        apply_resolved_song_metadata(&mut song, &resolved);

        assert_eq!(song.file_path, "ncm://4934355");
        assert_eq!(
            song.cover_path.as_deref(),
            Some("https://example.invalid/cover.jpg")
        );
    }

    #[test]
    fn queue_artist_metadata_retains_every_structured_artist_by_song_id() {
        let artists = vec![
            ArtistSummary {
                id: 12,
                name: "Artist A".to_string(),
                image_url: String::new(),
            },
            ArtistSummary {
                id: 34,
                name: "Artist B".to_string(),
                image_url: String::new(),
            },
        ];
        let track = Track {
            id: 56,
            artists: artists.clone(),
            ..Track::default()
        };
        let mut cache = HashMap::new();

        extend_ncm_artist_metadata(&mut cache, &[track]);

        assert_eq!(cache.get(&56), Some(&artists));
    }

    #[test]
    fn preloaded_handoff_does_not_cancel_the_outgoing_stream() {
        let outgoing = SharedBuffer::new(16);
        let outgoing_observer = outgoing.clone();
        let incoming = SharedBuffer::new(32);
        let incoming_observer = incoming.clone();
        let mut active = Some(outgoing);

        handoff_active_streaming_buffer(&mut active, Some(incoming));

        assert!(!outgoing_observer.is_cancelled());
        assert!(
            active
                .as_ref()
                .is_some_and(|buffer| buffer.total_size() == incoming_observer.total_size())
        );
    }
}
