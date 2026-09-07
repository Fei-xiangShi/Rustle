//! Embedded SQLx migrations and staged legacy-schema routing.

use sqlx::SqlitePool;

use super::error::StorageError;
use super::{StorageResult as Result, schema};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Managed,
    Legacy,
}

impl SchemaState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Managed => "managed",
            Self::Legacy => "legacy",
        }
    }
}

pub(crate) async fn initialize(pool: &SqlitePool) -> Result<SchemaState> {
    let state = classify(pool).await?;
    match state {
        SchemaState::Empty | SchemaState::Managed => MIGRATOR.run(pool).await?,
        SchemaState::Legacy => schema::run_migrations(pool).await?,
    }

    let version = if state == SchemaState::Legacy {
        None
    } else {
        Some(current_version(pool).await?)
    };
    tracing::info!(
        event = "database_schema_ready",
        schema_state = state.as_str(),
        migration_version = version,
        legacy_compatibility = state == SchemaState::Legacy,
        "Database schema initialization completed"
    );
    Ok(state)
}

pub(crate) async fn classify(pool: &SqlitePool) -> Result<SchemaState> {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    if tables.is_empty() {
        Ok(SchemaState::Empty)
    } else if tables.iter().any(|name| name == "_sqlx_migrations") {
        Ok(SchemaState::Managed)
    } else {
        Ok(SchemaState::Legacy)
    }
}

async fn current_version(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| {
        StorageError::Migration(sqlx::migrate::MigrateError::VersionNotPresent(
            latest_version(),
        ))
    })
}

fn latest_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::database::connection;
    use sqlx::Row;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "rustle-migration-{name}-{}-{}",
                std::process::id(),
                TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("rustle.db");
            Self { directory, path }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    async fn schema_fingerprint(pool: &SqlitePool) -> Vec<String> {
        sqlx::query(
            "SELECT type, name, tbl_name, COALESCE(sql, '') AS sql \
             FROM sqlite_master \
             WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' \
             ORDER BY type, name",
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let object_type = row.get::<String, _>("type");
            let name = row.get::<String, _>("name");
            let table = row.get::<String, _>("tbl_name");
            let sql = row
                .get::<String, _>("sql")
                .replace("IF NOT EXISTS", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            format!("{object_type}|{name}|{table}|{sql}")
        })
        .collect()
    }

    #[tokio::test]
    async fn empty_database_migrates_to_the_canonical_schema_and_is_repeatable() {
        let database = TestDatabase::new("empty");
        let pool = connection::connect(&database.path).await.unwrap();

        assert_eq!(initialize(&pool).await.unwrap(), SchemaState::Empty);
        assert_eq!(classify(&pool).await.unwrap(), SchemaState::Managed);
        assert_eq!(initialize(&pool).await.unwrap(), SchemaState::Managed);

        let tables = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for expected in [
            "_sqlx_migrations",
            "downloads",
            "play_history",
            "playback_state",
            "playlist_songs",
            "playlists",
            "queue",
            "songs",
            "watched_folders",
        ] {
            assert!(
                tables.iter().any(|table| table == expected),
                "missing {expected}"
            );
        }
        let indexes = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master \
             WHERE type = 'index' AND name NOT LIKE 'sqlite_autoindex_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for expected in [
            "idx_downloads_downloaded_at",
            "idx_downloads_song_id",
            "idx_play_history_played_at",
            "idx_play_history_song",
            "idx_playlist_songs_playlist",
            "idx_queue_position",
            "idx_songs_album",
            "idx_songs_artist",
            "idx_songs_file_hash",
            "idx_songs_file_path",
            "idx_watched_folders_playlist",
        ] {
            assert!(
                indexes.iter().any(|index| index == expected),
                "missing {expected}"
            );
        }
        let playlist_foreign_keys = sqlx::query("PRAGMA foreign_key_list(playlist_songs)")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("table"),
                    row.get::<String, _>("on_delete"),
                )
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            playlist_foreign_keys,
            HashSet::from([
                ("playlists".to_string(), "CASCADE".to_string()),
                ("songs".to_string(), "CASCADE".to_string()),
            ])
        );
        let playback_rows = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM playback_state")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(playback_rows, 1);
        let migration_success =
            sqlx::query_scalar::<_, bool>("SELECT success FROM _sqlx_migrations WHERE version = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(migration_success);
        assert_eq!(current_version(&pool).await.unwrap(), latest_version());
        pool.close().await;
    }

    #[tokio::test]
    async fn changed_migration_checksum_is_rejected() {
        let database = TestDatabase::new("checksum");
        let pool = connection::connect(&database.path).await.unwrap();
        initialize(&pool).await.unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
            .execute(&pool)
            .await
            .unwrap();

        let error = initialize(&pool).await.unwrap_err();
        assert_eq!(
            error.code(),
            crate::error::ErrorCode::StorageMigrationFailed
        );
        assert!(matches!(
            error,
            StorageError::Migration(sqlx::migrate::MigrateError::VersionMismatch(1))
        ));
        pool.close().await;
    }

    #[tokio::test]
    async fn embedded_baseline_matches_the_legacy_canonical_schema() {
        let managed_database = TestDatabase::new("managed-parity");
        let managed_pool = connection::connect(&managed_database.path).await.unwrap();
        initialize(&managed_pool).await.unwrap();

        let legacy_database = TestDatabase::new("legacy-parity");
        let legacy_pool = connection::connect(&legacy_database.path).await.unwrap();
        schema::run_migrations(&legacy_pool).await.unwrap();

        assert_eq!(
            schema_fingerprint(&managed_pool).await,
            schema_fingerprint(&legacy_pool).await
        );
        let managed_playback_rows =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM playback_state")
                .fetch_one(&managed_pool)
                .await
                .unwrap();
        let legacy_playback_rows =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM playback_state")
                .fetch_one(&legacy_pool)
                .await
                .unwrap();
        assert_eq!(managed_playback_rows, legacy_playback_rows);

        managed_pool.close().await;
        legacy_pool.close().await;
    }

    #[tokio::test]
    async fn current_unversioned_schema_stays_on_the_legacy_route() {
        let database = TestDatabase::new("legacy");
        let pool = connection::connect(&database.path).await.unwrap();
        schema::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO playlists (name, created_at, updated_at) VALUES ('kept', 1, 1)")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(classify(&pool).await.unwrap(), SchemaState::Legacy);
        assert_eq!(initialize(&pool).await.unwrap(), SchemaState::Legacy);
        let playlist_name =
            sqlx::query_scalar::<_, String>("SELECT name FROM playlists WHERE name = 'kept'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(playlist_name, "kept");
        let ledger_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ledger_exists, 0);
        pool.close().await;
    }
}
