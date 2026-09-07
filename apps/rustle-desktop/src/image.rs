//! Unified image type taxonomy — pure data, zero UI dependencies.
//!
//! Every image category the app manages flows through a single resolution
//! pipeline defined here.  UI widgets (cover_image, avatar_image) and the
//! update handler (app::update::images) both depend on this module.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Image category
// ---------------------------------------------------------------------------

/// Every distinct image domain the app manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKind {
    /// NCM song / track cover art (identified by song's NCM id)
    SongCover,
    /// Local-library song cover art (identified by local database song id)
    LocalSongCover,
    /// NCM playlist cover (identified by playlist id)
    PlaylistCover,
    /// Local-library playlist cover (identified by local database playlist id)
    LocalPlaylistCover,
    /// Artist portrait / avatar (identified by artist id)
    ArtistCover,
    /// Album cover (identified by album id)
    AlbumCover,
    /// NCM video / MV cover (identified by video id)
    VideoCover,
    /// NCM podcast / DJ radio cover (identified by radio id)
    RadioCover,
    /// User avatar (identified by user id)
    UserAvatar,
    /// API-provided membership badge (identified by a tier-aware badge key)
    VipBadge,
}

impl ImageKind {
    /// Which cache directory stores images of this kind.
    pub fn cache_dir(&self) -> PathBuf {
        match self {
            Self::SongCover
            | Self::LocalSongCover
            | Self::PlaylistCover
            | Self::LocalPlaylistCover
            | Self::ArtistCover
            | Self::AlbumCover
            | Self::VideoCover
            | Self::RadioCover => crate::utils::covers_cache_dir(),
            Self::UserAvatar => crate::utils::avatars_cache_dir(),
            Self::VipBadge => crate::utils::vip_badges_cache_dir(),
        }
    }

    /// File stem used in the cache directory (without extension).
    pub fn file_stem(&self, id: u64) -> String {
        match self {
            Self::SongCover => format!("cover_{}", id),
            Self::LocalSongCover => format!("local_song_{}", id),
            Self::PlaylistCover => format!("playlist_{}", id),
            Self::LocalPlaylistCover => format!("local_playlist_{}", id),
            Self::ArtistCover => format!("artist_{}", id),
            Self::AlbumCover => format!("album_{}", id),
            Self::VideoCover => format!("video_{}", id),
            Self::RadioCover => format!("radio_{}", id),
            Self::UserAvatar => format!("avatar_{}", id),
            Self::VipBadge => format!("vip_{}", id),
        }
    }
}

/// Return the NCM resource ID carried by an application song identity.
///
/// Persisted NCM rows use a positive SQLite row ID together with an
/// `ncm://<id>` source, while queue/view models commonly use the negative NCM
/// ID directly. The source URI is therefore authoritative when it is present.
pub fn ncm_song_id(song_id: i64, file_path: &str) -> Option<u64> {
    if let Some(id) = file_path.strip_prefix("ncm://") {
        return id.parse::<u64>().ok().filter(|id| *id > 0);
    }

    song_id
        .checked_neg()
        .and_then(|id| u64::try_from(id).ok())
        .filter(|id| *id > 0)
}

/// Resolve a song-cover cache identity from both the application ID and its
/// source. This must be used whenever a complete `DbSong` is available.
pub fn song_cover_key_for_source(song_id: i64, file_path: &str) -> Option<(ImageKind, u64)> {
    if let Some(id) = ncm_song_id(song_id, file_path) {
        Some((ImageKind::SongCover, id))
    } else {
        u64::try_from(song_id)
            .ok()
            .map(|id| (ImageKind::LocalSongCover, id))
    }
}

/// Cache identity for a membership badge. User, semantic tier, and icon URL
/// are all part of the key so Black Vinyl VIP and SVIP cannot share imagery,
/// even when the API returns the same URL. FNV-1a keeps the key deterministic
/// across application restarts.
pub fn vip_badge_key(user_id: u64, tier: crate::api::VipTier, icon_url: &str) -> u64 {
    // Version 2 stores the original horizontal API image instead of the old
    // square CDN derivative. Keep the processing version in the identity so
    // an already-cached distorted badge cannot survive the behavior change.
    const CACHE_VERSION: u8 = 2;
    let mut hash = 0xcbf29ce484222325u64;
    for byte in user_id
        .to_le_bytes()
        .into_iter()
        .chain([tier.cache_discriminant(), CACHE_VERSION])
        .chain([0xff])
        .chain(icon_url.as_bytes().iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{ImageKind, song_cover_key_for_source, vip_badge_key};
    use crate::api::VipTier;

    #[test]
    fn persisted_ncm_song_uses_remote_cover_identity() {
        assert_eq!(
            song_cover_key_for_source(1476, "ncm://4934355"),
            Some((ImageKind::SongCover, 4_934_355))
        );
        assert_eq!(
            song_cover_key_for_source(1476, "/music/local.flac"),
            Some((ImageKind::LocalSongCover, 1476))
        );
    }

    #[test]
    fn vip_badge_key_includes_membership_tier() {
        assert_ne!(
            vip_badge_key(42, VipTier::BlackVinylVip, "https://vip/icon.png"),
            vip_badge_key(42, VipTier::Svip, "https://vip/icon.png")
        );
        assert_ne!(
            vip_badge_key(42, VipTier::None, "https://vip/icon.png"),
            vip_badge_key(42, VipTier::BlackVinylVip, "https://vip/icon.png")
        );
        assert_ne!(
            vip_badge_key(42, VipTier::Svip, "https://vip/svip-a.png"),
            vip_badge_key(42, VipTier::Svip, "https://vip/svip-b.png")
        );
    }
}

/// The result of a successful async download.
#[derive(Debug, Clone)]
pub struct ImageResult {
    pub kind: ImageKind,
    pub id: u64,
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Pure cache-resolver (no side effects)
// ---------------------------------------------------------------------------

/// Synchronous cache probe — returns `Some(path)` if a file for this
/// `(kind, id)` already exists on disk, `None` otherwise.
///
/// Pure function: no queue push, no state mutation, no download trigger.
pub fn resolve_cached(kind: ImageKind, id: u64) -> Option<PathBuf> {
    crate::utils::find_cached_image(&kind.cache_dir(), &kind.file_stem(id))
}

/// True when a string points at a remote HTTP(S) image source.
pub fn is_remote_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// True when a string field (`cover_path` / `cover_img_url`) holds a valid
/// local path rather than an http(s) URL.
pub fn is_valid_local_path(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if is_remote_url(s) {
        return false;
    }
    std::path::Path::new(s).exists()
}
