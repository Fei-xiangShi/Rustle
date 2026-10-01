//! Utility functions

pub mod audio_index;

use iced::Color;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;

pub use rustle_storage::paths::{
    automix_cache_dir, avatars_cache_dir, cache_dir, covers_cache_dir, lyrics_cache_dir,
    songs_cache_dir, vip_badges_cache_dir,
};

// ============================================================================
// Image Extensions
// ============================================================================

/// Common image file extensions for cache lookup
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "png", "gif", "webp", "bmp"];

/// Find an existing cached image file with any common extension
///
/// # Arguments
/// * `dir` - The directory to search in
/// * `stem` - The filename without extension (e.g., "cover_123")
///
/// # Returns
/// The path to the existing file if found, None otherwise
pub(crate) fn find_cached_image(dir: &Path, stem: &str) -> Option<PathBuf> {
    IMAGE_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{}.{}", stem, ext)))
        .find(|p| p.exists())
}

/// Remove all cached image variants for a cache key.
///
/// This is used for remote resources whose URL changes while their logical
/// application ID stays the same. Cache entries are disposable, so a failed
/// removal is intentionally ignored and the next request can still recover by
/// replacing whichever variant remains.
pub(crate) fn remove_cached_image(dir: &Path, stem: &str) {
    for ext in IMAGE_EXTENSIONS {
        let _ = std::fs::remove_file(dir.join(format!("{}.{}", stem, ext)));
    }
    let _ = std::fs::remove_file(dir.join(format!("{}.tmp", stem)));
    if let Ok(entries) = std::fs::read_dir(dir) {
        let prefix = format!(".{stem}.");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|ext| ext == "tmp")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

// ============================================================================
// Color Extraction
// ============================================================================

/// Extracted color palette from an image (simple 2-color version)
#[derive(Debug, Clone)]
pub struct ColorPalette {
    /// Primary dominant color
    pub primary: Color,
}

impl ColorPalette {
    /// Extract dominant color from an image file
    #[cfg(test)]
    pub fn from_image_path(path: &Path) -> Option<Self> {
        match DominantColors::extract_from_path(path) {
            Some(colors) => {
                tracing::debug!(
                    "Extracted colors from {:?}: primary=({:.2}, {:.2}, {:.2})",
                    path,
                    colors.primary.r,
                    colors.primary.g,
                    colors.primary.b
                );
                Some(Self {
                    primary: colors.primary,
                })
            }
            None => {
                tracing::warn!("Failed to extract colors from {:?}", path);
                None
            }
        }
    }
}

/// Dominant colors extracted from an image using k-means clustering
#[derive(Debug, Clone, Default)]
pub struct DominantColors {
    /// Primary dominant color
    pub primary: Color,
    /// Secondary dominant color
    pub secondary: Color,
    /// Tertiary dominant color
    pub tertiary: Color,
}

impl DominantColors {
    /// Create default dark colors for when no image is available
    pub fn dark_default() -> Self {
        Self {
            primary: rustle_ui::color::artwork::FALLBACK[0].to_color(),
            secondary: rustle_ui::color::artwork::FALLBACK[1].to_color(),
            tertiary: rustle_ui::color::artwork::FALLBACK[2].to_color(),
        }
    }

    /// Extract dominant colors from an image file path
    #[cfg(test)]
    pub fn extract_from_path(path: &Path) -> Option<Self> {
        let img = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                tracing::warn!("Failed to open image {:?}: {}", path, e);
                return None;
            }
        };
        Some(Self::from_image(&img))
    }

    pub fn from_image(img: &image::DynamicImage) -> Self {
        let sources = rustle_ui::color::artwork::dominant(img);
        let colors: [Color; 3] = std::array::from_fn(|i| {
            rustle_ui::color::artwork::background(sources[i], i).to_color()
        });
        Self {
            primary: colors[0],
            secondary: colors[1],
            tertiary: colors[2],
        }
    }
}

// ============================================================================
// Time & Path Utilities
// ============================================================================

/// Format timestamp as relative time (e.g., "2天前")
pub fn format_relative_time(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let diff_secs = now - timestamp;
    let diff_mins = diff_secs / 60;
    let diff_hours = diff_mins / 60;
    let diff_days = diff_hours / 24;

    if diff_days > 30 {
        let months = diff_days / 30;
        format!("{}个月前", months)
    } else if diff_days > 0 {
        format!("{}天前", diff_days)
    } else if diff_hours > 0 {
        format!("{}小时前", diff_hours)
    } else if diff_mins > 0 {
        format!("{}分钟前", diff_mins)
    } else {
        "刚刚".to_string()
    }
}

// ============================================================================
// Audio Format Detection
// ============================================================================

/// Common audio file extensions for cache lookup
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "wav", "opus", "wma", "aiff",
];

/// Enumerate cache formats so callers can validate every candidate instead
/// of allowing a stale extension to hide a complete file.
pub fn cached_audio_candidates<'a>(
    dir: &'a Path,
    stem: &'a str,
) -> impl Iterator<Item = PathBuf> + 'a {
    AUDIO_EXTENSIONS
        .iter()
        .map(move |ext| dir.join(format!("{stem}.{ext}")))
}

/// Detect audio format from magic bytes
/// Returns the correct file extension (without dot)
pub fn detect_audio_format(bytes: &[u8]) -> Option<&'static str> {
    crate::domain::audio::detect_audio_format(bytes)
}

// ============================================================================
// Image Format Detection
// ============================================================================

/// Detect image format from magic bytes
/// Returns the correct file extension (without dot)
pub(crate) fn detect_image_format(bytes: &[u8]) -> &'static str {
    if bytes.len() < 8 {
        return "jpg"; // Default fallback
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "png";
    }

    // JPEG: FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "jpg";
    }

    // GIF: 47 49 46 38
    if bytes.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return "gif";
    }

    // WebP: 52 49 46 46 ... 57 45 42 50
    if bytes.len() >= 12 && bytes.starts_with(&[0x52, 0x49, 0x46, 0x46]) && &bytes[8..12] == b"WEBP"
    {
        return "webp";
    }

    // BMP: 42 4D
    if bytes.starts_with(&[0x42, 0x4D]) {
        return "bmp";
    }

    "jpg" // Default fallback
}

/// Download an image from URL to local path.
///
/// The function detects the actual image format from magic bytes and saves
/// with the correct extension, regardless of what extension was requested.
pub async fn download_img(
    client: &crate::api::NcmClient,
    url: &str,
    base_path: PathBuf,
    resize: Option<(u16, u16)>,
) -> Option<PathBuf> {
    // Ensure parent directory exists
    if let Some(parent) = base_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        error!("Failed to create cache directory: {}", e);
        return None;
    }

    // Get the stem (filename without extension)
    let stem = base_path.file_stem()?.to_str()?;
    let parent = base_path.parent()?;

    // Check if file already exists with any common image extension
    if let Some(existing) = find_cached_image(parent, stem) {
        return Some(existing);
    }

    // Download to a process-unique temporary path first to detect format.
    let temp_anchor = parent.join(stem);
    let temp_path = crate::cache::unique_temp_path(&temp_anchor);

    match client.download_img(url, temp_path.clone(), resize).await {
        Ok(_) => publish_downloaded_image(&temp_path, parent, stem),
        Err(e) => {
            error!("Failed to download image: {}", e);
            crate::cache::cleanup_temp_file(&temp_path);
            None
        }
    }
}

fn publish_downloaded_image(temp_path: &Path, parent: &Path, stem: &str) -> Option<PathBuf> {
    // `NcmClient::download_img` uses `std::fs::write`, so its writable handle
    // is already closed when it returns. Do not reopen the file read-only and
    // call `sync_all`: Windows rejects `FlushFileBuffers` on that handle.
    let bytes = match std::fs::read(temp_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            error!("Failed to read downloaded image: {}", error);
            crate::cache::cleanup_temp_file(temp_path);
            return None;
        }
    };
    if let Err(error) = image::load_from_memory(&bytes) {
        error!("Downloaded image could not be decoded: {}", error);
        crate::cache::cleanup_temp_file(temp_path);
        return None;
    }
    let ext = detect_image_format(&bytes);
    let final_path = parent.join(format!("{}.{}", stem, ext));
    if let Err(error) = crate::cache::publish_or_reuse(temp_path, &final_path, None) {
        error!("Failed to publish downloaded image: {}", error);
        crate::cache::cleanup_temp_file(temp_path);
        return None;
    }
    Some(final_path)
}

// ============================================================================
// Time Formatting
// ============================================================================

/// Format duration in seconds to "M:SS" or "MM:SS" string
pub fn format_time(seconds: f32) -> String {
    if seconds.is_finite() && seconds > 0.0 {
        let total_secs = seconds as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{}:{:02}", mins, secs)
    } else {
        "0:00".to_string()
    }
}

/// Format duration in seconds to "MM:SS" string (padded minutes)
pub fn format_time_padded(seconds: f32) -> String {
    if seconds.is_finite() && seconds > 0.0 {
        let total_secs = seconds as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    } else {
        "00:00".to_string()
    }
}

// ============================================================================
// Source Detection
// ============================================================================

/// Song source origin for display badges
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Local file with absolute path that exists on disk (imported or downloaded)
    Local,
    /// Fully downloaded streaming cache file (managed by Rustle).
    Cached,
    /// NCM song only available online (not downloaded or cached)
    Online,
}

/// Determine the source of a song at runtime by checking file system state
///
/// Checks in order:
/// 1. Absolute path exists on disk → Cached inside the streaming cache, otherwise Local
/// 2. NCM song with downloaded file in download dir → Local
/// 3. NCM song with a streaming cache file → Cached
/// 4. Otherwise → Online
pub fn compute_source(
    file_path: &str,
    song_id: i64,
    artist: Option<&str>,
    title: Option<&str>,
) -> Source {
    let index = audio_index::snapshot();
    match index.locate(file_path, song_id, artist, title) {
        Some(file) if file.path.starts_with(&index.cache_dir) => Source::Cached,
        Some(_) => Source::Local,
        None => Source::Online,
    }
}

/// Return the longest UTF-8-valid prefix that fits within `max_bytes`.
///
/// `str::len` and string slicing use byte offsets, while user-facing text is
/// encoded as UTF-8. This helper makes the byte-oriented limit explicit and
/// backs up to the previous character boundary when the limit falls inside a
/// multi-byte character. The returned slice is always valid UTF-8 and never
/// exceeds `max_bytes` bytes.
#[must_use]
pub fn truncate_utf8_to_bytes(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }

    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

const MAX_FILENAME_BYTES: usize = 200;

/// Replace filename-unsafe characters and limit the result to 200 bytes.
///
/// The extension is kept intact when it can fit within the limit. Both the
/// filename and extension may contain multi-byte UTF-8 characters, so all
/// truncation goes through [`truncate_utf8_to_bytes`].
pub fn sanitize_filename(input: &str) -> String {
    let mut result: String = input
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            '\0' => '\0', // will be removed below
            other => other,
        })
        .collect();
    result.retain(|c| c != '\0');
    // Limit length, keeping extension intact
    if result.len() > MAX_FILENAME_BYTES {
        if let Some(dot) = result.rfind('.')
            && dot > 0
            && result.len() - dot <= MAX_FILENAME_BYTES
        {
            let ext = &result[dot..];
            let name = truncate_utf8_to_bytes(&result[..dot], MAX_FILENAME_BYTES - ext.len());
            result = format!("{}{}", name, ext);
        } else {
            result = truncate_utf8_to_bytes(&result, MAX_FILENAME_BYTES).to_string();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_palette_returns_none_when_extraction_fails() {
        let missing = std::env::temp_dir().join(format!(
            "rustle-missing-palette-source-{}.png",
            std::process::id()
        ));

        assert!(!missing.exists());
        assert!(ColorPalette::from_image_path(&missing).is_none());
    }

    #[test]
    fn downloaded_image_is_published_after_the_writer_handle_is_closed() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rustle-image-publish-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let temp = crate::cache::unique_temp_path(&root.join("cover"));
        let image = image::DynamicImage::new_rgb8(1, 1);
        image
            .save_with_format(&temp, image::ImageFormat::Png)
            .unwrap();

        let published = publish_downloaded_image(&temp, &root, "cover").unwrap();

        assert_eq!(published, root.join("cover.png"));
        assert!(published.exists());
        assert!(!temp.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_html_response_is_not_published_as_an_image() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rustle-invalid-image-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let temp = crate::cache::unique_temp_path(&root.join("cover"));
        std::fs::write(&temp, b"<html>upstream error</html>").unwrap();

        assert!(publish_downloaded_image(&temp, &root, "cover").is_none());
        assert!(!temp.exists());
        assert!(find_cached_image(&root, "cover").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn truncate_utf8_to_bytes_never_splits_a_character() {
        let input = "a中b";

        assert_eq!(truncate_utf8_to_bytes(input, 0), "");
        assert_eq!(truncate_utf8_to_bytes(input, 2), "a");
        assert_eq!(truncate_utf8_to_bytes(input, 4), "a中");
        assert_eq!(truncate_utf8_to_bytes(input, input.len()), input);
        assert_eq!(truncate_utf8_to_bytes(input, input.len() + 1), input);
    }

    #[test]
    fn sanitize_filename_handles_long_multibyte_input() {
        let input = "中".repeat(100);
        let output = sanitize_filename(&input);

        assert!(output.len() <= MAX_FILENAME_BYTES);
        assert!(output.is_char_boundary(output.len()));
        assert_eq!(output.chars().count(), 66);
    }

    #[test]
    fn sanitize_filename_preserves_extension_with_multibyte_name() {
        let input = format!("{} .mp3", "中".repeat(100));
        let output = sanitize_filename(&input);

        assert!(output.len() <= MAX_FILENAME_BYTES);
        assert!(output.ends_with(".mp3"));
        assert!(output.is_char_boundary(output.len()));
    }

    #[test]
    fn sanitize_filename_handles_extension_larger_than_limit() {
        let input = format!("name.{}", "中".repeat(100));
        let output = sanitize_filename(&input);

        assert!(output.len() <= MAX_FILENAME_BYTES);
        assert!(output.is_char_boundary(output.len()));
    }
}
