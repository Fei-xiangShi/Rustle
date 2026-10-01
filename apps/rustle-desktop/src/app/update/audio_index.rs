use crate::{
    app::{App, Message},
    image::ImageKind,
    utils::audio_index::{self, AudioFile, AudioIndex},
};
use iced::Task;
use std::{collections::HashSet, path::PathBuf, sync::Arc};

// Becoming available offline does not change artwork content.
fn artwork_changed(before: Option<&AudioFile>, after: Option<&AudioFile>) -> bool {
    matches!((before, after), (Some(old), Some(new)) if old.path == new.path && old != new)
}

impl App {
    pub(super) fn handle_audio_index(&mut self, message: &Message) -> Option<Task<Message>> {
        let index = match message {
            Message::AudioFilesChanged(_) | Message::AudioIndexRefresh => return Some(Task::none()),
            Message::AudioIndexReady(index) => index,
            _ => return None,
        };
        self.library.audio_index.running = false;
        // Directory/settings changes supersede a snapshot built with old roots.
        if self.library.audio_index.full_pending {
            return Some(Task::none());
        }
        let Some(index) = index else {
            return Some(Task::none());
        };
        let previous = audio_index::snapshot();
        if previous == *index {
            return Some(Task::none());
        }

        let mut changed_covers = HashSet::new();
        // A resident cover can belong to a page that is no longer open. Its
        // extraction path still lets an external edit invalidate only that image.
        let old_files: std::collections::HashMap<_, _> = previous
            .cached
            .values()
            .chain(previous.downloaded.values())
            .chain(previous.local.values())
            .map(|file| (&file.path, file))
            .collect();
        let changed_files: HashSet<_> = index
            .cached
            .values()
            .chain(index.downloaded.values())
            .chain(index.local.values())
            .filter(|file| old_files.get(&file.path).is_some_and(|old| *old != *file))
            .map(|file| &file.path)
            .collect();
        for ((kind, id, _), entry) in &self.ui.image_state.entries {
            if matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover)
                && entry
                    .backing_file
                    .as_ref()
                    .is_some_and(|path| changed_files.contains(path))
            {
                changed_covers.insert((*kind, *id));
            }
        }
        let mut newly_available = HashSet::new();
        // Only resident, pending or failed artwork needs reconciliation. The
        // remembered URL map can contain thousands of previously visited songs.
        let resources: HashSet<_> = self
            .ui
            .image_state
            .entries
            .keys()
            .chain(self.ui.image_state.inflight.keys())
            .chain(self.ui.image_state.failures.keys())
            .map(|(kind, id, _)| (*kind, *id))
            .collect();
        let mut descriptions: std::collections::HashMap<_, _> = self
            .playback
            .queue
            .iter()
            .chain(self.playback.current_song.iter())
            .filter_map(|song| {
                crate::image::ncm_song_id(song.id, &song.file_path)
                    .map(|id| (id, (song.artist.as_str(), song.title.as_str())))
            })
            .collect();
        if let Some(page) = &self.ui.playlist_page.current {
            descriptions.extend(page.songs.iter().filter_map(|song| match song.cover_key {
                Some((ImageKind::SongCover, id)) => {
                    Some((id, (song.artist.as_str(), song.title.as_str())))
                }
                _ => None,
            }));
        }
        for (kind, id) in resources {
            let (before, after) = match kind {
                ImageKind::LocalSongCover => {
                    let source = self
                        .ui
                        .image_state
                        .known_sources
                        .get(&(kind, id))
                        .map(String::as_str)
                        .unwrap_or("");
                    (previous.local_file(source), index.local_file(source))
                }
                ImageKind::SongCover => {
                    let description = descriptions.get(&id);
                    let artist = description.map(|(artist, _)| *artist);
                    let title = description.map(|(_, title)| *title);
                    (
                        previous.locate("", -(id as i64), artist, title),
                        index.locate("", -(id as i64), artist, title),
                    )
                }
                _ => (None, None),
            };
            if artwork_changed(before, after) {
                changed_covers.insert((kind, id));
            }
            if before != after && after.is_some() {
                newly_available.insert((kind, id));
            }
        }
        for (kind, id) in &changed_covers {
            self.ui.image_state.refresh_retaining_image(*kind, *id);
        }
        let failures_before = self.ui.image_state.failures.len();
        self.ui
            .image_state
            .failures
            .retain(|(kind, id, _), _| !newly_available.contains(&(*kind, *id)));
        let retry_failed = failures_before != self.ui.image_state.failures.len();
        if !changed_covers.is_empty() || retry_failed {
            self.ui.image_state.artwork_epoch = self.ui.image_state.artwork_epoch.wrapping_add(1);
        }
        audio_index::publish(index.clone());
        let sources: std::collections::HashMap<_, _> = self
            .library
            .db_songs
            .iter()
            .map(|song| (song.id, song.file_path.as_str()))
            .collect();
        if let Some(page) = &mut self.ui.playlist_page.current {
            for song in &mut page.songs {
                let source = sources.get(&song.id).copied().unwrap_or("");
                let before =
                    previous.locate(source, song.id, Some(&song.artist), Some(&song.title));
                let after = index.locate(source, song.id, Some(&song.artist), Some(&song.title));
                if before != after {
                    song.source = match after {
                        Some(file) if file.path.starts_with(&index.cache_dir) => {
                            crate::utils::Source::Cached
                        }
                        Some(_) => crate::utils::Source::Local,
                        None => crate::utils::Source::Online,
                    };
                }
            }
        }
        Some(Task::none())
    }

    pub(super) fn refresh_audio_index_after_message(&mut self, message: &Message) -> Task<Message> {
        match message {
            Message::AudioIndexRefresh
            | Message::CacheCleared(..)
            | Message::SongsLoaded(..)
            | Message::UpdateDownloadDir(..) => self.library.audio_index.full_pending = true,
            Message::AudioFilesChanged(paths) => {
                self.library.audio_index.paths.extend(paths.iter().cloned())
            }
            Message::DownloadCompleted(_, path) => {
                self.library.audio_index.paths.insert(PathBuf::from(path));
            }
            Message::WatcherEvent(event) => {
                self.library.audio_index.paths.extend(watch_paths(event));
            }
            Message::SongEditsSaved(id) => {
                if let Some(song) = self.find_song_anywhere(*id) {
                    self.library
                        .audio_index
                        .paths
                        .insert(PathBuf::from(song.file_path));
                }
            }
            _ => {}
        }
        let state = &mut self.library.audio_index;
        if !state.needs_refresh() {
            return Task::none();
        }
        let full = !state.initialized || state.full_pending;
        state.running = true;
        state.initialized = true;
        state.full_pending = false;
        let mut paths = std::mem::take(&mut state.paths);
        if full {
            paths.extend(
                self.library
                    .db_songs
                    .iter()
                    .chain(self.playback.queue.iter())
                    .chain(self.playback.current_song.iter())
                    .map(|song| PathBuf::from(&song.file_path))
                    .filter(|path| path.is_absolute()),
            );
        }
        let previous = audio_index::snapshot();
        let cache = crate::utils::songs_cache_dir();
        let downloads = self.core.settings.storage.effective_download_dir();
        tracing::debug!(
            full,
            changed_paths = paths.len(),
            "Refreshing audio availability index"
        );
        Task::perform(
            async move {
                match tokio::task::spawn_blocking(move || {
                    Arc::new(if full {
                        AudioIndex::scan(cache, downloads, paths.into_iter().collect())
                    } else {
                        previous.refresh(&downloads, &paths)
                    })
                })
                .await
                {
                    Ok(index) => Some(index),
                    Err(error) => {
                        tracing::warn!(%error, "Audio index worker failed");
                        None
                    }
                }
            },
            Message::AudioIndexReady,
        )
    }
}

fn watch_paths(event: &rustle_media::watch::WatchEvent) -> Vec<PathBuf> {
    use rustle_media::watch::WatchEvent;
    match event {
        WatchEvent::FileCreated(path)
        | WatchEvent::FileModified(path)
        | WatchEvent::FileDeleted(path) => vec![path.clone()],
        WatchEvent::FileRenamed(old, new) => vec![old.clone(), new.clone()],
        WatchEvent::Error(error) => {
            tracing::warn!(%error, "Audio index watcher failed");
            vec![]
        }
    }
}

// External changes are coalesced. In-process cache publication also arrives
// here without depending on a particular playback/preload event subscriber.
pub(crate) fn subscription(downloads: PathBuf) -> iced::Subscription<Message> {
    iced::Subscription::run_with(downloads, |downloads| changes(downloads.as_path()))
}

fn changes(
    downloads: &std::path::Path,
) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = Message> + Send>> {
    let downloads = downloads.to_path_buf();
    Box::pin(async_stream::stream! {
        let mut published = audio_index::subscribe_changes();
        let (tx, mut rx) = rustle_media::watch::watch_channel();
        let mut watcher = match rustle_media::watch::FolderWatcher::new(tx) {
            Ok(watcher) => Some(watcher),
            Err(error) => { tracing::warn!(%error, "Audio index watcher unavailable"); None }
        };
        let roots = [crate::utils::songs_cache_dir(), downloads];
        let mut fallback = tokio::time::interval(std::time::Duration::from_secs(120));
        fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut pending = HashSet::new();
        let debounce = tokio::time::sleep(std::time::Duration::from_millis(250));
        tokio::pin!(debounce);
        loop {
            tokio::select! {
                _ = fallback.tick() => {
                    if let Some(watcher) = &mut watcher {
                        for root in &roots {
                            if let Err(error) = watcher.watch(root) { tracing::debug!(%error, "Audio directory is not yet watchable"); }
                        }
                    }
                    yield Message::AudioIndexRefresh;
                }
                event = rx.recv(), if watcher.is_some() => {
                    if let Some(event) = event {
                        if pending.is_empty() {
                            debounce.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(250));
                        }
                        for path in watch_paths(&event) {
                            // notify can use canonical Windows paths; preserve configured root spelling.
                            let path = roots.iter().find_map(|root| root.canonicalize().ok()
                                .and_then(|canonical| path.strip_prefix(canonical).ok().map(|rest| root.join(rest))))
                                .unwrap_or(path);
                            pending.insert(path);
                        }
                    }
                }
                event = published.recv() => match event {
                    Ok(path) => {
                        if pending.is_empty() {
                            debounce.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(250));
                        }
                        pending.insert(path);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { yield Message::AudioIndexRefresh; }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = &mut debounce, if !pending.is_empty() => {
                    yield Message::AudioFilesChanged(pending.drain().collect::<Vec<_>>().into());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_availability_does_not_change_artwork() {
        let before = AudioFile {
            path: "song.mp3".into(),
            size: 10,
            modified: None,
        };
        let after = AudioFile {
            size: 11,
            ..before.clone()
        };
        assert!(!artwork_changed(None, Some(&before)));
        assert!(!artwork_changed(Some(&before), None));
        assert!(!artwork_changed(Some(&before), Some(&before)));
        assert!(artwork_changed(Some(&before), Some(&after)));
        let moved = AudioFile {
            path: "download.mp3".into(),
            ..before.clone()
        };
        assert!(!artwork_changed(Some(&before), Some(&moved)));
    }
}
