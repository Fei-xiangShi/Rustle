//! Lyrics preload manager - tracks per-song lyrics cache warmup and display fetch state
//!
//! Coordinates between background warmup (when lyrics page is closed) and
//! display fetch (when lyrics page is opened for a song), avoiding duplicate
//! network requests.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsPreloadStatus {
    Fetching,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct LyricsPreloadEntry {
    pub ncm_id: u64,
    pub status: LyricsPreloadStatus,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFetchAction {
    StartFetch,
    AwaitExisting,
    UseCache,
}

#[derive(Debug, Default, Clone)]
pub struct LyricsPreloadManager {
    pub entries: HashMap<i64, LyricsPreloadEntry>,
}

impl LyricsPreloadManager {
    fn has_any_cache(ncm_id: u64) -> bool {
        crate::features::lyrics::has_cached_lyrics(ncm_id)
    }

    pub fn should_schedule_warmup(&self, song_id: i64) -> bool {
        // Validate version and full content in the background fetch, once per
        // session. UI scheduling must never deserialize the cache to probe it.
        !self.entries.contains_key(&song_id)
    }

    pub fn begin_warmup(&mut self, song_id: i64, ncm_id: u64) -> bool {
        if matches!(
            self.entries.get(&song_id).map(|entry| entry.status),
            Some(LyricsPreloadStatus::Fetching | LyricsPreloadStatus::Ready)
        ) {
            return false;
        }

        self.entries.insert(
            song_id,
            LyricsPreloadEntry {
                ncm_id,
                status: LyricsPreloadStatus::Fetching,
                last_error: None,
            },
        );
        true
    }

    pub fn register_display_fetch(&mut self, song_id: i64, ncm_id: u64) -> DisplayFetchAction {
        match self.entries.get(&song_id).map(|entry| entry.status) {
            Some(LyricsPreloadStatus::Ready) if Self::has_any_cache(ncm_id) => {
                DisplayFetchAction::UseCache
            }
            Some(LyricsPreloadStatus::Ready) => {
                self.entries.remove(&song_id);
                self.begin_fetch_entry(song_id, ncm_id);
                DisplayFetchAction::StartFetch
            }
            Some(LyricsPreloadStatus::Fetching) => DisplayFetchAction::AwaitExisting,
            Some(LyricsPreloadStatus::Failed) | None => {
                self.begin_fetch_entry(song_id, ncm_id);
                DisplayFetchAction::StartFetch
            }
        }
    }

    pub fn finish_warmup(&mut self, song_id: i64, result: Result<(), String>) {
        let Some(entry) = self.entries.get_mut(&song_id) else {
            return;
        };

        match result {
            Ok(()) => {
                if Self::has_any_cache(entry.ncm_id) {
                    entry.status = LyricsPreloadStatus::Ready;
                    entry.last_error = None;
                } else {
                    entry.status = LyricsPreloadStatus::Failed;
                    entry.last_error = Some("Lyrics fetched but cache file was not created".into());
                }
            }
            Err(error) => {
                entry.status = LyricsPreloadStatus::Failed;
                entry.last_error = Some(error);
            }
        }
    }

    pub fn mark_ready(&mut self, song_id: i64, ncm_id: u64) {
        self.entries.insert(
            song_id,
            LyricsPreloadEntry {
                ncm_id,
                status: LyricsPreloadStatus::Ready,
                last_error: None,
            },
        );
    }

    fn begin_fetch_entry(&mut self, song_id: i64, ncm_id: u64) {
        self.entries.insert(
            song_id,
            LyricsPreloadEntry {
                ncm_id,
                status: LyricsPreloadStatus::Fetching,
                last_error: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_is_scheduled_once_and_display_joins_the_pending_fetch() {
        let mut manager = LyricsPreloadManager::default();
        assert!(manager.should_schedule_warmup(-42));
        assert!(manager.begin_warmup(-42, 42));
        assert!(!manager.should_schedule_warmup(-42));
        assert!(!manager.begin_warmup(-42, 42));
        assert_eq!(
            manager.register_display_fetch(-42, 42),
            DisplayFetchAction::AwaitExisting
        );
        manager.mark_ready(-42, 42);
        assert!(!manager.should_schedule_warmup(-42));
        assert!(!manager.begin_warmup(-42, 42));
    }

    #[test]
    fn invalid_ready_cache_can_fall_back_to_a_single_online_fetch() {
        let mut manager = LyricsPreloadManager::default();
        manager.mark_ready(-42, 42);
        manager.finish_warmup(-42, Err("Cached lyrics are unavailable".into()));
        assert_eq!(
            manager.register_display_fetch(-42, 42),
            DisplayFetchAction::StartFetch
        );
        assert_eq!(
            manager.register_display_fetch(-42, 42),
            DisplayFetchAction::AwaitExisting
        );
    }
}
