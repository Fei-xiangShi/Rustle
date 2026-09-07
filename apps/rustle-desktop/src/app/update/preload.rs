//! Audio preloading for seamless track switching
//!
//! This module uses the new architecture:
//! - QueueNavigator for consistent index calculations (Single Source of Truth)
//! - AudioPreloadManager for state tracking (prevents duplicate requests)
//! - AudioPreloadSlot contains request_id to reference sink in audio thread
//! - Sinks are created and stored in the audio thread via AudioHandle commands

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::app::message::Message;
use crate::app::state::App;
use crate::audio::identity::PreloadIdentity;

use super::audio_preload_manager::{self, PreloadDirection, SlotState};
use super::player_controller::PlaybackSource;
use super::queue_navigator;

struct PreloadBufferReady {
    idx: usize,
    finalized_cache_path: Option<String>,
    direction: PreloadDirection,
    buffer: crate::audio::SharedBuffer,
    duration_secs: u64,
    quality: Option<super::song_resolver::ResolvedAudioQuality>,
    request_identity: PreloadIdentity,
}

impl App {
    fn release_preload_request_ids<I>(&self, request_ids: I)
    where
        I: IntoIterator<Item = PreloadIdentity>,
    {
        self.release_preload_requests(request_ids);
    }

    /// 触发相邻曲目预加载
    /// 歌曲开始播放后调用。使用 coordinator 窗口作为唯一数据源。
    pub fn preload_adjacent_tracks_with_ncm(&mut self) -> Task<Message> {
        let (next_idx, prev_idx) = self.playback.preload_coordinator.adjacent_indices();

        tracing::info!(
            "Preload scan: current_index={:?}, queue_len={}, play_mode={:?}, effective_mode={:?}, next={:?}, prev={:?}",
            self.playback.current_index,
            self.playback.queue.len(),
            self.core.settings.play_mode,
            self.effective_queue_play_mode(),
            next_idx,
            prev_idx
        );

        // Skip preloading if next/prev are the same as current (LoopOne mode)
        let current_idx = self.playback.current_index;
        if next_idx == current_idx && prev_idx == current_idx {
            tracing::debug!("Preload scan skipped: next and prev are same as current");
            return Task::none();
        }
        let stale_request_ids = self
            .playback
            .audio_preload_manager
            .invalidate_stale(next_idx, prev_idx);
        self.release_preload_request_ids(stale_request_ids);

        let mut tasks = Vec::new();
        let current_song_id = self.playback.current_song.as_ref().map(|song| song.id);
        let mut planned_song_ids = HashSet::new();

        for (candidate_idx, direction) in [
            (next_idx, PreloadDirection::Next),
            (prev_idx, PreloadDirection::Previous),
        ] {
            let Some(idx) = candidate_idx else {
                continue;
            };
            let Some(song_id) = self.playback.queue.get(idx).map(|song| song.id) else {
                continue;
            };
            if Some(song_id) == current_song_id || !planned_song_ids.insert(song_id) {
                tracing::debug!(
                    index = idx,
                    song_id,
                    %direction,
                    "Preload skipped to keep one decoder per shared cache key"
                );
                continue;
            }
            if let Some(task) = self.preload_track(idx, direction) {
                tasks.push(task);
            }
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    /// Preload a specific track
    /// Returns None if already preloaded or preloading
    fn preload_track(&mut self, idx: usize, direction: PreloadDirection) -> Option<Task<Message>> {
        let song = self.playback.queue.get(idx)?.clone();

        // Check if we should preload
        if !self
            .playback
            .audio_preload_manager
            .should_preload(idx, direction)
        {
            tracing::debug!(
                "Preload skipped ({}): idx={}, reason=slot_not_eligible",
                direction,
                idx
            );
            return None;
        }

        // Check if it's a local song with existing file
        if let Some(local_path) = queue_navigator::get_local_path(&song) {
            let track_gain = self.resolve_track_gain_for_song(
                &song,
                super::player_controller::TrackGainMode::AnalyzeIfMissing,
            );
            let Ok(identity) = self.reserve_preload_identity() else {
                return None;
            };
            let released_identities = self
                .playback
                .audio_preload_manager
                .mark_pending(idx, direction);
            if let Some(slot) = self.playback.audio_preload_manager.slot_mut(direction) {
                slot.set_pending_request_id(identity.clone());
            }
            if self
                .create_preload_sink_for_file(identity.clone(), local_path.clone(), track_gain)
                .is_ok()
            {
                tracing::info!(
                    "Preload request ({}): idx={}, source=local, path={:?}, identity={:?}",
                    direction,
                    idx,
                    local_path,
                    identity
                );

                self.release_preload_request_ids(released_identities);
                return Some(Task::none());
            }
            tracing::warn!(
                "Preload request failed ({}): idx={}, source=local, path={:?}",
                direction,
                idx,
                local_path
            );
            self.release_preload_request(identity.clone());
            self.playback
                .audio_preload_manager
                .mark_failed_if_pending(idx, direction, &identity);
            self.release_preload_request_ids(released_identities);
            return None;
        }

        // NCM song - needs download
        if !queue_navigator::needs_ncm_download(&song) {
            tracing::debug!(
                "Preload skipped ({}): idx={}, reason=no_download_needed, file_path={}",
                direction,
                idx,
                song.file_path
            );
            return None;
        }

        // Mark as pending and create download task
        let Ok(identity) = self.reserve_preload_identity() else {
            return None;
        };
        let released_identities = self
            .playback
            .audio_preload_manager
            .mark_pending(idx, direction);
        self.release_preload_request_ids(released_identities);
        if let Some(slot) = self.playback.audio_preload_manager.slot_mut(direction) {
            slot.set_pending_request_id(identity.clone());
        }

        // Create async download task
        if let Some(client) = &self.core.ncm_client {
            let client = Arc::new(client.clone());
            tracing::info!(
                "Preload request ({}): idx={}, source=streaming-download, song_id={}, title={}",
                direction,
                idx,
                song.id,
                song.title
            );
            Some(audio_preload_manager::create_preload_task(
                client, idx, song, direction, identity,
            ))
        } else {
            tracing::warn!(
                "Preload skipped ({}): idx={}, reason=no_ncm_client",
                direction,
                idx
            );
            self.release_preload_request(identity.clone());
            self.playback
                .audio_preload_manager
                .mark_failed_if_pending(idx, direction, &identity);
            None
        }
    }

    /// Handle preload-related messages
    pub fn handle_preload(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            // Preload ready message
            Message::PreloadReady(idx, file_path, direction, quality, identity) => self
                .handle_preload_complete(
                    *idx,
                    file_path.clone(),
                    *direction,
                    quality.clone(),
                    identity.clone(),
                ),

            // Preload ready with SharedBuffer for streaming playback
            Message::PreloadBufferReady(
                idx,
                finalized_cache_path,
                direction,
                buffer,
                duration_secs,
                quality,
                identity,
            ) => self.handle_preload_buffer_ready(PreloadBufferReady {
                idx: *idx,
                finalized_cache_path: finalized_cache_path.clone(),
                direction: *direction,
                buffer: buffer.clone(),
                duration_secs: *duration_secs,
                quality: quality.clone(),
                request_identity: identity.clone(),
            }),

            Message::PreloadAudioFailed(idx, direction, identity) => {
                tracing::warn!("Preload failed ({}): idx={}", direction, idx);
                if !self
                    .playback
                    .audio_preload_manager
                    .has_pending_request(*idx, *direction, identity)
                {
                    self.release_preload_request(identity.clone());
                    return Some(Task::none());
                }
                if self
                    .playback
                    .audio_preload_manager
                    .mark_failed_if_pending(*idx, *direction, identity)
                {
                    tracing::warn!("Preload failed ({}): idx={}", direction, idx);
                }
                Some(Task::none())
            }

            _ => None,
        }
    }

    /// Handle AudioEvent::PreloadReady from audio thread
    pub fn handle_audio_preload_ready(
        &mut self,
        identity: PreloadIdentity,
        duration: Duration,
        path: PathBuf,
    ) {
        for direction in PreloadDirection::ALL {
            let matches = self
                .playback
                .audio_preload_manager
                .slot(direction)
                .map(|slot| slot.has_pending_request(&identity))
                .unwrap_or(false);

            if !matches {
                continue;
            }

            if let Some(slot) = self.playback.audio_preload_manager.slot_mut(direction) {
                slot.request_id = Some(identity.clone());
                slot.pending_request_id = None;
                slot.path = path.clone();
                slot.duration = duration;
                slot.state = SlotState::Ready;
                tracing::info!(
                    "Preload ready ({}): identity={:?}, path={:?}",
                    direction,
                    identity,
                    path
                );
            }
            if let Some(current) = self.playback.current_song.clone() {
                self.schedule_automix_analysis_window(&current);
            }
            return;
        }

        tracing::debug!(
            "PreloadReady received but no matching pending slot: identity={:?} (stale)",
            identity
        );
        self.release_preload_request(identity);
    }

    /// Handle successful preload completion
    fn handle_preload_complete(
        &mut self,
        idx: usize,
        file_path: String,
        direction: PreloadDirection,
        quality: Option<super::song_resolver::ResolvedAudioQuality>,
        request_identity: PreloadIdentity,
    ) -> Option<Task<Message>> {
        if !self.playback.audio_preload_manager.has_pending_request(
            idx,
            direction,
            &request_identity,
        ) || !self.accepts_audio_preload_identity(&request_identity)
        {
            self.release_preload_request(request_identity);
            return Some(Task::none());
        }

        let track_gain = self
            .playback
            .queue
            .get(idx)
            .cloned()
            .map(|song| {
                self.resolve_track_gain_for_song(
                    &song,
                    super::player_controller::TrackGainMode::AnalyzeIfMissing,
                )
            })
            .unwrap_or(1.0);
        let Ok(identity) = self.reserve_preload_handoff(&request_identity) else {
            self.release_preload_request(request_identity);
            return Some(Task::none());
        };
        if !self.playback.audio_preload_manager.replace_pending_request(
            direction,
            &request_identity,
            identity.clone(),
        ) {
            self.release_preload_request(identity);
            return Some(Task::none());
        }
        if let Some(slot) = self
            .playback
            .audio_preload_manager
            .slot_mut_for_identity(&identity)
        {
            slot.quality = quality;
        }
        if self
            .create_preload_sink_for_file(identity.clone(), PathBuf::from(&file_path), track_gain)
            .is_ok()
        {
            // The cached file is owned by the preload transport. Do not replace
            // the queue song's logical `ncm://<id>` source with it.
            tracing::info!(
                "Preload file ready ({}): idx={}, path={}, identity={:?}",
                direction,
                idx,
                file_path,
                identity
            );
            return Some(Task::none());
        }

        tracing::warn!(
            "Preload file handoff failed ({}): idx={}, path={}",
            direction,
            idx,
            file_path
        );
        self.release_preload_request(identity.clone());
        self.playback
            .audio_preload_manager
            .mark_failed_if_pending(idx, direction, &identity);
        Some(Task::none())
    }

    /// Handle preload ready with SharedBuffer (streaming playback)
    fn handle_preload_buffer_ready(&mut self, ready: PreloadBufferReady) -> Option<Task<Message>> {
        let PreloadBufferReady {
            idx,
            finalized_cache_path,
            direction,
            buffer,
            duration_secs,
            quality,
            request_identity,
        } = ready;
        if !self.playback.audio_preload_manager.has_pending_request(
            idx,
            direction,
            &request_identity,
        ) || !self.accepts_audio_preload_identity(&request_identity)
        {
            self.release_preload_request(request_identity);
            return Some(Task::none());
        }

        let track_gain = self
            .playback
            .queue
            .get(idx)
            .cloned()
            .map(|song| {
                self.resolve_track_gain_for_song(
                    &song,
                    super::player_controller::TrackGainMode::MetadataOnly,
                )
            })
            .unwrap_or(1.0);
        let Ok(identity) = self.reserve_preload_handoff(&request_identity) else {
            self.release_preload_request(request_identity);
            return Some(Task::none());
        };
        if !self.playback.audio_preload_manager.replace_pending_request(
            direction,
            &request_identity,
            identity.clone(),
        ) {
            self.release_preload_request(identity);
            return Some(Task::none());
        }
        if let Some(slot) = self
            .playback
            .audio_preload_manager
            .slot_mut_for_identity(&identity)
        {
            slot.quality = quality;
        }
        if self
            .create_preload_sink_for_stream(
                identity.clone(),
                buffer.clone(),
                Duration::from_secs(duration_secs),
                track_gain,
            )
            .is_ok()
        {
            // A finalized cache path remains transport metadata; the queue
            // keeps the stable NCM source identity used by every other layer.
            if let Some(slot) = self
                .playback
                .audio_preload_manager
                .slot_mut_for_identity(&identity)
            {
                slot.buffer = Some(buffer);
            }
            tracing::info!(
                "NCM streaming track buffer ready at index {}, finalized_cache_path={:?}, requesting {} preload: identity={:?}",
                idx,
                finalized_cache_path,
                direction,
                identity
            );
            return Some(Task::none());
        }

        tracing::warn!(
            "Preload buffer handoff failed ({}): idx={}, finalized_cache_path={:?}",
            direction,
            idx,
            finalized_cache_path
        );
        self.release_preload_request(identity.clone());
        self.playback
            .audio_preload_manager
            .mark_failed_if_pending(idx, direction, &identity);
        Some(Task::none())
    }

    pub(super) fn take_preloaded_source(
        &mut self,
        idx: usize,
        direction: PreloadDirection,
    ) -> Option<PlaybackSource> {
        let target_song_id = self.playback.queue.get(idx).map(|song| song.id)?;
        let (preloaded_idx, preloaded_direction) =
            if self.playback.audio_preload_manager.is_ready(idx, direction) {
                (idx, direction)
            } else {
                PreloadDirection::ALL
                    .into_iter()
                    .find_map(|candidate_direction| {
                        let slot = self
                            .playback
                            .audio_preload_manager
                            .slot(candidate_direction)?;
                        let same_audio = slot.is_ready()
                            && self
                                .playback
                                .queue
                                .get(slot.idx)
                                .is_some_and(|song| song.id == target_song_id);
                        same_audio.then_some((slot.idx, candidate_direction))
                    })?
            };
        if let Some(mut slot) = self
            .playback
            .audio_preload_manager
            .take_ready(preloaded_idx, preloaded_direction)
            && let Some(identity) = slot.take_request_id()
        {
            let path = slot.path.clone();
            let buffer = slot.take_buffer();
            self.playback.current_quality = slot.quality.take();
            tracing::info!(
                "Using {} preloaded track: idx={}, identity={:?}, path={:?}, streaming_buffer={}",
                preloaded_direction,
                idx,
                identity,
                path,
                buffer.is_some()
            );
            return Some(PlaybackSource::Preloaded { identity, buffer });
        }
        tracing::debug!("No {} preloaded track available for idx={}", direction, idx);
        None
    }
}
