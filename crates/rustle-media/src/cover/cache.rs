//! Cover art caching system
//!
//! Extracts cover art from audio files, generates thumbnails,
//! and caches them to disk for fast access.

use image::ImageFormat;
use image::imageops::FilterType;
use lofty::{config::ParseOptions, file::TaggedFileExt, probe::Probe};
use rustle_application::ports::cache::CachePublisher;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xxhash_rust::xxh3::xxh3_64;

use crate::error::{MediaError, MediaResult};

/// Default thumbnail size (width and height)
pub const THUMBNAIL_SIZE: u32 = 300;

/// Cover cache manager
pub struct CoverCache {
    cache_dir: PathBuf,
    publisher: Arc<dyn CachePublisher>,
}

impl std::fmt::Debug for CoverCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverCache")
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

impl CoverCache {
    /// Create a new cover cache with the specified cache directory
    pub fn new(cache_dir: PathBuf, publisher: Arc<dyn CachePublisher>) -> MediaResult<Self> {
        std::fs::create_dir_all(&cache_dir)
            .map_err(|error| MediaError::io("create cover cache directory", error))?;
        Ok(Self {
            cache_dir,
            publisher,
        })
    }

    /// Recover disposable artwork from the original audio, without decoding audio.
    pub fn restore_cover(&self, audio_path: &Path) -> MediaResult<Option<PathBuf>> {
        let tagged = Probe::open(audio_path).and_then(|probe| {
            probe
                .options(ParseOptions::new().read_properties(false))
                .read()
        });
        match tagged {
            Ok(tagged) => {
                let pictures = || tagged.tags().iter().flat_map(|tag| tag.pictures());
                if let Some(picture) = pictures()
                    .find(|picture| picture.pic_type() == lofty::picture::PictureType::CoverFront)
                    .or_else(|| pictures().next())
                {
                    return match self.save_cover(picture.data()) {
                        Ok((_, path)) => Ok(Some(path)),
                        Err(error) => super::find_cover_art(audio_path).map(Some).ok_or(error),
                    };
                }
            }
            Err(error) => {
                if let Some(path) = super::find_cover_art(audio_path) {
                    return Ok(Some(path));
                }
                return Err(MediaError::metadata("read cover tags", error));
            }
        }
        Ok(super::find_cover_art(audio_path))
    }

    /// Generate a hash for cover art data
    fn hash_cover(data: &[u8]) -> String {
        format!("{:016x}", xxh3_64(data))
    }

    /// Get the path where a cover with the given hash would be stored
    fn cover_path(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.jpg", hash))
    }

    /// Save cover art to cache, returning the hash and path
    ///
    /// The cover is resized to a thumbnail and saved as JPEG for consistency
    fn save_cover(&self, data: &[u8]) -> MediaResult<(String, PathBuf)> {
        let hash = Self::hash_cover(data);
        let path = self.cover_path(&hash);

        // Skip if already cached
        if path.is_file() {
            return Ok((hash, path));
        }

        // Load and resize image
        let img = image::load_from_memory(data).map_err(|source| MediaError::Image {
            operation: "decode cover",
            source,
        })?;

        // Resize to thumbnail, maintaining aspect ratio
        let thumbnail = img.resize(THUMBNAIL_SIZE, THUMBNAIL_SIZE, FilterType::Lanczos3);

        // Save as JPEG
        let mut output = Vec::new();
        thumbnail
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Jpeg)
            .map_err(|source| MediaError::Image {
                operation: "encode cover thumbnail",
                source,
            })?;

        std::fs::create_dir_all(&self.cache_dir)
            .map_err(|error| MediaError::io("create cover cache directory", error))?;
        let temp = self.publisher.unique_temp_path(&path);
        let result = std::fs::write(&temp, &output).and_then(|()| {
            self.publisher
                .publish_or_reuse(&temp, &path, Some(output.len() as u64))
        });
        if let Err(error) = result {
            self.publisher.cleanup_temp_file(&temp);
            return Err(MediaError::io("publish cover cache", error));
        }

        Ok((hash, path))
    }

    /// Save cover art from raw bytes, with optional MIME type hint
    pub fn save_cover_with_mime(
        &self,
        data: &[u8],
        _mime: Option<&str>,
    ) -> MediaResult<(String, PathBuf)> {
        // image crate auto-detects format, so we ignore MIME hint
        self.save_cover(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_consistency() {
        let data = b"test cover data";
        let hash1 = CoverCache::hash_cover(data);
        let hash2 = CoverCache::hash_cover(data);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_uniqueness() {
        let data1 = b"test cover data 1";
        let data2 = b"test cover data 2";
        let hash1 = CoverCache::hash_cover(data1);
        let hash2 = CoverCache::hash_cover(data2);
        assert_ne!(hash1, hash2);
    }
}
