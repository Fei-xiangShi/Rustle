//! Unified queue navigation - Single Source of Truth for index calculations
//!
//! This module provides a single, consistent way to calculate next/prev indices
//! across all play modes. All code that needs to determine which song comes next
//! or previous should use this module.

use std::collections::HashSet;

use crate::database::DbSong;
use crate::features::PlayMode;

/// Stable shuffle navigation shared by preloading and actual playback.
///
/// `remaining` is the current shuffled deck. A candidate is only removed when
/// the app confirms that song as current or its playback fails; speculative
/// preload reads never advance navigation. Failed entries stay excluded until
/// successful manual playback or a queue/mode reset. `history` and `cursor`
/// provide real Previous semantics without recording failed attempts as plays.
#[derive(Debug, Clone, Default)]
pub struct ShuffleCache {
    next: Option<usize>,
    prev: Option<usize>,
    history: Vec<usize>,
    cursor: Option<usize>,
    remaining: Vec<usize>,
    failed: HashSet<usize>,
    queue_len: usize,
    current: Option<usize>,
}

impl ShuffleCache {
    /// Synchronize navigation after a song has actually become current.
    pub fn sync_current(&mut self, queue_len: usize, current: Option<usize>) {
        let Some(current) = current.filter(|idx| *idx < queue_len) else {
            self.clear();
            return;
        };

        if self.queue_len != queue_len || self.current.is_none() {
            self.reset_at(queue_len, current);
            return;
        }

        self.failed.remove(&current);
        if self.current != Some(current) {
            let cursor = self.cursor.unwrap_or(0);
            let forward = self.next_history_cursor();
            let previous = self.previous_history_cursor();
            if forward.is_some_and(|pos| self.history[pos] == current) {
                self.cursor = forward;
            } else if previous.is_some_and(|pos| self.history[pos] == current) {
                self.cursor = previous;
            } else {
                self.history.truncate(cursor.saturating_add(1));
                self.history.push(current);
                self.cursor = Some(self.history.len() - 1);
            }
            self.remaining.retain(|idx| *idx != current);
            self.current = Some(current);
        }

        self.refresh_adjacent();
    }

    fn reset_at(&mut self, queue_len: usize, current: usize) {
        if self.queue_len != queue_len {
            self.failed.clear();
        }
        self.failed.remove(&current);
        self.queue_len = queue_len;
        self.current = Some(current);
        self.history.clear();
        self.history.push(current);
        self.cursor = Some(0);
        self.remaining.clear();
        self.refill_deck();
        self.refresh_adjacent();
    }

    fn refill_deck(&mut self) {
        self.remaining = (0..self.queue_len)
            .filter(|idx| Some(*idx) != self.current && !self.failed.contains(idx))
            .collect();

        use rand::RngExt;
        let mut rng = rand::rng();
        for i in (1..self.remaining.len()).rev() {
            let j = rng.random_range(0..=i);
            self.remaining.swap(i, j);
        }
    }

    fn next_history_cursor(&self) -> Option<usize> {
        (self.cursor? + 1..self.history.len()).find(|pos| {
            Some(self.history[*pos]) != self.current && !self.failed.contains(&self.history[*pos])
        })
    }

    fn previous_history_cursor(&self) -> Option<usize> {
        (0..self.cursor?).rev().find(|pos| {
            Some(self.history[*pos]) != self.current && !self.failed.contains(&self.history[*pos])
        })
    }

    fn refresh_adjacent(&mut self) {
        self.prev = self.previous_history_cursor().map(|pos| self.history[pos]);
        self.next = if let Some(forward) = self.next_history_cursor() {
            Some(self.history[forward])
        } else {
            if self.remaining.is_empty() && self.queue_len > 1 {
                self.refill_deck();
            }
            self.remaining.last().copied()
        };

        tracing::debug!(
            "Shuffle navigation synchronized: current={:?}, next={:?}, prev={:?}, history_len={}, remaining={}",
            self.current,
            self.next,
            self.prev,
            self.history.len(),
            self.remaining.len()
        );
    }

    /// Exclude an actual playback failure without committing it to history.
    fn skip_failed(&mut self, queue_len: usize, failed_idx: usize) -> Option<usize> {
        if self.queue_len != queue_len {
            self.clear();
            self.queue_len = queue_len;
        }
        self.failed.insert(failed_idx);
        self.remaining.retain(|idx| *idx != failed_idx);
        self.refresh_adjacent();
        self.next
    }

    fn next_for(&self, queue_len: usize, current: Option<usize>) -> Option<usize> {
        (self.queue_len == queue_len && self.current == current)
            .then_some(self.next)
            .flatten()
    }

    fn prev_for(&self, queue_len: usize, current: Option<usize>) -> Option<usize> {
        (self.queue_len == queue_len && self.current == current)
            .then_some(self.prev)
            .flatten()
    }

    /// Clear the cache (call when queue or play mode changes)
    pub fn clear(&mut self) {
        self.next = None;
        self.prev = None;
        self.history.clear();
        self.cursor = None;
        self.remaining.clear();
        self.failed.clear();
        self.queue_len = 0;
        self.current = None;
    }
}

/// 队列导航器 - 根据播放模式计算 next/prev 索引
///
/// 索引计算的唯一数据源
/// All code paths (playback, preloading, UI) should use this.
pub struct QueueNavigator<'a> {
    queue_len: usize,
    current_idx: Option<usize>,
    play_mode: PlayMode,
    shuffle_cache: &'a ShuffleCache,
}

impl<'a> QueueNavigator<'a> {
    /// Create a new navigator
    pub fn new(
        queue_len: usize,
        current_idx: Option<usize>,
        play_mode: PlayMode,
        shuffle_cache: &'a ShuffleCache,
    ) -> Self {
        Self {
            queue_len,
            current_idx,
            play_mode,
            shuffle_cache,
        }
    }

    /// Calculate the next track index
    pub fn next_index(&self) -> Option<usize> {
        if self.queue_len == 0 {
            return None;
        }

        match self.play_mode {
            PlayMode::Shuffle => self
                .shuffle_cache
                .next_for(self.queue_len, self.current_idx),
            PlayMode::LoopOne => self.current_idx,
            PlayMode::LoopAll => Some((self.current_idx? + 1) % self.queue_len),
            PlayMode::Sequential => {
                let next = self.current_idx? + 1;
                if next >= self.queue_len {
                    None
                } else {
                    Some(next)
                }
            }
        }
    }

    /// Calculate the previous track index
    pub fn prev_index(&self) -> Option<usize> {
        if self.queue_len == 0 {
            return None;
        }

        match self.play_mode {
            PlayMode::Shuffle => self
                .shuffle_cache
                .prev_for(self.queue_len, self.current_idx),
            PlayMode::LoopOne => self.current_idx,
            PlayMode::LoopAll => {
                let current_idx = self.current_idx?;
                if current_idx == 0 {
                    Some(self.queue_len - 1)
                } else {
                    Some(current_idx - 1)
                }
            }
            PlayMode::Sequential => {
                let current_idx = self.current_idx?;
                if current_idx == 0 {
                    None
                } else {
                    Some(current_idx - 1)
                }
            }
        }
    }

    /// Get both adjacent indices at once (more efficient for preloading)
    pub fn adjacent_indices(&self) -> AdjacentIndices {
        AdjacentIndices {
            next: self.next_index(),
            prev: self.prev_index(),
        }
    }

    /// Current index used for navigation
    pub fn current_index(&self) -> Option<usize> {
        self.current_idx.filter(|idx| *idx < self.queue_len)
    }
}

/// Result of adjacent index calculation
#[derive(Debug, Clone, Copy)]
pub struct AdjacentIndices {
    pub next: Option<usize>,
    pub prev: Option<usize>,
}

/// Helper to check if a song needs NCM resolution
pub fn needs_ncm_download(song: &DbSong) -> bool {
    let is_ncm = song.id < 0 || song.file_path.is_empty() || song.file_path.starts_with("ncm://");
    if !is_ncm {
        return false;
    }

    // Streaming cache entries are quality-specific and cannot determine the
    // official level for the current preference. Let URL negotiation decide.
    song.file_path.is_empty()
        || song.file_path.starts_with("ncm://")
        || !std::path::Path::new(&song.file_path).exists()
}

/// Skip to next playable track, handling failures
/// Returns the next index to try, skipping the failed index
/// IMPORTANT: This always moves to a DIFFERENT song, never returns failed_idx
pub fn skip_to_next_playable(
    queue_len: usize,
    failed_idx: usize,
    play_mode: PlayMode,
    shuffle_cache: &mut ShuffleCache,
) -> Option<usize> {
    if failed_idx >= queue_len {
        return None;
    }

    if play_mode == PlayMode::Shuffle {
        return shuffle_cache.skip_failed(queue_len, failed_idx);
    }

    // If only one song in queue, can't skip to another
    if queue_len == 1 {
        return None;
    }

    // Preserve the sequential failure fallback for the other playback modes.
    let next = (failed_idx + 1) % queue_len;

    // Make sure we're not returning the same index
    if next == failed_idx {
        return None;
    }

    Some(next)
}

/// Helper to get local file path for a song (if available)
pub fn get_local_path(song: &DbSong) -> Option<std::path::PathBuf> {
    // Check if it's an NCM song
    let is_ncm = song.id < 0 || song.file_path.is_empty() || song.file_path.starts_with("ncm://");

    if is_ncm {
        // Quality-scoped streaming cache files are not local-library paths.
        // Only an explicit downloaded file can be opened as a local source.
        let path = std::path::PathBuf::from(&song.file_path);
        return path
            .is_absolute()
            .then_some(path)
            .filter(|path| path.exists());
    }

    // For local songs, check if file exists
    let path = std::path::PathBuf::from(&song.file_path);
    if path.exists() { Some(path) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_next_is_stable_and_never_repeats_current_when_queue_has_choices() {
        let mut cache = ShuffleCache::default();
        cache.sync_current(5, Some(2));
        let first = cache.next.expect("shuffle should plan a next track");

        assert_ne!(first, 2);
        cache.sync_current(5, Some(2));
        assert_eq!(cache.next, Some(first));

        let navigator = QueueNavigator::new(5, Some(2), PlayMode::Shuffle, &cache);
        assert_eq!(navigator.next_index(), Some(first));
    }

    #[test]
    fn shuffle_previous_uses_real_history_and_forward_reuses_it() {
        let mut cache = ShuffleCache::default();
        cache.sync_current(4, Some(0));
        let first = cache.next.unwrap();
        cache.sync_current(4, Some(first));

        assert_eq!(cache.prev, Some(0));
        cache.sync_current(4, Some(0));
        assert_eq!(cache.next, Some(first));
    }

    #[test]
    fn two_track_shuffle_converges_next_and_previous_after_first_transition() {
        let mut cache = ShuffleCache::default();
        cache.sync_current(2, Some(0));
        cache.sync_current(2, Some(1));

        assert_eq!(cache.next, Some(0));
        assert_eq!(cache.prev, Some(0));
    }

    fn planned_shuffle() -> ShuffleCache {
        ShuffleCache {
            next: Some(1),
            history: vec![0],
            cursor: Some(0),
            remaining: vec![2, 3, 1],
            queue_len: 4,
            current: Some(0),
            ..ShuffleCache::default()
        }
    }

    #[test]
    fn shuffle_failure_advances_the_deck_and_does_not_replay_the_fallback() {
        let mut cache = planned_shuffle();
        let next = skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache);
        assert_eq!(next, Some(3));
        cache.sync_current(4, next);
        assert_eq!(cache.prev, Some(0));
        assert_eq!(cache.next, Some(2));

        // Multiple deck refills must never restore the failed candidate.
        for _ in 0..12 {
            let next = cache.next.unwrap();
            assert_ne!(next, 1);
            assert_ne!(Some(next), cache.current);
            cache.sync_current(4, Some(next));
        }
    }

    #[test]
    fn shuffle_failure_skips_unavailable_history_without_recording_a_play() {
        let mut cache = planned_shuffle();
        cache.sync_current(4, Some(1));
        cache.sync_current(4, Some(3));
        cache.sync_current(4, Some(1));
        cache.sync_current(4, Some(0));

        let next = skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache);
        assert_eq!(next, Some(3));
        cache.sync_current(4, next);
        assert_eq!(cache.history, vec![0, 1, 3]);
        assert_eq!(cache.prev, Some(0));
        assert_eq!(cache.next, Some(2));
        cache.sync_current(4, Some(0));
        assert_eq!(cache.next, Some(3));
    }

    #[test]
    fn shuffle_failure_stops_when_no_other_candidate_is_available() {
        let mut cache = planned_shuffle();
        assert_eq!(
            skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache),
            Some(3)
        );
        assert_eq!(
            skip_to_next_playable(4, 3, PlayMode::Shuffle, &mut cache),
            Some(2)
        );
        assert_eq!(
            skip_to_next_playable(4, 2, PlayMode::Shuffle, &mut cache),
            None
        );
        assert_eq!(cache.history, vec![0]);
        assert_eq!(cache.next, None);
    }

    #[test]
    fn shuffle_failed_track_can_recover_after_manual_play_or_queue_reset() {
        let mut cache = planned_shuffle();
        skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache);
        cache.sync_current(4, Some(1));
        cache.sync_current(4, Some(0));
        assert_eq!(cache.next, Some(1));

        skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache);
        cache.clear();
        cache.sync_current(2, Some(0));
        assert_eq!(cache.next, Some(1));
    }

    #[test]
    fn shuffle_failure_before_first_play_does_not_create_fake_history() {
        let mut cache = ShuffleCache::default();
        assert_eq!(
            skip_to_next_playable(2, 0, PlayMode::Shuffle, &mut cache),
            Some(1)
        );
        cache.sync_current(2, Some(1));
        assert_eq!(cache.prev, None);
        assert_eq!(cache.next, None);
        assert_eq!(cache.history, vec![1]);
    }

    #[test]
    fn shuffle_failure_does_not_replay_current_through_repeated_history() {
        let mut cache = ShuffleCache {
            history: vec![0, 1, 0, 3],
            remaining: vec![2],
            ..planned_shuffle()
        };
        assert_eq!(
            skip_to_next_playable(4, 1, PlayMode::Shuffle, &mut cache),
            Some(3)
        );
        cache.sync_current(4, Some(3));
        assert_eq!(cache.prev, Some(0));
        cache.sync_current(4, Some(0));
        assert_eq!(cache.prev, None);
        assert_eq!(cache.next, Some(3));
    }

    #[test]
    fn shuffle_failure_handles_current_track_and_invalid_queue_indices() {
        let mut cache = planned_shuffle();
        assert_eq!(
            skip_to_next_playable(4, 0, PlayMode::Shuffle, &mut cache),
            Some(1)
        );
        cache.sync_current(4, Some(1));
        assert_eq!(cache.prev, None);

        let next = cache.next;
        assert_eq!(
            skip_to_next_playable(4, 4, PlayMode::Shuffle, &mut cache),
            None
        );
        assert_eq!(cache.next, next);
        cache.clear();
        assert_eq!(
            skip_to_next_playable(1, 0, PlayMode::Shuffle, &mut cache),
            None
        );
        assert_eq!(
            skip_to_next_playable(0, 0, PlayMode::Shuffle, &mut cache),
            None
        );
    }

    #[test]
    fn non_shuffle_failure_keeps_sequential_fallback() {
        for mode in [PlayMode::Sequential, PlayMode::LoopAll, PlayMode::LoopOne] {
            let mut cache = ShuffleCache::default();
            assert_eq!(skip_to_next_playable(4, 3, mode, &mut cache), Some(0));
            assert_eq!(skip_to_next_playable(1, 0, mode, &mut cache), None);
            assert_eq!(skip_to_next_playable(0, 0, mode, &mut cache), None);
        }
    }
}
