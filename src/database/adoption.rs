//! Verified backup, transactional legacy adoption, and recovery.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::migrate::{Migration, Migrator};
use sqlx::{SqliteConnection, SqlitePool};

use super::StorageResult as Result;
use super::connection;
use super::error::StorageError;
use super::legacy::{self, LegacySchemaVersion, SchemaFingerprint};

const BACKUP_FORMAT_VERSION: u8 = 1;
const BACKUP_DIRECTORY: &str = "database-backups";
const BACKUP_PREFIX: &str = "rustle-pre-adoption-";
const BACKUP_SUFFIX: &str = ".sqlite3";
const MANIFEST_SUFFIX: &str = ".manifest.json";
const BASELINE_VERSION: i64 = 1;
const BACKUP_RETENTION_COUNT: usize = 3;
const SPACE_SAFETY_BYTES: u64 = 1024 * 1024;
static BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct BackupArtifact {
    database_path: PathBuf,
    manifest_path: PathBuf,
    manifest: BackupManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackupManifest {
    format_version: u8,
    application_version: String,
    source_schema: String,
    source_fingerprint: String,
    target_migration_version: i64,
    target_migration_checksum: String,
    created_unix_ms: u64,
    database_bytes: u64,
    database_xxh3_128: String,
}

#[cfg(not(test))]
#[derive(Debug, Clone, Copy, Default)]
struct AdoptionOptions {
    _private: (),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AdoptionOptions {
    pub(crate) available_space_override: Option<u64>,
    pub(crate) failpoint: Option<AdoptionFailpoint>,
    pub(crate) schema_change_after_backup: Option<&'static str>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdoptionFailpoint {
    AfterNormalize,
    AfterCommit,
}

enum TransactionFailure {
    RolledBack(anyhow::Error),
    RecoveryRequired(anyhow::Error),
}

pub(crate) async fn adopt(
    pool: SqlitePool,
    database_path: PathBuf,
    version: LegacySchemaVersion,
    migrator: &'static Migrator,
) -> Result<BackupArtifact> {
    adopt_with_options(
        pool,
        database_path,
        version,
        migrator,
        AdoptionOptions::default(),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn adopt_for_test(
    pool: SqlitePool,
    database_path: PathBuf,
    version: LegacySchemaVersion,
    migrator: &'static Migrator,
    options: AdoptionOptions,
) -> Result<BackupArtifact> {
    adopt_with_options(pool, database_path, version, migrator, options).await
}

async fn adopt_with_options(
    pool: SqlitePool,
    database_path: PathBuf,
    version: LegacySchemaVersion,
    migrator: &'static Migrator,
    options: AdoptionOptions,
) -> Result<BackupArtifact> {
    let baseline = baseline_migration(migrator)?;
    let expected = legacy::expected_fingerprint(version);
    let backup = create_backup(&pool, &database_path, version, expected, baseline, options).await?;

    tracing::info!(
        event = "database_backup_ready",
        legacy_version = version.as_str(),
        schema_fingerprint = %expected,
        backup_bytes = backup.manifest.database_bytes,
        backup_digest = %backup.manifest.database_xxh3_128,
        "Verified pre-adoption database backup"
    );

    #[cfg(test)]
    if let Some(statement) = options.schema_change_after_backup {
        sqlx::raw_sql(statement).execute(&pool).await?;
    }

    match apply_adoption_transaction(&pool, version, expected, baseline, options).await {
        Ok(()) => {}
        Err(TransactionFailure::RolledBack(source)) => {
            return Err(StorageError::Adoption {
                legacy_version: version.as_str(),
                source,
            });
        }
        Err(TransactionFailure::RecoveryRequired(source)) => {
            return recover_after_failure(pool, database_path, version, backup, baseline, source)
                .await;
        }
    }

    #[cfg(test)]
    if options.failpoint == Some(AdoptionFailpoint::AfterCommit) {
        return recover_after_failure(
            pool,
            database_path,
            version,
            backup,
            baseline,
            anyhow!("injected failure after adoption commit"),
        )
        .await;
    }

    if let Err(source) = run_migrator(pool.clone(), migrator)
        .await
        .map_err(anyhow::Error::from)
    {
        return recover_after_failure(pool, database_path, version, backup, baseline, source).await;
    }
    if let Err(source) = verify_managed_health(pool.clone(), migrator).await {
        return recover_after_failure(pool, database_path, version, backup, baseline, source).await;
    }

    tracing::info!(
        event = "database_legacy_adopted",
        legacy_version = version.as_str(),
        migration_version = latest_version(migrator),
        "Legacy database adopted into the SQLx migration ledger"
    );
    if prune_verified_backups(backup.clone(), baseline)
        .await
        .is_err()
    {
        tracing::warn!(
            event = "database_backup_retention_failed",
            error_code = "storage.backup_failed",
            "Could not prune old verified database backups"
        );
    }
    Ok(backup)
}

fn baseline_migration(migrator: &'static Migrator) -> Result<&'static Migration> {
    migrator
        .iter()
        .find(|migration| migration.version == BASELINE_VERSION)
        .ok_or_else(|| StorageError::Adoption {
            legacy_version: "unknown",
            source: anyhow!("embedded SQLx baseline migration is missing"),
        })
}

async fn create_backup(
    pool: &SqlitePool,
    database_path: &Path,
    version: LegacySchemaVersion,
    expected: SchemaFingerprint,
    baseline: &'static Migration,
    options: AdoptionOptions,
) -> Result<BackupArtifact> {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let backup_directory = parent.join(BACKUP_DIRECTORY);
    fs::create_dir_all(&backup_directory).map_err(backup_error)?;

    let required_bytes = required_backup_space(pool).await.map_err(backup_error)?;
    let available_bytes = available_space(&backup_directory, &options).map_err(backup_error)?;
    if available_bytes < required_bytes {
        return Err(StorageError::InsufficientSpace {
            required_bytes,
            available_bytes,
        });
    }

    let created_unix_ms = unix_millis();
    let counter = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let base_name = format!(
        "{BACKUP_PREFIX}{}-{created_unix_ms}-{}-{counter}",
        version.as_str(),
        std::process::id()
    );
    let backup_path = backup_directory.join(format!("{base_name}{BACKUP_SUFFIX}"));
    let manifest_path = backup_directory.join(format!("{base_name}{MANIFEST_SUFFIX}"));
    let backup_name = backup_path
        .to_str()
        .ok_or_else(|| backup_error(anyhow!("backup path is not valid UTF-8")))?;

    tracing::info!(
        event = "database_backup_started",
        legacy_version = version.as_str(),
        required_bytes,
        available_bytes,
        "Creating pre-adoption database backup"
    );
    if let Err(source) = sqlx::query("VACUUM INTO ?1")
        .bind(backup_name)
        .execute(pool)
        .await
    {
        remove_if_exists(&backup_path);
        return Err(backup_error(source));
    }

    if let Err(source) = verify_legacy_database(&backup_path, version, expected).await {
        remove_if_exists(&backup_path);
        return Err(backup_error(source));
    }
    let database_bytes = fs::metadata(&backup_path).map_err(backup_error)?.len();
    let database_xxh3_128 = hash_file(&backup_path).await.map_err(backup_error)?;
    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        source_schema: version.as_str().to_string(),
        source_fingerprint: expected.to_string(),
        target_migration_version: baseline.version,
        target_migration_checksum: bytes_to_hex(&baseline.checksum),
        created_unix_ms,
        database_bytes,
        database_xxh3_128,
    };

    if let Err(source) = write_manifest(&manifest_path, &manifest) {
        remove_if_exists(&backup_path);
        return Err(backup_error(source));
    }
    let artifact = BackupArtifact {
        database_path: backup_path,
        manifest_path,
        manifest,
    };
    if let Err(source) = verify_artifact(&artifact, version, expected, baseline).await {
        remove_if_exists(&artifact.manifest_path);
        remove_if_exists(&artifact.database_path);
        return Err(backup_error(source));
    }
    Ok(artifact)
}

async fn required_backup_space(pool: &SqlitePool) -> anyhow::Result<u64> {
    let page_count = sqlx::query_scalar::<_, i64>("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    let page_size = sqlx::query_scalar::<_, i64>("PRAGMA page_size")
        .fetch_one(pool)
        .await?;
    let page_count = u64::try_from(page_count).context("negative SQLite page count")?;
    let page_size = u64::try_from(page_size).context("negative SQLite page size")?;
    let logical_bytes = page_count
        .checked_mul(page_size)
        .context("SQLite backup size overflow")?;
    logical_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(SPACE_SAFETY_BYTES))
        .context("SQLite backup space requirement overflow")
}

fn available_space(path: &Path, options: &AdoptionOptions) -> anyhow::Result<u64> {
    #[cfg(test)]
    if let Some(bytes) = options.available_space_override {
        return Ok(bytes);
    }
    #[cfg(not(test))]
    let _ = options;
    Ok(fs4::available_space(path)?)
}

async fn apply_adoption_transaction(
    pool: &SqlitePool,
    version: LegacySchemaVersion,
    expected: SchemaFingerprint,
    baseline: &'static Migration,
    options: AdoptionOptions,
) -> std::result::Result<(), TransactionFailure> {
    #[cfg(not(test))]
    let _ = options;
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| TransactionFailure::RolledBack(error.into()))?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|error| TransactionFailure::RolledBack(error.into()))?;

    let identity = legacy::identify_connection(&mut connection).await;
    let (actual_version, actual_fingerprint) = match identity {
        Ok(identity) => identity,
        Err(source) => return rollback_transaction(&mut connection, source.into()).await,
    };
    if actual_version != Some(version) || actual_fingerprint != expected {
        return rollback_transaction(
            &mut connection,
            anyhow!(
                "legacy schema changed after backup: expected {} {}, found {:?} {}",
                version.as_str(),
                expected,
                actual_version,
                actual_fingerprint
            ),
        )
        .await;
    }

    for script in version.upgrade_scripts() {
        if let Err(source) = sqlx::raw_sql(script).execute(&mut *connection).await {
            return rollback_transaction(&mut connection, source.into()).await;
        }
    }

    let canonical = legacy::identify_connection(&mut connection).await;
    let (canonical_version, canonical_fingerprint) = match canonical {
        Ok(identity) => identity,
        Err(source) => return rollback_transaction(&mut connection, source.into()).await,
    };
    if canonical_version != Some(LegacySchemaVersion::V5) {
        return rollback_transaction(
            &mut connection,
            anyhow!("legacy normalization did not produce canonical V5 ({canonical_fingerprint})"),
        )
        .await;
    }

    #[cfg(test)]
    if options.failpoint == Some(AdoptionFailpoint::AfterNormalize) {
        return rollback_transaction(
            &mut connection,
            anyhow!("injected failure inside adoption transaction"),
        )
        .await;
    }

    if let Err(source) = sqlx::raw_sql(
        "CREATE TABLE _sqlx_migrations (\
             version BIGINT PRIMARY KEY,\
             description TEXT NOT NULL,\
             installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
             success BOOLEAN NOT NULL,\
             checksum BLOB NOT NULL,\
             execution_time BIGINT NOT NULL\
         )",
    )
    .execute(&mut *connection)
    .await
    {
        return rollback_transaction(&mut connection, source.into()).await;
    }
    if let Err(source) = sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version, description, success, checksum, execution_time) \
         VALUES (?1, ?2, TRUE, ?3, 0)",
    )
    .bind(baseline.version)
    .bind(&*baseline.description)
    .bind(&*baseline.checksum)
    .execute(&mut *connection)
    .await
    {
        return rollback_transaction(&mut connection, source.into()).await;
    }

    if let Err(commit) = sqlx::query("COMMIT").execute(&mut *connection).await {
        let rollback = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        return Err(TransactionFailure::RecoveryRequired(anyhow!(
            "adoption commit failed ({commit}); rollback result: {rollback:?}"
        )));
    }
    Ok(())
}

async fn rollback_transaction(
    connection: &mut SqliteConnection,
    source: anyhow::Error,
) -> std::result::Result<(), TransactionFailure> {
    match sqlx::query("ROLLBACK").execute(&mut *connection).await {
        Ok(_) => Err(TransactionFailure::RolledBack(source)),
        Err(rollback) => Err(TransactionFailure::RecoveryRequired(anyhow!(
            "adoption failed ({source}); rollback also failed ({rollback})"
        ))),
    }
}

async fn verify_managed_health(
    pool: SqlitePool,
    migrator: &'static Migrator,
) -> anyhow::Result<()> {
    verify_integrity(&pool).await?;
    if super::migrations::classify(pool.clone()).await? != super::migrations::SchemaState::Managed {
        return Err(anyhow!("adopted database is not classified as managed"));
    }
    let migration_version = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(&pool)
    .await?
    .context("managed database has no successful migration")?;
    let expected_version = latest_version(migrator);
    if migration_version != expected_version {
        return Err(anyhow!(
            "managed database is at migration {migration_version}, expected {expected_version}"
        ));
    }
    let playback_rows =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM playback_state WHERE id = 1")
            .fetch_one(&pool)
            .await?;
    if playback_rows != 1 {
        return Err(anyhow!(
            "canonical playback singleton is invalid ({playback_rows} rows)"
        ));
    }
    Ok(())
}

async fn verify_integrity(pool: &SqlitePool) -> anyhow::Result<()> {
    let quick_check = sqlx::query_scalar::<_, String>("PRAGMA quick_check")
        .fetch_all(pool)
        .await?;
    if quick_check.as_slice() != ["ok"] {
        return Err(anyhow!("SQLite quick_check failed: {quick_check:?}"));
    }
    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?;
    if !foreign_key_violations.is_empty() {
        return Err(anyhow!(
            "SQLite foreign_key_check reported {} violations",
            foreign_key_violations.len()
        ));
    }
    Ok(())
}

async fn verify_legacy_database(
    path: &Path,
    version: LegacySchemaVersion,
    expected: SchemaFingerprint,
) -> anyhow::Result<()> {
    let pool = connection::connect_read_only(path.to_path_buf()).await?;
    let result = async {
        verify_integrity(&pool).await?;
        let (actual_version, actual_fingerprint) = legacy::identify(pool.clone()).await?;
        if actual_version != Some(version) || actual_fingerprint != expected {
            return Err(anyhow!(
                "backup schema mismatch: expected {} {}, found {:?} {}",
                version.as_str(),
                expected,
                actual_version,
                actual_fingerprint
            ));
        }
        Ok(())
    }
    .await;
    pool.close().await;
    result
}

async fn verify_artifact(
    artifact: &BackupArtifact,
    version: LegacySchemaVersion,
    expected: SchemaFingerprint,
    baseline: &Migration,
) -> anyhow::Result<()> {
    let bytes = fs::read(&artifact.manifest_path)?;
    let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
    if manifest != artifact.manifest {
        return Err(anyhow!(
            "database backup manifest changed after publication"
        ));
    }
    if manifest.format_version != BACKUP_FORMAT_VERSION
        || manifest.source_schema != version.as_str()
        || manifest.source_fingerprint != expected.to_string()
        || manifest.target_migration_version != baseline.version
        || manifest.target_migration_checksum != bytes_to_hex(&baseline.checksum)
    {
        return Err(anyhow!("database backup manifest metadata mismatch"));
    }
    let metadata = fs::metadata(&artifact.database_path)?;
    if !metadata.is_file() || metadata.len() != manifest.database_bytes {
        return Err(anyhow!("database backup byte size mismatch"));
    }
    if hash_file(&artifact.database_path).await? != manifest.database_xxh3_128 {
        return Err(anyhow!("database backup digest mismatch"));
    }
    verify_legacy_database(&artifact.database_path, version, expected).await
}

async fn recover_after_failure(
    pool: SqlitePool,
    database_path: PathBuf,
    version: LegacySchemaVersion,
    backup: BackupArtifact,
    baseline: &'static Migration,
    adoption_source: anyhow::Error,
) -> Result<BackupArtifact> {
    tracing::error!(
        event = "database_legacy_adoption_failed",
        legacy_version = version.as_str(),
        error_code = "storage.adoption_failed",
        "Legacy database adoption failed; restoring verified backup"
    );
    pool.close().await;
    if let Err(recovery_source) = restore_backup(&database_path, version, &backup, baseline).await {
        return Err(StorageError::Recovery {
            source: anyhow!(
                "adoption failed ({adoption_source}); automatic recovery failed ({recovery_source})"
            ),
        });
    }
    tracing::info!(
        event = "database_backup_restored",
        legacy_version = version.as_str(),
        schema_fingerprint = %backup.manifest.source_fingerprint,
        "Restored verified pre-adoption database backup"
    );
    Err(StorageError::Adoption {
        legacy_version: version.as_str(),
        source: adoption_source,
    })
}

async fn restore_backup(
    database_path: &Path,
    version: LegacySchemaVersion,
    artifact: &BackupArtifact,
    baseline: &'static Migration,
) -> anyhow::Result<()> {
    let expected = legacy::expected_fingerprint(version);
    let manifest_bytes = fs::read(&artifact.manifest_path)?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest != artifact.manifest
        || manifest.target_migration_version != baseline.version
        || manifest.target_migration_checksum != bytes_to_hex(&baseline.checksum)
    {
        return Err(anyhow!(
            "backup manifest is not the verified adoption artifact"
        ));
    }
    let metadata = fs::metadata(&artifact.database_path)?;
    if metadata.len() != manifest.database_bytes
        || hash_file(&artifact.database_path).await? != manifest.database_xxh3_128
    {
        return Err(anyhow!("backup content no longer matches its manifest"));
    }
    verify_legacy_database(&artifact.database_path, version, expected).await?;

    let recovery_path = recovery_temp_path(database_path);
    copy_and_sync(&artifact.database_path, &recovery_path)
        .context("create and flush database recovery copy")?;
    if hash_file(&recovery_path).await? != manifest.database_xxh3_128 {
        return Err(anyhow!("recovery copy digest mismatch"));
    }
    retry_closed_file_operation(|| remove_sqlite_sidecar(database_path, "-wal"))
        .await
        .context("remove closed database WAL sidecar before recovery")?;
    retry_closed_file_operation(|| remove_sqlite_sidecar(database_path, "-shm"))
        .await
        .context("remove closed database shared-memory sidecar before recovery")?;
    retry_closed_file_operation(|| replace_closed_file(&recovery_path, database_path))
        .await
        .context("atomically publish recovered database")?;
    verify_legacy_database(database_path, version, expected).await
}

async fn prune_verified_backups(
    current: BackupArtifact,
    baseline: &'static Migration,
) -> anyhow::Result<()> {
    let directory = current
        .database_path
        .parent()
        .context("backup has no parent directory")?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let Some((version, timestamp)) = parse_owned_backup_name(&path) else {
            continue;
        };
        let manifest_path = manifest_path_for_database(&path);
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = match fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let manifest = match serde_json::from_slice::<BackupManifest>(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        let artifact = BackupArtifact {
            database_path: path,
            manifest_path,
            manifest,
        };
        if verify_retention_artifact(&artifact, version, baseline)
            .await
            .is_ok()
        {
            candidates.push((timestamp, artifact));
        }
    }
    candidates.sort_by_key(|(timestamp, _)| std::cmp::Reverse(*timestamp));

    let mut retained = 0usize;
    for (_, artifact) in candidates {
        let is_current = artifact.database_path == current.database_path;
        if is_current || retained < BACKUP_RETENTION_COUNT.saturating_sub(1) {
            retained += usize::from(!is_current);
            continue;
        }
        fs::remove_file(&artifact.database_path)?;
        fs::remove_file(&artifact.manifest_path)?;
    }
    Ok(())
}

async fn verify_retention_artifact(
    artifact: &BackupArtifact,
    version: LegacySchemaVersion,
    baseline: &'static Migration,
) -> anyhow::Result<()> {
    let expected = legacy::expected_fingerprint(version);
    if artifact.manifest.format_version != BACKUP_FORMAT_VERSION
        || artifact.manifest.source_schema != version.as_str()
        || artifact.manifest.source_fingerprint != expected.to_string()
        || artifact.manifest.target_migration_version != baseline.version
        || artifact.manifest.target_migration_checksum != bytes_to_hex(&baseline.checksum)
    {
        return Err(anyhow!("retention manifest metadata mismatch"));
    }
    let metadata = fs::metadata(&artifact.database_path)?;
    if metadata.len() != artifact.manifest.database_bytes
        || hash_file(&artifact.database_path).await? != artifact.manifest.database_xxh3_128
    {
        return Err(anyhow!("retention artifact content mismatch"));
    }
    verify_legacy_database(&artifact.database_path, version, expected).await
}

fn parse_owned_backup_name(path: &Path) -> Option<(LegacySchemaVersion, u64)> {
    let name = path.file_name()?.to_str()?;
    let body = name
        .strip_prefix(BACKUP_PREFIX)?
        .strip_suffix(BACKUP_SUFFIX)?;
    let mut parts = body.split('-');
    let version = LegacySchemaVersion::parse(parts.next()?)?;
    let timestamp = parts.next()?.parse().ok()?;
    let _process_id = parts.next()?.parse::<u32>().ok()?;
    let _counter = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((version, timestamp))
}

fn manifest_path_for_database(database_path: &Path) -> PathBuf {
    let name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .strip_suffix(BACKUP_SUFFIX)
        .unwrap_or_default();
    database_path.with_file_name(format!("{name}{MANIFEST_SUFFIX}"))
}

fn write_manifest(path: &Path, manifest: &BackupManifest) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup-manifest"),
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path)?;
        fs::remove_file(&temporary)
    })();
    if result.is_err() {
        remove_if_exists(&temporary);
    }
    Ok(result?)
}

async fn hash_file(path: &Path) -> io::Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(path)?;
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("{:032x}", hasher.digest128()))
    })
    .await
    .map_err(io::Error::other)?
}

fn copy_and_sync(source: &Path, destination: &Path) -> io::Result<()> {
    let mut source = File::open(source)?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut source, &mut destination)?;
    destination.sync_all()
}

fn recovery_temp_path(database_path: &Path) -> PathBuf {
    let counter = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rustle.db");
    database_path.with_file_name(format!(
        ".{name}.recovery.{}.{}.tmp",
        std::process::id(),
        counter
    ))
}

fn remove_sqlite_sidecar(database_path: &Path, suffix: &str) -> io::Result<()> {
    let mut sidecar = OsString::from(database_path.as_os_str());
    sidecar.push(suffix);
    match fs::remove_file(PathBuf::from(sidecar)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "windows")]
async fn retry_closed_file_operation(
    mut operation: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    const RETRY_DELAYS_MILLIS: [u64; 5] = [10, 25, 50, 100, 200];
    for delay in RETRY_DELAYS_MILLIS {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            Err(error) => return Err(error),
        }
    }
    operation()
}

#[cfg(not(target_os = "windows"))]
async fn retry_closed_file_operation(
    mut operation: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    operation()
}

#[cfg(target_os = "windows")]
fn replace_closed_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are NUL-terminated and remain allocated for the
    // synchronous call. SQLite handles are closed before this function runs.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_closed_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn latest_version(migrator: &Migrator) -> i64 {
    migrator
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or_default()
}

fn run_migrator(
    pool: SqlitePool,
    migrator: &'static Migrator,
) -> BoxFuture<'static, std::result::Result<(), sqlx::migrate::MigrateError>> {
    Box::pin(async move {
        let mut connection = pool.acquire().await?;
        migrator.run_direct(&mut *connection).await
    })
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn backup_error(source: impl Into<anyhow::Error>) -> StorageError {
    StorageError::Backup {
        source: source.into(),
    }
}

fn remove_if_exists(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(
            event = "database_backup_cleanup_failed",
            error_code = "storage.backup_failed",
            "Could not remove an incomplete owned backup artifact"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::database::legacy;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(name: &str) -> Self {
            let directory = loop {
                let directory = std::env::temp_dir().join(format!(
                    "rustle-adoption-{name}-{}-{}",
                    std::process::id(),
                    TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&directory) {
                    Ok(()) => break directory,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create test directory: {error}"),
                }
            };
            let path = directory.join("rustle.db");
            Self { directory, path }
        }

        fn backup_directory(&self) -> PathBuf {
            self.directory.join(BACKUP_DIRECTORY)
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    async fn fixture(name: &str, version: LegacySchemaVersion) -> (TestDatabase, SqlitePool) {
        let database = TestDatabase::new(name);
        let pool = connection::connect(database.path.clone()).await.unwrap();
        legacy::apply_fixture(&pool, version).await.unwrap();
        (database, pool)
    }

    async fn assert_legacy_source(path: &Path, version: LegacySchemaVersion, expected_title: &str) {
        let pool = connection::connect(path.to_path_buf()).await.unwrap();
        assert_eq!(
            legacy::identify(pool.clone()).await.unwrap().0,
            Some(version)
        );
        let title = sqlx::query_scalar::<_, String>("SELECT title FROM songs WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let ledger = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(title, expected_title);
        assert_eq!(ledger, 0);
        pool.close().await;
    }

    #[tokio::test]
    async fn online_backup_includes_wal_rows_and_has_private_verified_manifest() {
        let (database, pool) = fixture("wal", LegacySchemaVersion::V5).await;
        sqlx::query(
            "INSERT INTO playlists (name, created_at, updated_at) VALUES ('WAL Fixture', 2, 2)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let artifact = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V5,
            &MIGRATOR,
            AdoptionOptions::default(),
        )
        .await
        .unwrap();

        let backup = connection::connect_read_only(artifact.database_path.clone())
            .await
            .unwrap();
        let row = sqlx::query_scalar::<_, String>(
            "SELECT name FROM playlists WHERE name = 'WAL Fixture'",
        )
        .fetch_one(&backup)
        .await
        .unwrap();
        let ledger = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&backup)
        .await
        .unwrap();
        assert_eq!(row, "WAL Fixture");
        assert_eq!(ledger, 0);
        backup.close().await;

        let manifest = fs::read_to_string(&artifact.manifest_path).unwrap();
        let database_directory = database.directory.to_string_lossy().into_owned();
        for forbidden in [
            database_directory.as_str(),
            "Fixture Song",
            "WAL Fixture",
            "http://",
            "https://",
            "cookie",
        ] {
            assert!(!manifest.contains(forbidden));
        }
        assert_eq!(
            super::super::migrations::classify(pool.clone())
                .await
                .unwrap(),
            super::super::migrations::SchemaState::Managed
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn insufficient_space_and_backup_directory_failure_precede_mutation() {
        let (database, pool) = fixture("space", LegacySchemaVersion::V1).await;
        let before = legacy::fingerprint(pool.clone()).await.unwrap();
        let error = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V1,
            &MIGRATOR,
            AdoptionOptions {
                available_space_override: Some(0),
                ..AdoptionOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, StorageError::InsufficientSpace { .. }));
        assert_eq!(legacy::fingerprint(pool.clone()).await.unwrap(), before);

        let blocked = database.backup_directory();
        fs::remove_dir_all(&blocked).unwrap();
        fs::write(&blocked, b"not a directory").unwrap();
        let error = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V1,
            &MIGRATOR,
            AdoptionOptions::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), crate::error::ErrorCode::StorageBackupFailed);
        assert_eq!(legacy::fingerprint(pool.clone()).await.unwrap(), before);
        pool.close().await;
    }

    #[tokio::test]
    async fn interrupted_transaction_rolls_back_and_keeps_verified_backup() {
        let (database, pool) = fixture("rollback", LegacySchemaVersion::V1).await;
        let error = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V1,
            &MIGRATOR,
            AdoptionOptions {
                failpoint: Some(AdoptionFailpoint::AfterNormalize),
                ..AdoptionOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code(),
            crate::error::ErrorCode::StorageAdoptionFailed,
            "{error:?}"
        );
        assert_eq!(
            legacy::identify(pool.clone()).await.unwrap().0,
            Some(LegacySchemaVersion::V1)
        );
        let owned = fs::read_dir(database.backup_directory())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .count();
        assert_eq!(owned, 2);
        pool.close().await;
    }

    #[tokio::test]
    async fn post_commit_failure_restores_original_legacy_database() {
        let (database, pool) = fixture("restore", LegacySchemaVersion::V2).await;
        let error = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V2,
            &MIGRATOR,
            AdoptionOptions {
                failpoint: Some(AdoptionFailpoint::AfterCommit),
                ..AdoptionOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code(),
            crate::error::ErrorCode::StorageAdoptionFailed,
            "{error:?}"
        );
        assert!(pool.is_closed());
        assert_legacy_source(&database.path, LegacySchemaVersion::V2, "Fixture Song").await;
        assert_eq!(
            fs::read_dir(database.backup_directory())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn schema_change_after_backup_is_rejected_before_adoption_ddl() {
        let (database, pool) = fixture("changed", LegacySchemaVersion::V5).await;
        let error = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V5,
            &MIGRATOR,
            AdoptionOptions {
                schema_change_after_backup: Some(
                    "ALTER TABLE songs ADD COLUMN concurrent_change TEXT",
                ),
                ..AdoptionOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), crate::error::ErrorCode::StorageAdoptionFailed);
        assert_eq!(legacy::identify(pool.clone()).await.unwrap().0, None);
        let ledger = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ledger, 0);
        pool.close().await;
    }

    #[tokio::test]
    async fn corrupted_backup_or_manifest_is_refused_before_replacement() {
        let (database, pool) = fixture("corrupt", LegacySchemaVersion::V5).await;
        let artifact = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V5,
            &MIGRATOR,
            AdoptionOptions::default(),
        )
        .await
        .unwrap();
        let baseline = baseline_migration(&MIGRATOR).unwrap();

        let mut backup = OpenOptions::new()
            .append(true)
            .open(&artifact.database_path)
            .unwrap();
        backup.write_all(b"corruption").unwrap();
        backup.sync_all().unwrap();
        assert!(
            restore_backup(&database.path, LegacySchemaVersion::V5, &artifact, baseline)
                .await
                .is_err()
        );

        fs::write(&artifact.manifest_path, b"{}").unwrap();
        assert!(
            restore_backup(&database.path, LegacySchemaVersion::V5, &artifact, baseline)
                .await
                .is_err()
        );
        assert_eq!(
            super::super::migrations::classify(pool.clone())
                .await
                .unwrap(),
            super::super::migrations::SchemaState::Managed
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn retention_keeps_current_and_touches_only_verified_owned_pairs() {
        let (database, pool) = fixture("retention", LegacySchemaVersion::V5).await;
        let current = adopt_for_test(
            pool.clone(),
            database.path.clone(),
            LegacySchemaVersion::V5,
            &MIGRATOR,
            AdoptionOptions::default(),
        )
        .await
        .unwrap();
        let unrelated = database.backup_directory().join("keep-me.txt");
        fs::write(&unrelated, b"unrelated").unwrap();

        for timestamp in 1..=5u64 {
            let base = format!(
                "{BACKUP_PREFIX}v5-{timestamp}-{}-{timestamp}",
                std::process::id()
            );
            fs::copy(
                &current.database_path,
                database
                    .backup_directory()
                    .join(format!("{base}{BACKUP_SUFFIX}")),
            )
            .unwrap();
            fs::copy(
                &current.manifest_path,
                database
                    .backup_directory()
                    .join(format!("{base}{MANIFEST_SUFFIX}")),
            )
            .unwrap();
        }

        prune_verified_backups(current.clone(), baseline_migration(&MIGRATOR).unwrap())
            .await
            .unwrap();
        let owned = fs::read_dir(database.backup_directory())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| parse_owned_backup_name(&entry.path()).is_some())
            .count();
        assert_eq!(owned, BACKUP_RETENTION_COUNT);
        assert!(current.database_path.exists());
        assert!(current.manifest_path.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"unrelated");
        pool.close().await;
    }

    #[test]
    fn owned_backup_name_parser_is_strict() {
        let valid = Path::new("rustle-pre-adoption-v3-10-20-30.sqlite3");
        assert_eq!(
            parse_owned_backup_name(valid),
            Some((LegacySchemaVersion::V3, 10))
        );
        for invalid in [
            "rustle-pre-adoption-v6-10-20-30.sqlite3",
            "rustle-pre-adoption-v3-10-20.sqlite3",
            "rustle-pre-adoption-v3-10-20-30.sqlite3.bak",
            "prefix-rustle-pre-adoption-v3-10-20-30.sqlite3",
            "rustle-pre-adoption-v3-ten-20-30.sqlite3",
        ] {
            assert_eq!(parse_owned_backup_name(Path::new(invalid)), None);
        }
    }
}
