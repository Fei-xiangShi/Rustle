use std::error::Error;
use std::io;

use rustle_application::error::{AppError, ErrorCode, RecoveryHint};

pub type NcmResult<T> = Result<T, NcmError>;

/// Errors owned by Rustle's NetEase adapter.
#[derive(Debug, thiserror::Error)]
pub enum NcmError {
    #[error("NCM transport failed")]
    Upstream(#[from] ncm_api_rs::NcmError),
    #[error("NCM direct HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("NCM request `{operation}` timed out")]
    Timeout { operation: &'static str },
    #[error("NCM session I/O failed")]
    Io(#[from] io::Error),
    #[error("NCM JSON conversion failed")]
    Json(#[from] serde_json::Error),
    #[error("NCM protocol response is invalid: {0}")]
    Protocol(String),
    #[error("NCM protocol operation `{operation}` failed")]
    ProtocolSource {
        operation: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("NCM authentication is required: {0}")]
    Authentication(String),
    #[error("NCM returned no official playback URL for song {song_id}")]
    PlaybackUnavailable { song_id: u64 },
    #[error("NCM rejected the operation: {0}")]
    Business(String),
}

impl NcmError {
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }

    pub fn protocol_source<E>(operation: &'static str, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::ProtocolSource {
            operation,
            source: Box::new(source),
        }
    }

    pub fn business(message: impl Into<String>) -> Self {
        Self::Business(message.into())
    }

    pub(crate) fn is_authentication(&self) -> bool {
        matches!(
            self,
            Self::Upstream(ncm_api_rs::NcmError::AuthRequired(_)) | Self::Authentication(_)
        )
    }

    pub(crate) fn is_playback_auth_candidate(&self) -> bool {
        self.is_authentication() || matches!(self, Self::PlaybackUnavailable { .. })
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Upstream(ncm_api_rs::NcmError::Http(error)) if error.is_timeout() => {
                ErrorCode::NetworkTimeout
            }
            Self::Http(error) if error.is_timeout() => ErrorCode::NetworkTimeout,
            Self::Upstream(ncm_api_rs::NcmError::Http(_)) | Self::Http(_) => {
                ErrorCode::NetworkRequestFailed
            }
            Self::Upstream(ncm_api_rs::NcmError::Timeout(_)) => ErrorCode::NetworkTimeout,
            Self::Timeout { .. } => ErrorCode::NetworkTimeout,
            Self::Upstream(ncm_api_rs::NcmError::AuthRequired(_)) | Self::Authentication(_) => {
                ErrorCode::AuthenticationRequired
            }
            Self::Upstream(
                ncm_api_rs::NcmError::Api { .. } | ncm_api_rs::NcmError::RateLimited(_),
            )
            | Self::PlaybackUnavailable { .. }
            | Self::Business(_) => ErrorCode::BusinessRejected,
            Self::Io(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                ErrorCode::StoragePermissionDenied
            }
            Self::Io(_) => ErrorCode::StorageQueryFailed,
            Self::Upstream(
                ncm_api_rs::NcmError::InvalidParam(_)
                | ncm_api_rs::NcmError::Crypto(_)
                | ncm_api_rs::NcmError::Json(_)
                | ncm_api_rs::NcmError::Unknown(_),
            )
            | Self::Json(_)
            | Self::Protocol(_)
            | Self::ProtocolSource { .. } => ErrorCode::ProtocolInvalidResponse,
        }
    }
}

impl From<NcmError> for AppError {
    fn from(error: NcmError) -> Self {
        let code = error.code();
        let summary = match code {
            ErrorCode::AuthenticationRequired => "Please sign in to continue",
            ErrorCode::BusinessRejected => "The music service rejected the operation",
            ErrorCode::NetworkTimeout => "The music service request timed out",
            ErrorCode::NetworkRequestFailed => "Could not reach the music service",
            _ => code.default_summary(),
        };
        let app_error = AppError::with_source(code, summary, error);
        if code == ErrorCode::BusinessRejected {
            app_error.with_recovery(RecoveryHint::None)
        } else {
            app_error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_error_variants_map_by_type() {
        let auth = NcmError::Upstream(ncm_api_rs::NcmError::AuthRequired("expired".into()));
        let business = NcmError::Upstream(ncm_api_rs::NcmError::Api {
            code: 403,
            msg: "forbidden".into(),
        });
        let protocol = NcmError::Upstream(ncm_api_rs::NcmError::Crypto("bad payload".into()));

        assert_eq!(auth.code(), ErrorCode::AuthenticationRequired);
        assert_eq!(business.code(), ErrorCode::BusinessRejected);
        assert_eq!(protocol.code(), ErrorCode::ProtocolInvalidResponse);
    }
}
