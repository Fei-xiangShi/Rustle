//! Stable application-facing error categories, codes, and recovery policy.

use std::fmt;

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
    StorageMigrationFailed,
    StorageBackupFailed,
    StorageInsufficientSpace,
    StorageAdoptionFailed,
    StorageRecoveryFailed,
    StorageSchemaUnsupported,
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
    pub const ALL: [Self; 32] = [
        Self::OperationCancelled,
        Self::NetworkRequestFailed,
        Self::NetworkTimeout,
        Self::AuthenticationRequired,
        Self::AuthenticationExpired,
        Self::ProtocolInvalidResponse,
        Self::ProtocolUnsupported,
        Self::BusinessRejected,
        Self::StorageOpenFailed,
        Self::StorageMigrationFailed,
        Self::StorageBackupFailed,
        Self::StorageInsufficientSpace,
        Self::StorageAdoptionFailed,
        Self::StorageRecoveryFailed,
        Self::StorageSchemaUnsupported,
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
            Self::StorageMigrationFailed => "storage.migration_failed",
            Self::StorageBackupFailed => "storage.backup_failed",
            Self::StorageInsufficientSpace => "storage.insufficient_space",
            Self::StorageAdoptionFailed => "storage.adoption_failed",
            Self::StorageRecoveryFailed => "storage.recovery_failed",
            Self::StorageSchemaUnsupported => "storage.schema_unsupported",
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
            | Self::StorageMigrationFailed
            | Self::StorageBackupFailed
            | Self::StorageInsufficientSpace
            | Self::StorageAdoptionFailed
            | Self::StorageRecoveryFailed
            | Self::StorageSchemaUnsupported
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
            | Self::StorageMigrationFailed
            | Self::StorageBackupFailed
            | Self::StorageAdoptionFailed
            | Self::StorageRecoveryFailed
            | Self::StorageSchemaUnsupported
            | Self::InvariantViolation => RecoveryHint::ExportDiagnostics,
            Self::StorageInsufficientSpace => RecoveryHint::Retry,
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

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
}
