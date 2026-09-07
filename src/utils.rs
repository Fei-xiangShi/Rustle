//! Utility functions

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
            primary: Color::from_rgb(0.08, 0.06, 0.12),
            secondary: Color::from_rgb(0.05, 0.05, 0.08),
            tertiary: Color::from_rgb(0.02, 0.02, 0.04),
        }
    }

    /// Extract dominant colors from an image file path (string version)
    pub fn from_image_path(path: &str) -> Option<Self> {
        Self::extract_from_path(Path::new(path))
    }

    /// Extract dominant colors from an image file path
    pub fn extract_from_path(path: &Path) -> Option<Self> {
        let img = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                tracing::warn!("Failed to open image {:?}: {}", path, e);
                return None;
            }
        };
        let img = img.to_rgb8();
        let img = image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Nearest);

        let mut pixels: Vec<(u8, u8, u8)> = Vec::new();
        for pixel in img.pixels() {
            pixels.push((pixel[0], pixel[1], pixel[2]));
        }

        if pixels.is_empty() {
            return None;
        }

        let colors = kmeans_colors(&pixels, 3);

        let to_background_color =
            |r: u8, g: u8, b: u8, brightness_factor: f32, saturation_boost: f32| -> Color {
                let rf = r as f32 / 255.0;
                let gf = g as f32 / 255.0;
                let bf = b as f32 / 255.0;

                let max = rf.max(gf).max(bf);
                let min = rf.min(gf).min(bf);
                let delta = max - min;

                let (r_out, g_out, b_out) = if delta < 0.01 {
                    (
                        rf * brightness_factor,
                        gf * brightness_factor,
                        bf * brightness_factor,
                    )
                } else {
                    let avg = (rf + gf + bf) / 3.0;
                    let boost = |v: f32| -> f32 {
                        let diff = v - avg;
                        (avg + diff * saturation_boost).clamp(0.0, 1.0) * brightness_factor
                    };
                    (boost(rf), boost(gf), boost(bf))
                };

                Color::from_rgb(r_out, g_out, b_out)
            };

        Some(Self {
            primary: to_background_color(colors[0].0, colors[0].1, colors[0].2, 0.65, 1.6),
            secondary: to_background_color(colors[1].0, colors[1].1, colors[1].2, 0.50, 1.5),
            tertiary: to_background_color(colors[2].0, colors[2].1, colors[2].2, 0.25, 1.3),
        })
    }
}

/// Simple k-means clustering for color extraction
fn kmeans_colors(pixels: &[(u8, u8, u8)], k: usize) -> Vec<(u8, u8, u8)> {
    if pixels.is_empty() || k == 0 {
        return vec![(20, 15, 30); k];
    }

    let mut centroids: Vec<(f32, f32, f32)> = (0..k)
        .map(|i| {
            let idx = i * pixels.len() / k;
            let p = pixels[idx.min(pixels.len() - 1)];
            (p.0 as f32, p.1 as f32, p.2 as f32)
        })
        .collect();

    for _ in 0..10 {
        let mut clusters: Vec<Vec<(u8, u8, u8)>> = vec![Vec::new(); k];

        for &pixel in pixels {
            let mut min_dist = f32::MAX;
            let mut min_idx = 0;

            for (idx, centroid) in centroids.iter().enumerate() {
                let dist = color_distance(pixel, *centroid);
                if dist < min_dist {
                    min_dist = dist;
                    min_idx = idx;
                }
            }

            clusters[min_idx].push(pixel);
        }

        for (idx, cluster) in clusters.iter().enumerate() {
            if !cluster.is_empty() {
                let sum: (u32, u32, u32) = cluster.iter().fold((0, 0, 0), |acc, p| {
                    (acc.0 + p.0 as u32, acc.1 + p.1 as u32, acc.2 + p.2 as u32)
                });
                let len = cluster.len() as f32;
                centroids[idx] = (sum.0 as f32 / len, sum.1 as f32 / len, sum.2 as f32 / len);
            }
        }
    }

    centroids.sort_by(|a, b| {
        let brightness_a = a.0 * 0.299 + a.1 * 0.587 + a.2 * 0.114;
        let brightness_b = b.0 * 0.299 + b.1 * 0.587 + b.2 * 0.114;
        brightness_a
            .partial_cmp(&brightness_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    centroids
        .iter()
        .map(|(r, g, b)| (*r as u8, *g as u8, *b as u8))
        .collect()
}

/// Calculate squared distance between a pixel and a centroid
fn color_distance(pixel: (u8, u8, u8), centroid: (f32, f32, f32)) -> f32 {
    let dr = pixel.0 as f32 - centroid.0;
    let dg = pixel.1 as f32 - centroid.1;
    let db = pixel.2 as f32 - centroid.2;
    dr * dr + dg * dg + db * db
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

/// Find an existing cached audio file with any common extension
///
/// # Arguments
/// * `dir` - The directory to search in
/// * `stem` - The filename without extension (e.g., "12345")
///
/// # Returns
/// The path to the existing file if found, None otherwise
pub fn find_cached_audio(dir: &Path, stem: &str) -> Option<PathBuf> {
    AUDIO_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{}.{}", stem, ext)))
        .find(|p| p.exists())
}

/// Detect audio format from magic bytes
/// Returns the correct file extension (without dot)
pub fn detect_audio_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }

    // FLAC: fLaC
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }

    // MP3: MPEG audio frame sync or ID3 tag
    if bytes.starts_with(&[0xFF, 0xFB])
        || bytes.starts_with(&[0xFF, 0xFA])
        || bytes.starts_with(&[0xFF, 0xF3])
        || bytes.starts_with(&[0xFF, 0xF2])
        || bytes.starts_with(b"ID3")
    {
        return Some("mp3");
    }

    // M4A/AAC: ISO-BMFF ftyp box
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some("m4a");
    }

    // OGG: OggS; identify Opus when the codec marker is present.
    if bytes.starts_with(b"OggS") {
        return Some(if bytes.windows(8).any(|w| w == b"OpusHead") {
            "opus"
        } else {
            "ogg"
        });
    }

    // WAV: RIFF....WAVE
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return Some("wav");
    }

    None
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
    /// NCM song only available online (not downloaded or cached)
    Online,
}

/// Determine the source of a song at runtime by checking file system state
///
/// Checks in order:
/// 1. Absolute path exists on disk → Local
/// 2. NCM song with downloaded file in download dir → Local
/// 3. Otherwise → Online
pub fn compute_source(
    file_path: &str,
    song_id: i64,
    artist: Option<&str>,
    title: Option<&str>,
) -> Source {
    let path = Path::new(file_path);
    if path.is_absolute() && path.exists() {
        return Source::Local;
    }
    if song_id < 0 {
        // Downloaded file is local; quality-scoped streaming cache is not
        // sufficient to classify a song without the requested quality.
        if let (Some(a), Some(t)) = (artist, title) {
            let dl = crate::features::settings::StorageSettings::default().effective_download_dir();
            let stem = format!("{} - {}", sanitize_filename(a), sanitize_filename(t));
            if AUDIO_EXTENSIONS
                .iter()
                .map(|e| dl.join(format!("{}.{}", stem, e)))
                .any(|p| p.exists())
            {
                return Source::Local;
            }
        }
    }
    Source::Online
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
        std::fs::write(&temp, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();

        let published = publish_downloaded_image(&temp, &root, "cover").unwrap();

        assert_eq!(published, root.join("cover.png"));
        assert!(published.exists());
        assert!(!temp.exists());
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
