//! Database repository - main entry point
//! Delegates to ops modules for actual operations

use sqlx::{Pool, Sqlite};
use std::path::PathBuf;

use super::{StorageResult as Result, connection, migrations, models::*, ops};

/// Database connection pool wrapper
#[derive(Debug)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    /// Create and initialize database at the given path
    pub async fn new(db_path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let pool = connection::connect(db_path.clone()).await?;
        migrations::initialize(pool.clone(), db_path.clone()).await?;
        let pool = if pool.is_closed() {
            connection::connect(db_path).await?
        } else {
            pool
        };

        Ok(Self { pool })
    }

    // ============ Song Operations ============

    pub async fn upsert_local_song(&self, song: NewSong) -> Result<i64> {
        ops::upsert_local_song(&self.pool, song).await
    }

    pub async fn upsert_local_songs(&self, songs: Vec<NewSong>) -> Result<Vec<i64>> {
        ops::upsert_local_songs(&self.pool, songs).await
    }

    pub async fn get_song_by_path(&self, path: &str) -> Result<Option<DbSong>> {
        ops::get_song_by_path(&self.pool, path).await
    }

    pub async fn get_all_songs(&self) -> Result<Vec<DbSong>> {
        ops::get_all_songs(&self.pool).await
    }

    pub async fn get_all_songs_including_missing(&self) -> Result<Vec<DbSong>> {
        ops::get_all_songs_including_missing(&self.pool).await
    }

    pub async fn mark_song_missing_by_path(&self, path: &str) -> Result<()> {
        ops::mark_song_missing_by_path(&self.pool, path).await
    }

    pub async fn mark_song_available_by_path(&self, path: &str) -> Result<()> {
        ops::mark_song_available_by_path(&self.pool, path).await
    }

    pub async fn update_song_path(&self, old_path: &str, new_path: &str) -> Result<()> {
        ops::update_song_path(&self.pool, old_path, new_path).await
    }

    pub async fn update_song_cover(&self, song_id: i64, cover_path: &str) -> Result<()> {
        ops::update_song_cover(&self.pool, song_id, cover_path).await
    }

    /// Publish a recovered cover only while the source and old cover still match.
    pub async fn restore_song_cover(&self, song: &DbSong, cover_path: &str) -> Result<bool> {
        ops::restore_song_cover(&self.pool, song, cover_path).await
    }

    pub async fn refresh_song_metadata(&self, song: &DbSong) -> Result<()> {
        ops::refresh_song_metadata(&self.pool, song).await
    }

    pub async fn insert_download(&self, download: NewDownload<'_>) -> Result<()> {
        ops::insert_download(&self.pool, download).await?;
        Ok(())
    }

    pub async fn get_all_downloads(&self) -> Result<Vec<DownloadRow>> {
        Ok(ops::get_all_downloads(&self.pool).await?)
    }

    pub async fn delete_download(&self, song_id: i64) -> Result<()> {
        ops::delete_download(&self.pool, song_id).await?;
        Ok(())
    }

    pub async fn update_song_normalization(
        &self,
        song_id: i64,
        file_path: &str,
        normalization_gain: f64,
    ) -> Result<()> {
        ops::update_song_normalization(&self.pool, song_id, file_path, normalization_gain).await
    }

    pub async fn upsert_ncm_song(&self, song: &DbSong) -> Result<i64> {
        ops::upsert_ncm_song(&self.pool, song).await
    }

    // ============ Playlist Operations ============

    pub async fn create_playlist(&self, playlist: NewPlaylist) -> Result<i64> {
        ops::create_playlist(&self.pool, playlist).await
    }

    pub async fn get_all_playlists(&self) -> Result<Vec<DbPlaylist>> {
        ops::get_all_playlists(&self.pool).await
    }

    pub async fn get_playlist(&self, id: i64) -> Result<Option<DbPlaylist>> {
        ops::get_playlist(&self.pool, id).await
    }

    pub async fn add_song_to_playlist(&self, playlist_id: i64, song_id: i64) -> Result<()> {
        ops::add_song_to_playlist(&self.pool, playlist_id, song_id).await
    }

    pub async fn get_playlist_songs(&self, playlist_id: i64) -> Result<Vec<DbSong>> {
        ops::get_playlist_songs(&self.pool, playlist_id).await
    }

    pub async fn get_playlist_songs_with_date(
        &self,
        playlist_id: i64,
    ) -> Result<Vec<crate::database::DbPlaylistSongWithDate>> {
        ops::get_playlist_songs_with_date(&self.pool, playlist_id).await
    }

    pub async fn remove_song_from_playlist(&self, playlist_id: i64, song_id: i64) -> Result<()> {
        ops::remove_song_from_playlist(&self.pool, playlist_id, song_id).await
    }

    pub async fn delete_playlist(&self, id: i64) -> Result<()> {
        ops::delete_playlist(&self.pool, id).await
    }

    pub async fn update_playlist_full(
        &self,
        id: i64,
        name: &str,
        description: Option<&str>,
        cover_path: Option<&str>,
    ) -> Result<()> {
        ops::update_playlist_full(&self.pool, id, name, description, cover_path).await
    }

    // ============ Queue Operations ============

    pub async fn clear_queue(&self) -> Result<()> {
        ops::clear_queue(&self.pool).await
    }

    /// Save queue with full song data, handling NCM songs properly
    /// NCM songs (negative ID) will be upserted to the database first
    /// Uses a transaction for better performance
    pub async fn save_queue_with_songs(
        &self,
        songs: &[DbSong],
        source_playlist_id: Option<i64>,
    ) -> Result<()> {
        use sqlx::Acquire;

        let mut conn = self.pool.acquire().await?;
        let mut tx = conn.begin().await?;

        let mut db_song_ids = Vec::with_capacity(songs.len());

        for song in songs {
            if song.id < 0 || song.file_path.starts_with("ncm://") {
                // NCM song - upsert to database and get the real ID
                let db_id = ops::upsert_ncm_song_tx(&mut tx, song).await?;
                db_song_ids.push(db_id);
            } else {
                // Local song - use existing ID
                db_song_ids.push(song.id);
            }
        }

        ops::set_queue_tx(&mut tx, &db_song_ids, source_playlist_id).await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_queue(&self) -> Result<Vec<DbSong>> {
        ops::get_queue(&self.pool).await
    }

    pub async fn remove_from_queue(&self, position: i64) -> Result<()> {
        ops::remove_from_queue(&self.pool, position).await
    }

    // ============ Playback State Operations ============

    pub async fn get_playback_state(&self) -> Result<DbPlaybackState> {
        ops::get_playback_state(&self.pool).await
    }

    pub async fn update_playback_position(
        &self,
        song_id: Option<i64>,
        queue_position: i64,
        position_secs: f64,
    ) -> Result<()> {
        ops::update_playback_position(&self.pool, song_id, queue_position, position_secs).await
    }

    pub async fn update_volume(&self, volume: f64) -> Result<()> {
        ops::update_volume(&self.pool, volume).await
    }

    pub async fn update_personal_fm_mode(&self, enabled: bool) -> Result<()> {
        ops::update_personal_fm_mode(&self.pool, enabled).await
    }

    // ============ Play History Operations ============

    pub async fn record_play(
        &self,
        song_id: i64,
        listened_secs: i64,
        completed: bool,
    ) -> Result<()> {
        ops::record_play(&self.pool, song_id, listened_secs, completed).await
    }

    pub async fn get_recently_played(&self, limit: i64) -> Result<Vec<DbSong>> {
        ops::get_recently_played(&self.pool, limit).await
    }

    // ============ Watched Folder Operations ============

    pub async fn get_all_watched_folders(&self) -> Result<Vec<DbWatchedFolder>> {
        ops::get_all_watched_folders(&self.pool).await
    }

    pub async fn get_watched_folder_by_playlist(
        &self,
        playlist_id: i64,
    ) -> Result<Option<DbWatchedFolder>> {
        ops::get_watched_folder_by_playlist(&self.pool, playlist_id).await
    }

    pub async fn get_watched_folder_by_path(&self, path: &str) -> Result<Option<DbWatchedFolder>> {
        ops::get_watched_folder_by_path(&self.pool, path).await
    }

    pub async fn upsert_watched_folder(&self, folder: NewWatchedFolder) -> Result<i64> {
        ops::upsert_watched_folder(&self.pool, folder).await
    }

    pub async fn set_watched_folder_enabled(&self, playlist_id: i64, enabled: bool) -> Result<()> {
        ops::set_watched_folder_enabled(&self.pool, playlist_id, enabled).await
    }

    pub async fn touch_watched_folder_scan(&self, playlist_id: i64) -> Result<()> {
        ops::touch_watched_folder_scan(&self.pool, playlist_id).await
    }

    pub async fn delete_watched_folder_by_playlist(&self, playlist_id: i64) -> Result<()> {
        ops::delete_watched_folder_by_playlist(&self.pool, playlist_id).await
    }
}
