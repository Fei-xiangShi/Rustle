//! Unified song metadata interchange format
//!
//! `SongMetadata` is the canonical representation of a song's descriptive metadata,
//! regardless of source (local file tags, NCM API, or database cache).
//!
//! ## Source priority for `resolve`
//! 1. Local file tags (authoritative — read fresh from disk)
//! 2. NCM API data   (current — more recent than DB cache)
//! 3. Database cache (fallback — may be stale)

use std::path::PathBuf;
use std::time::Duration;

/// Source of cover art
#[derive(Debug, Clone)]
pub enum CoverSource {
    /// Raw cover art data with MIME type (from file tags)
    Embedded { data: Vec<u8>, mime: String },
    /// Remote URL (NCM songs before download/caching)
    Url(String),
    /// Local file path (cached or downloaded cover)
    Path(PathBuf),
}

/// Normalized song metadata from any source.
///
/// Fields that are `None` mean the source genuinely does not provide them
/// (e.g. NCM API has no track_number/year/genre, local files have no singer_id).
#[derive(Debug, Clone)]
pub struct SongMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub cover: Option<CoverSource>,
    pub format: Option<String>,
}

impl Default for SongMetadata {
    fn default() -> Self {
        Self {
            title: "Unknown Title".into(),
            artist: "Unknown Artist".into(),
            album: "Unknown Album".into(),
            duration: Duration::ZERO,
            track_number: None,
            year: None,
            genre: None,
            cover: None,
            format: None,
        }
    }
}

impl SongMetadata {
    pub fn duration_display(&self) -> String {
        crate::utils::format_time_padded(self.duration.as_secs_f32())
    }

    /// Resolve the best available cover for display.
    ///
    /// If `file_path` points to an existing file, extracts embedded cover art.
    /// For NCM songs (`song_id < 0`), delegates to [`crate::utils::find_cover`].
    pub fn resolve_cover(&self, file_path: Option<&str>, song_id: i64) -> Option<PathBuf> {
        // 1. Local file: extract embedded cover
        if let Some(p) = file_path {
            let path = std::path::Path::new(p);
            if path.is_absolute()
                && path.exists()
                && let Ok(m) = crate::features::import::extract_metadata(path)
                && let Some(data) = m.cover_data
            {
                let dir = std::env::temp_dir().join("rustle_covers");
                let _ = std::fs::create_dir_all(&dir);
                let out = dir.join(format!("{}.jpg", song_id));
                let _ = std::fs::write(&out, &data);
                return Some(out);
            }
            // File exists but no embedded cover — still use the file path
            // (cover might be external, e.g. cover.jpg in same folder)
        }
        // 2. NCM song: check cache, auto-download if missing
        if song_id < 0 {
            return crate::image::resolve_cached(
                crate::image::ImageKind::SongCover,
                (-song_id) as u64,
            );
        }
        // 3. Local song without file: check explicit cover_path
        if let Some(CoverSource::Path(p)) = &self.cover
            && p.exists()
        {
            return Some(p.clone());
        }
        None
    }

    /// Resolve metadata from the best available source.
    ///
    /// Priority: local file tags (if file exists on disk) → database cache.
    pub fn resolve(song: &crate::database::DbSong) -> Self {
        let path = std::path::Path::new(&song.file_path);
        if path.is_absolute()
            && path.exists()
            && let Ok(meta) = crate::features::import::extract_metadata(path)
        {
            return SongMetadata::from(meta);
        }
        SongMetadata::from(song)
    }

    /// Create a new DbSong with default values for DB-managed fields.
    ///
    /// Fields that require computation (file_hash, file_size, normalization_gain)
    /// start as None/0 — the DB upsert layer fills them in for local files.
    pub fn to_db_song(&self, id: i64) -> crate::database::DbSong {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        crate::database::DbSong {
            id,
            file_path: String::new(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_secs: self.duration.as_secs() as i64,
            track_number: self.track_number.map(|n| n as i64),
            year: self.year.map(|y| y as i64),
            genre: self.genre.clone(),
            cover_path: self.cover.as_ref().and_then(|c| match c {
                CoverSource::Url(url) => Some(url.clone()),
                CoverSource::Path(p) => Some(p.to_string_lossy().to_string()),
                CoverSource::Embedded { .. } => None,
            }),
            file_hash: None,
            file_size: 0,
            format: self.format.clone(),
            normalization_gain: None,
            play_count: 0,
            last_played: Some(now),
            last_modified: now,
            is_missing: false,
            created_at: now,
        }
    }

    /// Write metadata fields into an existing DbSong, preserving computed fields.
    ///
    /// Only overwrites descriptive fields (title, artist, album, duration, etc.).
    /// Leaves file_hash, file_size, normalization_gain, play stats, and timestamps intact.
    pub fn merge_into(&self, song: &mut crate::database::DbSong) {
        song.title = self.title.clone();
        song.artist = self.artist.clone();
        song.album = self.album.clone();
        song.duration_secs = self.duration.as_secs() as i64;
        song.track_number = self.track_number.map(|n| n as i64);
        song.year = self.year.map(|y| y as i64);
        song.genre = self.genre.clone();
        song.cover_path = self.cover.as_ref().and_then(|c| match c {
            CoverSource::Url(url) => Some(url.clone()),
            CoverSource::Path(p) => Some(p.to_string_lossy().to_string()),
            CoverSource::Embedded { .. } => None,
        });
        song.format = self.format.clone();
    }

    /// Convert to MetadataEdits for writing tags back to a file
    pub fn to_metadata_edits(&self) -> crate::features::import::MetadataEdits {
        crate::features::import::MetadataEdits {
            title: Some(self.title.clone()),
            artist: Some(self.artist.clone()),
            album: Some(self.album.clone()),
            track_number: self.track_number,
            year: self.year,
            genre: self.genre.clone(),
            cover_data: self.cover.as_ref().and_then(|c| match c {
                CoverSource::Embedded { data, .. } => Some(data.clone()),
                _ => None,
            }),
            cover_mime: self.cover.as_ref().and_then(|c| match c {
                CoverSource::Embedded { mime, .. } => Some(mime.clone()),
                _ => None,
            }),
        }
    }
}

// ---- From impls: each source type to SongMetadata ----

impl From<crate::features::import::AudioMetadata> for SongMetadata {
    fn from(m: crate::features::import::AudioMetadata) -> Self {
        let cover = m.cover_data.map(|data| CoverSource::Embedded {
            data,
            mime: m.cover_mime.unwrap_or_else(|| "image/jpeg".into()),
        });
        SongMetadata {
            title: m.title,
            artist: m.artist,
            album: m.album,
            duration: Duration::from_secs(m.duration_secs as u64),
            track_number: m.track_number.map(|n| n as u32),
            year: m.year.map(|y| y as u32),
            genre: m.genre,
            cover,
            format: Some(m.format),
        }
    }
}

impl From<&crate::api::Track> for SongMetadata {
    fn from(track: &crate::api::Track) -> Self {
        let cover_url = track.cover_url();
        let cover = if cover_url.is_empty() {
            None
        } else {
            Some(CoverSource::Url(cover_url.to_string()))
        };
        let artist = track.artist_names();
        SongMetadata {
            title: if track.title.is_empty() {
                "Unknown Title".into()
            } else {
                track.title.clone()
            },
            artist: if artist.is_empty() {
                "Unknown Artist".into()
            } else {
                artist
            },
            album: if track.album.name.is_empty() {
                "Unknown Album".into()
            } else {
                track.album.name.clone()
            },
            duration: Duration::from_millis(track.duration_ms),
            track_number: track.track_number,
            year: track.year,
            genre: track.genre.clone(),
            cover,
            format: None,
        }
    }
}

impl From<&crate::database::DbSong> for SongMetadata {
    fn from(s: &crate::database::DbSong) -> Self {
        let cover = s.cover_path.as_ref().map(|p| {
            if p.starts_with("http") {
                CoverSource::Url(p.clone())
            } else {
                CoverSource::Path(PathBuf::from(p))
            }
        });
        SongMetadata {
            title: if s.title.is_empty() {
                "Unknown Title".into()
            } else {
                s.title.clone()
            },
            artist: if s.artist.is_empty() {
                "Unknown Artist".into()
            } else {
                s.artist.clone()
            },
            album: if s.album.is_empty() {
                "Unknown Album".into()
            } else {
                s.album.clone()
            },
            duration: Duration::from_secs(s.duration_secs as u64),
            track_number: s.track_number.map(|n| n as u32),
            year: s.year.map(|y| y as u32),
            genre: s.genre.clone(),
            cover,
            format: s.format.clone(),
        }
    }
}
