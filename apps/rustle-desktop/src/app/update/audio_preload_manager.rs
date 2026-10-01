//! Audio preload state machine - manages track preloading with proper state tracking
//!
//! This module provides:
//! - State tracking for preload operations (prevents duplicate requests)
//! - Request ID tracking for audio thread preloaded sinks
//! - Streaming download support for NCM songs
//! - Retry logic for failed downloads
//!
//! ## Architecture
//! AudioPreloadManager is the SINGLE SOURCE OF TRUTH for all audio preload state.
//! Sinks are created and stored in the audio thread.
//! AudioPreloadSlot contains request_id to reference the preloaded sink.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::api::NcmClient;
use crate::app::message::Message;
use crate::audio::identity::PreloadIdentity;
use crate::audio::streaming::{
    SharedBuffer, StreamingIdentity, start_buffer_download, wait_for_buffer_playable,
};
use crate::database::DbSong;

/// Maximum retry attempts for failed downloads
const MAX_RETRIES: u8 = 2;

// ============ Core Types ============

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadDirection {
    Next,
    Previous,
}

impl PreloadDirection {
    pub const ALL: [Self; 2] = [Self::Next, Self::Previous];

    pub fn label(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Previous => "previous",
        }
    }
}

impl std::fmt::Display for PreloadDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// State of an audio preload slot
#[derive(Debug, Clone, Default, PartialEq)]
pub enum SlotState {
    #[default]
    Idle,
    Pending,
    Ready,
    Failed {
        retry_count: u8,
    },
}

/// An audio preload slot containing state for a preloaded track
///
/// Contains request_id to reference sink stored in audio thread.
/// When switching tracks, we send PlayPreloaded command with the request_id.
pub struct AudioPreloadSlot {
    pub idx: usize,
    pub path: PathBuf,
    pub state: SlotState,
    pub request_id: Option<PreloadIdentity>,
    pub pending_request_id: Option<PreloadIdentity>,
    pub duration: Duration,
    pub buffer: Option<SharedBuffer>,
    pub quality: Option<super::song_resolver::ResolvedAudioQuality>,
}

impl std::fmt::Debug for AudioPreloadSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPreloadSlot")
            .field("idx", &self.idx)
            .field("path", &self.path)
            .field("state", &self.state)
            .field("request_id", &self.request_id)
            .field("pending_request_id", &self.pending_request_id)
            .field("duration", &self.duration)
            .field("has_buffer", &self.buffer.is_some())
            .field("quality", &self.quality)
            .finish()
    }
}

impl AudioPreloadSlot {
    pub fn pending(idx: usize) -> Self {
        Self {
            idx,
            path: PathBuf::new(),
            state: SlotState::Pending,
            request_id: None,
            pending_request_id: None,
            duration: Duration::ZERO,
            buffer: None,
            quality: None,
        }
    }

    pub fn is_for_index(&self, target_idx: usize) -> bool {
        self.idx == target_idx
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, SlotState::Ready) && self.request_id.is_some()
    }

    pub fn has_pending_request(&self, identity: &PreloadIdentity) -> bool {
        self.pending_request_id.as_ref() == Some(identity)
    }

    pub fn set_pending_request_id(&mut self, identity: PreloadIdentity) {
        self.pending_request_id = Some(identity);
    }

    pub fn take_request_id(&mut self) -> Option<PreloadIdentity> {
        self.request_id.take()
    }

    pub fn take_buffer(&mut self) -> Option<SharedBuffer> {
        self.buffer.take()
    }

    pub fn retry_count(&self) -> u8 {
        match &self.state {
            SlotState::Failed { retry_count } => *retry_count,
            _ => 0,
        }
    }
}

/// Manages audio preloading for next and previous tracks
#[derive(Default)]
pub struct AudioPreloadManager {
    next: Option<AudioPreloadSlot>,
    prev: Option<AudioPreloadSlot>,
}

impl std::fmt::Debug for AudioPreloadManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPreloadManager")
            .field("next", &self.next.as_ref().map(|s| (s.idx, &s.state)))
            .field("prev", &self.prev.as_ref().map(|s| (s.idx, &s.state)))
            .finish()
    }
}

impl AudioPreloadManager {
    fn slot_ref(&self, direction: PreloadDirection) -> &Option<AudioPreloadSlot> {
        match direction {
            PreloadDirection::Next => &self.next,
            PreloadDirection::Previous => &self.prev,
        }
    }

    fn slot_entry_mut(&mut self, direction: PreloadDirection) -> &mut Option<AudioPreloadSlot> {
        match direction {
            PreloadDirection::Next => &mut self.next,
            PreloadDirection::Previous => &mut self.prev,
        }
    }

    fn direction_for_index(
        &self,
        idx: usize,
        preferred: PreloadDirection,
    ) -> Option<PreloadDirection> {
        if self
            .slot(preferred)
            .is_some_and(|slot| slot.is_for_index(idx))
        {
            return Some(preferred);
        }
        let alternate = match preferred {
            PreloadDirection::Next => PreloadDirection::Previous,
            PreloadDirection::Previous => PreloadDirection::Next,
        };
        self.slot(alternate)
            .is_some_and(|slot| slot.is_for_index(idx))
            .then_some(alternate)
    }

    fn slot_for_index(&self, idx: usize, preferred: PreloadDirection) -> Option<&AudioPreloadSlot> {
        self.direction_for_index(idx, preferred)
            .and_then(|direction| self.slot(direction))
    }

    fn slot_mut_by_pending_identity(
        &mut self,
        identity: &PreloadIdentity,
    ) -> Option<&mut AudioPreloadSlot> {
        if self
            .next
            .as_ref()
            .is_some_and(|slot| slot.pending_request_id.as_ref() == Some(identity))
        {
            return self.next.as_mut();
        }
        self.prev
            .as_mut()
            .filter(|slot| slot.pending_request_id.as_ref() == Some(identity))
    }

    fn clear_slot(slot: &mut Option<AudioPreloadSlot>) -> Vec<PreloadIdentity> {
        let Some(slot) = slot.take() else {
            return Vec::new();
        };

        [slot.request_id, slot.pending_request_id]
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn reset(&mut self) -> Vec<PreloadIdentity> {
        let mut released = Self::clear_slot(&mut self.next);
        released.extend(Self::clear_slot(&mut self.prev));
        released
    }

    pub fn should_preload(&self, idx: usize, direction: PreloadDirection) -> bool {
        match self.slot_for_index(idx, direction) {
            None => true,
            Some(s) => match &s.state {
                SlotState::Failed { retry_count } => *retry_count < MAX_RETRIES,
                SlotState::Idle => true,
                SlotState::Pending => false,
                SlotState::Ready => false,
            },
        }
    }

    pub fn is_ready(&self, idx: usize, direction: PreloadDirection) -> bool {
        self.slot_for_index(idx, direction)
            .is_some_and(AudioPreloadSlot::is_ready)
    }

    pub fn mark_pending(
        &mut self,
        idx: usize,
        direction: PreloadDirection,
    ) -> Vec<PreloadIdentity> {
        let should_replace = self
            .slot_ref(direction)
            .as_ref()
            .is_some_and(|slot| !slot.is_for_index(idx));

        let released = if should_replace {
            Self::clear_slot(self.slot_entry_mut(direction))
        } else {
            Vec::new()
        };

        *self.slot_entry_mut(direction) = Some(AudioPreloadSlot::pending(idx));
        released
    }

    pub fn mark_failed_if_pending(
        &mut self,
        idx: usize,
        _direction: PreloadDirection,
        identity: &PreloadIdentity,
    ) -> bool {
        let Some(slot) = self.slot_mut_by_pending_identity(identity) else {
            return false;
        };
        if slot.idx != idx {
            return false;
        }

        let retry_count = slot.retry_count().saturating_add(1);
        slot.pending_request_id = None;
        slot.buffer.take();
        slot.state = SlotState::Failed { retry_count };
        true
    }

    pub fn mark_failed_by_identity(&mut self, identity: &PreloadIdentity) -> bool {
        for direction in PreloadDirection::ALL {
            let Some(slot) = self.slot_mut(direction) else {
                continue;
            };
            if slot.pending_request_id.as_ref() != Some(identity) {
                continue;
            }

            let retry_count = slot.retry_count().saturating_add(1);
            slot.pending_request_id = None;
            slot.buffer.take();
            slot.state = SlotState::Failed { retry_count };
            return true;
        }
        false
    }

    pub fn take_ready(
        &mut self,
        idx: usize,
        direction: PreloadDirection,
    ) -> Option<AudioPreloadSlot> {
        let direction = self.direction_for_index(idx, direction)?;
        let slot_ref = self.slot_entry_mut(direction);
        match slot_ref {
            Some(slot) if slot.is_for_index(idx) && slot.is_ready() => slot_ref.take(),
            _ => None,
        }
    }

    pub fn invalidate_stale(
        &mut self,
        next_idx: Option<usize>,
        prev_idx: Option<usize>,
    ) -> Vec<PreloadIdentity> {
        let mut released = Vec::new();
        for (direction, expected_idx) in [
            (PreloadDirection::Next, next_idx),
            (PreloadDirection::Previous, prev_idx),
        ] {
            released.extend(self.invalidate_direction(direction, expected_idx));
        }
        released
    }

    fn invalidate_direction(
        &mut self,
        direction: PreloadDirection,
        expected_idx: Option<usize>,
    ) -> Vec<PreloadIdentity> {
        let should_clear = match expected_idx {
            Some(expected_idx) => self
                .slot(direction)
                .map(|slot| !slot.is_for_index(expected_idx))
                .unwrap_or(false),
            None => self.slot(direction).is_some(),
        };

        if should_clear {
            Self::clear_slot(self.slot_entry_mut(direction))
        } else {
            Vec::new()
        }
    }

    pub fn has_pending_request(
        &self,
        idx: usize,
        direction: PreloadDirection,
        identity: &PreloadIdentity,
    ) -> bool {
        self.slot_for_index(idx, direction)
            .is_some_and(|slot| slot.idx == idx && slot.has_pending_request(identity))
    }

    pub fn replace_pending_request(
        &mut self,
        _direction: PreloadDirection,
        parent: &PreloadIdentity,
        handoff: PreloadIdentity,
    ) -> bool {
        let Some(slot) = self.slot_mut_by_pending_identity(parent) else {
            return false;
        };
        slot.pending_request_id = Some(handoff);
        true
    }

    pub fn slot_mut_for_identity(
        &mut self,
        identity: &PreloadIdentity,
    ) -> Option<&mut AudioPreloadSlot> {
        if self.next.as_ref().is_some_and(|slot| {
            slot.request_id.as_ref() == Some(identity)
                || slot.pending_request_id.as_ref() == Some(identity)
        }) {
            return self.next.as_mut();
        }
        self.prev.as_mut().filter(|slot| {
            slot.request_id.as_ref() == Some(identity)
                || slot.pending_request_id.as_ref() == Some(identity)
        })
    }

    pub fn accepts_identity(&self, identity: &PreloadIdentity) -> bool {
        PreloadDirection::ALL.iter().any(|&direction| {
            self.slot(direction).is_some_and(|slot| {
                slot.request_id.as_ref() == Some(identity)
                    || slot.pending_request_id.as_ref() == Some(identity)
            })
        })
    }

    pub fn slot(&self, direction: PreloadDirection) -> Option<&AudioPreloadSlot> {
        self.slot_ref(direction).as_ref()
    }

    pub fn slot_mut(&mut self, direction: PreloadDirection) -> Option<&mut AudioPreloadSlot> {
        self.slot_entry_mut(direction).as_mut()
    }

    #[cfg(test)]
    fn set_slot_for_test(&mut self, direction: PreloadDirection, slot: AudioPreloadSlot) {
        *self.slot_entry_mut(direction) = Some(slot);
    }
}

// ============ Preload Task Creation ============

/// Create a preload task for an NCM song with streaming support
pub fn create_preload_task(
    client: Arc<NcmClient>,
    idx: usize,
    song: DbSong,
    direction: PreloadDirection,
    identity: PreloadIdentity,
) -> Task<Message> {
    Task::perform(
        async move { download_audio_streaming(client, idx, song, direction, identity).await },
        |result| result,
    )
}

async fn download_audio_streaming(
    client: Arc<NcmClient>,
    idx: usize,
    song: DbSong,
    direction: PreloadDirection,
    identity: PreloadIdentity,
) -> Message {
    let cancellation = identity.cancellation.clone();
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Message::PreloadAudioFailed(idx, direction, identity.clone()),
        result = download_audio_streaming_inner(client, idx, song, direction, identity.clone()) => result,
    }
}

async fn download_audio_streaming_inner(
    client: Arc<NcmClient>,
    idx: usize,
    song: DbSong,
    direction: PreloadDirection,
    identity: PreloadIdentity,
) -> Message {
    let ncm_id = super::song_resolver::get_ncm_id(&song);

    tracing::info!(
        "Preload: downloading audio for song {} (streaming buffer)",
        ncm_id
    );

    let source = match super::song_resolver::resolve_audio_source(&client, &song).await {
        Ok(source) => source,
        Err(error) => {
            tracing::error!("Preload: failed to resolve song {}: {}", ncm_id, error);
            return Message::PreloadAudioFailed(idx, direction, identity);
        }
    };
    let (shared_buffer, quality) = match source {
        super::song_resolver::ResolvedAudioSource::Cached { path, quality } => {
            return Message::PreloadReady(
                idx,
                path.to_string_lossy().to_string(),
                direction,
                Some(quality),
                identity,
            );
        }
        super::song_resolver::ResolvedAudioSource::Streaming {
            url,
            cache_path,
            cache_key,
            quality,
        } => (
            start_buffer_download(
                url,
                cache_path,
                cache_key,
                crate::cache::tagged_audio_cache_store(client.clone(), &song),
                quality.bitrate,
                StreamingIdentity::Preload(identity.clone()),
                None,
            ),
            quality,
        ),
    };

    // Buffer health, not a short-lived event receiver, remains authoritative
    // after the first startup watermark. The audio thread rechecks the same
    // buffer before promotion and rejects Ready-then-failed preloads.
    if wait_for_buffer_playable(&shared_buffer, 30).await {
        if let Some(path) = shared_buffer.finalized_cache_path() {
            return Message::PreloadReady(
                idx,
                path.to_string_lossy().into_owned(),
                direction,
                Some(quality),
                identity,
            );
        }
        tracing::info!(
            "Preload: returning SharedBuffer for song {} (downloaded: {} bytes)",
            ncm_id,
            shared_buffer.downloaded()
        );
        Message::PreloadBufferReady(
            idx,
            None,
            direction,
            shared_buffer,
            song.duration_secs as u64,
            Some(quality),
            identity,
        )
    } else {
        tracing::error!("Preload: download failed for song {}", ncm_id);
        Message::PreloadAudioFailed(idx, direction, identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::identity::PlaybackGenerationController;

    fn identities() -> (
        PreloadIdentity,
        PreloadIdentity,
        PreloadIdentity,
        PreloadIdentity,
    ) {
        let controller = PlaybackGenerationController::new();
        controller.activate_generation();
        (
            controller.reserve_preload_identity().unwrap(),
            controller.reserve_preload_identity().unwrap(),
            controller.reserve_preload_identity().unwrap(),
            controller.reserve_preload_identity().unwrap(),
        )
    }

    #[test]
    fn reset_releases_ready_and_pending_identities_from_both_directions() {
        let (next_ready, next_pending, prev_ready, prev_pending) = identities();
        let mut manager = AudioPreloadManager::default();

        let mut next = AudioPreloadSlot::pending(1);
        next.state = SlotState::Ready;
        next.request_id = Some(next_ready.clone());
        next.pending_request_id = Some(next_pending.clone());
        let mut prev = AudioPreloadSlot::pending(2);
        prev.state = SlotState::Ready;
        prev.request_id = Some(prev_ready.clone());
        prev.pending_request_id = Some(prev_pending.clone());
        manager.set_slot_for_test(PreloadDirection::Next, next);
        manager.set_slot_for_test(PreloadDirection::Previous, prev);

        let released = manager.reset();

        assert_eq!(released.len(), 4);
        assert!(released.contains(&next_ready));
        assert!(released.contains(&next_pending));
        assert!(released.contains(&prev_ready));
        assert!(released.contains(&prev_pending));
        assert!(manager.slot(PreloadDirection::Next).is_none());
        assert!(manager.slot(PreloadDirection::Previous).is_none());
    }

    #[test]
    fn clearing_a_preload_slot_does_not_cancel_a_shared_download() {
        let mut manager = AudioPreloadManager::default();
        let buffer = SharedBuffer::new(100);
        let mut slot = AudioPreloadSlot::pending(1);
        slot.buffer = Some(buffer.clone());
        manager.set_slot_for_test(PreloadDirection::Next, slot);

        manager.reset();

        assert!(!buffer.is_cancelled());
    }

    #[test]
    fn stale_failure_does_not_modify_newer_pending_identity() {
        let (stale, current, _, _) = identities();
        let mut manager = AudioPreloadManager::default();
        let mut slot = AudioPreloadSlot::pending(7);
        slot.pending_request_id = Some(current.clone());
        manager.set_slot_for_test(PreloadDirection::Next, slot);

        assert!(!manager.mark_failed_by_identity(&stale));
        assert!(manager.has_pending_request(7, PreloadDirection::Next, &current));
        assert_eq!(
            manager.slot(PreloadDirection::Next).unwrap().state,
            SlotState::Pending
        );
    }

    #[test]
    fn exact_failure_transitions_pending_slot_and_clears_identity() {
        let (identity, _, _, _) = identities();
        let mut manager = AudioPreloadManager::default();
        let mut slot = AudioPreloadSlot::pending(9);
        slot.pending_request_id = Some(identity.clone());
        manager.set_slot_for_test(PreloadDirection::Previous, slot);

        assert!(manager.mark_failed_if_pending(9, PreloadDirection::Previous, &identity));
        let slot = manager.slot(PreloadDirection::Previous).unwrap();
        assert_eq!(slot.state, SlotState::Failed { retry_count: 1 });
        assert!(slot.pending_request_id.is_none());
    }

    #[test]
    fn replacement_requires_exact_parent_identity() {
        let (parent, other, handoff, _) = identities();
        let mut manager = AudioPreloadManager::default();
        let mut slot = AudioPreloadSlot::pending(3);
        slot.pending_request_id = Some(parent.clone());
        manager.set_slot_for_test(PreloadDirection::Next, slot);

        assert!(!manager.replace_pending_request(PreloadDirection::Next, &other, handoff.clone()));
        assert!(manager.has_pending_request(3, PreloadDirection::Next, &parent));
        assert!(manager.replace_pending_request(PreloadDirection::Next, &parent, handoff.clone()));
        assert!(manager.has_pending_request(3, PreloadDirection::Next, &handoff));
    }

    #[test]
    fn same_target_is_one_preload_entity_for_both_directions() {
        let (identity, _, _, _) = identities();
        let mut manager = AudioPreloadManager::default();
        manager.mark_pending(1, PreloadDirection::Next);
        manager
            .slot_mut(PreloadDirection::Next)
            .unwrap()
            .pending_request_id = Some(identity.clone());

        assert!(!manager.should_preload(1, PreloadDirection::Previous));
        assert!(manager.has_pending_request(1, PreloadDirection::Previous, &identity));

        let slot = manager.slot_mut(PreloadDirection::Next).unwrap();
        slot.pending_request_id = None;
        slot.request_id = Some(identity);
        slot.state = SlotState::Ready;
        assert!(manager.is_ready(1, PreloadDirection::Previous));
        assert!(manager.take_ready(1, PreloadDirection::Previous).is_some());
        assert!(manager.slot(PreloadDirection::Next).is_none());
    }
}
