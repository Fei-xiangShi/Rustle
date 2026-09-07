//! Stable application-facing error categories, codes, and recovery policy.
//!
//! Adapter errors keep their concrete sources. They convert into [`AppError`]
//! only when crossing the application boundary, where Iced messages need a
//! cloneable payload and UI code needs deterministic recovery semantics.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// Coarse failure groups used for metrics, policy, and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    Cancelled,
    Network,
    Authentication,
    Protocol,
    Business,
    Storage,
    Media,
    Audio,
    Platform,
    Invariant,
}

/// Stable machine-readable failure identifier.
///
/// The dotted strings returned by [`ErrorCode::as_str`] are compatibility
/// surface. Presentation text may change; these identifiers may not be renamed
/// without an explicit migration review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    OperationCancelled,
    NetworkRequestFailed,
    NetworkTimeout,
    AuthenticationRequired,
    AuthenticationExpired,
    ProtocolInvalidResponse,
    ProtocolUnsupported,
    BusinessRejected,
    StorageOpenFailed,
    StorageQueryFailed,
    StorageTransactionFailed,
    StorageNotFound,
    StoragePermissionDenied,
    MediaReadFailed,
    MediaUnsupportedFormat,
    MediaIntegrityFailed,
    MediaMetadataFailed,
    AudioDeviceUnavailable,
    AudioSourceUnavailable,
    AudioStreamingFailed,
    AudioDecodeFailed,
    AudioControlUnavailable,
    PlatformUnsupported,
    PlatformConflict,
    PlatformUnavailable,
    InvariantViolation,
}

impl ErrorCode {
    /// Exhaustive stable code list used by compatibility tests and diagnostics.
    pub const ALL: [Self; 26] = [
        Self::OperationCancelled,
        Self::NetworkRequestFailed,
        Self::NetworkTimeout,
        Self::AuthenticationRequired,
        Self::AuthenticationExpired,
        Self::ProtocolInvalidResponse,
        Self::ProtocolUnsupported,
        Self::BusinessRejected,
        Self::StorageOpenFailed,
        Self::StorageQueryFailed,
        Self::StorageTransactionFailed,
        Self::StorageNotFound,
        Self::StoragePermissionDenied,
        Self::MediaReadFailed,
        Self::MediaUnsupportedFormat,
        Self::MediaIntegrityFailed,
        Self::MediaMetadataFailed,
        Self::AudioDeviceUnavailable,
        Self::AudioSourceUnavailable,
        Self::AudioStreamingFailed,
        Self::AudioDecodeFailed,
        Self::AudioControlUnavailable,
        Self::PlatformUnsupported,
        Self::PlatformConflict,
        Self::PlatformUnavailable,
        Self::InvariantViolation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OperationCancelled => "operation.cancelled",
            Self::NetworkRequestFailed => "network.request_failed",
            Self::NetworkTimeout => "network.timeout",
            Self::AuthenticationRequired => "authentication.required",
            Self::AuthenticationExpired => "authentication.expired",
            Self::ProtocolInvalidResponse => "protocol.invalid_response",
            Self::ProtocolUnsupported => "protocol.unsupported",
            Self::BusinessRejected => "business.rejected",
            Self::StorageOpenFailed => "storage.open_failed",
            Self::StorageQueryFailed => "storage.query_failed",
            Self::StorageTransactionFailed => "storage.transaction_failed",
            Self::StorageNotFound => "storage.not_found",
            Self::StoragePermissionDenied => "storage.permission_denied",
            Self::MediaReadFailed => "media.read_failed",
            Self::MediaUnsupportedFormat => "media.unsupported_format",
            Self::MediaIntegrityFailed => "media.integrity_failed",
            Self::MediaMetadataFailed => "media.metadata_failed",
            Self::AudioDeviceUnavailable => "audio.device_unavailable",
            Self::AudioSourceUnavailable => "audio.source_unavailable",
            Self::AudioStreamingFailed => "audio.streaming_failed",
            Self::AudioDecodeFailed => "audio.decode_failed",
            Self::AudioControlUnavailable => "audio.control_unavailable",
            Self::PlatformUnsupported => "platform.unsupported",
            Self::PlatformConflict => "platform.conflict",
            Self::PlatformUnavailable => "platform.unavailable",
            Self::InvariantViolation => "invariant.violation",
        }
    }

    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::OperationCancelled => ErrorCategory::Cancelled,
            Self::NetworkRequestFailed | Self::NetworkTimeout => ErrorCategory::Network,
            Self::AuthenticationRequired | Self::AuthenticationExpired => {
                ErrorCategory::Authentication
            }
            Self::ProtocolInvalidResponse | Self::ProtocolUnsupported => ErrorCategory::Protocol,
            Self::BusinessRejected => ErrorCategory::Business,
            Self::StorageOpenFailed
            | Self::StorageQueryFailed
            | Self::StorageTransactionFailed
            | Self::StorageNotFound
            | Self::StoragePermissionDenied => ErrorCategory::Storage,
            Self::MediaReadFailed
            | Self::MediaUnsupportedFormat
            | Self::MediaIntegrityFailed
            | Self::MediaMetadataFailed => ErrorCategory::Media,
            Self::AudioDeviceUnavailable
            | Self::AudioSourceUnavailable
            | Self::AudioStreamingFailed
            | Self::AudioDecodeFailed
            | Self::AudioControlUnavailable => ErrorCategory::Audio,
            Self::PlatformUnsupported | Self::PlatformConflict | Self::PlatformUnavailable => {
                ErrorCategory::Platform
            }
            Self::InvariantViolation => ErrorCategory::Invariant,
        }
    }

    pub const fn default_recovery(self) -> RecoveryHint {
        match self {
            Self::OperationCancelled => RecoveryHint::None,
            Self::NetworkRequestFailed | Self::NetworkTimeout => RecoveryHint::Retry,
            Self::AuthenticationRequired | Self::AuthenticationExpired => {
                RecoveryHint::Reauthenticate
            }
            Self::ProtocolInvalidResponse
            | Self::ProtocolUnsupported
            | Self::InvariantViolation => RecoveryHint::ExportDiagnostics,
            Self::BusinessRejected | Self::StorageNotFound => RecoveryHint::None,
            Self::StoragePermissionDenied
            | Self::StorageOpenFailed
            | Self::StorageQueryFailed
            | Self::StorageTransactionFailed
            | Self::MediaReadFailed
            | Self::MediaMetadataFailed => RecoveryHint::CheckPermissions,
            Self::MediaUnsupportedFormat | Self::MediaIntegrityFailed => RecoveryHint::None,
            Self::AudioDeviceUnavailable => RecoveryHint::ChooseDevice,
            Self::AudioSourceUnavailable
            | Self::AudioStreamingFailed
            | Self::AudioDecodeFailed
            | Self::AudioControlUnavailable => RecoveryHint::Retry,
            Self::PlatformUnsupported | Self::PlatformConflict => RecoveryHint::None,
            Self::PlatformUnavailable => RecoveryHint::Retry,
        }
    }

    pub const fn default_summary(self) -> &'static str {
        match self.category() {
            ErrorCategory::Cancelled => "The operation was cancelled",
            ErrorCategory::Network => "The network request failed",
            ErrorCategory::Authentication => "Authentication is required",
            ErrorCategory::Protocol => "The service returned an invalid response",
            ErrorCategory::Business => "The service rejected the operation",
            ErrorCategory::Storage => "The storage operation failed",
            ErrorCategory::Media => "The media operation failed",
            ErrorCategory::Audio => "The audio operation failed",
            ErrorCategory::Platform => "The platform operation failed",
            ErrorCategory::Invariant => "An internal invariant was violated",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Deterministic recovery behavior attached to an application error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryHint {
    None,
    Retry,
    Reauthenticate,
    ChooseDevice,
    CheckPermissions,
    ExportDiagnostics,
}

type SharedSource = Arc<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
struct MessageSource(String);

impl fmt::Display for MessageSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MessageSource {}

/// Cloneable error envelope transported through application messages.
#[derive(Clone)]
pub struct AppError {
    code: ErrorCode,
    recovery: RecoveryHint,
    user_summary: Arc<str>,
    source: Option<SharedSource>,
}

impl AppError {
    pub fn new(code: ErrorCode, user_summary: impl Into<Arc<str>>) -> Self {
        Self {
            code,
            recovery: code.default_recovery(),
            user_summary: user_summary.into(),
            source: None,
        }
    }

    pub fn from_code(code: ErrorCode) -> Self {
        Self::new(code, code.default_summary())
    }

    pub fn with_source<E>(code: ErrorCode, user_summary: impl Into<Arc<str>>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            code,
            recovery: code.default_recovery(),
            user_summary: user_summary.into(),
            source: Some(Arc::new(source)),
        }
    }

    /// Attach a legacy internal message after the owning boundary has selected
    /// a stable code. The message is retained as a source and is never parsed
    /// to classify the failure.
    pub fn with_message_source(
        code: ErrorCode,
        user_summary: impl Into<Arc<str>>,
        message: impl Into<String>,
    ) -> Self {
        Self::with_source(code, user_summary, MessageSource(message.into()))
    }

    pub fn with_recovery(mut self, recovery: RecoveryHint) -> Self {
        self.recovery = recovery;
        self
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub const fn category(&self) -> ErrorCategory {
        self.code.category()
    }

    pub const fn recovery(&self) -> RecoveryHint {
        self.recovery
    }

    pub fn user_summary(&self) -> &str {
        &self.user_summary
    }

    pub fn is_cancelled(&self) -> bool {
        self.category() == ErrorCategory::Cancelled
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.user_summary())
    }
}

impl fmt::Debug for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppError")
            .field("code", &self.code)
            .field("recovery", &self.recovery)
            .field("user_summary", &self.user_summary)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io;

    #[test]
    fn stable_codes_are_unique_and_have_complete_policy() {
        let mut identifiers = HashSet::new();
        let mut categories = HashSet::new();

        for code in ErrorCode::ALL {
            assert!(identifiers.insert(code.as_str()), "duplicate code: {code}");
            assert!(code.as_str().contains('.'));
            assert!(!code.default_summary().is_empty());
            categories.insert(code.category());
        }

        assert_eq!(categories.len(), 10);
    }

    #[test]
    fn application_error_keeps_source_but_safe_projections_hide_it() {
        let secret = "cookie=MUSIC_U=not-for-ui";
        let error = AppError::with_source(
            ErrorCode::NetworkRequestFailed,
            "Could not reach the music service",
            io::Error::other(secret),
        );

        assert_eq!(error.code(), ErrorCode::NetworkRequestFailed);
        assert_eq!(error.recovery(), RecoveryHint::Retry);
        assert_eq!(error.to_string(), "Could not reach the music service");
        assert!(!format!("{error:?}").contains(secret));
        assert_eq!(error.source().expect("source chain").to_string(), secret);
    }

    #[test]
    fn cancellation_is_silent_control_flow() {
        let error = AppError::from_code(ErrorCode::OperationCancelled);
        assert!(error.is_cancelled());
        assert_eq!(error.recovery(), RecoveryHint::None);
    }
}
