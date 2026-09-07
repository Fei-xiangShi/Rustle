//! Cloneable error envelope transported across application boundaries.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

pub use rustle_domain::error::{ErrorCategory, ErrorCode, RecoveryHint};

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
    use std::io;

    use super::*;

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
