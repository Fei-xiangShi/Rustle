//! Local music library import module
//!
//! Handles:
//! - Recursive folder scanning
//! - Metadata extraction with encoding fallback
//! - Cover art caching
//! - File deduplication
//! - .m3u/.pls playlist parsing
//! - Folder watching for auto-import
//! - Smart filename parsing

mod cover;
mod encoding;
mod metadata;
mod progress;
mod scanner;
mod watcher;

pub use cover::{CoverCache, default_cache_dir};
pub use metadata::{
    AudioMetadata, MetadataEdits, extract_metadata, extract_track_gain, resolve_track_gain,
    save_metadata,
};
pub use progress::{ScanHandle, ScanProgress, ScanState, SkipReason, progress_channel};
pub use scanner::{ScanConfig, ScanResult, scan_and_import, scan_audio_file};
pub use watcher::{FolderWatcher, WatchEvent, spawn_debounced_processor, watch_channel};

use std::path::Path;

/// Supported audio file extensions (re-exported from utils)
pub use crate::utils::AUDIO_EXTENSIONS;

/// Check if a file extension is a supported audio format
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}
