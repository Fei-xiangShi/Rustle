//! Playback intent and request ownership, independent of the audio runtime.
//!
//! The audio generation controller remains the authority for audio cancellation.
//! This machine owns selection, immutable target provenance, and acknowledgement.

use crate::audio::identity::PlaybackContext;
use crate::database::DbSong;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaybackOrigin {
    #[default]
    Normal,
    Ncm {
        source_id: u64,
    },
    PersonalFm {
        session: u64,
    },
}

impl PlaybackOrigin {
    pub fn is_fm(self) -> bool {
        matches!(self, Self::PersonalFm { .. })
    }

    pub fn scrobble_source(self) -> Option<u64> {
        match self {
            Self::Ncm { source_id } => Some(source_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueEntryId(u64);

/// The only structural mutation surface for the queue. Metadata updates may
/// mutate songs, but cannot reorder entries without moving their identities.
#[derive(Debug, Default)]
pub struct PlaybackQueue {
    songs: Vec<DbSong>,
    entries: Vec<(QueueEntryId, PlaybackOrigin)>,
    next_id: u64,
    revision: u64,
}

impl std::ops::Deref for PlaybackQueue {
    type Target = [DbSong];
    fn deref(&self) -> &Self::Target {
        &self.songs
    }
}

impl PlaybackQueue {
    pub fn replace(&mut self, songs: Vec<DbSong>, origin: PlaybackOrigin) {
        self.clear();
        self.extend_with_origin(songs, origin);
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear(&mut self) {
        self.revision += 1;
        self.songs.clear();
        self.entries.clear();
    }
    pub fn push(&mut self, song: DbSong) {
        self.insert(self.len(), song);
    }
    pub fn insert(&mut self, idx: usize, song: DbSong) {
        self.revision += 1;
        self.next_id += 1;
        self.songs.insert(idx, song);
        self.entries
            .insert(idx, (QueueEntryId(self.next_id), PlaybackOrigin::Normal));
    }
    pub fn extend_with_origin(&mut self, songs: Vec<DbSong>, origin: PlaybackOrigin) {
        for song in songs {
            self.push(song);
            self.entries.last_mut().expect("just inserted").1 = origin;
        }
    }
    pub fn remove(&mut self, idx: usize) -> DbSong {
        self.revision += 1;
        self.entries.remove(idx);
        self.songs.remove(idx)
    }
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut DbSong> {
        self.revision += 1;
        self.songs.get_mut(idx)
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, DbSong> {
        self.revision += 1;
        self.songs.iter_mut()
    }
    pub fn entry(&self, idx: usize) -> Option<(QueueEntryId, PlaybackOrigin)> {
        self.entries.get(idx).copied()
    }
    pub fn index_of(&self, id: QueueEntryId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.0 == id)
    }
    pub fn set_origin(&mut self, idx: usize, origin: PlaybackOrigin) {
        self.revision += 1;
        self.entries[idx].1 = origin;
    }
    pub fn has_origin(&self, origin: PlaybackOrigin) -> bool {
        self.entries.iter().any(|entry| entry.1 == origin)
    }

    /// Drop every Personal FM entry, keeping the order and identities of the
    /// rest. Returns how many entries were removed.
    pub fn remove_personal_fm_entries(&mut self) -> usize {
        let before = self.entries.len();
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].1.is_fm() {
                self.entries.remove(index);
                self.songs.remove(index);
            } else {
                index += 1;
            }
        }
        let removed = before - self.entries.len();
        if removed > 0 {
            self.revision += 1;
        }
        removed
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransportIntent {
    #[default]
    Pause,
    Play,
    Stop,
}

#[derive(Debug, Clone)]
pub struct PlaybackTarget {
    pub entry: Option<QueueEntryId>,
    pub song: DbSong,
    pub origin: PlaybackOrigin,
    pub quality: Option<super::update::song_resolver::ResolvedAudioQuality>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPlaybackKind {
    StartPlaying,
    LoadPaused,
    RestartCurrent,
}

pub struct Handoff {
    pub buffer: Option<crate::audio::SharedBuffer>,
    pub scheduled: bool,
}

pub struct PendingPlaybackRequest {
    pub request_id: u64,
    pub target: PlaybackTarget,
    pub kind: PendingPlaybackKind,
    pub handoff: Option<Handoff>,
}

pub enum PlaybackTransaction {
    Selecting {
        id: u64,
        origin: PlaybackOrigin,
    },
    Preparing {
        target: PlaybackTarget,
        context: PlaybackContext,
    },
    AwaitingActivation(PendingPlaybackRequest),
}

pub struct FmRefill {
    pub id: u64,
    pub origin: PlaybackOrigin,
    pub anchor: Option<QueueEntryId>,
    pub play: bool,
}

#[derive(Default)]
pub struct PlaybackMachine {
    pub pending: Option<PlaybackTransaction>,
    pub current: Option<PlaybackTarget>,
    pub intent: TransportIntent,
    next_selection: u64,
    pub refill: Option<FmRefill>,
    /// Natural end of the confirmed source while a replacement is selecting.
    /// Retain its audio identity because Finished is emitted only once.
    deferred_finish: Option<PlaybackContext>,
}

impl PlaybackMachine {
    pub fn observe_restored_origin(&mut self, origin: PlaybackOrigin) {
        if let PlaybackOrigin::PersonalFm { session } = origin {
            self.next_selection = self.next_selection.max(session);
        }
    }

    pub fn select(&mut self, fm: bool) -> (u64, PlaybackOrigin) {
        self.refill = None;
        self.next_selection += 1;
        let id = self.next_selection;
        let origin = if fm {
            PlaybackOrigin::PersonalFm { session: id }
        } else {
            PlaybackOrigin::Normal
        };
        self.intent = TransportIntent::Play;
        self.pending = Some(PlaybackTransaction::Selecting { id, origin });
        (id, origin)
    }

    pub fn selection(&self, id: u64) -> Option<PlaybackOrigin> {
        match self.pending.as_ref()? {
            PlaybackTransaction::Selecting {
                id: pending,
                origin,
            } if *pending == id => Some(*origin),
            _ => None,
        }
    }

    pub fn defer_finish_during_selection(&mut self, context: PlaybackContext) {
        if self.current.is_some()
            && matches!(self.pending, Some(PlaybackTransaction::Selecting { .. }))
        {
            self.deferred_finish = Some(context);
        }
    }

    /// Reject only the matching selection. The caller must still validate the
    /// returned audio context before advancing the confirmed queue.
    pub fn reject_selection(&mut self, id: u64) -> Option<PlaybackContext> {
        self.selection(id)?;
        self.pending = None;
        self.deferred_finish
            .take()
            .filter(|_| self.intent == TransportIntent::Play)
    }

    pub fn discard_deferred_finish(&mut self) {
        self.deferred_finish = None;
    }

    pub fn target(&self) -> Option<&PlaybackTarget> {
        match self.pending.as_ref()? {
            PlaybackTransaction::Preparing { target, .. } => Some(target),
            PlaybackTransaction::AwaitingActivation(request) => Some(&request.target),
            PlaybackTransaction::Selecting { .. } => None,
        }
    }

    pub fn preparing(&self, context: &PlaybackContext) -> Option<&PlaybackTarget> {
        match self.pending.as_ref()? {
            PlaybackTransaction::Preparing {
                target,
                context: owner,
            } if owner == context => Some(target),
            _ => None,
        }
    }

    pub fn take_activation(&mut self, request_id: u64) -> Option<PendingPlaybackRequest> {
        if !matches!(&self.pending, Some(PlaybackTransaction::AwaitingActivation(request)) if request.request_id == request_id)
        {
            return None;
        }
        match self.pending.take()? {
            PlaybackTransaction::AwaitingActivation(request) => Some(request),
            _ => unreachable!(),
        }
    }

    pub fn activate(&mut self, request_id: u64) -> Option<PendingPlaybackRequest> {
        let request = self.take_activation(request_id)?;
        self.discard_deferred_finish();
        self.current = Some(request.target.clone());
        Some(request)
    }

    pub fn scheduled(&self) -> bool {
        matches!(&self.pending, Some(PlaybackTransaction::AwaitingActivation(request)) if request.handoff.as_ref().is_some_and(|handoff| handoff.scheduled))
    }

    pub fn origin(&self) -> PlaybackOrigin {
        self.current
            .as_ref()
            .map_or(PlaybackOrigin::Normal, |current| current.origin)
    }

    pub fn begin_fm_refill(&mut self, play: bool) -> Option<(u64, PlaybackOrigin)> {
        let current = self.current.as_ref()?;
        if !current.origin.is_fm() {
            return None;
        }
        if let Some(refill) = &mut self.refill
            && refill.origin == current.origin
        {
            refill.play |= play;
            return None;
        }
        self.next_selection += 1;
        let id = self.next_selection;
        self.refill = Some(FmRefill {
            id,
            origin: current.origin,
            anchor: current.entry,
            play,
        });
        Some((id, current.origin))
    }

    pub fn finish_fm_refill(&mut self, id: u64, queue: &PlaybackQueue) -> Option<FmRefill> {
        if self.refill.as_ref()?.id != id {
            return None;
        }
        let mut refill = self.refill.take()?;
        if self.origin() != refill.origin || !queue.has_origin(refill.origin) {
            return None;
        }
        if self.pending.as_ref().is_some_and(|pending| match pending {
            PlaybackTransaction::Selecting { .. } => true,
            _ => self
                .target()
                .is_none_or(|target| target.origin != refill.origin),
        }) {
            return None;
        }
        refill.play &= self.pending.is_none()
            && self.current.as_ref().and_then(|current| current.entry) == refill.anchor;
        Some(refill)
    }
}

/// Serialize/coalesce snapshots so an older async write cannot overwrite a newer commit.
pub struct PlaybackPersistence {
    pub revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub writer: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub saved_queue_revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Default for PlaybackPersistence {
    fn default() -> Self {
        Self {
            revision: Default::default(),
            writer: Default::default(),
            saved_queue_revision: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Track;
    use crate::app::state::App;

    fn song(id: u64) -> DbSong {
        App::ncm_track_to_db_song(&Track {
            id,
            ..Track::default()
        })
    }

    fn target(queue: &PlaybackQueue, idx: usize) -> PlaybackTarget {
        let (entry, origin) = queue.entry(idx).unwrap();
        PlaybackTarget {
            entry: Some(entry),
            origin,
            song: queue[idx].clone(),
            quality: None,
        }
    }

    fn awaiting(machine: &mut PlaybackMachine, id: u64, target: PlaybackTarget, scheduled: bool) {
        machine.pending = Some(PlaybackTransaction::AwaitingActivation(
            PendingPlaybackRequest {
                request_id: id,
                target,
                kind: PendingPlaybackKind::StartPlaying,
                handoff: scheduled.then_some(Handoff {
                    buffer: None,
                    scheduled: true,
                }),
            },
        ));
    }

    #[test]
    fn failed_selection_recovers_the_deferred_end_once_without_changing_the_current_track() {
        let generations = crate::audio::identity::PlaybackGenerationController::new();
        let context = generations.activate_generation();
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(vec![song(1), song(2)], PlaybackOrigin::Normal);
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();

        let (old_selection, _) = machine.select(true);
        machine.defer_finish_during_selection(context.clone());
        assert!(machine.pending.is_some());
        assert_eq!(machine.current.as_ref().unwrap().song.id, -1);

        // Replacing the lookup must retain the ended source, while a stale
        // failure must not clear the new lookup or consume its deferred end.
        let (latest, _) = machine.select(false);
        assert!(machine.reject_selection(old_selection).is_none());
        assert!(machine.selection(latest).is_some());
        assert_eq!(machine.reject_selection(latest), Some(context));
        assert!(machine.pending.is_none());
        assert_eq!(machine.current.as_ref().unwrap().song.id, -1);
        assert_eq!(queue.len(), 2);
        assert!(machine.reject_selection(latest).is_none());
    }

    #[test]
    fn selection_failure_does_not_autoplay_without_an_end_or_after_pause() {
        let generations = crate::audio::identity::PlaybackGenerationController::new();
        let context = generations.activate_generation();
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.push(song(1));
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();

        let (selection, _) = machine.select(false);
        assert!(machine.reject_selection(selection).is_none());
        assert!(machine.pending.is_none());

        let (selection, _) = machine.select(false);
        machine.defer_finish_during_selection(context);
        machine.intent = TransportIntent::Pause;
        assert!(machine.reject_selection(selection).is_none());
        assert_eq!(machine.intent, TransportIntent::Pause);
        assert!(machine.pending.is_none());
        let (later, _) = machine.select(false);
        assert!(machine.reject_selection(later).is_none());
    }

    #[test]
    fn successful_activation_and_explicit_cancellation_discard_deferred_ends() {
        let generations = crate::audio::identity::PlaybackGenerationController::new();
        let context = generations.activate_generation();
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(vec![song(1), song(2)], PlaybackOrigin::Normal);
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();

        machine.select(false);
        machine.defer_finish_during_selection(context.clone());
        awaiting(&mut machine, 2, target(&queue, 1), false);
        machine.activate(2).unwrap();
        let (selection, _) = machine.select(false);
        assert!(machine.reject_selection(selection).is_none());
        assert_eq!(machine.current.as_ref().unwrap().song.id, -2);

        let (selection, _) = machine.select(false);
        machine.defer_finish_during_selection(context);
        // Stop, clear queue, seek, and explicit replacement use this reset.
        machine.discard_deferred_finish();
        assert!(machine.reject_selection(selection).is_none());
    }

    #[test]
    fn fm_changes_only_when_the_matching_request_activates() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        let (selection, fm) = machine.select(true);
        assert!(!machine.origin().is_fm());
        assert_eq!(machine.selection(selection), Some(fm));
        queue.replace(vec![song(1)], fm);
        awaiting(&mut machine, 10, target(&queue, 0), false);
        assert!(!machine.origin().is_fm());
        machine.activate(10).unwrap();
        assert_eq!(machine.origin(), fm);

        queue.push(song(2));
        awaiting(&mut machine, 11, target(&queue, 1), false);
        // A failed replacement does not mutate the confirmed source.
        machine.take_activation(11).unwrap();
        assert_eq!(machine.origin(), fm);
        awaiting(&mut machine, 12, target(&queue, 1), false);
        machine.activate(12).unwrap();
        assert_eq!(machine.origin(), PlaybackOrigin::Normal);
    }

    #[test]
    fn leaving_fm_removes_only_radio_entries_and_keeps_identities() {
        let mut queue = PlaybackQueue::default();
        let fm = PlaybackOrigin::PersonalFm { session: 3 };
        queue.replace(vec![song(1), song(2)], fm);
        queue.push(song(9));
        let (kept, _) = queue.entry(2).expect("normal entry");
        queue.extend_with_origin(vec![song(3)], fm);
        let revision = queue.revision();

        assert_eq!(queue.remove_personal_fm_entries(), 3);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, song(9).id);
        assert_eq!(queue.index_of(kept), Some(0));
        assert!(!queue.has_origin(fm));
        assert!(queue.revision() > revision);

        let revision = queue.revision();
        assert_eq!(queue.remove_personal_fm_entries(), 0);
        assert_eq!(queue.revision(), revision);
    }

    #[test]
    fn rapid_replacement_rejects_old_selection_and_audio_acknowledgements() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(vec![song(1), song(2), song(3)], PlaybackOrigin::Normal);
        let (old, _) = machine.select(true);
        let (latest, _) = machine.select(false);
        assert!(machine.selection(old).is_none());
        assert!(machine.selection(latest).is_some());
        awaiting(&mut machine, 1, target(&queue, 0), false);
        awaiting(&mut machine, 2, target(&queue, 1), false);
        awaiting(&mut machine, 3, target(&queue, 2), false);
        assert!(machine.activate(1).is_none());
        assert!(machine.activate(2).is_none());
        machine.activate(3).unwrap();
        assert_eq!(machine.current.as_ref().unwrap().song.id, -3);
        assert!(machine.activate(3).is_none());
    }

    #[test]
    fn pause_during_preparation_survives_preload_and_restore_acknowledgement() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.push(song(1));
        let generations = crate::audio::identity::PlaybackGenerationController::new();
        let context = generations.activate_generation();
        machine.pending = Some(PlaybackTransaction::Preparing {
            target: target(&queue, 0),
            context: context.clone(),
        });
        machine.intent = TransportIntent::Pause;
        assert!(machine.preparing(&context).is_some());
        awaiting(&mut machine, 7, target(&queue, 0), false);
        machine.activate(7).unwrap();
        assert_eq!(machine.intent, TransportIntent::Pause);
    }

    #[test]
    fn queue_edits_keep_duplicate_occurrences_and_resolution_targets_distinct() {
        let mut queue = PlaybackQueue::default();
        queue.replace(vec![song(1), song(1), song(2)], PlaybackOrigin::Normal);
        let first = queue.entry(0).unwrap().0;
        let second = queue.entry(1).unwrap().0;
        let requested = target(&queue, 2);
        queue.remove(0);
        assert_eq!(queue.index_of(first), None);
        assert_eq!(queue.index_of(second), Some(0));
        assert_eq!(queue.index_of(requested.entry.unwrap()), Some(1));
        queue.insert(0, song(3));
        assert_eq!(queue.index_of(requested.entry.unwrap()), Some(2));
        queue.replace(vec![song(2)], PlaybackOrigin::Normal);
        assert_eq!(queue.index_of(requested.entry.unwrap()), None);
    }

    #[test]
    fn scheduled_handoff_does_not_commit_until_audio_clock_activation() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(
            vec![song(1), song(2)],
            PlaybackOrigin::PersonalFm { session: 4 },
        );
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();
        awaiting(&mut machine, 2, target(&queue, 1), true);
        assert!(machine.scheduled());
        assert_eq!(machine.current.as_ref().unwrap().song.id, -1);
        // A manual replacement cancels the scheduled request before activation.
        machine.select(false);
        assert!(machine.activate(2).is_none());
        assert_eq!(machine.current.as_ref().unwrap().song.id, -1);
    }

    #[test]
    fn fm_refills_are_deduplicated_and_cannot_take_over_new_selections() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        let fm = PlaybackOrigin::PersonalFm { session: 3 };
        queue.replace(vec![song(1)], fm);
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();
        let (refill, _) = machine.begin_fm_refill(false).unwrap();
        assert!(machine.begin_fm_refill(true).is_none());
        assert!(machine.finish_fm_refill(refill, &queue).unwrap().play);
        assert!(machine.finish_fm_refill(refill, &queue).is_none());

        let (refill, _) = machine.begin_fm_refill(true).unwrap();
        machine.select(false);
        assert!(machine.finish_fm_refill(refill, &queue).is_none());
    }

    #[test]
    fn fm_refill_cannot_autoplay_after_the_user_already_advanced() {
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(
            vec![song(1), song(2)],
            PlaybackOrigin::PersonalFm { session: 3 },
        );
        awaiting(&mut machine, 1, target(&queue, 0), false);
        machine.activate(1).unwrap();
        let (refill, _) = machine.begin_fm_refill(true).unwrap();
        awaiting(&mut machine, 2, target(&queue, 1), false);
        machine.activate(2).unwrap();
        assert!(!machine.finish_fm_refill(refill, &queue).unwrap().play);
    }

    #[test]
    fn stale_resolution_cannot_relabel_a_new_target() {
        let generations = crate::audio::identity::PlaybackGenerationController::new();
        let old = generations.activate_generation();
        let current = generations.activate_generation();
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.push(song(2));
        machine.pending = Some(PlaybackTransaction::Preparing {
            target: target(&queue, 0),
            context: current.clone(),
        });
        assert!(machine.preparing(&old).is_none());
        assert_eq!(machine.preparing(&current).unwrap().song.id, -2);
    }

    #[test]
    fn restored_fm_sessions_and_duplicate_origins_remain_distinct() {
        let origins = vec![
            PlaybackOrigin::PersonalFm { session: 41 },
            PlaybackOrigin::Normal,
        ];
        let serialized = serde_json::to_string(&origins).unwrap();
        let restored: Vec<PlaybackOrigin> = serde_json::from_str(&serialized).unwrap();
        let mut machine = PlaybackMachine::default();
        let mut queue = PlaybackQueue::default();
        queue.replace(vec![song(1), song(1)], PlaybackOrigin::Normal);
        for (idx, origin) in restored.into_iter().enumerate() {
            machine.observe_restored_origin(origin);
            queue.set_origin(idx, origin);
        }
        // Restore the second occurrence, not the first matching song ID.
        awaiting(&mut machine, 7, target(&queue, 1), false);
        if let Some(PlaybackTransaction::AwaitingActivation(request)) = &mut machine.pending {
            request.kind = PendingPlaybackKind::LoadPaused;
        }
        machine.activate(7).unwrap();
        assert_eq!(machine.origin(), PlaybackOrigin::Normal);
        assert_eq!(machine.intent, TransportIntent::Pause);
        assert_eq!(
            machine.current.as_ref().unwrap().entry,
            queue.entry(1).map(|entry| entry.0)
        );
        let (_, next_fm) = machine.select(true);
        assert_eq!(next_fm, PlaybackOrigin::PersonalFm { session: 42 });
    }
}
