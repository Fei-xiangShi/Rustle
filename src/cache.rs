//! Cache management module
//!
//! Handles cache size calculation, cleanup, and automatic eviction.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use crate::application::ports::cache::{AudioCacheStore, CachePublisher};
use crate::domain::audio::QualityLevel as NcmQualityLevel;
use crate::utils::{
    automix_cache_dir, avatars_cache_dir, banners_cache_dir, cache_dir, covers_cache_dir,
    lyrics_cache_dir, songs_cache_dir, vip_badges_cache_dir,
};

const AUDIO_MANIFEST_VERSION: u8 = 1;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub use crate::application::ports::cache::PublishOutcome as PublishResult;

#[derive(Debug, Clone, Copy, Default)]
pub struct FileCacheStore;

pub fn audio_cache_store() -> std::sync::Arc<dyn AudioCacheStore> {
    std::sync::Arc::new(FileCacheStore)
}

pub fn cache_publisher() -> std::sync::Arc<dyn CachePublisher> {
    std::sync::Arc::new(FileCacheStore)
}

impl CachePublisher for FileCacheStore {
    fn unique_temp_path(&self, final_path: &Path) -> PathBuf {
        unique_temp_path(final_path)
    }

    fn cleanup_temp_file(&self, path: &Path) {
        cleanup_temp_file(path);
    }

    fn publish_or_reuse(
        &self,
        temp_path: &Path,
        final_path: &Path,
        expected_size: Option<u64>,
    ) -> std::io::Result<PublishResult> {
        publish_or_reuse(temp_path, final_path, expected_size)
    }
}

impl AudioCacheStore for FileCacheStore {
    fn write_audio_manifest(
        &self,
        path: &Path,
        song_id: u64,
        actual_quality: NcmQualityLevel,
        size: u64,
        format: &str,
    ) -> std::io::Result<()> {
        write_audio_manifest(path, song_id, actual_quality, size, format)
    }

    fn remove_audio_cache(&self, path: &Path) {
        remove_audio_cache(path);
    }
}

/// Build a process-unique temporary path next to the eventual destination.
/// The caller must close the file before publishing or cleaning it up.
pub fn unique_temp_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("cache");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{name}.{}.{}.{}.tmp",
        std::process::id(),
        nonce,
        counter
    ))
}

pub fn cleanup_temp_file(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(target_os = "windows")]
fn replace_closed_file(temp_path: &Path, final_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temp_wide = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let final_wide = final_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both vectors are NUL-terminated and remain allocated for the
    // synchronous call. The caller guarantees both files are closed; failure
    // is returned so the publishing layer can remove the temporary file.
    let moved = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            final_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_closed_file(temp_path: &Path, final_path: &Path) -> std::io::Result<()> {
    fs::rename(temp_path, final_path)
}

/// Atomically replace the destination with a closed temporary file.
pub fn publish_replace(temp_path: &Path, final_path: &Path) -> std::io::Result<()> {
    replace_closed_file(temp_path, final_path).inspect_err(|_error| {
        cleanup_temp_file(temp_path);
    })
}

pub fn remove_audio_cache(path: &Path) {
    cleanup_temp_file(path);
    cleanup_temp_file(&audio_manifest_path(path));
}

/// Publish a closed temporary file, reusing a compatible existing destination
/// when possible. Unix rename is atomic; on Windows an existing destination is
/// handled as an explicit conflict because `rename` does not replace it.
pub fn publish_or_reuse(
    temp_path: &Path,
    final_path: &Path,
    expected_size: Option<u64>,
) -> std::io::Result<PublishResult> {
    let temp_size = fs::metadata(temp_path)?.len();
    if let Some(expected_size) = expected_size
        && temp_size != expected_size
    {
        cleanup_temp_file(temp_path);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("temporary cache size {temp_size} does not match expected {expected_size}"),
        ));
    }
    let compatible_size = expected_size.unwrap_or(temp_size);
    let reusable = || {
        let metadata = fs::metadata(final_path).ok()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return None;
        }
        Some(metadata.len() == compatible_size)
    };

    if reusable().unwrap_or(false) {
        cleanup_temp_file(temp_path);
        return Ok(PublishResult::Reused);
    }

    match fs::rename(temp_path, final_path) {
        Ok(()) => Ok(PublishResult::Published),
        Err(first_error) if final_path.exists() => {
            if reusable().unwrap_or(false) {
                cleanup_temp_file(temp_path);
                return Ok(PublishResult::Reused);
            }
            replace_closed_file(temp_path, final_path).map_err(|second_error| {
                cleanup_temp_file(temp_path);
                std::io::Error::new(
                    second_error.kind(),
                    format!(
                        "failed to replace cache destination after conflict: {second_error}; first rename error: {first_error}"
                    ),
                )
            })?;
            Ok(PublishResult::Published)
        }
        Err(error) => {
            cleanup_temp_file(temp_path);
            Err(error)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AudioCacheManifest {
    pub version: u8,
    pub song_id: u64,
    pub actual_quality: NcmQualityLevel,
    pub size: u64,
    pub format: String,
}

pub fn audio_manifest_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}.manifest.json"))
}

pub fn read_audio_manifest(path: &Path) -> Option<AudioCacheManifest> {
    let bytes = fs::read(audio_manifest_path(path)).ok()?;
    let manifest: AudioCacheManifest = serde_json::from_slice(&bytes).ok()?;
    (manifest.version == AUDIO_MANIFEST_VERSION).then_some(manifest)
}

pub fn is_audio_cache_complete(
    path: &Path,
    song_id: u64,
    actual_quality: NcmQualityLevel,
    expected_size: Option<u64>,
) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return false;
    }
    if let Some(size) = expected_size {
        return metadata.len() == size;
    }
    read_audio_manifest(path).is_some_and(|manifest| {
        let cached_format = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        manifest.song_id == song_id
            && manifest.actual_quality == actual_quality
            && manifest.size == metadata.len()
            && manifest.format.eq_ignore_ascii_case(cached_format)
    })
}

pub fn write_audio_manifest(
    path: &Path,
    song_id: u64,
    actual_quality: NcmQualityLevel,
    size: u64,
    format: &str,
) -> std::io::Result<()> {
    let manifest = AudioCacheManifest {
        version: AUDIO_MANIFEST_VERSION,
        song_id,
        actual_quality,
        size,
        format: format.to_string(),
    };
    let bytes = serde_json::to_vec(&manifest)
        .map_err(|error| std::io::Error::other(format!("serialize cache manifest: {error}")))?;
    let manifest_path = audio_manifest_path(path);
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = unique_temp_path(&manifest_path);
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        cleanup_temp_file(&temp_path);
        return Err(error);
    }
    publish_replace(&temp_path, &manifest_path)
}

/// Directory for serialized remote playlist snapshots.
pub fn playlists_cache_dir() -> PathBuf {
    cache_dir().join("playlists")
}

/// Load a cached NCM playlist snapshot.
pub async fn load_ncm_playlist_cache(playlist_id: u64) -> Option<crate::api::PlaylistDetail> {
    let path = playlists_cache_dir().join(format!("{playlist_id}.json"));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!(playlist_id, ?error, "NCM playlist cache miss");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(detail) => {
            tracing::info!(playlist_id, bytes = bytes.len(), "NCM playlist cache hit");
            Some(detail)
        }
        Err(error) => {
            tracing::warn!(playlist_id, ?error, "NCM playlist cache is invalid");
            None
        }
    }
}

/// Save a complete NCM playlist snapshot for fast subsequent entry.
pub async fn save_ncm_playlist_cache(detail: &crate::api::PlaylistDetail) {
    let dir = playlists_cache_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        tracing::warn!(
            playlist_id = detail.id,
            "Failed to create NCM playlist cache directory"
        );
        return;
    }
    let path = dir.join(format!("{}.json", detail.id));
    let bytes = match serde_json::to_vec(detail) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                playlist_id = detail.id,
                ?error,
                "Failed to serialize NCM playlist cache"
            );
            return;
        }
    };
    let tmp = unique_temp_path(&path);
    if let Err(error) = tokio::fs::write(&tmp, &bytes).await {
        tracing::warn!(
            playlist_id = detail.id,
            ?error,
            "Failed to write NCM playlist cache"
        );
        cleanup_temp_file(&tmp);
        return;
    }

    if let Err(error) = crate::cache::publish_replace(&tmp, &path) {
        tracing::warn!(
            playlist_id = detail.id,
            ?error,
            "Failed to replace NCM playlist cache"
        );
        return;
    }
    tracing::info!(
        playlist_id = detail.id,
        tracks = detail.tracks.len(),
        "NCM playlist cache saved"
    );
}

/// Information about a cached file
#[derive(Debug)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total size in bytes
    pub total_bytes: u64,
    /// Number of files
    pub file_count: usize,
    /// Size by category
    pub covers_bytes: u64,
    pub songs_bytes: u64,
    pub banners_bytes: u64,
    pub avatars_bytes: u64,
    pub vip_badges_bytes: u64,
    pub lyrics_bytes: u64,
    pub automix_bytes: u64,
    pub playlists_bytes: u64,
    pub root_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
enum CacheCategory {
    Covers,
    Songs,
    Banners,
    Avatars,
    VipBadges,
    Lyrics,
    Automix,
    Playlists,
}

const CACHE_CATEGORIES: &[CacheCategory] = &[
    CacheCategory::Covers,
    CacheCategory::Songs,
    CacheCategory::Banners,
    CacheCategory::Avatars,
    CacheCategory::VipBadges,
    CacheCategory::Lyrics,
    CacheCategory::Automix,
    CacheCategory::Playlists,
];

impl CacheCategory {
    fn dir(self) -> PathBuf {
        match self {
            Self::Covers => covers_cache_dir(),
            Self::Songs => songs_cache_dir(),
            Self::Banners => banners_cache_dir(),
            Self::Avatars => avatars_cache_dir(),
            Self::VipBadges => vip_badges_cache_dir(),
            Self::Lyrics => lyrics_cache_dir(),
            Self::Automix => automix_cache_dir(),
            Self::Playlists => playlists_cache_dir(),
        }
    }
}

/// Get all cache directories from the single category registry.
fn cache_directories() -> impl Iterator<Item = PathBuf> {
    CACHE_CATEGORIES.iter().copied().map(CacheCategory::dir)
}

fn is_cache_root_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    path.extension().is_some_and(|ext| ext == "tmp")
        || (name.starts_with("qrimage_") && name.ends_with(".png"))
}

fn collect_root_entries() -> Vec<CacheEntry> {
    let root = cache_dir();
    if !root.exists() {
        return Vec::new();
    }
    let Ok(read_dir) = fs::read_dir(root) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || !is_cache_root_file(&path) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some(CacheEntry {
                path,
                size: metadata.len(),
                modified: metadata.modified().unwrap_or(UNIX_EPOCH),
            })
        })
        .collect()
}

fn collect_all_entries() -> Vec<CacheEntry> {
    let mut entries = Vec::new();
    for category in CACHE_CATEGORIES {
        let dir = category.dir();
        entries.extend(collect_entries(&dir));
    }
    entries.extend(collect_root_entries());
    entries
}

/// Collect all cache entries from a directory
fn collect_entries(dir: &PathBuf) -> Vec<CacheEntry> {
    let mut entries = Vec::new();

    if !dir.exists() {
        return entries;
    }

    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!("Failed to read cache directory {:?}: {}", dir, e);
            return entries;
        }
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        entries.push(CacheEntry {
            path,
            size: metadata.len(),
            modified,
        });
    }

    entries
}

/// Calculate cache statistics
pub fn calculate_cache_stats() -> CacheStats {
    let mut stats = CacheStats::default();

    for category in CACHE_CATEGORIES {
        let entries = collect_entries(&category.dir());
        stats.file_count += entries.len();
        let bytes = entries.iter().map(|entry| entry.size).sum::<u64>();
        match category {
            CacheCategory::Covers => stats.covers_bytes = bytes,
            CacheCategory::Songs => stats.songs_bytes = bytes,
            CacheCategory::Banners => stats.banners_bytes = bytes,
            CacheCategory::Avatars => stats.avatars_bytes = bytes,
            CacheCategory::VipBadges => stats.vip_badges_bytes = bytes,
            CacheCategory::Lyrics => stats.lyrics_bytes = bytes,
            CacheCategory::Automix => stats.automix_bytes = bytes,
            CacheCategory::Playlists => stats.playlists_bytes = bytes,
        }
    }

    for entry in collect_root_entries() {
        stats.root_bytes += entry.size;
        stats.file_count += 1;
    }

    stats.total_bytes = stats.covers_bytes
        + stats.songs_bytes
        + stats.banners_bytes
        + stats.avatars_bytes
        + stats.vip_badges_bytes
        + stats.lyrics_bytes
        + stats.automix_bytes
        + stats.playlists_bytes;
    stats.total_bytes += stats.root_bytes;

    stats
}

/// Clear all cache
pub fn clear_all_cache() -> ClearResult {
    let mut result = ClearResult::default();

    for dir in cache_directories() {
        if !dir.exists() {
            continue;
        }

        let entries = collect_entries(&dir);
        for entry in entries {
            match fs::remove_file(&entry.path) {
                Ok(_) => {
                    result.files_deleted += 1;
                    result.bytes_freed += entry.size;
                }
                Err(e) => {
                    warn!("Failed to delete cache file {:?}: {}", entry.path, e);
                    result.errors += 1;
                }
            }
        }
    }
    for entry in collect_root_entries() {
        match fs::remove_file(&entry.path) {
            Ok(_) => {
                result.files_deleted += 1;
                result.bytes_freed += entry.size;
            }
            Err(e) => {
                warn!("Failed to delete cache root file {:?}: {}", entry.path, e);
                result.errors += 1;
            }
        }
    }

    info!(
        "Cache cleared: {} files deleted, {} MB freed, {} errors",
        result.files_deleted,
        result.bytes_freed / (1024 * 1024),
        result.errors
    );

    result
}

/// Enforce cache size limit by deleting oldest files
///
/// Returns the number of bytes freed
pub fn enforce_cache_limit(max_cache_mb: u64) -> ClearResult {
    let max_bytes = max_cache_mb * 1024 * 1024;
    let mut result = ClearResult::default();

    // Collect all cache entries
    let mut all_entries = collect_all_entries();

    // Calculate current total size
    let current_size: u64 = all_entries.iter().map(|e| e.size).sum();

    if current_size <= max_bytes {
        info!(
            "Cache size {} MB is within limit {} MB",
            current_size / (1024 * 1024),
            max_cache_mb
        );
        return result;
    }

    // Sort by modification time (oldest first)
    all_entries.sort_by_key(|a| a.modified);

    let mut freed: u64 = 0;
    let target_free = current_size - max_bytes;

    // Delete oldest files until we're under the limit
    for entry in all_entries {
        if freed >= target_free {
            break;
        }

        match fs::remove_file(&entry.path) {
            Ok(_) => {
                freed += entry.size;
                result.files_deleted += 1;
                result.bytes_freed += entry.size;
            }
            Err(e) => {
                warn!("Failed to delete cache file {:?}: {}", entry.path, e);
                result.errors += 1;
            }
        }
    }

    info!(
        "Cache cleanup: {} files deleted, {} MB freed (target was {} MB)",
        result.files_deleted,
        result.bytes_freed / (1024 * 1024),
        target_free / (1024 * 1024)
    );

    result
}

/// Result of a cache clear operation
#[derive(Debug, Clone, Default)]
pub struct ClearResult {
    pub files_deleted: usize,
    pub bytes_freed: u64,
    pub errors: usize,
}

impl ClearResult {
    /// Get bytes freed in megabytes
    pub fn mb_freed(&self) -> u64 {
        self.bytes_freed / (1024 * 1024)
    }
}

/// Clean up orphan .tmp files from incomplete downloads
///
/// This should be called at application startup to remove any temp files
/// left behind from interrupted downloads.
pub fn cleanup_temp_files() -> ClearResult {
    let mut result = ClearResult::default();

    // Clean cache directories
    for dir in cache_directories() {
        cleanup_temp_files_in_dir(&dir, &mut result);
    }
    cleanup_temp_files_in_dir(&cache_dir(), &mut result);

    // Also clean download directory
    let default_dl = {
        let default_settings = crate::features::Settings::default();
        default_settings.storage.effective_download_dir()
    };
    cleanup_temp_files_in_dir(&default_dl, &mut result);

    if result.files_deleted > 0 {
        info!(
            "Temp file cleanup: {} files deleted, {} bytes freed",
            result.files_deleted, result.bytes_freed
        );
    }

    result
}

fn cleanup_temp_files_in_dir(dir: &std::path::PathBuf, result: &mut ClearResult) {
    if !dir.exists() {
        return;
    }

    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!("Failed to read directory {:?}: {}", dir, e);
            return;
        }
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check if it's a .tmp file
        let is_tmp = path.extension().map(|e| e == "tmp").unwrap_or(false);
        let is_qr = dir == &cache_dir()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("qrimage_") && name.ends_with(".png"));
        if is_tmp || is_qr {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            match fs::remove_file(&path) {
                Ok(_) => {
                    info!("Cleaned up orphan temp file: {:?} ({} bytes)", path, size);
                    result.files_deleted += 1;
                    result.bytes_freed += size;
                }
                Err(e) => {
                    warn!("Failed to delete temp file {:?}: {}", path, e);
                    result.errors += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rustle-cache-{label}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn unique_temp_paths_are_distinct_and_scoped_to_destination() {
        let root = test_root("temp");
        let destination = root.join("songs").join("42_standard.mp3");
        let first = unique_temp_path(&destination);
        let second = unique_temp_path(&destination);
        assert_ne!(first, second);
        assert_eq!(first.parent(), destination.parent());
        assert_eq!(
            first.extension().and_then(|value| value.to_str()),
            Some("tmp")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_reuses_matching_destination_and_replaces_conflicting_one() {
        let root = test_root("publish");
        let destination = root.join("audio.mp3");
        fs::write(&destination, b"old").unwrap();

        let temp = unique_temp_path(&destination);
        fs::write(&temp, b"new").unwrap();
        assert_eq!(
            publish_or_reuse(&temp, &destination, Some(3)).unwrap(),
            PublishResult::Reused
        );
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert!(!temp.exists());

        let temp = unique_temp_path(&destination);
        fs::write(&temp, b"newer").unwrap();
        assert_eq!(
            publish_or_reuse(&temp, &destination, Some(5)).unwrap(),
            PublishResult::Published
        );
        assert_eq!(fs::read(&destination).unwrap(), b"newer");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_without_expected_size_compares_the_temporary_file_size() {
        let root = test_root("publish-derived-size");
        let destination = root.join("image.bin");
        fs::write(&destination, b"old").unwrap();

        let temp = unique_temp_path(&destination);
        fs::write(&temp, b"newer").unwrap();
        assert_eq!(
            publish_or_reuse(&temp, &destination, None).unwrap(),
            PublishResult::Published
        );
        assert_eq!(fs::read(&destination).unwrap(), b"newer");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_rejects_a_temporary_file_with_the_wrong_size() {
        let root = test_root("publish-size-mismatch");
        let destination = root.join("audio.bin");
        let temp = unique_temp_path(&destination);
        fs::write(&temp, b"short").unwrap();

        assert_eq!(
            publish_or_reuse(&temp, &destination, Some(10))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        assert!(!temp.exists());
        assert!(!destination.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_replace_updates_same_sized_mutable_cache() {
        let root = test_root("replace");
        let destination = root.join("mutable.json");
        fs::write(&destination, b"old").unwrap();
        let temp = unique_temp_path(&destination);
        fs::write(&temp, b"new").unwrap();

        publish_replace(&temp, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!temp.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audio_manifest_is_the_fallback_when_api_size_is_unknown() {
        let root = test_root("manifest");
        let path = root.join("7_lossless.flac");
        fs::write(&path, b"audio").unwrap();
        assert!(!is_audio_cache_complete(
            &path,
            7,
            NcmQualityLevel::Lossless,
            None
        ));
        write_audio_manifest(&path, 7, NcmQualityLevel::Lossless, 5, "flac").unwrap();
        assert!(is_audio_cache_complete(
            &path,
            7,
            NcmQualityLevel::Lossless,
            None
        ));
        assert!(!is_audio_cache_complete(
            &path,
            7,
            NcmQualityLevel::Lossless,
            Some(4)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
