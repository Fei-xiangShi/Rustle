//! Song CRUD operations

use crate::database::StorageResult as Result;
use sqlx::{Pool, Sqlite};

use super::current_timestamp;
use crate::database::{DbSong, NewSong};

/// Upsert a local song by storage path while preserving the existing row id.
pub async fn upsert_local_song(pool: &Pool<Sqlite>, song: NewSong) -> Result<i64> {
    let now = current_timestamp();

    if let Some(existing_id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM songs WHERE file_path = ?")
            .bind(&song.file_path)
            .fetch_optional(pool)
            .await?
    {
        sqlx::query(
            r#"
            UPDATE songs SET
                title = ?, artist = ?, album = ?, duration_secs = ?, track_number = ?,
                year = ?, genre = ?, cover_path = ?, file_hash = ?, file_size = ?,
                format = ?, normalization_gain = ?, last_modified = ?, is_missing = 0
            WHERE id = ?
            "#,
        )
        .bind(&song.title)
        .bind(&song.artist)
        .bind(&song.album)
        .bind(song.duration_secs)
        .bind(song.track_number)
        .bind(song.year)
        .bind(&song.genre)
        .bind(&song.cover_path)
        .bind(&song.file_hash)
        .bind(song.file_size)
        .bind(&song.format)
        .bind(song.normalization_gain)
        .bind(now)
        .bind(existing_id)
        .execute(pool)
        .await?;

        return Ok(existing_id);
    }

    let result = sqlx::query(
        r#"
        INSERT INTO songs (
            file_path, title, artist, album, duration_secs, track_number, year, genre,
            cover_path, file_hash, file_size, format, normalization_gain,
            last_modified, is_missing, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&song.file_path)
    .bind(&song.title)
    .bind(&song.artist)
    .bind(&song.album)
    .bind(song.duration_secs)
    .bind(song.track_number)
    .bind(song.year)
    .bind(&song.genre)
    .bind(&song.cover_path)
    .bind(&song.file_hash)
    .bind(song.file_size)
    .bind(&song.format)
    .bind(song.normalization_gain)
    .bind(now)
    .bind(false)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Upsert local songs by storage path in one transaction while preserving row ids.
pub async fn upsert_local_songs(pool: &Pool<Sqlite>, songs: Vec<NewSong>) -> Result<Vec<i64>> {
    if songs.is_empty() {
        return Ok(Vec::new());
    }

    let now = current_timestamp();
    let mut tx = pool.begin().await?;
    let mut ids = Vec::with_capacity(songs.len());

    for song in songs {
        sqlx::query(
            r#"
            INSERT INTO songs (
                file_path, title, artist, album, duration_secs, track_number, year, genre,
                cover_path, file_hash, file_size, format, normalization_gain,
                last_modified, is_missing, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
            ON CONFLICT(file_path) DO UPDATE SET
                title = excluded.title,
                artist = excluded.artist,
                album = excluded.album,
                duration_secs = excluded.duration_secs,
                track_number = excluded.track_number,
                year = excluded.year,
                genre = excluded.genre,
                cover_path = excluded.cover_path,
                file_hash = excluded.file_hash,
                file_size = excluded.file_size,
                format = excluded.format,
                normalization_gain = excluded.normalization_gain,
                last_modified = excluded.last_modified,
                is_missing = 0
            "#,
        )
        .bind(&song.file_path)
        .bind(&song.title)
        .bind(&song.artist)
        .bind(&song.album)
        .bind(song.duration_secs)
        .bind(song.track_number)
        .bind(song.year)
        .bind(&song.genre)
        .bind(&song.cover_path)
        .bind(&song.file_hash)
        .bind(song.file_size)
        .bind(&song.format)
        .bind(song.normalization_gain)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let id = sqlx::query_scalar::<_, i64>("SELECT id FROM songs WHERE file_path = ?")
            .bind(&song.file_path)
            .fetch_one(&mut *tx)
            .await?;

        ids.push(id);
    }

    tx.commit().await?;
    Ok(ids)
}

/// Get song by file path
pub async fn get_song_by_path(pool: &Pool<Sqlite>, path: &str) -> Result<Option<DbSong>> {
    let song = sqlx::query_as::<_, DbSong>("SELECT * FROM songs WHERE file_path = ?")
        .bind(path)
        .fetch_optional(pool)
        .await?;
    Ok(song)
}

/// Get all songs
pub async fn get_all_songs(pool: &Pool<Sqlite>) -> Result<Vec<DbSong>> {
    let songs = sqlx::query_as::<_, DbSong>(
        "SELECT * FROM songs WHERE is_missing = 0 ORDER BY artist, album, track_number",
    )
    .fetch_all(pool)
    .await?;
    Ok(songs)
}

/// Get all songs including rows whose local file is currently missing.
pub async fn get_all_songs_including_missing(pool: &Pool<Sqlite>) -> Result<Vec<DbSong>> {
    let songs =
        sqlx::query_as::<_, DbSong>("SELECT * FROM songs ORDER BY artist, album, track_number")
            .fetch_all(pool)
            .await?;
    Ok(songs)
}

/// Mark a local song as temporarily unavailable without deleting history.
pub async fn mark_song_missing_by_path(pool: &Pool<Sqlite>, path: &str) -> Result<()> {
    let now = current_timestamp();
    sqlx::query("UPDATE songs SET is_missing = 1, last_modified = ? WHERE file_path = ?")
        .bind(now)
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark a local song as available again when the file reappears.
pub async fn mark_song_available_by_path(pool: &Pool<Sqlite>, path: &str) -> Result<()> {
    let now = current_timestamp();
    sqlx::query("UPDATE songs SET is_missing = 0, last_modified = ? WHERE file_path = ?")
        .bind(now)
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update song file path (for handling file renames)
pub async fn update_song_path(pool: &Pool<Sqlite>, old_path: &str, new_path: &str) -> Result<()> {
    let now = super::current_timestamp();
    sqlx::query(
        "UPDATE songs SET file_path = ?, last_modified = ?, is_missing = 0 WHERE file_path = ?",
    )
    .bind(new_path)
    .bind(now)
    .bind(old_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update cover_path for a song by ID
pub async fn restore_song_cover(
    pool: &Pool<Sqlite>,
    song: &DbSong,
    cover_path: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE songs SET cover_path = ? WHERE id = ? AND file_path = ? AND cover_path IS ?",
    )
    .bind(cover_path)
    .bind(song.id)
    .bind(&song.file_path)
    .bind(song.cover_path.as_deref())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn update_song_cover(pool: &Pool<Sqlite>, song_id: i64, cover_path: &str) -> Result<()> {
    let now = super::current_timestamp();
    sqlx::query("UPDATE songs SET cover_path = ?, last_modified = ? WHERE id = ?")
        .bind(cover_path)
        .bind(now)
        .bind(song_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update cached normalization gain for a song identified by storage path.
pub async fn update_song_normalization(
    pool: &Pool<Sqlite>,
    song_id: i64,
    file_path: &str,
    normalization_gain: f64,
) -> Result<()> {
    if song_id > 0 {
        let result = sqlx::query("UPDATE songs SET normalization_gain = ? WHERE id = ?")
            .bind(normalization_gain)
            .bind(song_id)
            .execute(pool)
            .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    let lookup_path = if song_id < 0 {
        format!("ncm://{}", -song_id)
    } else {
        file_path.to_string()
    };

    sqlx::query("UPDATE songs SET normalization_gain = ? WHERE file_path = ?")
        .bind(normalization_gain)
        .bind(lookup_path)
        .execute(pool)
        .await?;

    Ok(())
}

/// Refresh descriptive metadata for any song (local or NCM) by ID.
/// Only updates fields that come from file tags / API, never computed fields.
pub async fn refresh_song_metadata(pool: &Pool<Sqlite>, song: &DbSong) -> Result<()> {
    let now = super::current_timestamp();

    sqlx::query(
        r#"
        UPDATE songs SET
            title = ?, artist = ?, album = ?, duration_secs = ?,
            track_number = ?, year = ?, genre = ?, cover_path = ?,
            format = ?, last_modified = ?
        WHERE id = ?
        "#,
    )
    .bind(&song.title)
    .bind(&song.artist)
    .bind(&song.album)
    .bind(song.duration_secs)
    .bind(song.track_number)
    .bind(song.year)
    .bind(song.genre.as_deref())
    .bind(
        song.cover_path
            .as_deref()
            .filter(|p| !p.starts_with("http")),
    )
    .bind(song.format.as_deref())
    .bind(now)
    .bind(song.id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert NCM song (insert if not exists, update if exists)
/// Returns the database ID of the song
///
/// cover_path should be a local file path, not a URL.
/// If cover_path starts with "http", it will be set to None.
pub async fn upsert_ncm_song(pool: &Pool<Sqlite>, song: &DbSong) -> Result<i64> {
    let now = super::current_timestamp();

    // Determine NCM ID from song.id (negative) or extract from file_path
    let ncm_id = if song.id < 0 {
        (-song.id) as u64
    } else if song.file_path.starts_with("ncm://") {
        song.file_path
            .trim_start_matches("ncm://")
            .parse()
            .unwrap_or(0)
    } else {
        song.id as u64
    };

    // NCM songs are identified by "ncm://<id>" in file_path for DB storage
    let file_path = format!("ncm://{}", ncm_id);

    // Only store local cover paths, not URLs
    let cover_path = song.cover_path.as_ref().and_then(|p| {
        if p.starts_with("http") {
            None
        } else {
            Some(p.clone())
        }
    });

    // Check if exists
    let existing = sqlx::query_scalar::<_, i64>("SELECT id FROM songs WHERE file_path = ?")
        .bind(&file_path)
        .fetch_optional(pool)
        .await?;

    if let Some(id) = existing {
        // Update - only update cover_path if we have a valid local path
        if cover_path.is_some() {
            sqlx::query(
                r#"
                UPDATE songs SET 
                    title = ?, artist = ?, album = ?, duration_secs = ?, 
                    cover_path = ?, last_modified = ?
                WHERE id = ?
                "#,
            )
            .bind(&song.title)
            .bind(&song.artist)
            .bind(&song.album)
            .bind(song.duration_secs)
            .bind(&cover_path)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
        } else {
            // Don't overwrite existing cover_path with None
            sqlx::query(
                r#"
                UPDATE songs SET 
                    title = ?, artist = ?, album = ?, duration_secs = ?, 
                    last_modified = ?
                WHERE id = ?
                "#,
            )
            .bind(&song.title)
            .bind(&song.artist)
            .bind(&song.album)
            .bind(song.duration_secs)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
        }

        Ok(id)
    } else {
        // Insert
        let result = sqlx::query(
            r#"
            INSERT INTO songs (
                file_path, title, artist, album, duration_secs, 
                cover_path, format, last_modified, created_at, play_count
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
            "#,
        )
        .bind(&file_path)
        .bind(&song.title)
        .bind(&song.artist)
        .bind(&song.album)
        .bind(song.duration_secs)
        .bind(&cover_path)
        .bind("ncm")
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Ok(result.last_insert_rowid())
    }
}

/// Upsert NCM song (transaction version for batch operations)
pub async fn upsert_ncm_song_tx(conn: &mut sqlx::SqliteConnection, song: &DbSong) -> Result<i64> {
    let now = super::current_timestamp();

    let ncm_id = if song.id < 0 {
        (-song.id) as u64
    } else if song.file_path.starts_with("ncm://") {
        song.file_path
            .trim_start_matches("ncm://")
            .parse()
            .unwrap_or(0)
    } else {
        song.id as u64
    };

    let file_path = format!("ncm://{}", ncm_id);

    // Keep cover_path as-is (can be URL or local path)
    // UI layer will handle URL vs local path distinction
    let cover_path = song.cover_path.clone();

    // Check if exists
    let existing = sqlx::query_scalar::<_, i64>("SELECT id FROM songs WHERE file_path = ?")
        .bind(&file_path)
        .fetch_optional(&mut *conn)
        .await?;

    if let Some(id) = existing {
        // Only update cover_path if we have a new value AND it's a local path
        // This prevents overwriting local path with URL
        let should_update_cover = cover_path
            .as_ref()
            .map(|p| !p.starts_with("http"))
            .unwrap_or(false);

        if should_update_cover {
            sqlx::query(
                r#"
                UPDATE songs SET 
                    title = ?, artist = ?, album = ?, duration_secs = ?, 
                    cover_path = ?, last_modified = ?
                WHERE id = ?
                "#,
            )
            .bind(&song.title)
            .bind(&song.artist)
            .bind(&song.album)
            .bind(song.duration_secs)
            .bind(&cover_path)
            .bind(now)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE songs SET 
                    title = ?, artist = ?, album = ?, duration_secs = ?, 
                    last_modified = ?
                WHERE id = ?
                "#,
            )
            .bind(&song.title)
            .bind(&song.artist)
            .bind(&song.album)
            .bind(song.duration_secs)
            .bind(now)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }

        Ok(id)
    } else {
        let result = sqlx::query(
            r#"
            INSERT INTO songs (
                file_path, title, artist, album, duration_secs, 
                cover_path, format, last_modified, created_at, play_count
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
            "#,
        )
        .bind(&file_path)
        .bind(&song.title)
        .bind(&song.artist)
        .bind(&song.album)
        .bind(song.duration_secs)
        .bind(&cover_path)
        .bind("ncm")
        .bind(now)
        .bind(now)
        .execute(&mut *conn)
        .await?;

        Ok(result.last_insert_rowid())
    }
}
