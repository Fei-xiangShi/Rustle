use std::io;

use crate::error::{AppError, ErrorCode};

pub type DownloadResult<T> = Result<T, DownloadError>;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download request failed")]
    Request(#[from] reqwest::Error),
    #[error("download returned HTTP status {0}")]
    HttpStatus(reqwest::StatusCode),
    #[error("download filesystem operation `{operation}` failed")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("downloaded media could not be read")]
    MediaRead(#[from] lofty::error::FileParseError),
    #[error("downloaded media has zero duration")]
    ZeroDuration,
    #[error("downloaded media is empty")]
    Empty,
    #[error("downloaded media size {actual} does not match expected {expected}")]
    SizeMismatch { actual: u64, expected: u64 },
    #[error("downloaded media has an unsupported or damaged format")]
    UnsupportedFormat,
    #[error("downloaded media failed integrity validation")]
    Integrity {
        #[source]
        source: Box<DownloadError>,
    },
}

impl DownloadError {
    pub fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Request(error) if error.is_timeout() => ErrorCode::NetworkTimeout,
            Self::Request(_) | Self::HttpStatus(_) => ErrorCode::NetworkRequestFailed,
            Self::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied => {
                ErrorCode::StoragePermissionDenied
            }
            Self::Io { .. } => ErrorCode::StorageQueryFailed,
            Self::MediaRead(_) => ErrorCode::MediaReadFailed,
            Self::UnsupportedFormat => ErrorCode::MediaUnsupportedFormat,
            Self::ZeroDuration
            | Self::Empty
            | Self::SizeMismatch { .. }
            | Self::Integrity { .. } => ErrorCode::MediaIntegrityFailed,
        }
    }
}

impl From<DownloadError> for AppError {
    fn from(error: DownloadError) -> Self {
        let code = error.code();
        let summary = match code {
            ErrorCode::NetworkTimeout => "The download timed out",
            ErrorCode::NetworkRequestFailed => "The download request failed",
            ErrorCode::StoragePermissionDenied => "Rustle cannot write to the download folder",
            ErrorCode::StorageQueryFailed => "Rustle could not save the downloaded file",
            ErrorCode::MediaReadFailed
            | ErrorCode::MediaUnsupportedFormat
            | ErrorCode::MediaIntegrityFailed => "The downloaded audio file is invalid",
            _ => code.default_summary(),
        };
        AppError::with_source(code, summary, error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_failures_keep_network_storage_and_media_distinct() {
        let storage = DownloadError::io(
            "create destination",
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );
        assert_eq!(storage.code(), ErrorCode::StoragePermissionDenied);
        assert_eq!(DownloadError::Empty.code(), ErrorCode::MediaIntegrityFailed);
        assert_eq!(
            DownloadError::UnsupportedFormat.code(),
            ErrorCode::MediaUnsupportedFormat
        );
    }
}
