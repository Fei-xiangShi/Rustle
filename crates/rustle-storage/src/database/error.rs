use std::io;

use rustle_application::error::{AppError, ErrorCode};

pub type StorageResult<T> = Result<T, StorageError>;

/// Typed failure returned by Rustle's public storage adapter surface.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage filesystem operation failed")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed")]
    Sqlx(#[from] sqlx::Error),
    #[error("database migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("database backup failed")]
    Backup {
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "insufficient space for database backup (required {required_bytes} bytes, available {available_bytes} bytes)"
    )]
    InsufficientSpace {
        required_bytes: u64,
        available_bytes: u64,
    },
    #[error("legacy database adoption failed for {legacy_version}")]
    Adoption {
        legacy_version: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("database recovery failed")]
    Recovery {
        #[source]
        source: anyhow::Error,
    },
    #[error("database schema is not a recognized Rustle release ({fingerprint})")]
    UnsupportedSchema { fingerprint: String },
    #[error("storage operation failed")]
    Operation(#[from] anyhow::Error),
    #[error("{entity} `{identity}` was not found")]
    NotFound {
        entity: &'static str,
        identity: String,
    },
    #[error("storage transaction `{operation}` failed")]
    Transaction {
        operation: &'static str,
        #[source]
        source: anyhow::Error,
    },
}

impl StorageError {
    pub fn not_found(entity: &'static str, identity: impl ToString) -> Self {
        Self::NotFound {
            entity,
            identity: identity.to_string(),
        }
    }

    pub fn transaction(operation: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self::Transaction {
            operation,
            source: source.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Io(source) if source.kind() == io::ErrorKind::PermissionDenied => {
                ErrorCode::StoragePermissionDenied
            }
            Self::Io(_) => ErrorCode::StorageOpenFailed,
            Self::Sqlx(sqlx::Error::RowNotFound) | Self::NotFound { .. } => {
                ErrorCode::StorageNotFound
            }
            Self::Transaction { .. } => ErrorCode::StorageTransactionFailed,
            Self::Migration(_) => ErrorCode::StorageMigrationFailed,
            Self::Backup { .. } => ErrorCode::StorageBackupFailed,
            Self::InsufficientSpace { .. } => ErrorCode::StorageInsufficientSpace,
            Self::Adoption { .. } => ErrorCode::StorageAdoptionFailed,
            Self::Recovery { .. } => ErrorCode::StorageRecoveryFailed,
            Self::UnsupportedSchema { .. } => ErrorCode::StorageSchemaUnsupported,
            Self::Sqlx(_) | Self::Operation(_) => ErrorCode::StorageQueryFailed,
        }
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        let code = error.code();
        AppError::with_source(code, code.default_summary(), error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn storage_errors_map_without_inspecting_display_text() {
        let denied = StorageError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "localized operating-system text",
        ));
        assert_eq!(denied.code(), ErrorCode::StoragePermissionDenied);

        let transaction =
            StorageError::transaction("save_queue", io::Error::other("inner database failure"));
        assert_eq!(transaction.code(), ErrorCode::StorageTransactionFailed);

        let app_error: AppError = transaction.into();
        assert!(app_error.source().is_some());
        assert_eq!(app_error.code(), ErrorCode::StorageTransactionFailed);

        let migration = StorageError::Migration(sqlx::migrate::MigrateError::VersionMismatch(1));
        assert_eq!(migration.code(), ErrorCode::StorageMigrationFailed);

        let unsupported = StorageError::UnsupportedSchema {
            fingerprint: "fixture".to_string(),
        };
        assert_eq!(unsupported.code(), ErrorCode::StorageSchemaUnsupported);

        let backup = StorageError::Backup {
            source: anyhow::anyhow!("private backup detail"),
        };
        assert_eq!(backup.code(), ErrorCode::StorageBackupFailed);
        assert_eq!(backup.to_string(), "database backup failed");

        let insufficient = StorageError::InsufficientSpace {
            required_bytes: 2,
            available_bytes: 1,
        };
        assert_eq!(insufficient.code(), ErrorCode::StorageInsufficientSpace);

        let adoption = StorageError::Adoption {
            legacy_version: "v1",
            source: anyhow::anyhow!("private adoption detail"),
        };
        assert_eq!(adoption.code(), ErrorCode::StorageAdoptionFailed);

        let recovery = StorageError::Recovery {
            source: anyhow::anyhow!("private recovery detail"),
        };
        assert_eq!(recovery.code(), ErrorCode::StorageRecoveryFailed);
        assert_eq!(recovery.to_string(), "database recovery failed");
    }
}
