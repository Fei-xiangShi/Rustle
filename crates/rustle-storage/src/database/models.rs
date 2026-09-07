//! Database models for persistent storage
//! These models map directly to SQLite tables

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Borrowed input for recording a completed download.
#[derive(Debug, Clone, Copy)]
pub struct NewDownload<'a> {
    pub song_id: i64,
    pub ncm_id: u64,
    pub title: &'a str,
    pub artist: &'a str,
    pub file_path: &'a str,
    pub file_size: u64,
    pub quality: &'a str,
}

/// Song metadata stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DbSong {
    /// Unique identifier (auto-increment)
    pub id: i64,
    /// File path on disk
    pub file_path: String,
    /// Song title
    pub title: String,
    /// Artist name
    pub artist: String,
    /// Album name
    pub album: String,
    /// Duration in seconds
    pub duration_secs: i64,
    /// Track number in album
    pub track_number: Option<i64>,
    /// Year released
    pub year: Option<i64>,
    /// Genre
    pub genre: Option<String>,
    /// Cover art path (cached locally)
    pub cover_path: Option<String>,
    /// File hash for deduplication
    pub file_hash: Option<String>,
    /// File size in bytes
    pub file_size: i64,
    /// Audio format (mp3, flac, etc.)
    pub format: Option<String>,
    /// Cached normalization gain (linear multiplier)
    pub normalization_gain: Option<f64>,
    /// Play count
    pub play_count: i64,
    /// Last played timestamp
    pub last_played: Option<i64>,
    /// Last modified timestamp
    pub last_modified: i64,
    /// Whether the local file is currently unavailable
    pub is_missing: bool,
    /// Created timestamp
    pub created_at: i64,
}

/// Playlist stored in database
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DbPlaylist {
    /// Unique identifier
    pub id: i64,
    /// Playlist name
    pub name: String,
    /// Description
    pub description: Option<String>,
    /// Cover image path
    pub cover_path: Option<String>,
    /// Is this a smart/auto playlist
    pub is_smart: bool,
    /// Created timestamp
    pub created_at: i64,
    /// Last modified timestamp
    pub updated_at: i64,
}

/// Playback state for resuming
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DbPlaybackState {
    /// Always id=1, singleton row
    pub id: i64,
    /// Currently playing song id
    pub current_song_id: Option<i64>,
    /// Current position in queue
    pub queue_position: i64,
    /// Playback position in seconds
    pub position_secs: f64,
    /// Volume level (0.0 - 1.0)
    pub volume: f64,
    /// Is shuffle enabled
    pub shuffle: bool,
    /// Repeat mode: 0=off, 1=all, 2=one
    pub repeat_mode: i64,
    /// Whether Personal FM mode was active when playback state was saved
    pub personal_fm_mode: bool,
    /// Last updated timestamp
    pub updated_at: i64,
}

// ============ Input structs for creating new records ============

/// Input for creating a new song
#[derive(Debug, Clone)]
pub struct NewSong {
    pub file_path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: i64,
    pub track_number: Option<i64>,
    pub year: Option<i64>,
    pub genre: Option<String>,
    pub cover_path: Option<String>,
    pub file_hash: Option<String>,
    pub file_size: i64,
    pub format: Option<String>,
    pub normalization_gain: Option<f64>,
}

/// Watched folder for auto-scanning
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DbWatchedFolder {
    pub id: i64,
    pub path: String,
    pub playlist_id: Option<i64>,
    pub enabled: bool,
    pub last_scanned: Option<i64>,
    pub created_at: i64,
}

/// Input for creating a new watched folder
#[derive(Debug, Clone)]
pub struct NewWatchedFolder {
    pub path: String,
    pub playlist_id: Option<i64>,
    pub enabled: bool,
}

/// Input for creating a new playlist
#[derive(Debug, Clone)]
pub struct NewPlaylist {
    pub name: String,
    pub description: Option<String>,
    pub cover_path: Option<String>,
    pub is_smart: bool,
}

/// Song with playlist-specific data (like added_at)
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DbPlaylistSongWithDate {
    #[sqlx(flatten)]
    pub song: DbSong,
    pub added_at: i64,
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DownloadRow {
    pub song_id: i64,
    pub ncm_id: i64,
    pub title: String,
    pub artist: String,
    pub file_path: String,
    pub file_size: i64,
    pub quality: String,
    pub downloaded_at: i64,
}
