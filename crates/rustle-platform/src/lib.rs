//! Platform abstraction layer
//!
//! This module provides unified interfaces for platform-specific functionality,
//! organized by feature with platform implementations inside each feature module.
//!
//! # Structure
//! - `tray/` - System tray functionality
//! - `media_controls/` - Media control integration (MPRIS on Linux)
//! - `window/` - Window behavior differences
//! - `theme.rs` - Platform-specific theme constants
//! - `keybindings.rs` - Keybinding display format

#[cfg(target_os = "linux")]
pub const APP_BINARY_NAME: &str = "rustle";
#[cfg(target_os = "linux")]
pub const APP_DISPLAY_NAME: &str = "Rustle";
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub const APP_ID: &str = "life.fxs.rustle";

pub mod discord;
pub mod global_hotkeys;
pub mod keybindings;
pub mod media_controls;
pub mod protocol;
pub mod runtime;
pub mod theme;
pub mod tray;
pub mod window;

pub fn init() {
    theme::configure_iced_font_system();
    window::initialize_process();
}

/// Open the parent directory of the given file path in the system file manager.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("failed to open the path in the system file manager")]
    FileManager(#[source] std::io::Error),
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    #[error("the system file manager is not supported on this platform")]
    Unsupported,
}

impl PlatformError {
    pub const fn code(&self) -> rustle_domain::error::ErrorCode {
        match self {
            Self::FileManager(_) => rustle_domain::error::ErrorCode::PlatformUnavailable,
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            Self::Unsupported => rustle_domain::error::ErrorCode::PlatformUnsupported,
        }
    }
}

impl From<PlatformError> for rustle_application::error::AppError {
    fn from(error: PlatformError) -> Self {
        let code = error.code();
        let summary = error.to_string();
        rustle_application::error::AppError::with_source(code, summary, error)
    }
}

pub fn open_in_file_manager(path: &std::path::Path) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        let dir = path.parent().unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map(|_| ())
            .map_err(PlatformError::FileManager)
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(PlatformError::FileManager)
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(PlatformError::FileManager)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = path;
        Err(PlatformError::Unsupported)
    }
}
