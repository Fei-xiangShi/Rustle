use std::error::Error;
use std::io;

use rustle_application::error::{AppError, ErrorCode};

pub type MediaResult<T> = Result<T, MediaError>;

/// Failures owned by the local-media adapter.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media file is not a supported audio file")]
    UnsupportedFormat,
    #[error("media file is empty or structurally invalid")]
    InvalidMedia,
    #[error("media file is empty")]
    EmptyMedia,
    #[error("media I/O operation `{operation}` failed")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("media metadata operation `{operation}` failed")]
    Metadata {
        operation: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("cover image operation `{operation}` failed")]
    Image {
        operation: &'static str,
        #[source]
        source: image::ImageError,
    },
    #[error("media watcher operation `{operation}` failed")]
    Watch {
        operation: &'static str,
        #[source]
        source: notify::Error,
    },
    #[error("audio file has no editable metadata tag")]
    MissingEditableTag,
    #[error("metadata year `{0}` is outside the supported range")]
    InvalidYear(u32),
}

impl MediaError {
    pub fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub fn metadata<E>(operation: &'static str, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Metadata {
            operation,
            source: Box::new(source),
        }
    }

    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::UnsupportedFormat => ErrorCode::MediaUnsupportedFormat,
            Self::InvalidMedia | Self::EmptyMedia => ErrorCode::MediaIntegrityFailed,
            Self::Metadata { .. } | Self::MissingEditableTag | Self::InvalidYear(_) => {
                ErrorCode::MediaMetadataFailed
            }
            Self::Io { .. } | Self::Image { .. } | Self::Watch { .. } => ErrorCode::MediaReadFailed,
        }
    }
}

impl From<MediaError> for AppError {
    fn from(error: MediaError) -> Self {
        let code = error.code();
        AppError::with_source(code, code.default_summary(), error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_media_error_maps_to_stable_media_policy() {
        assert_eq!(
            MediaError::UnsupportedFormat.code(),
            ErrorCode::MediaUnsupportedFormat
        );
        assert_eq!(
            MediaError::InvalidMedia.code(),
            ErrorCode::MediaIntegrityFailed
        );
        assert_eq!(
            MediaError::MissingEditableTag.code(),
            ErrorCode::MediaMetadataFailed
        );
        assert_eq!(
            MediaError::io("read", io::Error::other("private path")).code(),
            ErrorCode::MediaReadFailed
        );
    }
}
