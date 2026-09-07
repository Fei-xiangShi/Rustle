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
    fn has_word_level_cache(ncm_id: u64) -> bool {
        crate::features::lyrics::has_cached_word_level_lyrics(ncm_id)
    }

    fn has_any_cache(ncm_id: u64) -> bool {
        crate::features::lyrics::load_cached_lyrics(ncm_id).is_some()
    }

    pub fn should_schedule_warmup(&mut self, song_id: i64, ncm_id: u64) -> bool {
        // A YRC cache is complete. An old LRC-only cache still needs one
        // online upgrade attempt, so it must not short-circuit warmup here.
        if Self::has_word_level_cache(ncm_id) {
            self.mark_ready(song_id, ncm_id);
            return false;
        }

        if self
            .entries
            .get(&song_id)
            .is_some_and(|entry| entry.status == LyricsPreloadStatus::Ready)
        {
            self.entries.remove(&song_id);
            return true;
        }

        !matches!(
            self.entries.get(&song_id).map(|entry| entry.status),
            Some(LyricsPreloadStatus::Fetching) | Some(LyricsPreloadStatus::Failed)
        )
    }

    pub fn begin_warmup(&mut self, song_id: i64, ncm_id: u64) -> bool {
        if Self::has_word_level_cache(ncm_id) {
            self.mark_ready(song_id, ncm_id);
            return false;
        }

        if self
            .entries
            .get(&song_id)
            .is_some_and(|entry| entry.status == LyricsPreloadStatus::Ready)
        {
            self.entries.remove(&song_id);
        }

        if matches!(
            self.entries.get(&song_id).map(|entry| entry.status),
            Some(LyricsPreloadStatus::Fetching)
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
