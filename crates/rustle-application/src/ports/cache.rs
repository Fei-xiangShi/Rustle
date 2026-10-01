//! Cache publication contracts consumed by audio policy.

use std::io;
use std::path::{Path, PathBuf};

use rustle_domain::audio::QualityLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    Reused,
}

/// Atomic file-publication operations shared by cache-producing adapters.
pub trait CachePublisher: Send + Sync {
    fn unique_temp_path(&self, final_path: &Path) -> PathBuf;
    fn cleanup_temp_file(&self, path: &Path);
    fn publish_or_reuse(
        &self,
        temp_path: &Path,
        final_path: &Path,
        expected_size: Option<u64>,
    ) -> io::Result<PublishOutcome>;
}

/// Audio-cache operations needed by strict Range streaming.
pub trait AudioCacheStore: CachePublisher {
    /// Optionally produce a tagged copy. The original bytes remain the backing
    /// for active streaming readers, whose offsets must never change.
    /// On failure, clean up any partial copy and leave the original untouched:
    /// streaming retains it for playback without publishing a successful cache.
    fn prepare_audio_cache(
        &self,
        _path: PathBuf,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Option<PathBuf>>> + Send + '_>>
    {
        Box::pin(async { Ok(None) })
    }

    fn write_audio_manifest(
        &self,
        path: &Path,
        song_id: u64,
        actual_quality: QualityLevel,
        size: u64,
        format: &str,
    ) -> io::Result<()>;

    fn remove_audio_cache(&self, path: &Path);
}
