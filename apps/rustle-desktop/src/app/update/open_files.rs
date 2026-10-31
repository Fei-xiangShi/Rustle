//! Opening local audio files from outside the library.
//!
//! Files arrive from launch arguments, the single-instance IPC channel (file
//! association / "Open with"), and window drag and drop. Requests are
//! debounced so that shells launching one process per selected file, or drops
//! delivering one event per file, still produce a single playback queue.
//! Imports run one at a time. Only the latest request may replace the queue;
//! subsequent playback choices revoke older requests' autoplay permission.
//!
//! Opened files are upserted into the songs table (without joining any
//! playlist) so the regular local playback, cover, lyrics, and loudness
//! pipelines apply unchanged.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iced::Task;

use super::import::build_new_song;
use crate::app::helpers::load_songs;
use crate::app::message::{ContextMenuAction, Message};
use crate::app::state::{App, OpenFilesState};
use crate::database::{Database, DbSong};
use crate::features::import::{CoverCache, ScanConfig, is_audio_file, scan_audio_file};

/// Quiet period that ends one open gesture. Each new request restarts it.
const OPEN_FILES_DEBOUNCE: Duration = Duration::from_millis(400);

/// Library extensions the playback decoder cannot handle (no Opus or WMA
/// codec is available); opening them directly would only fail at playback.
const UNDECODABLE_EXTENSIONS: &[&str] = &["opus", "wma"];

impl OpenFilesState {
    fn invalidate_for_message(&mut self, message: &Message) {
        if matches!(
            message,
            Message::PlaySong(_)
                | Message::PlayPlaylist(_)
                | Message::PlayQueueIndex(_)
                | Message::PlayNcmSong(_)
                | Message::PlaySearchSong(_)
                | Message::PlayDiscoverPlaylist(_)
                | Message::ContextMenuAction(ContextMenuAction::PlayNow, _)
                | Message::NextSong
                | Message::PrevSong
                | Message::ClearQueue
                | Message::MprisCommand(crate::platform::media_controls::MediaCommand::Stop)
        ) {
            self.invalidate_autoplay();
        }
    }

    fn invalidate_autoplay(&mut self) {
        self.debounce_generation = self.debounce_generation.wrapping_add(1);
        self.pending.clear();
        self.flush_scheduled = false;
        // Keep import ownership until its completion; newer batches must wait.
    }
}

impl App {
    /// Whether opened files are waiting to replace the queue or importing.
    pub(super) fn open_files_requested(&self) -> bool {
        let open = &self.playback.open_files;
        !open.pending.is_empty() || open.import_in_flight
    }

    /// A later playback choice revokes pending file-open autoplay, including
    /// requests still waiting for the database or debounce timer. An active
    /// import may finish writing metadata, but its result cannot replace audio.
    ///
    /// Messages that choose other audio or cancel playback/queue work are listed.
    /// Conditional playback choices call [`Self::invalidate_open_files_autoplay`]
    /// at the point they commit: personal FM routing, `rustle://` play links,
    /// and search suggestion songs.
    pub(super) fn invalidate_open_files_for_message(&mut self, message: &Message) {
        self.playback.open_files.invalidate_for_message(message);
    }

    pub(super) fn invalidate_open_files_autoplay(&mut self) {
        self.playback.open_files.invalidate_autoplay();
    }

    /// Handle local file open messages
    pub(super) fn handle_open_files(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::OpenLocalFiles(paths) => {
                if paths.is_empty() || !self.cancel_playback_transaction() {
                    return Some(Task::none());
                }
                self.playback.machine.discard_deferred_finish();
                let open = &mut self.playback.open_files;
                open.pending.extend(paths.iter().cloned());
                open.debounce_generation = open.debounce_generation.wrapping_add(1);
                open.flush_scheduled = true;
                let generation = open.debounce_generation;
                Some(Task::batch([
                    Task::done(Message::ShowOrFocusWindow),
                    Task::perform(tokio::time::sleep(OPEN_FILES_DEBOUNCE), move |()| {
                        Message::FlushOpenLocalFiles(generation)
                    }),
                ]))
            }

            Message::FlushOpenLocalFiles(generation) => {
                if *generation != self.playback.open_files.debounce_generation {
                    // A newer request restarted the debounce window.
                    return Some(Task::none());
                }
                self.playback.open_files.flush_scheduled = false;
                Some(self.flush_open_local_files())
            }

            Message::LocalFilesOpened(generation, songs, skipped) => {
                tracing::info!(
                    opened = songs.len(),
                    skipped,
                    "Local files opened for playback"
                );
                self.playback.open_files.import_in_flight = false;

                let mut tasks = Vec::new();
                if *skipped > 0 {
                    let text = if songs.is_empty() {
                        "无法打开所选文件：格式不受支持或文件不可读".to_string()
                    } else {
                        format!("已跳过 {skipped} 个无法打开的文件")
                    };
                    tasks.push(Self::toast_warning(text));
                }
                if !songs.is_empty() {
                    if *generation == self.playback.open_files.debounce_generation {
                        // Commit under the same generation check. A deferred
                        // QueueLoaded message would allow a later user action
                        // to slip between validation and queue replacement.
                        tasks.push(self.play_loaded_local_queue(
                            songs,
                            crate::app::playback_machine::TransportIntent::Play,
                        ));
                    }
                    // Opened files are now library songs; refresh the library view.
                    if let Some(db) = &self.core.db {
                        tasks.push(Task::perform(load_songs(db.clone()), Message::SongsLoaded));
                    }
                }
                // Requests that completed their debounce while this import ran
                // were held back; process them now, after this batch.
                if !self.playback.open_files.flush_scheduled {
                    tasks.push(self.flush_open_local_files());
                }
                Some(Task::batch(tasks))
            }

            _ => None,
        }
    }

    /// Import and play every pending file once storage and the restored queue
    /// are available and no other import is running. Otherwise the files stay
    /// pending until `QueueRestored` or the running import calls this again.
    pub(super) fn flush_open_local_files(&mut self) -> Task<Message> {
        if self.playback.open_files.pending.is_empty() || self.playback.open_files.flush_scheduled {
            return Task::none();
        }
        if self.playback.open_files.import_in_flight {
            tracing::debug!("Deferring opened files until the running import finishes");
            return Task::none();
        }
        let Some(db) = self.core.db.clone() else {
            tracing::debug!("Deferring opened files until the database is ready");
            return Task::none();
        };
        if !self.playback.startup_restore.queue_loaded {
            tracing::debug!("Deferring opened files until the saved queue is restored");
            return Task::none();
        }

        // Explicitly opened files replace whatever the previous session was
        // about to resume, including a restore that is already resolving.
        self.finish_startup_restore();

        self.playback.open_files.import_in_flight = true;
        let paths = std::mem::take(&mut self.playback.open_files.pending);
        let generation = self.playback.open_files.debounce_generation;
        let cache = self.core.cover_cache.clone();
        Task::perform(
            import_opened_files(db, cache, paths),
            move |(songs, skipped)| Message::LocalFilesOpened(generation, songs, skipped),
        )
    }
}

fn is_playable_audio_file(path: &Path) -> bool {
    is_audio_file(path)
        && !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                UNDECODABLE_EXTENSIONS
                    .iter()
                    .any(|undecodable| extension.eq_ignore_ascii_case(undecodable))
            })
}

/// Import files in request order, dropping duplicates and unreadable files.
async fn import_opened_files(
    db: Arc<Database>,
    cache: Option<Arc<CoverCache>>,
    paths: Vec<PathBuf>,
) -> (Vec<DbSong>, usize) {
    let mut seen = HashSet::new();
    let mut songs = Vec::with_capacity(paths.len());
    let mut skipped = 0;

    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        match import_opened_file(&db, cache.clone(), path).await {
            Ok(song) => songs.push(song),
            Err(error) => {
                tracing::warn!(error = %error, "Failed to open local file");
                skipped += 1;
            }
        }
    }

    (songs, skipped)
}

async fn import_opened_file(
    db: &Database,
    cache: Option<Arc<CoverCache>>,
    path: PathBuf,
) -> Result<DbSong> {
    anyhow::ensure!(is_playable_audio_file(&path), "unsupported audio format");
    anyhow::ensure!(path.is_file(), "file is not available");

    // Playback starts immediately; loudness is analyzed lazily by the player
    // when the file carries no gain tag.
    let config = ScanConfig {
        analyze_loudness: false,
        ..ScanConfig::default()
    };
    let scan_path = path.clone();
    let scan_result =
        tokio::task::spawn_blocking(move || scan_audio_file(&scan_path, &config, cache.as_deref()))
            .await
            .context("Failed to join file scan task")??;

    db.upsert_local_song(build_new_song(&path, scan_result))
        .await?;
    db.get_song_by_path(&path.to_string_lossy())
        .await?
        .context("opened song was not persisted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::media_controls::MediaCommand;

    #[test]
    fn stop_and_clear_queue_revoke_debounced_and_inflight_file_autoplay() {
        for message in [Message::ClearQueue, Message::MprisCommand(MediaCommand::Stop)] {
            for importing in [false, true] {
                let mut open = OpenFilesState {
                    pending: vec![PathBuf::from("pending.flac")],
                    debounce_generation: 7,
                    flush_scheduled: true,
                    import_in_flight: importing,
                };
                let request_generation = open.debounce_generation;

                open.invalidate_for_message(&message);

                // Neither the queued timer nor an older import completion may
                // flush/start playback after the explicit cancellation.
                assert_ne!(open.debounce_generation, request_generation);
                assert!(open.pending.is_empty());
                assert!(!open.flush_scheduled);
                // Preserve serialization: another import must wait for the
                // existing worker even though it has lost autoplay permission.
                assert_eq!(open.import_in_flight, importing);
            }
        }
    }

    #[test]
    fn showing_the_window_keeps_file_autoplay_pending() {
        let path = PathBuf::from("pending.flac");
        let mut open = OpenFilesState {
            pending: vec![path.clone()],
            debounce_generation: 7,
            flush_scheduled: true,
            ..OpenFilesState::default()
        };
        // OpenLocalFiles itself requests focus; focus must not cancel its timer.
        open.invalidate_for_message(&Message::ShowOrFocusWindow);
        assert_eq!(open.debounce_generation, 7);
        assert_eq!(open.pending, vec![path]);
        assert!(open.flush_scheduled);
    }
}
