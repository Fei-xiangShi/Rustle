//! System tray abstraction.
//!
//! The platform backends intentionally have different ownership models:
//! Linux delegates to the StatusNotifierItem service, Windows owns Win32
//! objects on the Iced/Winit event-loop thread, and macOS owns its status item
//! on that same UI thread.

pub use rustle_application::tray::{
    TrayAvailability, TrayCommand, TrayPresentation, TrayWindowCommand,
};
use std::error::Error;
use std::fmt;
use std::sync::OnceLock;
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

const COMMAND_CHANNEL_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayErrorKind {
    AlreadyInitialized,
    NotInitialized,
    BackendUnavailable,
}

type BoxError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
pub struct TrayError {
    kind: TrayErrorKind,
    operation: Option<&'static str>,
    native_code: Option<u32>,
    source: Option<BoxError>,
}

impl TrayError {
    pub(crate) const fn already_initialized() -> Self {
        Self {
            kind: TrayErrorKind::AlreadyInitialized,
            operation: None,
            native_code: None,
            source: None,
        }
    }

    pub(crate) const fn not_initialized() -> Self {
        Self {
            kind: TrayErrorKind::NotInitialized,
            operation: None,
            native_code: None,
            source: None,
        }
    }

    pub(crate) fn backend<E>(operation: &'static str, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: TrayErrorKind::BackendUnavailable,
            operation: Some(operation),
            native_code: None,
            source: Some(Box::new(source)),
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) const fn native(operation: &'static str, native_code: u32) -> Self {
        Self {
            kind: TrayErrorKind::BackendUnavailable,
            operation: Some(operation),
            native_code: Some(native_code),
            source: None,
        }
    }

    pub const fn kind(&self) -> TrayErrorKind {
        self.kind
    }

    pub const fn code(&self) -> rustle_domain::error::ErrorCode {
        rustle_domain::error::ErrorCode::PlatformUnavailable
    }

    pub const fn operation(&self) -> Option<&'static str> {
        self.operation
    }

    pub const fn native_code(&self) -> Option<u32> {
        self.native_code
    }
}

impl fmt::Display for TrayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TrayErrorKind::AlreadyInitialized => "The system tray is already initialized",
            TrayErrorKind::NotInitialized => "The system tray is not initialized",
            TrayErrorKind::BackendUnavailable => "The system tray backend is unavailable",
        })
    }
}

impl Error for TrayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<TrayError> for rustle_application::error::AppError {
    fn from(error: TrayError) -> Self {
        let code = error.code();
        let summary = error.to_string();
        rustle_application::error::AppError::with_source(code, summary, error)
    }
}

pub(crate) trait TrayResultExt<T> {
    fn tray_context(self, operation: &'static str) -> Result<T, TrayError>;
}

impl<T, E> TrayResultExt<T> for Result<T, E>
where
    E: Error + Send + Sync + 'static,
{
    fn tray_context(self, operation: &'static str) -> Result<T, TrayError> {
        self.map_err(|source| TrayError::backend(operation, source))
    }
}

/// Lightweight application-side handle. Native objects never live here.
#[derive(Clone)]
pub struct TrayHandle {
    #[cfg(target_os = "linux")]
    handle: ksni::Handle<linux::LinuxTray>,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

#[cfg(target_os = "linux")]
impl TrayHandle {
    /// Submit the newest state to the asynchronous StatusNotifierItem service.
    pub fn update(&self, presentation: TrayPresentation) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            if handle
                .update(|tray| tray.update_state(presentation))
                .await
                .is_none()
            {
                tracing::warn!("Linux system tray service stopped before state update");
            }
        });
    }
}

#[cfg(target_os = "windows")]
impl TrayHandle {
    /// Apply the newest state synchronously on the Win32/Winit UI thread.
    pub fn update(&self, presentation: TrayPresentation) {
        if let Err(error) = windows::update_state(presentation) {
            tracing::warn!(%error, "Failed to update Windows system tray state");
        }
    }
}

#[cfg(target_os = "macos")]
impl TrayHandle {
    /// Apply the newest state synchronously on the AppKit main thread.
    pub fn update(&self, presentation: TrayPresentation) {
        if let Err(error) = macos::update_state(presentation) {
            tracing::warn!(%error, "Failed to update macOS status item state");
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
impl TrayHandle {
    pub fn update(&self, presentation: TrayPresentation) {
        let _ = presentation;
    }
}

pub type TrayResult = std::sync::Arc<tokio::sync::Mutex<mpsc::Receiver<TrayCommand>>>;

static TRAY_HANDLE: OnceLock<TrayHandle> = OnceLock::new();

pub fn get_handle() -> Option<&'static TrayHandle> {
    TRAY_HANDLE.get()
}

/// Confirm that the native recovery surface is currently registered.
///
/// Application state is still used for diagnostics, but the close-to-tray
/// path also consults this live value so a delayed/dropped lifecycle event
/// cannot hide the final window after the shell integration has failed.
pub fn is_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::is_available()
    }

    #[cfg(target_os = "macos")]
    {
        macos::is_available()
    }

    #[cfg(target_os = "linux")]
    {
        TRAY_HANDLE
            .get()
            .is_some_and(|handle| !handle.handle.is_closed())
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
pub async fn initialize(presentation: TrayPresentation) -> Result<TrayResult, TrayError> {
    let (handle, rx) = linux::start_linux_tray(presentation, COMMAND_CHANNEL_CAPACITY).await?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| TrayError::already_initialized())?;
    tracing::info!("System tray started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(target_os = "windows")]
pub fn initialize(presentation: TrayPresentation) -> Result<TrayResult, TrayError> {
    let (handle, rx) = windows::start_windows_tray(presentation, COMMAND_CHANNEL_CAPACITY)?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| TrayError::already_initialized())?;
    tracing::info!("Windows system tray started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(target_os = "macos")]
pub fn initialize(presentation: TrayPresentation) -> Result<TrayResult, TrayError> {
    let (handle, rx) = macos::start_macos_tray(presentation, COMMAND_CHANNEL_CAPACITY)?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| TrayError::already_initialized())?;
    tracing::info!("macOS status item started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn initialize(_presentation: TrayPresentation) -> Result<TrayResult, TrayError> {
    let (_tx, rx) = mpsc::channel(1);
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

/// Explicitly release native resources while the UI thread is still alive.
pub fn shutdown() {
    #[cfg(target_os = "windows")]
    windows::shutdown();

    #[cfg(target_os = "macos")]
    macos::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_errors_keep_sources_out_of_safe_display() {
        let error = TrayError::backend(
            "initialize test tray",
            std::io::Error::other("private native detail"),
        );

        assert_eq!(error.kind(), TrayErrorKind::BackendUnavailable);
        assert_eq!(error.operation(), Some("initialize test tray"));
        assert_eq!(
            error.code(),
            rustle_domain::error::ErrorCode::PlatformUnavailable
        );
        assert!(!error.to_string().contains("private native detail"));
        assert_eq!(
            error.source().expect("native source").to_string(),
            "private native detail"
        );
    }
}
