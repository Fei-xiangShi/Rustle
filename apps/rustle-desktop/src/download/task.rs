//! Audio download module
//!
//! Provides functions for downloading songs from NCM, reusing cached files,
//! verifying downloaded file integrity, and writing metadata tags.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use tracing::info;

use crate::download::{DownloadError, DownloadResult as Result};
use crate::utils::{detect_audio_format, sanitize_filename};

/// Shared with blocking tag writers so cancellation cannot remove an open file
/// (or leave one behind after the async owner stops waiting).
struct TemporaryDownload(PathBuf);

impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        crate::cache::cleanup_temp_file(&self.0);
    }
}

/// Verify downloaded audio file integrity using lofty
pub fn verify_integrity(path: &Path) -> Result<()> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let tagged_file = Probe::open(path)?
        .guess_file_type()
        .map_err(|e| DownloadError::io("probe audio format", e))?
        .read()?;

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
    lyrics: Option<String>,
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
    let temporary_guard = std::sync::Arc::new(TemporaryDownload(tmp.clone()));
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
    // Finish the private file before exposing it to the library or reporting success.
    let result = async {
        let mut edits = crate::cache::prepare_song_tags(ncm_id, meta).await?;
        edits.lyrics = lyrics;
        let writer_guard = temporary_guard.clone();
        tokio::task::spawn_blocking(move || {
            verify_integrity(&writer_guard.0)?;
            crate::features::import::save_metadata(&writer_guard.0, &edits)?;
            verify_integrity(&writer_guard.0)
        })
        .await
        .map_err(|e| DownloadError::io("join audio tagging", std::io::Error::other(e)))??;
        crate::cache::publish_replace(&tmp, &dest)
            .map_err(|e| DownloadError::io("publish tagged download", e))?;
        Ok::<_, DownloadError>(())
    }
    .await;
    if let Err(error) = result {
        crate::cache::cleanup_temp_file(&tmp);
        return Err(error);
    }

    info!(
        "Downloaded: {:?} ({} bytes, {})",
        dest.file_name().unwrap_or_default(),
        downloaded,
        ext
    );
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aborting_a_download_removes_its_partial_file() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\npartial")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let root = crate::cache::unique_temp_path(&std::env::temp_dir().join("cancel-download"));
        let directory = root.clone();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let download = tokio::spawn(async move {
            download_song(
                1,
                &url,
                &directory,
                &crate::metadata::SongMetadata::default(),
                None,
                |_, _| {
                    let _ = progress_tx.send(());
                },
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), progress_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        download.abort();
        assert!(download.await.unwrap_err().is_cancelled());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        server.abort();
        let _ = server.await;
        std::fs::remove_dir(root).unwrap();
    }
}
