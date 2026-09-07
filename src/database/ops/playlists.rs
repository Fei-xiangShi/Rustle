//! Playlist CRUD operations

use crate::database::StorageResult as Result;
use sqlx::{Pool, Sqlite};

use super::current_timestamp;
use crate::database::{DbPlaylist, DbPlaylistSongWithDate, DbSong, NewPlaylist};

/// Create a new playlist
pub async fn create_playlist(pool: &Pool<Sqlite>, playlist: NewPlaylist) -> Result<i64> {
    let now = current_timestamp();

    let result = sqlx::query(
        "INSERT INTO playlists (name, description, cover_path, is_smart, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&playlist.name)
    .bind(&playlist.description)
    .bind(&playlist.cover_path)
    .bind(playlist.is_smart)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Get all playlists
pub async fn get_all_playlists(pool: &Pool<Sqlite>) -> Result<Vec<DbPlaylist>> {
    let playlists = sqlx::query_as::<_, DbPlaylist>("SELECT * FROM playlists ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(playlists)
}

/// Get playlist by id
pub async fn get_playlist(pool: &Pool<Sqlite>, id: i64) -> Result<Option<DbPlaylist>> {
    let playlist = sqlx::query_as::<_, DbPlaylist>("SELECT * FROM playlists WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(playlist)
}

/// Add song to playlist
pub async fn add_song_to_playlist(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
    song_id: i64,
) -> Result<()> {
    let now = current_timestamp();

    // Get next position
    let max_pos: Option<i64> =
        sqlx::query_scalar("SELECT MAX(position) FROM playlist_songs WHERE playlist_id = ?")
            .bind(playlist_id)
            .fetch_one(pool)
            .await?;

    let position = max_pos.unwrap_or(-1) + 1;

    sqlx::query(
        "INSERT OR IGNORE INTO playlist_songs (playlist_id, song_id, position, added_at) VALUES (?, ?, ?, ?)",
    )
    .bind(playlist_id)
    .bind(song_id)
    .bind(position)
    .bind(now)
    .execute(pool)
    .await?;

    // Update playlist timestamp
    sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(playlist_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get songs in playlist
pub async fn get_playlist_songs(pool: &Pool<Sqlite>, playlist_id: i64) -> Result<Vec<DbSong>> {
    let songs = sqlx::query_as::<_, DbSong>(
        r#"
        SELECT s.* FROM songs s
        INNER JOIN playlist_songs ps ON s.id = ps.song_id
        WHERE ps.playlist_id = ? AND s.is_missing = 0
        ORDER BY ps.position
        "#,
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await?;
    Ok(songs)
}

/// Get songs in playlist with added_at date
pub async fn get_playlist_songs_with_date(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
) -> Result<Vec<DbPlaylistSongWithDate>> {
    let songs = sqlx::query_as::<_, DbPlaylistSongWithDate>(
        r#"
        SELECT s.*, ps.added_at FROM songs s
        INNER JOIN playlist_songs ps ON s.id = ps.song_id
        WHERE ps.playlist_id = ? AND s.is_missing = 0
        ORDER BY ps.position
        "#,
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await?;
    Ok(songs)
}

/// Remove song from playlist
pub async fn remove_song_from_playlist(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
    song_id: i64,
) -> Result<()> {
    sqlx::query("DELETE FROM playlist_songs WHERE playlist_id = ? AND song_id = ?")
        .bind(playlist_id)
        .bind(song_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete playlist
pub async fn delete_playlist(pool: &Pool<Sqlite>, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM playlists WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update playlist with all fields including cover
pub async fn update_playlist_full(
    pool: &Pool<Sqlite>,
    id: i64,
    name: &str,
    description: Option<&str>,
    cover_path: Option<&str>,
) -> Result<()> {
    let now = current_timestamp();
    sqlx::query("UPDATE playlists SET name = ?, description = ?, cover_path = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(description)
        .bind(cover_path)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
