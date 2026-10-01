//! Unified image type taxonomy — pure data, zero UI dependencies.
//!
//! Every image category the app manages flows through a single resolution
//! pipeline defined here.  UI widgets (cover_image, avatar_image) and the
//! update handler (app::update::images) both depend on this module.

pub mod artwork;

use std::path::PathBuf;

/// Cache processing policy version. Bump this when derivative encoding or
/// sizing semantics change so old files cannot masquerade as current output.
pub const IMAGE_PROCESSING_VERSION: u8 = 2;

/// Stable display roles for image derivatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageVariant {
    /// Compact cards, rows, sidebars, dialogs, and player chrome.
    #[default]
    Thumbnail,
    /// Collection and identity detail headers.
    Detail,
    /// Full-screen artwork.
    Hero,
}

impl ImageVariant {
    pub const fn edge(self) -> u16 {
        match self {
            Self::Thumbnail => 300,
            Self::Detail => 600,
            Self::Hero => 1024,
        }
    }

    const fn cache_tag(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumb",
            Self::Detail => "detail",
            Self::Hero => "hero",
        }
    }
}

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

    /// Requested CDN derivative for a display role. Membership badges keep
    /// their original non-square composition.
    pub const fn requested_resize(self, variant: ImageVariant) -> Option<(u16, u16)> {
        if matches!(self, Self::VipBadge) {
            None
        } else {
            let edge = variant.edge();
            Some((edge, edge))
        }
    }

    /// Whether a decoded derivative is large enough for the requested role.
    /// Square artwork must meet the edge on both axes; video covers preserve
    /// their source aspect ratio, so their long edge carries the role size.
    #[cfg(test)]
    pub const fn cached_dimensions_satisfy(
        self,
        variant: ImageVariant,
        width: u32,
        height: u32,
    ) -> bool {
        if width == 0 || height == 0 {
            return false;
        }

        let edge = variant.edge() as u32;
        match self {
            Self::VipBadge => true,
            Self::VideoCover => width >= edge || height >= edge,
            _ => width >= edge && height >= edge,
        }
    }
}

/// Deterministic, non-reversible source identity used in cache filenames.
/// FNV-1a is sufficient here because this is cache invalidation, not a
/// security boundary.
pub fn source_url_digest(url: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// File stem for a current remote derivative. Variant, source identity, and
/// processing policy are all explicit so independent display roles coexist.
pub fn remote_file_stem(
    kind: ImageKind,
    id: u64,
    variant: ImageVariant,
    source_url: &str,
) -> String {
    format!(
        "{}__{}_v{}_{:016x}",
        kind.file_stem(id),
        variant.cache_tag(),
        IMAGE_PROCESSING_VERSION,
        source_url_digest(source_url)
    )
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

/// Resolve a current remote derivative for the exact role and source.
///
/// The requested edge is a CDN target, not a promise: user-uploaded artwork
/// can have a smaller natural maximum (for example 320 px even when Detail
/// requests 600 px). A role/source-specific file is therefore reusable when
/// it fully decodes. Song artwork bypasses persistent image caches entirely;
/// this resolver is for collection art and other remote identity images.
pub fn resolve_remote_cached(
    cache_dir: &std::path::Path,
    kind: ImageKind,
    id: u64,
    variant: ImageVariant,
    source_url: &str,
) -> Option<PathBuf> {
    let stem = remote_file_stem(kind, id, variant, source_url);
    let path = crate::utils::find_cached_image(cache_dir, &stem)?;
    if remote_variant_file_is_usable(&path) {
        Some(path)
    } else {
        // An exact identity with corrupt bytes would otherwise short-circuit
        // `download_img` forever. Only the invalid derivative is discarded;
        // legacy ID-only fallbacks remain untouched.
        crate::utils::remove_cached_image(cache_dir, &stem);
        None
    }
}

fn remote_variant_file_is_usable(path: &std::path::Path) -> bool {
    ::image::open(path)
        .ok()
        .is_some_and(|image| image.width() > 0 && image.height() > 0)
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

#[cfg(test)]
mod tests {
    use super::{
        IMAGE_PROCESSING_VERSION, ImageKind, ImageVariant, remote_file_stem,
        remote_variant_file_is_usable, song_cover_key_for_source, source_url_digest, vip_badge_key,
    };
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

    #[test]
    fn image_variants_have_role_appropriate_dimensions() {
        assert_eq!(ImageVariant::Thumbnail.edge(), 300);
        assert_eq!(ImageVariant::Detail.edge(), 600);
        assert_eq!(ImageVariant::Hero.edge(), 1024);
        assert_eq!(
            ImageKind::SongCover.requested_resize(ImageVariant::Hero),
            Some((1024, 1024))
        );
        assert_eq!(
            ImageKind::VipBadge.requested_resize(ImageVariant::Thumbnail),
            None
        );
        assert!(ImageKind::PlaylistCover.cached_dimensions_satisfy(ImageVariant::Detail, 600, 600));
        assert!(!ImageKind::PlaylistCover.cached_dimensions_satisfy(
            ImageVariant::Detail,
            599,
            600
        ));
        assert!(ImageKind::VideoCover.cached_dimensions_satisfy(ImageVariant::Thumbnail, 300, 169));
        assert!(!ImageKind::VideoCover.cached_dimensions_satisfy(
            ImageVariant::Thumbnail,
            299,
            168
        ));
    }

    #[test]
    fn remote_cache_identity_includes_variant_source_and_processing_version() {
        let thumbnail = remote_file_stem(
            ImageKind::PlaylistCover,
            42,
            ImageVariant::Thumbnail,
            "https://example.invalid/a.jpg",
        );
        let detail = remote_file_stem(
            ImageKind::PlaylistCover,
            42,
            ImageVariant::Detail,
            "https://example.invalid/a.jpg",
        );
        let changed_source = remote_file_stem(
            ImageKind::PlaylistCover,
            42,
            ImageVariant::Thumbnail,
            "https://example.invalid/b.jpg",
        );

        assert_ne!(thumbnail, detail);
        assert_ne!(thumbnail, changed_source);
        assert!(thumbnail.contains(&format!("_v{IMAGE_PROCESSING_VERSION}_")));
        assert_eq!(
            source_url_digest("https://example.invalid/a.jpg"),
            source_url_digest("https://example.invalid/a.jpg")
        );
    }

    #[test]
    fn source_limited_exact_variant_is_best_available_but_corrupt_bytes_are_not() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rustle-image-variant-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let best_available = root.join("detail.png");
        let corrupt = root.join("corrupt.jpg");
        ::image::DynamicImage::new_rgb8(320, 320)
            .save_with_format(&best_available, ::image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&corrupt, b"<html>not an image</html>").unwrap();

        assert!(!ImageKind::PlaylistCover.cached_dimensions_satisfy(
            ImageVariant::Detail,
            320,
            320
        ));
        assert!(remote_variant_file_is_usable(&best_available));
        assert!(!remote_variant_file_is_usable(&corrupt));

        let _ = std::fs::remove_dir_all(root);
    }
}
