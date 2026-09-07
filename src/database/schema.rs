//! Database schema migrations

use sqlx::{Pool, Sqlite};

use super::StorageResult as Result;

/// Run database migrations to create/update schema
pub async fn run_migrations(pool: &Pool<Sqlite>) -> Result<()> {
    // Songs table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS songs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_path TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            artist TEXT NOT NULL DEFAULT 'Unknown Artist',
            album TEXT NOT NULL DEFAULT 'Unknown Album',
            duration_secs INTEGER NOT NULL DEFAULT 0,
            track_number INTEGER,
            year INTEGER,
            genre TEXT,
            cover_path TEXT,
            file_hash TEXT,
            file_size INTEGER NOT NULL DEFAULT 0,
            format TEXT,
            normalization_gain REAL,
            play_count INTEGER NOT NULL DEFAULT 0,
            last_played INTEGER,
            last_modified INTEGER NOT NULL,
            is_missing INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        
        CREATE INDEX IF NOT EXISTS idx_songs_file_path ON songs(file_path);
        CREATE INDEX IF NOT EXISTS idx_songs_artist ON songs(artist);
        CREATE INDEX IF NOT EXISTS idx_songs_album ON songs(album);
        CREATE INDEX IF NOT EXISTS idx_songs_file_hash ON songs(file_hash);
        "#,
    )
    .execute(pool)
    .await?;

    // Playlists table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS playlists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT,
            cover_path TEXT,
            is_smart INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Playlist songs junction table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS playlist_songs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            playlist_id INTEGER NOT NULL,
            song_id INTEGER NOT NULL,
            position INTEGER NOT NULL,
            added_at INTEGER NOT NULL,
            FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
            FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE,
            UNIQUE(playlist_id, song_id)
        );
        
        CREATE INDEX IF NOT EXISTS idx_playlist_songs_playlist ON playlist_songs(playlist_id);
        "#,
    )
    .execute(pool)
    .await?;

    // Queue table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            song_id INTEGER NOT NULL,
            position INTEGER NOT NULL,
            source_playlist_id INTEGER,
            FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE,
            FOREIGN KEY (source_playlist_id) REFERENCES playlists(id) ON DELETE SET NULL
        );
        
        CREATE INDEX IF NOT EXISTS idx_queue_position ON queue(position);
        "#,
    )
    .execute(pool)
    .await?;

    // Playback state table (singleton)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS playback_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            current_song_id INTEGER,
            queue_position INTEGER NOT NULL DEFAULT 0,
            position_secs REAL NOT NULL DEFAULT 0.0,
            volume REAL NOT NULL DEFAULT 1.0,
            shuffle INTEGER NOT NULL DEFAULT 0,
            repeat_mode INTEGER NOT NULL DEFAULT 0,
            personal_fm_mode INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (current_song_id) REFERENCES songs(id) ON DELETE SET NULL
        );
        
        -- Insert default playback state if not exists
        INSERT OR IGNORE INTO playback_state (id, queue_position, position_secs, volume, shuffle, repeat_mode, updated_at)
        VALUES (1, 0, 0.0, 1.0, 0, 0, 0);
        "#,
    )
    .execute(pool)
    .await?;

    // Play history table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS play_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            song_id INTEGER NOT NULL,
            played_at INTEGER NOT NULL,
            listened_secs INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE
        );
        
        CREATE INDEX IF NOT EXISTS idx_play_history_song ON play_history(song_id);
        CREATE INDEX IF NOT EXISTS idx_play_history_played_at ON play_history(played_at);
        "#,
    )
    .execute(pool)
    .await?;

    // Watched folders table for folder monitoring
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS watched_folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            playlist_id INTEGER,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_scanned INTEGER,
            created_at INTEGER NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Add new columns to songs table if they don't exist (migration)
    // SQLite doesn't support IF NOT EXISTS for columns, so we use a try approach
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN file_size INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN format TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN last_played INTEGER")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN normalization_gain REAL")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE songs ADD COLUMN is_missing INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE watched_folders ADD COLUMN playlist_id INTEGER")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "ALTER TABLE playback_state ADD COLUMN personal_fm_mode INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_watched_folders_playlist ON watched_folders(playlist_id) WHERE playlist_id IS NOT NULL",
    )
    .execute(pool)
    .await?;

    // Downloads history table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS downloads (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            song_id INTEGER NOT NULL,
            ncm_id INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            artist TEXT NOT NULL DEFAULT '',
            file_path TEXT NOT NULL,
            file_size INTEGER NOT NULL DEFAULT 0,
            quality TEXT NOT NULL DEFAULT '',
            downloaded_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_downloads_song_id ON downloads(song_id);
        CREATE INDEX IF NOT EXISTS idx_downloads_downloaded_at ON downloads(downloaded_at);
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
