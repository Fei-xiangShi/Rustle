//! Audio download module
//!
//! Provides functions for downloading songs from NCM, reusing cached files,
//! verifying downloaded file integrity, and writing metadata tags.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::download::{DownloadError, DownloadResult as Result};
use crate::utils::{detect_audio_format, sanitize_filename};

/// Verify downloaded audio file integrity using lofty
pub fn verify_integrity(path: &Path) -> Result<()> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let tagged_file = Probe::open(path)?.read()?;

    let props = tagged_file.properties();
    let duration = props.duration().as_secs();
    if duration == 0 {
        return Err(DownloadError::ZeroDuration);
    }

    info!(
        "Verified audio file: {:?} ({}s)",
        path.file_name().unwrap_or_default(),
        duration
    );
    Ok(())
}

/// Download a song from URL, verify it, and write metadata tags.
///
/// `on_progress(downloaded, total)` is called with byte counts during download.
pub async fn download_song(
    ncm_id: u64,
    song_url: &str,
    download_dir: &Path,
    meta: &crate::metadata::SongMetadata,
    on_progress: impl Fn(u64, u64),
) -> Result<PathBuf> {
    // Ensure the download directory exists.
    fs::create_dir_all(download_dir)
        .map_err(|error| DownloadError::io("create download directory", error))?;

    // Build filename and paths.
    let stem = format!(
        "{} - {}",
        sanitize_filename(&meta.artist),
        sanitize_filename(&meta.title)
    );
    let temp_anchor = download_dir.join(&stem);
    let tmp = crate::cache::unique_temp_path(&temp_anchor);
    {
        let existing = crate::utils::AUDIO_EXTENSIONS
            .iter()
            .map(|e| download_dir.join(format!("{}.{}", stem, e)))
            .find(|p| p.exists());
        if let Some(p) = existing {
            info!("Download file already exists: {:?}", p);
            return Ok(p);
        }
    }

    // Download audio stream.
    let client = reqwest::Client::new();
    let response = client.get(song_url).send().await?;

    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::HttpStatus(status));
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(&tmp)
        .map_err(|error| DownloadError::io("create temporary download", error))?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                crate::cache::cleanup_temp_file(&tmp);
                return Err(DownloadError::Request(error));
            }
        };
        if let Err(error) = file.write_all(&chunk) {
            drop(file);
            crate::cache::cleanup_temp_file(&tmp);
            return Err(DownloadError::io("write temporary download", error));
        }
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }

    if let Err(error) = file.flush() {
        drop(file);
        crate::cache::cleanup_temp_file(&tmp);
        return Err(DownloadError::io("flush temporary download", error));
    }
    if let Err(error) = file.sync_all() {
        drop(file);
        crate::cache::cleanup_temp_file(&tmp);
        return Err(DownloadError::io("sync temporary download", error));
    }
    drop(file);

    if downloaded == 0 {
        crate::cache::cleanup_temp_file(&tmp);
        return Err(DownloadError::Empty);
    }
    if total > 0 && downloaded != total {
        crate::cache::cleanup_temp_file(&tmp);
        return Err(DownloadError::SizeMismatch {
            actual: downloaded,
            expected: total,
        });
    }

    // Detect format from magic bytes, then rename.
    let ext = {
        let mut buf = [0u8; 16];
        let mut f = match fs::File::open(&tmp) {
            Ok(file) => file,
            Err(error) => {
                crate::cache::cleanup_temp_file(&tmp);
                return Err(DownloadError::io(
                    "open temporary download for format detection",
                    error,
                ));
            }
        };
        let n = f.read(&mut buf).unwrap_or(0);
        let Some(extension) = detect_audio_format(&buf[..n]) else {
            drop(f);
            crate::cache::cleanup_temp_file(&tmp);
            return Err(DownloadError::UnsupportedFormat);
        };
        extension.to_string()
    };
    let dest = download_dir.join(format!("{}.{}", stem, ext));
    crate::cache::publish_or_reuse(&tmp, &dest, (total > 0).then_some(total))
        .map_err(|error| DownloadError::io("publish downloaded file", error))?;

    // Verify the final file is playable.
    if let Err(e) = verify_integrity(&dest) {
        let _ = fs::remove_file(&dest);
        return Err(DownloadError::Integrity {
            source: Box::new(e),
        });
    }

    // Write metadata tags, reusing the unified image cache when available.
    let mut edits = meta.to_metadata_edits();
    if edits.cover_data.is_none()
        && let Some(path) = crate::image::resolve_cached(crate::image::ImageKind::SongCover, ncm_id)
        && let Ok(data) = fs::read(&path)
    {
        edits.cover_mime = Some(
            match crate::utils::detect_image_format(&data) {
                "png" => "image/png",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                _ => "image/jpeg",
            }
            .to_string(),
        );
        edits.cover_data = Some(data);
    }
    if let Err(e) = crate::features::import::save_metadata(&dest, &edits) {
        warn!("Failed to write metadata tags to {:?}: {}", dest, e);
    }

    info!(
        "Downloaded: {:?} ({} bytes, {})",
        dest.file_name().unwrap_or_default(),
        downloaded,
        ext
    );
    Ok(dest)
}
