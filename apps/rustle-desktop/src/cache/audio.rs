//! Desktop composition of media tagging and storage publication.
use crate::download::{DownloadError, DownloadResult};
use crate::metadata::{CoverSource, SongMetadata};
use rustle_application::ports::cache::{AudioCacheStore, CachePublisher, PublishOutcome};
use rustle_domain::audio::QualityLevel;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Resolve artwork for both explicit downloads and streaming cache publication.
/// A supplied cover must be readable; never silently publish without it.
pub async fn prepare_song_tags(
    _id: u64,
    meta: &SongMetadata,
) -> DownloadResult<rustle_media::metadata::MetadataEdits> {
    let mut edits = meta.to_metadata_edits();
    let data = match &meta.cover {
        Some(CoverSource::Embedded { data, .. }) => Some(data.clone()),
        Some(CoverSource::Path(path)) if path.is_file() => Some(
            tokio::fs::read(path)
                .await
                .map_err(|e| DownloadError::io("read cover", e))?,
        ),
        Some(CoverSource::Url(url)) => {
            let response = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?
                .get(url)
                .send()
                .await?
                .error_for_status()?;
            Some(response.bytes().await?.to_vec())
        }
        _ if meta.cover.is_some() => {
            return Err(DownloadError::io(
                "read cover",
                io::Error::new(io::ErrorKind::NotFound, "cover is missing"),
            ));
        }
        _ => None,
    };
    if let Some(data) = data {
        // Bound the uncompressed pixels as well as the dimensions. FLAC picture
        // blocks have a 24-bit length; an original-size PNG can exceed it even
        // when the downloaded JPEG was small. RGBA8 at 1024px fits comfortably.
        let png = tokio::task::spawn_blocking(move || {
            let image = image::load_from_memory(&data)?;
            let image = image::DynamicImage::ImageRgba8(image.thumbnail(1024, 1024).to_rgba8());
            let mut output = io::Cursor::new(Vec::new());
            image.write_to(&mut output, image::ImageFormat::Png)?;
            Ok::<_, image::ImageError>(output.into_inner())
        })
        .await
        .map_err(|e| DownloadError::io("join cover encoding", io::Error::other(e)))?
        .map_err(|e| {
            DownloadError::Metadata(rustle_media::error::MediaError::metadata("encode cover", e))
        })?;
        edits.cover_data = Some(png);
        edits.cover_mime = Some("image/png".into());
    }
    Ok(edits)
}

pub fn tagged_audio_cache_store(
    client: Arc<crate::api::NcmClient>,
    song: &crate::database::DbSong,
) -> Arc<dyn AudioCacheStore> {
    Arc::new(TaggedAudioCache {
        client,
        song: song.clone(),
    })
}

struct TaggedAudioCache {
    client: Arc<crate::api::NcmClient>,
    song: crate::database::DbSong,
}

/// A blocking worker may outlive its async waiter. Its successful output must
/// own cleanup until the waiter actually accepts publication ownership.
struct TaggedCopy(Option<PathBuf>);

impl Drop for TaggedCopy {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            super::cleanup_temp_file(path);
        }
    }
}

impl CachePublisher for TaggedAudioCache {
    fn unique_temp_path(&self, path: &Path) -> PathBuf {
        super::unique_temp_path(path)
    }
    fn cleanup_temp_file(&self, path: &Path) {
        super::cleanup_temp_file(path);
    }
    fn publish_or_reuse(
        &self,
        temporary: &Path,
        path: &Path,
        size: Option<u64>,
    ) -> io::Result<PublishOutcome> {
        if size.is_some_and(|size| {
            std::fs::metadata(temporary)
                .map(|m| m.len() != size)
                .unwrap_or(true)
        }) {
            super::cleanup_temp_file(temporary);
            return Err(io::Error::other("tagged cache size mismatch"));
        }
        // The shared coordinator owns this key. A legacy file of the same
        // length is not evidence that its tags are complete.
        super::publish_replace(temporary, path)?;
        Ok(PublishOutcome::Published)
    }
}

impl AudioCacheStore for TaggedAudioCache {
    fn prepare_audio_cache(
        &self,
        path: PathBuf,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Option<PathBuf>>> + Send + '_>>
    {
        Box::pin(async move {
            let id = crate::image::ncm_song_id(self.song.id, &self.song.file_path)
                .ok_or_else(|| io::Error::other("missing online song identity"))?;
            let mut meta = SongMetadata::from(&self.song);
            // Recover the original cover URL if the DB only has a disposable thumbnail.
            if !matches!(
                meta.cover,
                Some(CoverSource::Url(_)) | Some(CoverSource::Embedded { .. })
            ) {
                let tracks = self
                    .client
                    .track_detail(&[id])
                    .await
                    .map_err(io::Error::other)?;
                let track = tracks
                    .iter()
                    .find(|track| track.id == id)
                    .ok_or_else(|| io::Error::other("missing track metadata"))?;
                meta = SongMetadata::from(track);
            }
            let edits = prepare_song_tags(id, &meta)
                .await
                .map_err(io::Error::other)?;
            let mut tagged = tokio::task::spawn_blocking(move || {
                let tagged = super::unique_temp_path(&path);
                let guard = TaggedCopy(Some(tagged.clone()));
                std::fs::copy(&path, &tagged)?;
                rustle_media::metadata::save_metadata(&tagged, &edits).map_err(io::Error::other)?;
                Ok::<_, io::Error>(guard)
            })
            .await
            .map_err(io::Error::other)??;
            Ok(tagged.0.take())
        })
    }
    fn write_audio_manifest(
        &self,
        path: &Path,
        id: u64,
        quality: QualityLevel,
        source_size: u64,
        format: &str,
    ) -> io::Result<()> {
        rustle_storage::cache::write_tagged_audio_manifest(path, id, quality, source_size, format)?;
        crate::utils::audio_index::notify_published(path);
        Ok(())
    }
    fn remove_audio_cache(&self, path: &Path) {
        rustle_storage::cache::remove_audio_cache(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::{file::TaggedFileExt, tag::Accessor};

    #[tokio::test]
    async fn abandoned_blocking_tag_result_cleans_up_its_private_copy() {
        let path = super::super::unique_temp_path(&std::env::temp_dir().join("abandoned-tag"));
        let worker_path = path.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let guard = TaggedCopy(Some(worker_path.clone()));
            std::fs::write(&worker_path, b"tagged audio").unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            guard
        });
        ready_rx.await.unwrap();
        assert!(path.exists());
        // Dropping the join handle models an aborted async cache-preparation
        // future. Blocking work continues and its discarded result must clean up.
        drop(worker);
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn embedded_cover_is_bounded_and_normalized_to_eight_bit_pixels() {
        let mut source = io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
            2048,
            1024,
            image::Rgba([32768, 16384, 8192, 65535]),
        ))
        .write_to(&mut source, image::ImageFormat::Png)
        .unwrap();
        let meta = SongMetadata {
            cover: Some(CoverSource::Embedded {
                data: source.into_inner(),
                mime: "image/png".into(),
            }),
            ..Default::default()
        };
        let edits = prepare_song_tags(1, &meta).await.unwrap();
        let bytes = edits.cover_data.unwrap();
        let cover = image::load_from_memory(&bytes).unwrap();
        assert_eq!((cover.width(), cover.height()), (1024, 512));
        assert_eq!(cover.color(), image::ColorType::Rgba8);
        assert!(bytes.len() < 1 << 23); // ample room in a FLAC picture block
        assert_eq!(edits.cover_mime.as_deref(), Some("image/png"));
    }

    fn wav() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&16036u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.resize(16044, 0);
        bytes
    }

    async fn server(cover_ok: bool) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).await.unwrap();
                let cover = String::from_utf8_lossy(&request[..count]).starts_with("GET /cover ");
                let mut png = io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgb8(2, 2)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .unwrap();
                let body = if cover { png.into_inner() } else { wav() };
                let status = if cover && !cover_ok {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        (url, handle)
    }

    fn assert_tags(path: &Path) {
        let file = lofty::probe::Probe::open(path)
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let tag = file.tag(lofty::tag::TagType::Id3v2).unwrap();
        assert_eq!(tag.title().as_deref(), Some("歌曲"));
        assert_eq!(tag.artist().as_deref(), Some("歌手"));
        assert_eq!(tag.album().as_deref(), Some("专辑"));
        assert_eq!(tag.pictures().len(), 1);
        assert_eq!(
            tag.pictures()[0].mime_type(),
            Some(&lofty::picture::MimeType::Png)
        );
    }

    #[tokio::test]
    async fn download_and_streaming_cache_embed_uncached_remote_cover() {
        let (url, server) = server(true).await;
        let root =
            super::super::unique_temp_path(&std::env::temp_dir().join("rustle-tagging-test"));
        std::fs::create_dir(&root).unwrap();
        let meta = SongMetadata {
            title: "歌曲".into(),
            artist: "歌手".into(),
            album: "专辑".into(),
            cover: Some(CoverSource::Url(format!("{url}/cover"))),
            ..Default::default()
        };
        let downloaded = crate::download::task::download_song(
            7,
            &format!("{url}/audio"),
            &root,
            &meta,
            Some("歌词".into()),
            |_, _| {},
        )
        .await
        .unwrap();
        assert_tags(&downloaded);
        let raw = root.join("raw.tmp");
        let bytes = wav();
        std::fs::write(&raw, &bytes).unwrap();
        let song = meta.to_db_song(-7);
        let store = tagged_audio_cache_store(Arc::new(crate::api::NcmClient::new()), &song);
        let tagged = store
            .prepare_audio_cache(raw.clone())
            .await
            .unwrap()
            .unwrap();
        assert_tags(&tagged);
        assert_eq!(std::fs::read(raw).unwrap(), bytes);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn missing_remote_cover_does_not_publish_a_successful_download() {
        let (url, server) = server(false).await;
        let root =
            super::super::unique_temp_path(&std::env::temp_dir().join("rustle-tagging-failure"));
        let meta = SongMetadata {
            cover: Some(CoverSource::Url(format!("{url}/cover"))),
            ..Default::default()
        };
        let result = crate::download::task::download_song(
            7,
            &format!("{url}/audio"),
            &root,
            &meta,
            None,
            |_, _| {},
        )
        .await;
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        server.abort();
        std::fs::remove_dir(root).unwrap();
    }
}
