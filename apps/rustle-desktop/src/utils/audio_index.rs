//! Filesystem snapshots built on a blocking worker; UI lookups never touch disk.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, RwLock},
    time::SystemTime,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioIndex {
    pub cached: HashMap<u64, AudioFile>,
    pub downloaded: HashMap<String, AudioFile>,
    pub local: HashMap<PathBuf, AudioFile>,
    pub cache_dir: PathBuf,
}

static INDEX: LazyLock<RwLock<Arc<AudioIndex>>> = LazyLock::new(|| RwLock::new(Arc::default()));

/// Cache publication is independent of a playback subscriber (including preload).
static CHANGES: LazyLock<tokio::sync::broadcast::Sender<PathBuf>> =
    LazyLock::new(|| tokio::sync::broadcast::channel(256).0);

pub fn notify_published(path: &Path) {
    let _ = CHANGES.send(path.to_owned());
}

pub fn subscribe_changes() -> tokio::sync::broadcast::Receiver<PathBuf> {
    CHANGES.subscribe()
}

#[derive(Default)]
pub struct AudioIndexState {
    pub running: bool,
    pub initialized: bool,
    pub full_pending: bool,
    pub paths: std::collections::HashSet<PathBuf>,
}

impl AudioIndexState {
    pub fn needs_refresh(&self) -> bool {
        !self.running && (!self.initialized || self.full_pending || !self.paths.is_empty())
    }
}

pub fn snapshot() -> Arc<AudioIndex> {
    INDEX.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn publish(index: Arc<AudioIndex>) {
    *INDEX.write().unwrap_or_else(|e| e.into_inner()) = index;
}

fn inspect(path: PathBuf) -> Option<AudioFile> {
    let path = normalize_path(&path);
    let metadata = path.metadata().ok()?;
    (metadata.is_file() && metadata.len() > 0).then(|| AudioFile {
        path,
        modified: metadata.modified().ok(),
        size: metadata.len(),
    })
}

/// notify canonicalizes Windows roots while file dialogs usually do not.
fn normalize_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(value) = path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return value.strip_prefix(r"UNC\").map_or_else(
            || PathBuf::from(value),
            |value| PathBuf::from(format!(r"\\{value}")),
        );
    }
    path.to_owned()
}

fn audio_files(dir: &Path) -> impl Iterator<Item = AudioFile> {
    audio_paths(dir).filter_map(inspect)
}

fn audio_paths(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| {
                super::AUDIO_EXTENSIONS
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
        })
}

fn cache_id(path: &Path) -> Option<u64> {
    path.file_stem()?.to_str()?.split_once('_')?.0.parse().ok()
}

fn download_key(path: &Path) -> Option<String> {
    path.file_stem()?.to_str().map(str::to_owned)
}

fn complete_cache(file: &AudioFile) -> bool {
    let Some(manifest) = rustle_storage::cache::read_audio_manifest(&file.path) else {
        return false;
    };
    manifest.song_id == cache_id(&file.path).unwrap_or_default()
        && manifest.size == file.size
        && file
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(&manifest.format))
        && file.path.file_stem().is_some_and(|stem| {
            stem == std::ffi::OsStr::new(&format!(
                "{}_{}",
                manifest.song_id,
                crate::api::quality_api_level(manifest.actual_quality)
            ))
        })
}

impl AudioIndex {
    /// Inspect changed resources only. Re-select another quality/extension if
    /// the previously preferred file was removed; unrelated files need no stat.
    pub fn refresh(&self, download_dir: &Path, paths: &std::collections::HashSet<PathBuf>) -> Self {
        let download_dir = normalize_path(download_dir);
        let download_dir = download_dir.as_path();
        let paths: std::collections::HashSet<_> =
            paths.iter().map(|path| normalize_path(path)).collect();
        let mut next = self.clone();
        let ids: std::collections::HashSet<_> = paths
            .iter()
            .filter(|p| p.parent() == Some(self.cache_dir.as_path()))
            .filter_map(|p| cache_id(p))
            .collect();
        let stems: std::collections::HashSet<_> = paths
            .iter()
            .filter(|p| p.parent() == Some(download_dir))
            .filter_map(|p| download_key(p))
            .collect();
        next.cached.retain(|id, _| !ids.contains(id));
        next.downloaded.retain(|stem, _| !stems.contains(stem));
        if !ids.is_empty() {
            for file in audio_paths(&self.cache_dir)
                .filter(|p| cache_id(p).is_some_and(|id| ids.contains(&id)))
                .filter_map(inspect)
                .filter(complete_cache)
            {
                let id = cache_id(&file.path).expect("filtered cache key");
                let entry = next.cached.entry(id).or_insert_with(|| file.clone());
                if (file.modified, &file.path) > (entry.modified, &entry.path) {
                    *entry = file;
                }
            }
        }
        if !stems.is_empty() {
            for file in audio_paths(download_dir)
                .filter(|p| download_key(p).is_some_and(|stem| stems.contains(&stem)))
                .filter_map(inspect)
            {
                let stem = download_key(&file.path).expect("filtered download key");
                let entry = next.downloaded.entry(stem).or_insert_with(|| file.clone());
                if (file.modified, &file.path) > (entry.modified, &entry.path) {
                    *entry = file;
                }
            }
        }
        for path in &paths {
            // Cache/download directories have their own identity maps.
            if self.local.contains_key(path)
                || (!path.starts_with(&self.cache_dir) && !path.starts_with(download_dir))
            {
                next.local.remove(path);
                if let Some(file) = inspect(path.clone()) {
                    next.local.insert(path.clone(), file);
                }
            }
        }
        next
    }

    pub fn scan(cache_dir: PathBuf, download_dir: PathBuf, paths: Vec<PathBuf>) -> Self {
        let cache_dir = normalize_path(&cache_dir);
        let download_dir = normalize_path(&download_dir);
        let mut index = Self {
            cache_dir,
            ..Self::default()
        };
        for file in audio_files(&index.cache_dir).filter(complete_cache) {
            let Some(id) = file
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split_once('_'))
                .and_then(|(id, _)| id.parse::<u64>().ok())
            else {
                continue;
            };
            let entry = index.cached.entry(id).or_insert_with(|| file.clone());
            if (file.modified, &file.path) > (entry.modified, &entry.path) {
                *entry = file;
            }
        }
        for file in audio_files(&download_dir) {
            if let Some(stem) = file.path.file_stem().and_then(|s| s.to_str()) {
                let entry = index
                    .downloaded
                    .entry(stem.to_owned())
                    .or_insert_with(|| file.clone());
                if (file.modified, &file.path) > (entry.modified, &entry.path) {
                    *entry = file;
                }
            }
        }
        for path in paths.into_iter().collect::<std::collections::HashSet<_>>() {
            if let Some(file) = inspect(path) {
                index.local.insert(file.path.clone(), file);
            }
        }
        index
    }

    pub fn locate(
        &self,
        source: &str,
        id: i64,
        artist: Option<&str>,
        title: Option<&str>,
    ) -> Option<&AudioFile> {
        if let Some(file) = self.local_file(source) {
            return Some(file);
        }
        let ncm_id = crate::image::ncm_song_id(id, source)?;
        if let (Some(artist), Some(title)) = (artist, title) {
            let stem = format!(
                "{} - {}",
                super::sanitize_filename(artist),
                super::sanitize_filename(title)
            );
            if let Some(file) = self.downloaded.get(&stem) {
                return Some(file);
            }
        }
        self.cached.get(&ncm_id)
    }

    pub fn local_file(&self, source: &str) -> Option<&AudioFile> {
        self.local.get(Path::new(source)).or_else(|| {
            source
                .starts_with(r"\\?\")
                .then(|| self.local.get(&normalize_path(Path::new(source))))
                .flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_playback_never_requests_a_scan_and_changes_wait_for_the_worker() {
        let mut state = AudioIndexState::default();
        assert!(state.needs_refresh());
        state.initialized = true;
        for _ in 0..1000 {
            assert!(!state.needs_refresh());
        }
        state.running = true;
        state.paths.insert("changed.mp3".into());
        assert!(!state.needs_refresh());
        state.running = false;
        assert!(state.needs_refresh());
    }

    #[tokio::test]
    async fn publication_is_observable_without_a_playback_subscriber() {
        let mut changes = subscribe_changes();
        let path = PathBuf::from("42_lossless.flac");
        notify_published(&path);
        assert_eq!(changes.recv().await.unwrap(), path);
    }

    #[test]
    fn incremental_refresh_only_inspects_affected_songs_and_reselects_extensions() {
        let root = crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-index-delta"));
        let cache = root.join("cache");
        let downloads = root.join("downloads");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        let first = downloads.join("Artist - First.mp3");
        let other = downloads.join("Artist - Other.mp3");
        std::fs::write(&first, b"old").unwrap();
        std::fs::write(&other, b"old").unwrap();
        let index = AudioIndex::scan(cache, downloads.clone(), vec![]);
        std::fs::write(&first, b"changed").unwrap();
        std::fs::write(&other, b"changed too").unwrap();
        let paths = [first.clone()].into_iter().collect();
        let refreshed = index.refresh(&downloads, &paths);
        assert_eq!(refreshed.downloaded["Artist - First"].size, 7);
        // No restat of unrelated files, and old readers retain their snapshot.
        assert_eq!(refreshed.downloaded["Artist - Other"].size, 3);
        assert_eq!(index.downloaded["Artist - First"].size, 3);
        let replacement = first.with_extension("flac");
        std::fs::write(&replacement, b"replacement").unwrap();
        std::fs::remove_file(&first).unwrap();
        let refreshed = refreshed.refresh(&downloads, &paths);
        assert_eq!(refreshed.downloaded["Artist - First"].path, replacement);
        std::fs::remove_file(replacement).unwrap();
        assert!(
            !refreshed
                .refresh(&downloads, &paths)
                .downloaded
                .contains_key("Artist - First")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn index_prefers_downloads_tracks_deletion_and_ignores_partial_files() {
        let root = crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-audio-index"));
        let cache = root.join("cache");
        let downloads = root.join("downloads");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        let cached = cache.join("42_lossless.FLAC");
        let downloaded = downloads.join("Artist - Title.mp3");
        std::fs::write(&cached, b"fLaC").unwrap();
        rustle_storage::cache::write_audio_manifest(
            &cached,
            42,
            crate::api::NcmQualityLevel::Lossless,
            4,
            "flac",
        )
        .unwrap();
        std::fs::write(&downloaded, b"ID3").unwrap();
        std::fs::write(cache.join("43_lossless.flac.tmp"), b"partial").unwrap();
        std::fs::write(cache.join("44_lossless.flac"), b"").unwrap();
        std::fs::write(cache.join("45_lossless.flac"), b"not published yet").unwrap();
        let index = AudioIndex::scan(cache.clone(), downloads.clone(), vec![]);
        assert_eq!(
            index
                .locate("ncm://42", 7, Some("Artist"), Some("Title"))
                .unwrap()
                .path,
            downloaded
        );
        assert_eq!(index.locate("", -42, None, None).unwrap().path, cached);
        assert!(!index.cached.contains_key(&43));
        assert!(!index.cached.contains_key(&44));
        assert!(!index.cached.contains_key(&45));
        std::fs::remove_file(&downloaded).unwrap();
        std::fs::remove_file(&cached).unwrap();
        // The old snapshot remains usable without filesystem access until a
        // worker atomically publishes the next snapshot.
        assert!(index.locate("", -42, None, None).is_some());
        let refreshed = AudioIndex::scan(cache, downloads, vec![]);
        assert!(
            refreshed
                .locate("", -42, Some("Artist"), Some("Title"))
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
