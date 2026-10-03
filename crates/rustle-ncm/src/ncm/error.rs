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

    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Self::Upstream(
                ncm_api_rs::NcmError::Api {
                    code: 406 | 429,
                    ..
                } | ncm_api_rs::NcmError::RateLimited(_)
            )
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
        if error.is_rate_limited() {
            return AppError::with_source(code, "音乐服务请求过于频繁，请稍后重试", error)
                .with_recovery(RecoveryHint::Retry);
        }
        if matches!(error, NcmError::PlaybackUnavailable { .. }) {
            return AppError::with_source(code, "未获取到可用的音频播放地址", error)
                .with_recovery(RecoveryHint::None);
        }
        if let NcmError::Upstream(ncm_api_rs::NcmError::Api {
            code: service_code, ..
        }) = &error
        {
            return AppError::with_source(
                code,
                format!("音乐服务返回错误（代码 {service_code}）"),
                error,
            )
            .with_recovery(RecoveryHint::None);
        }
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

    #[test]
    fn throttling_has_safe_feedback_and_a_retry_hint() {
        for code in [406, 429, 503] {
            let error = NcmError::Upstream(ncm_api_rs::NcmError::from_api(
                code,
                "private response".into(),
            ));
            assert!(error.is_rate_limited());
            let app: AppError = error.into();
            assert_eq!(app.recovery(), RecoveryHint::Retry);
            assert!(!app.to_string().contains("private response"));
        }
        assert!(
            !NcmError::Upstream(ncm_api_rs::NcmError::Api {
                code: 404,
                msg: String::new()
            })
            .is_rate_limited()
        );
    }

    #[test]
    fn unavailable_audio_is_distinct_from_a_server_rejection() {
        let unavailable: AppError = NcmError::PlaybackUnavailable { song_id: 7 }.into();
        assert_eq!(unavailable.user_summary(), "未获取到可用的音频播放地址");
        let rejected: AppError = NcmError::Upstream(ncm_api_rs::NcmError::Api {
            code: 403,
            msg: "private upstream payload".into(),
        })
        .into();
        assert!(rejected.user_summary().contains("403"));
        assert!(!rejected.user_summary().contains("private upstream payload"));
    }
}
