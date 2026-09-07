use std::io;

use crate::error::{AppError, ErrorCode};

pub type StorageResult<T> = Result<T, StorageError>;

/// Typed failure returned by Rustle's public storage adapter surface.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage filesystem operation failed")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed")]
    Sqlx(#[from] sqlx::Error),
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
    }
}
