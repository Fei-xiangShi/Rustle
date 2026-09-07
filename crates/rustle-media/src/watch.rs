//! Folder watching for automatic music library updates
//!
//! Uses the `notify` crate to monitor directories for changes
//! and automatically import new files or remove deleted ones.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::error::{MediaError, MediaResult};
use crate::scan::is_audio_file;

/// Events emitted by the folder watcher
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A new audio file was created
    FileCreated(PathBuf),
    /// An audio file was modified
    FileModified(PathBuf),
    /// An audio file was deleted
    FileDeleted(PathBuf),
    /// A file was renamed (old path, new path)
    FileRenamed(PathBuf, PathBuf),
    /// Watcher error
    Error(String),
}

/// Folder watcher that monitors directories for changes
pub struct FolderWatcher {
    watcher: RecommendedWatcher,
    watched_paths: HashSet<PathBuf>,
}

impl FolderWatcher {
    /// Create a new folder watcher
    ///
    /// Events will be sent to the provided channel
    pub fn new(event_tx: mpsc::UnboundedSender<WatchEvent>) -> MediaResult<Self> {
        let event_tx_clone = event_tx.clone();

        let watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    if let Some(watch_event) = process_notify_event(event) {
                        let _ = event_tx_clone.send(watch_event);
                    }
                }
                Err(e) => {
                    let _ = event_tx_clone.send(WatchEvent::Error(e.to_string()));
                }
            })
            .map_err(|source| MediaError::Watch {
                operation: "create watcher",
                source,
            })?;

        Ok(Self {
            watcher,
            watched_paths: HashSet::new(),
        })
    }

    /// Start watching a directory
    pub fn watch(&mut self, path: &Path) -> MediaResult<()> {
        let path = path
            .canonicalize()
            .map_err(|error| MediaError::io("canonicalize watch path", error))?;

        if self.watched_paths.contains(&path) {
            return Ok(()); // Already watching
        }

        self.watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|source| MediaError::Watch {
                operation: "watch directory",
                source,
            })?;

        self.watched_paths.insert(path);
        Ok(())
    }

    /// Stop watching a directory
    pub fn unwatch(&mut self, path: &Path) -> MediaResult<()> {
        let path = path
            .canonicalize()
            .ok()
            .or_else(|| {
                self.watched_paths
                    .iter()
                    .find(|candidate| candidate.as_path() == path)
                    .cloned()
            })
            .unwrap_or_else(|| path.to_path_buf());

        if !self.watched_paths.contains(&path) {
            return Ok(()); // Not watching
        }

        self.watcher
            .unwatch(&path)
            .map_err(|source| MediaError::Watch {
                operation: "unwatch directory",
                source,
            })?;

        self.watched_paths.remove(&path);
        Ok(())
    }

    /// Get list of currently watched paths
    pub fn watched_paths(&self) -> Vec<PathBuf> {
        self.watched_paths.iter().cloned().collect()
    }
}

/// Process a notify event and convert to WatchEvent if relevant
fn process_notify_event(event: Event) -> Option<WatchEvent> {
    use notify::event::ModifyKind;

    // Filter to only audio files (but keep all paths for rename detection)
    let all_paths = event.paths.clone();
    let audio_paths: Vec<PathBuf> = event
        .paths
        .into_iter()
        .filter(|p| is_audio_file(p) || p.is_dir())
        .collect();

    match event.kind {
        EventKind::Create(_) => {
            if audio_paths.is_empty() {
                return None;
            }
            // Only emit for files, not directories
            let path = audio_paths.into_iter().find(|p| p.is_file())?;
            Some(WatchEvent::FileCreated(path))
        }
        EventKind::Modify(ModifyKind::Name(_)) => {
            // Rename event - notify sends two paths: old and new
            // On some platforms, this comes as two separate events
            // On others, both paths are in the same event
            if all_paths.len() >= 2 {
                let old_path = all_paths[0].clone();
                let new_path = all_paths[1].clone();

                // Check if either path is an audio file
                if is_audio_file(&old_path) || is_audio_file(&new_path) {
                    return Some(WatchEvent::FileRenamed(old_path, new_path));
                }
            } else if all_paths.len() == 1 {
                // Single path rename - could be either "from" or "to"
                // We'll treat it as a potential rename and let the handler deal with it
                let path = all_paths[0].clone();
                if is_audio_file(&path) {
                    // If the file exists, it's the "to" path (file was renamed to this)
                    if path.exists() {
                        return Some(WatchEvent::FileCreated(path));
                    } else {
                        // File doesn't exist, it's the "from" path (file was renamed away)
                        return Some(WatchEvent::FileDeleted(path));
                    }
                }
            }
            None
        }
        EventKind::Modify(_) => {
            if audio_paths.is_empty() {
                return None;
            }
            let path = audio_paths.into_iter().find(|p| p.is_file())?;
            Some(WatchEvent::FileModified(path))
        }
        EventKind::Remove(_) => {
            if audio_paths.is_empty() {
                return None;
            }
            // For removals, we can't check if it's a file anymore
            let path = audio_paths.into_iter().next()?;
            Some(WatchEvent::FileDeleted(path))
        }
        EventKind::Access(_) => None, // Ignore access events
        EventKind::Other => None,
        EventKind::Any => None,
    }
}

/// Watch event receiver type
pub type WatchEventReceiver = mpsc::UnboundedReceiver<WatchEvent>;

/// Create a new watch event channel
pub fn watch_channel() -> (mpsc::UnboundedSender<WatchEvent>, WatchEventReceiver) {
    mpsc::unbounded_channel()
}

/// Spawn a background task that processes watch events with debouncing
pub async fn spawn_debounced_processor(
    mut rx: WatchEventReceiver,
    debounce_ms: u64,
    output_tx: mpsc::UnboundedSender<WatchEvent>,
) {
    use std::collections::HashMap;
    use tokio::time::{Instant, sleep};

    let debounce = Duration::from_millis(debounce_ms);
    let mut pending: HashMap<PathBuf, (WatchEvent, Instant)> = HashMap::new();

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                let path = match &event {
                    WatchEvent::FileCreated(p) |
                    WatchEvent::FileModified(p) |
                    WatchEvent::FileDeleted(p) => p.clone(),
                    WatchEvent::FileRenamed(_, new) => new.clone(),
                    WatchEvent::Error(_) => {
                        // Forward errors immediately
                        let _ = output_tx.send(event);
                        continue;
                    }
                };
                pending.insert(path, (event, Instant::now()));
            }
            _ = sleep(Duration::from_millis(100)) => {
                // Check for events that have been debounced long enough
                let now = Instant::now();
                let ready: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, (_, time))| now.duration_since(*time) >= debounce)
                    .map(|(path, _)| path.clone())
                    .collect();

                for path in ready {
                    if let Some((event, _)) = pending.remove(&path) {
                        let _ = output_tx.send(event);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_audio_file_filter() {
        assert!(is_audio_file(&PathBuf::from("song.mp3")));
        assert!(is_audio_file(&PathBuf::from("song.FLAC")));
        assert!(!is_audio_file(&PathBuf::from("image.jpg")));
        assert!(!is_audio_file(&PathBuf::from("document.txt")));
    }
}
