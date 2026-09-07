//! Compatibility facade for media-owned cover caching.

#[allow(unused_imports)]
pub use rustle_media::cover::{CoverCache, THUMBNAIL_SIZE};

pub fn default_cache_dir() -> std::path::PathBuf {
    crate::utils::covers_cache_dir()
}
