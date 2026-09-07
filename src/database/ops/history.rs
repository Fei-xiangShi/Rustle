//! Play history operations

use crate::database::StorageResult as Result;
use sqlx::{Pool, Sqlite};

use super::current_timestamp;
use crate::database::DbSong;

/// Record a play event
/// For NCM songs (negative ID), first ensure the song exists in the database
pub async fn record_play(
    pool: &Pool<Sqlite>,
    song_id: i64,
    listened_secs: i64,
    completed: bool,
) -> Result<()> {
    let now = current_timestamp();

    // For NCM songs (negative ID), we need to find the actual database ID
    let actual_song_id = if song_id < 0 {
        let ncm_id = (-song_id) as u64;
        let file_path = format!("ncm://{}", ncm_id);

        // Try to find existing song by file_path
        let existing = sqlx::query_scalar::<_, i64>("SELECT id FROM songs WHERE file_path = ?")
            .bind(&file_path)
            .fetch_optional(pool)
            .await?;

        match existing {
            Some(id) => id,
            None => {
                // Song doesn't exist in DB yet, skip recording
                tracing::debug!("NCM song {} not in database, skipping play record", ncm_id);
                return Ok(());
            }
        }
    } else {
        song_id
    };

    sqlx::query(
        "INSERT INTO play_history (song_id, played_at, listened_secs, completed) VALUES (?, ?, ?, ?)",
    )
    .bind(actual_song_id)
    .bind(now)
    .bind(listened_secs)
    .bind(completed)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get recently played songs (unique songs ordered by most recent play)
/// For NCM songs (file_path starts with "ncm://"), the ID is converted to negative
pub async fn get_recently_played(pool: &Pool<Sqlite>, limit: i64) -> Result<Vec<DbSong>> {
    let mut songs = sqlx::query_as::<_, DbSong>(
        r#"
        SELECT s.* FROM songs s
        INNER JOIN (
            SELECT song_id, MAX(played_at) as last_played_at
            FROM play_history
            GROUP BY song_id
            ORDER BY last_played_at DESC
            LIMIT ?
        ) ph ON s.id = ph.song_id
        WHERE s.is_missing = 0
        ORDER BY ph.last_played_at DESC
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    // Convert NCM song IDs to negative for consistency with the app
    for song in &mut songs {
        if song.file_path.starts_with("ncm://")
            && let Ok(ncm_id) = song.file_path.trim_start_matches("ncm://").parse::<i64>()
        {
            song.id = -ncm_id;
        }
    }

    Ok(songs)
}
