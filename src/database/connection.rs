//! SQLite connection and pool policy.

use std::io;
use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{SqliteConnection, SqlitePool};

const MAX_CONNECTIONS: u32 = 5;
pub(crate) const BUSY_TIMEOUT_MILLIS: u64 = 5_000;
pub(crate) const CACHE_SIZE_KIB: i64 = 32_000;

pub(crate) async fn connect(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MILLIS))
        .pragma("cache_size", format!("-{CACHE_SIZE_KIB}"));

    SqlitePoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .after_connect(|connection, _metadata| {
            Box::pin(async move { verify_connection_policy(connection).await })
        })
        .connect_with(options)
        .await
}

async fn verify_connection_policy(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let foreign_keys = pragma_i64(connection, "PRAGMA foreign_keys").await?;
    verify_value("foreign_keys", foreign_keys, 1)?;

    let journal_mode = sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
        .fetch_one(&mut *connection)
        .await?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(configuration_error(format!(
            "SQLite journal_mode must be WAL, got {journal_mode}"
        )));
    }

    let synchronous = pragma_i64(connection, "PRAGMA synchronous").await?;
    verify_value("synchronous", synchronous, 1)?;
    let busy_timeout = pragma_i64(connection, "PRAGMA busy_timeout").await?;
    verify_value("busy_timeout", busy_timeout, BUSY_TIMEOUT_MILLIS as i64)?;
    let cache_size = pragma_i64(connection, "PRAGMA cache_size").await?;
    verify_value("cache_size", cache_size, -CACHE_SIZE_KIB)?;
    Ok(())
}

async fn pragma_i64(
    connection: &mut SqliteConnection,
    statement: &'static str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(statement)
        .fetch_one(&mut *connection)
        .await
}

fn verify_value(name: &str, actual: i64, expected: i64) -> Result<(), sqlx::Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(configuration_error(format!(
            "SQLite {name} must be {expected}, got {actual}"
        )))
    }
}

fn configuration_error(message: String) -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(io::Error::other(message)))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "rustle-connection-policy-{}-{}",
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
    async fn every_pooled_connection_uses_the_sqlite_policy() {
        let database = TestDatabase::new();
        let pool = connect(&database.path).await.unwrap();
        let mut connections = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            connections.push(pool.acquire().await.unwrap());
        }

        for connection in &mut connections {
            let foreign_keys = sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
            let journal_mode = sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
            let synchronous = sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
            let busy_timeout = sqlx::query_scalar::<_, i64>("PRAGMA busy_timeout")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
            let cache_size = sqlx::query_scalar::<_, i64>("PRAGMA cache_size")
                .fetch_one(&mut **connection)
                .await
                .unwrap();

            assert_eq!(foreign_keys, 1);
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            assert_eq!(synchronous, 1);
            assert_eq!(busy_timeout, BUSY_TIMEOUT_MILLIS as i64);
            assert_eq!(cache_size, -CACHE_SIZE_KIB);
        }

        drop(connections);
        pool.close().await;
    }
}
