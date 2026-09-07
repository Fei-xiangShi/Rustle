//! Canonical persistent cache directory ownership.

use std::path::PathBuf;

/// Get the base cache directory for Rustle.
#[must_use]
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("life", "fxs", "rustle")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("~/.cache/rustle"))
}

#[must_use]
pub fn covers_cache_dir() -> PathBuf {
    cache_dir().join("covers")
}

#[must_use]
pub fn songs_cache_dir() -> PathBuf {
    cache_dir().join("songs")
}

#[must_use]
pub fn banners_cache_dir() -> PathBuf {
    cache_dir().join("banners")
}

#[must_use]
pub fn avatars_cache_dir() -> PathBuf {
    cache_dir().join("avatars")
}

#[must_use]
pub fn vip_badges_cache_dir() -> PathBuf {
    cache_dir().join("vip_badges")
}

#[must_use]
pub fn automix_cache_dir() -> PathBuf {
    cache_dir().join("automix")
}

#[must_use]
pub fn lyrics_cache_dir() -> PathBuf {
    cache_dir().join("lyrics")
}

#[must_use]
pub fn playlists_cache_dir() -> PathBuf {
    cache_dir().join("playlists")
}
