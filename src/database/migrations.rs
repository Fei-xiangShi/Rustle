//! Embedded SQLx migrations and verified legacy-schema adoption routing.

use std::path::PathBuf;

use futures_util::future::BoxFuture;
use sqlx::SqlitePool;

use super::StorageResult as Result;
use super::error::StorageError;
use super::legacy::{self, LegacySchemaVersion, SchemaFingerprint};
use super::{adoption, connection};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Managed,
    Legacy(LegacySchemaVersion),
    UnknownLegacy(SchemaFingerprint),
}

impl SchemaState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Managed => "managed",
            Self::Legacy(_) => "legacy",
            Self::UnknownLegacy(_) => "unknown_legacy",
        }
    }
}

pub(crate) async fn initialize(pool: SqlitePool, database_path: PathBuf) -> Result<SchemaState> {
    let state = classify(pool.clone()).await?;
    match state {
        SchemaState::Empty | SchemaState::Managed => run_migrator(pool.clone()).await?,
        SchemaState::Legacy(version) => {
            pool.close().await;
            run_legacy_adoption(database_path.clone(), version).await?;
        }
        SchemaState::UnknownLegacy(fingerprint) => {
            tracing::error!(
                event = "database_schema_unsupported",
                schema_state = state.as_str(),
                schema_fingerprint = %fingerprint,
                "Database schema is not a recognized Rustle release"
            );
            return Err(StorageError::UnsupportedSchema {
                fingerprint: fingerprint.to_string(),
            });
        }
    }

    let version = if pool.is_closed() {
        Some(latest_version())
    } else {
        Some(current_version(pool.clone()).await?)
    };
    let legacy_version = match state {
        SchemaState::Legacy(version) => Some(version.as_str()),
        _ => None,
    };
    tracing::info!(
        event = "database_schema_ready",
        schema_state = state.as_str(),
        migration_version = version,
        legacy_version,
        legacy_adopted = matches!(state, SchemaState::Legacy(_)),
        "Database schema initialization completed"
    );
    Ok(state)
}

async fn run_legacy_adoption(database_path: PathBuf, version: LegacySchemaVersion) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| StorageError::Adoption {
                legacy_version: version.as_str(),
                source: source.into(),
            })?;
        runtime.block_on(async move {
            let pool = connection::connect(database_path.clone()).await?;
            let state = classify(pool.clone()).await?;
            if state != SchemaState::Legacy(version) {
                pool.close().await;
                return Err(StorageError::Adoption {
                    legacy_version: version.as_str(),
                    source: anyhow::anyhow!(
                        "legacy schema identity changed before maintenance startup: {state:?}"
                    ),
                });
            }
            let result = adoption::adopt(pool.clone(), database_path, version, &MIGRATOR)
                .await
                .map(|_| ());
            pool.close().await;
            result
        })
    })
    .await
    .map_err(|source| StorageError::Adoption {
        legacy_version: version.as_str(),
        source: source.into(),
    })?
}

pub(crate) async fn classify(pool: SqlitePool) -> Result<SchemaState> {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await?;

    if tables.is_empty() {
        Ok(SchemaState::Empty)
    } else if tables.iter().any(|name| name == "_sqlx_migrations") {
        Ok(SchemaState::Managed)
    } else {
        let (version, fingerprint) = legacy::identify(pool).await?;
        Ok(match version {
            Some(version) => SchemaState::Legacy(version),
            None => SchemaState::UnknownLegacy(fingerprint),
        })
    }
}

async fn current_version(pool: SqlitePool) -> Result<i64> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(&pool)
    .await?
    .ok_or_else(|| {
        StorageError::Migration(sqlx::migrate::MigrateError::VersionNotPresent(
            latest_version(),
        ))
    })
}

fn run_migrator(
    pool: SqlitePool,
) -> BoxFuture<'static, std::result::Result<(), sqlx::migrate::MigrateError>> {
    Box::pin(async move {
        let mut connection = pool.acquire().await?;
        MIGRATOR.run_direct(&mut *connection).await
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
    use crate::database::{connection, legacy};
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
        let pool = connection::connect(database.path.clone()).await.unwrap();

        assert_eq!(
            initialize(pool.clone(), database.path.clone())
                .await
                .unwrap(),
            SchemaState::Empty
        );
        assert_eq!(classify(pool.clone()).await.unwrap(), SchemaState::Managed);
        assert_eq!(
            initialize(pool.clone(), database.path.clone())
                .await
                .unwrap(),
            SchemaState::Managed
        );

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
        assert_eq!(
            current_version(pool.clone()).await.unwrap(),
            latest_version()
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn changed_migration_checksum_is_rejected() {
        let database = TestDatabase::new("checksum");
        let pool = connection::connect(database.path.clone()).await.unwrap();
        initialize(pool.clone(), database.path.clone())
            .await
            .unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
            .execute(&pool)
            .await
            .unwrap();

        let error = initialize(pool.clone(), database.path.clone())
            .await
            .unwrap_err();
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
        let managed_pool = connection::connect(managed_database.path.clone())
            .await
            .unwrap();
        initialize(managed_pool.clone(), managed_database.path.clone())
            .await
            .unwrap();

        let legacy_database = TestDatabase::new("legacy-parity");
        let legacy_pool = connection::connect(legacy_database.path.clone())
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0001_canonical_schema.sql"))
            .execute(&legacy_pool)
            .await
            .unwrap();

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
    async fn current_unversioned_schema_is_adopted_with_data_preserved() {
        let database = TestDatabase::new("legacy");
        let mut pool = connection::connect(database.path.clone()).await.unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0001_canonical_schema.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO playlists (name, created_at, updated_at) VALUES ('kept', 1, 1)")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            classify(pool.clone()).await.unwrap(),
            SchemaState::Legacy(LegacySchemaVersion::V5)
        );
        assert_eq!(
            initialize(pool.clone(), database.path.clone())
                .await
                .unwrap(),
            SchemaState::Legacy(LegacySchemaVersion::V5)
        );
        assert!(pool.is_closed());
        pool = connection::connect(database.path.clone()).await.unwrap();
        assert_eq!(classify(pool.clone()).await.unwrap(), SchemaState::Managed);
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
        assert_eq!(ledger_exists, 1);
        assert_eq!(
            initialize(pool.clone(), database.path.clone())
                .await
                .unwrap(),
            SchemaState::Managed
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn every_released_fixture_upgrades_without_losing_synthetic_data() {
        for version in LegacySchemaVersion::ALL {
            let database = TestDatabase::new(&format!("upgrade-{}", version.as_str()));
            let mut pool = connection::connect(database.path.clone()).await.unwrap();
            legacy::apply_fixture(&pool, version).await.unwrap();

            assert_eq!(
                initialize(pool.clone(), database.path.clone())
                    .await
                    .unwrap(),
                SchemaState::Legacy(version)
            );
            assert!(pool.is_closed());
            pool = connection::connect(database.path.clone()).await.unwrap();
            assert_eq!(classify(pool.clone()).await.unwrap(), SchemaState::Managed);
            assert_eq!(
                initialize(pool.clone(), database.path.clone())
                    .await
                    .unwrap(),
                SchemaState::Managed
            );

            let song_title =
                sqlx::query_scalar::<_, String>("SELECT title FROM songs WHERE id = 1")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            let playlist_relation = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM playlist_songs WHERE playlist_id = 1 AND song_id = 1",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            let playback_position = sqlx::query_scalar::<_, f64>(
                "SELECT position_secs FROM playback_state WHERE id = 1",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            let ledger_exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'table' AND name = '_sqlx_migrations'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();

            assert_eq!(song_title, "Fixture Song");
            assert_eq!(playlist_relation, 1);
            assert_eq!(playback_position, 12.5);
            assert_eq!(ledger_exists, 1);
            pool.close().await;
        }
    }

    #[tokio::test]
    async fn unknown_schema_is_rejected_before_any_ddl_or_data_change() {
        let database = TestDatabase::new("unknown");
        let pool = connection::connect(database.path.clone()).await.unwrap();
        legacy::apply_fixture(&pool, LegacySchemaVersion::V5)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE songs ADD COLUMN unexpected TEXT")
            .execute(&pool)
            .await
            .unwrap();
        let before = legacy::fingerprint(pool.clone()).await.unwrap();

        let error = initialize(pool.clone(), database.path.clone())
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            crate::error::ErrorCode::StorageSchemaUnsupported
        );
        assert!(matches!(error, StorageError::UnsupportedSchema { .. }));
        assert_eq!(legacy::fingerprint(pool.clone()).await.unwrap(), before);
        let song_title = sqlx::query_scalar::<_, String>("SELECT title FROM songs WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let ledger_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(song_title, "Fixture Song");
        assert_eq!(ledger_exists, 0);
        pool.close().await;
    }
}
