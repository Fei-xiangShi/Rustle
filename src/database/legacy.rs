//! Structural fingerprints for released unversioned Rustle databases.

use std::fmt;

use futures_util::future::BoxFuture;
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::StorageResult as Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LegacySchemaVersion {
    V1,
    V2,
    V3,
    V4,
    V5,
}

impl LegacySchemaVersion {
    pub(crate) const ALL: [Self; 5] = [Self::V1, Self::V2, Self::V3, Self::V4, Self::V5];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
            Self::V3 => "v3",
            Self::V4 => "v4",
            Self::V5 => "v5",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|version| version.as_str() == value)
    }

    pub(crate) fn upgrade_scripts(self) -> &'static [&'static str] {
        match self {
            Self::V1 => &LEGACY_UPGRADE_SQL,
            Self::V2 => &LEGACY_UPGRADE_SQL[1..],
            Self::V3 => &LEGACY_UPGRADE_SQL[2..],
            Self::V4 => &LEGACY_UPGRADE_SQL[3..],
            Self::V5 => &[],
        }
    }

    #[cfg(test)]
    pub(crate) const fn fixture_count(self) -> usize {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
            Self::V4 => 4,
            Self::V5 => 5,
        }
    }
}

const LEGACY_UPGRADE_SQL: [&str; 4] = [
    include_str!("../../migrations/legacy/v1_to_v2.sql"),
    include_str!("../../migrations/legacy/v2_to_v3.sql"),
    include_str!("../../migrations/legacy/v3_to_v4.sql"),
    include_str!("../../migrations/legacy/v4_to_v5.sql"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SchemaFingerprint {
    digest: u128,
    components: usize,
}

impl SchemaFingerprint {
    const fn new(digest: u128, components: usize) -> Self {
        Self { digest, components }
    }
}

impl fmt::Display for SchemaFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}-{}", self.digest, self.components)
    }
}

const KNOWN_LEGACY_SCHEMAS: [(LegacySchemaVersion, SchemaFingerprint); 5] = [
    (
        LegacySchemaVersion::V1,
        SchemaFingerprint::new(0x0478_7227_05d9_e766_3d88_2d95_d866_4653, 76),
    ),
    (
        LegacySchemaVersion::V2,
        SchemaFingerprint::new(0xbbb2_d24d_22ed_f680_2544_6f9d_ecac_e4cb, 77),
    ),
    (
        LegacySchemaVersion::V3,
        SchemaFingerprint::new(0x374c_f7f4_7093_6a2d_a232_f46f_e3a6_97e9, 80),
    ),
    (
        LegacySchemaVersion::V4,
        SchemaFingerprint::new(0x7a06_6da7_bd78_a1f5_b75a_3526_38a0_d991, 92),
    ),
    (
        LegacySchemaVersion::V5,
        SchemaFingerprint::new(0x2b10_2c71_5192_57ad_2b7e_194d_218c_dfd9, 93),
    ),
];

pub(crate) fn expected_fingerprint(version: LegacySchemaVersion) -> SchemaFingerprint {
    KNOWN_LEGACY_SCHEMAS
        .iter()
        .find_map(|(candidate, fingerprint)| (*candidate == version).then_some(*fingerprint))
        .expect("every typed legacy version has a locked fingerprint")
}

pub(crate) async fn identify(
    pool: SqlitePool,
) -> Result<(Option<LegacySchemaVersion>, SchemaFingerprint)> {
    let mut connection = pool.acquire().await?;
    identify_connection(&mut connection).await
}

pub(crate) fn identify_connection(
    connection: &mut SqliteConnection,
) -> BoxFuture<'_, Result<(Option<LegacySchemaVersion>, SchemaFingerprint)>> {
    Box::pin(async move {
        let fingerprint = fingerprint_connection(connection).await?;
        let version = KNOWN_LEGACY_SCHEMAS
            .iter()
            .find_map(|(version, expected)| (*expected == fingerprint).then_some(*version));
        Ok((version, fingerprint))
    })
}

#[cfg(test)]
pub(crate) async fn fingerprint(pool: SqlitePool) -> Result<SchemaFingerprint> {
    let mut connection = pool.acquire().await?;
    fingerprint_connection(&mut connection).await
}

pub(crate) fn fingerprint_connection(
    connection: &mut SqliteConnection,
) -> BoxFuture<'_, Result<SchemaFingerprint>> {
    Box::pin(async move {
        let tables = sqlx::query(
            "SELECT name, COALESCE(sql, '') AS sql FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name <> '_sqlx_migrations' ORDER BY name",
        )
        .fetch_all(&mut *connection)
        .await?;
        let mut components = Vec::new();

        for table in tables {
            let table_name = table.get::<String, _>("name");
            let table_sql = table.get::<String, _>("sql");
            components.push(format!("table|{table_name}"));

            let mut columns = sqlx::query(&format!(
                "PRAGMA table_info({})",
                quote_identifier(&table_name)
            ))
            .fetch_all(&mut *connection)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("name"),
                    row.get::<String, _>("type").trim().to_ascii_uppercase(),
                    row.get::<i64, _>("notnull"),
                    row.get::<Option<String>, _>("dflt_value")
                        .map(|value| normalize_sql(&value))
                        .unwrap_or_else(|| "<none>".to_string()),
                    row.get::<i64, _>("pk"),
                )
            })
            .collect::<Vec<_>>();
            columns.sort();
            for (name, data_type, not_null, default, primary_key) in columns {
                components.push(format!(
                    "column|{table_name}|{name}|{data_type}|{not_null}|{default}|{primary_key}"
                ));
            }

            let mut foreign_keys = sqlx::query(&format!(
                "PRAGMA foreign_key_list({})",
                quote_identifier(&table_name)
            ))
            .fetch_all(&mut *connection)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("from"),
                    row.get::<String, _>("table"),
                    row.get::<Option<String>, _>("to")
                        .unwrap_or_else(|| "<implicit>".to_string()),
                    row.get::<String, _>("on_update"),
                    row.get::<String, _>("on_delete"),
                    row.get::<String, _>("match"),
                )
            })
            .collect::<Vec<_>>();
            foreign_keys.sort();
            for (from, target_table, to, on_update, on_delete, match_kind) in foreign_keys {
                components.push(format!(
                "foreign_key|{table_name}|{from}|{target_table}|{to}|{on_update}|{on_delete}|{match_kind}"
            ));
            }

            let mut indexes = sqlx::query(&format!(
                "PRAGMA index_list({})",
                quote_identifier(&table_name)
            ))
            .fetch_all(&mut *connection)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("name"),
                    row.get::<i64, _>("unique"),
                    row.get::<String, _>("origin"),
                    row.get::<i64, _>("partial"),
                )
            })
            .collect::<Vec<_>>();
            indexes.sort();
            for (index_name, unique, origin, partial) in indexes {
                let mut index_columns = sqlx::query(&format!(
                    "PRAGMA index_info({})",
                    quote_identifier(&index_name)
                ))
                .fetch_all(&mut *connection)
                .await?
                .into_iter()
                .map(|row| {
                    (
                        row.get::<i64, _>("seqno"),
                        row.get::<Option<String>, _>("name")
                            .unwrap_or_else(|| "<expression>".to_string()),
                    )
                })
                .collect::<Vec<_>>();
                index_columns.sort_by_key(|(sequence, _)| *sequence);
                let column_names = index_columns
                    .into_iter()
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>()
                    .join(",");
                let identity = if origin == "c" {
                    index_name.clone()
                } else {
                    format!("<{origin}>")
                };
                let predicate = if partial == 1 {
                    sqlx::query_scalar::<_, String>(
                        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
                    )
                    .bind(&index_name)
                    .fetch_one(&mut *connection)
                    .await
                    .map(|sql| normalize_sql(&sql).replace("IF NOT EXISTS ", ""))?
                } else {
                    "<none>".to_string()
                };
                components.push(format!(
                "index|{table_name}|{identity}|{unique}|{origin}|{partial}|{column_names}|{predicate}"
            ));
            }

            let mut checks = extract_check_clauses(&table_sql);
            checks.sort();
            for check in checks {
                components.push(format!("check|{table_name}|{check}"));
            }
        }

        let extra_objects = sqlx::query(
            "SELECT type, name, COALESCE(sql, '') AS sql FROM sqlite_master \
         WHERE type IN ('trigger', 'view') AND name NOT LIKE 'sqlite_%' \
         ORDER BY type, name",
        )
        .fetch_all(&mut *connection)
        .await?;
        for object in extra_objects {
            components.push(format!(
                "object|{}|{}|{}",
                object.get::<String, _>("type"),
                object.get::<String, _>("name"),
                normalize_sql(&object.get::<String, _>("sql"))
            ));
        }

        components.sort();
        let signature = components.join("\n");
        Ok(SchemaFingerprint::new(
            xxhash_rust::xxh3::xxh3_128(signature.as_bytes()),
            components.len(),
        ))
    })
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_check_clauses(sql: &str) -> Vec<String> {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut checks = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("check") {
        let keyword = cursor + relative;
        let mut start = keyword + "check".len();
        while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
            start += 1;
        }
        if bytes.get(start) != Some(&b'(') {
            cursor = start;
            continue;
        }
        let mut depth = 0usize;
        let mut end = start;
        for (offset, byte) in bytes[start..].iter().copied().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = start + offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if end > start {
            checks.push(normalize_sql(&lower[keyword..end]));
            cursor = end;
        } else {
            break;
        }
    }
    checks
}

#[cfg(test)]
pub(crate) const LEGACY_FIXTURE_SQL: [&str; 5] = [
    include_str!("../../tests/fixtures/database/legacy_v1.sql"),
    include_str!("../../tests/fixtures/database/legacy_v2_delta.sql"),
    include_str!("../../tests/fixtures/database/legacy_v3_delta.sql"),
    include_str!("../../tests/fixtures/database/legacy_v4_delta.sql"),
    include_str!("../../tests/fixtures/database/legacy_v5_delta.sql"),
];

#[cfg(test)]
pub(crate) async fn apply_fixture(pool: &SqlitePool, version: LegacySchemaVersion) -> Result<()> {
    for script in LEGACY_FIXTURE_SQL.iter().take(version.fixture_count()) {
        sqlx::raw_sql(script).execute(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::database::connection;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "rustle-legacy-{name}-{}-{}",
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

    #[tokio::test]
    async fn released_fixtures_have_locked_fingerprints() {
        for (version, expected) in KNOWN_LEGACY_SCHEMAS {
            let database = TestDatabase::new(version.as_str());
            let pool = connection::connect(database.path.clone()).await.unwrap();
            apply_fixture(&pool, version).await.unwrap();
            let actual = fingerprint(pool.clone()).await.unwrap();
            assert_eq!(actual, expected, "changed {} fixture", version.as_str());
            assert_eq!(identify(pool.clone()).await.unwrap().0, Some(version));
            pool.close().await;
        }
    }

    #[tokio::test]
    async fn fresh_and_altered_v5_column_order_share_one_identity() {
        let altered_database = TestDatabase::new("altered-v5");
        let altered_pool = connection::connect(altered_database.path.clone())
            .await
            .unwrap();
        apply_fixture(&altered_pool, LegacySchemaVersion::V5)
            .await
            .unwrap();

        let fresh_database = TestDatabase::new("fresh-v5");
        let fresh_pool = connection::connect(fresh_database.path.clone())
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0001_canonical_schema.sql"))
            .execute(&fresh_pool)
            .await
            .unwrap();

        assert_eq!(
            fingerprint(altered_pool.clone()).await.unwrap(),
            fingerprint(fresh_pool.clone()).await.unwrap()
        );
        assert_eq!(
            identify(fresh_pool.clone()).await.unwrap().0,
            Some(LegacySchemaVersion::V5)
        );
        altered_pool.close().await;
        fresh_pool.close().await;
    }

    #[tokio::test]
    async fn structural_changes_are_not_accepted_as_released_schemas() {
        let mutations = [
            "ALTER TABLE songs ADD COLUMN unexpected TEXT",
            "DROP TABLE play_history",
            "DROP INDEX idx_songs_album",
            "CREATE TRIGGER fixture_trigger AFTER UPDATE ON songs \
             BEGIN UPDATE songs SET play_count = play_count WHERE id = NEW.id; END",
            "DROP INDEX idx_watched_folders_playlist; \
             CREATE UNIQUE INDEX idx_watched_folders_playlist \
             ON watched_folders(playlist_id) WHERE playlist_id > 0",
            "DROP INDEX idx_playlist_songs_playlist; \
             ALTER TABLE playlist_songs RENAME TO playlist_songs_old; \
             CREATE TABLE playlist_songs ( \
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 playlist_id INTEGER NOT NULL, \
                 song_id INTEGER NOT NULL, \
                 position INTEGER NOT NULL, \
                 added_at INTEGER NOT NULL, \
                 UNIQUE(playlist_id, song_id) \
             ); \
             INSERT INTO playlist_songs SELECT * FROM playlist_songs_old; \
             DROP TABLE playlist_songs_old; \
             CREATE INDEX idx_playlist_songs_playlist ON playlist_songs(playlist_id)",
            "DROP INDEX idx_downloads_song_id; \
             DROP INDEX idx_downloads_downloaded_at; \
             ALTER TABLE downloads RENAME TO downloads_old; \
             CREATE TABLE downloads ( \
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 song_id INTEGER NOT NULL, \
                 ncm_id INTEGER NOT NULL DEFAULT 0, \
                 title TEXT NOT NULL, \
                 artist TEXT NOT NULL DEFAULT '', \
                 file_path TEXT NOT NULL, \
                 file_size INTEGER NOT NULL DEFAULT 0, \
                 downloaded_at INTEGER NOT NULL \
             ); \
             INSERT INTO downloads (id, song_id, ncm_id, title, artist, file_path, file_size, downloaded_at) \
             SELECT id, song_id, ncm_id, title, artist, file_path, file_size, downloaded_at \
             FROM downloads_old; \
             DROP TABLE downloads_old; \
             CREATE INDEX idx_downloads_song_id ON downloads(song_id); \
             CREATE INDEX idx_downloads_downloaded_at ON downloads(downloaded_at)",
            "ALTER TABLE playback_state RENAME TO playback_state_old; \
             CREATE TABLE playback_state ( \
                 id INTEGER PRIMARY KEY CHECK (id >= 1), \
                 current_song_id INTEGER, \
                 queue_position INTEGER NOT NULL DEFAULT 0, \
                 position_secs REAL NOT NULL DEFAULT 0.0, \
                 volume REAL NOT NULL DEFAULT 1.0, \
                 shuffle INTEGER NOT NULL DEFAULT 0, \
                 repeat_mode INTEGER NOT NULL DEFAULT 0, \
                 updated_at INTEGER NOT NULL, \
                 personal_fm_mode INTEGER NOT NULL DEFAULT 0, \
                 FOREIGN KEY (current_song_id) REFERENCES songs(id) ON DELETE SET NULL \
             ); \
             INSERT INTO playback_state SELECT * FROM playback_state_old; \
             DROP TABLE playback_state_old",
        ];

        for (index, mutation) in mutations.into_iter().enumerate() {
            let database = TestDatabase::new(&format!("mutation-{index}"));
            let pool = connection::connect(database.path.clone()).await.unwrap();
            apply_fixture(&pool, LegacySchemaVersion::V5).await.unwrap();
            sqlx::raw_sql(mutation).execute(&pool).await.unwrap();

            assert_eq!(
                identify(pool.clone()).await.unwrap().0,
                None,
                "mutation {index} matched a released schema"
            );
            pool.close().await;
        }
    }

    #[test]
    fn fixture_sql_contains_only_synthetic_private_free_values() {
        for fixture in LEGACY_FIXTURE_SQL {
            let lowercase = fixture.to_ascii_lowercase();
            for forbidden in [
                "http://",
                "https://",
                "cookie",
                "music_u",
                "c:\\users\\",
                "/home/",
            ] {
                assert!(
                    !lowercase.contains(forbidden),
                    "fixture contains {forbidden}"
                );
            }
        }
    }
}
