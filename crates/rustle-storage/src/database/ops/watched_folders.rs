//! Watched folder persistence for local library auto-sync

use crate::database::StorageResult as Result;
use sqlx::{Pool, Sqlite};

use super::current_timestamp;
use crate::database::{DbWatchedFolder, NewWatchedFolder};

/// Get all watched folders ordered by creation time.
pub async fn get_all_watched_folders(pool: &Pool<Sqlite>) -> Result<Vec<DbWatchedFolder>> {
    let folders = sqlx::query_as::<_, DbWatchedFolder>(
        "SELECT * FROM watched_folders ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await?;
    Ok(folders)
}

/// Get watched folder metadata linked to a local playlist.
pub async fn get_watched_folder_by_playlist(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
) -> Result<Option<DbWatchedFolder>> {
    let folder = sqlx::query_as::<_, DbWatchedFolder>(
        "SELECT * FROM watched_folders WHERE playlist_id = ? LIMIT 1",
    )
    .bind(playlist_id)
    .fetch_optional(pool)
    .await?;
    Ok(folder)
}

/// Get watched folder metadata by canonical path.
pub async fn get_watched_folder_by_path(
    pool: &Pool<Sqlite>,
    path: &str,
) -> Result<Option<DbWatchedFolder>> {
    let folder = sqlx::query_as::<_, DbWatchedFolder>(
        "SELECT * FROM watched_folders WHERE path = ? LIMIT 1",
    )
    .bind(path)
    .fetch_optional(pool)
    .await?;
    Ok(folder)
}

/// Insert or update a watched folder binding.
pub async fn upsert_watched_folder(pool: &Pool<Sqlite>, folder: NewWatchedFolder) -> Result<i64> {
    let now = current_timestamp();

    let result = sqlx::query(
        r#"
        INSERT INTO watched_folders (path, playlist_id, enabled, created_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(path) DO UPDATE SET
            playlist_id = excluded.playlist_id,
            enabled = excluded.enabled
        "#,
    )
    .bind(&folder.path)
    .bind(folder.playlist_id)
    .bind(folder.enabled)
    .bind(now)
    .execute(pool)
    .await?;

    if result.last_insert_rowid() != 0 {
        return Ok(result.last_insert_rowid());
    }

    let id = sqlx::query_scalar::<_, i64>("SELECT id FROM watched_folders WHERE path = ?")
        .bind(&folder.path)
        .fetch_one(pool)
        .await?;
    Ok(id)
}

/// Enable or disable an existing watched folder binding.
pub async fn set_watched_folder_enabled(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
    enabled: bool,
) -> Result<()> {
    sqlx::query("UPDATE watched_folders SET enabled = ? WHERE playlist_id = ?")
        .bind(enabled)
        .bind(playlist_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the last scan timestamp after processing a watcher event.
pub async fn touch_watched_folder_scan(pool: &Pool<Sqlite>, playlist_id: i64) -> Result<()> {
    sqlx::query("UPDATE watched_folders SET last_scanned = ? WHERE playlist_id = ?")
        .bind(current_timestamp())
        .bind(playlist_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove watched folder metadata when the linked playlist is deleted.
pub async fn delete_watched_folder_by_playlist(
    pool: &Pool<Sqlite>,
    playlist_id: i64,
) -> Result<()> {
    sqlx::query("DELETE FROM watched_folders WHERE playlist_id = ?")
        .bind(playlist_id)
        .execute(pool)
        .await?;
    Ok(())
}
