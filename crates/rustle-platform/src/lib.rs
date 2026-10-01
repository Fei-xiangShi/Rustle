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
        windows_file_manager::reveal(path).map_err(PlatformError::FileManager)
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

#[cfg(target_os = "windows")]
mod windows_file_manager {
    use std::{io, os::windows::ffi::OsStrExt, path::Path};
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems},
    };

    pub(super) fn reveal(path: &Path) -> io::Result<()> {
        // Canonicalize existing files, then remove the Win32 verbatim prefix
        // which the shell namespace does not consistently accept.
        let canonical = path.canonicalize()?;
        let wide = shell_path(&canonical)?;
        // SAFETY: NUL-terminated UTF-16 remains alive for the call. The PIDL
        // belongs to the shell and is freed exactly once after selection.
        unsafe {
            let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            // An existing MTA is usable too; all other initialization errors
            // must be reported instead of attempting an uninitialized shell call.
            if initialized < 0 && initialized != 0x80010106u32 as i32 {
                return Err(io::Error::other(format!(
                    "Shell initialization failed ({initialized:#x})"
                )));
            }
            let pidl = ILCreateFromPathW(wide.as_ptr());
            let result = if pidl.is_null() {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Cannot resolve shell item",
                ))
            } else {
                let status = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                ILFree(pidl);
                if status < 0 {
                    Err(io::Error::other(format!(
                        "File selection failed ({status:#x})"
                    )))
                } else {
                    Ok(())
                }
            };
            if initialized >= 0 {
                CoUninitialize();
            }
            result
        }
    }
    fn shell_path(path: &Path) -> io::Result<Vec<u16>> {
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
        let mut wide = if raw.starts_with(&prefix) {
            "\\\\"
                .encode_utf16()
                .chain(raw[8..].iter().copied())
                .collect::<Vec<_>>()
        } else if raw.starts_with(&"\\\\?\\".encode_utf16().collect::<Vec<_>>()) {
            raw[4..].to_vec()
        } else {
            raw
        };
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid file path",
            ));
        }
        wide.push(0);
        Ok(wide)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn text(path: &str) -> String {
            let encoded = shell_path(Path::new(path)).unwrap();
            assert_eq!(encoded.last(), Some(&0));
            String::from_utf16(&encoded[..encoded.len() - 1]).unwrap()
        }

        #[test]
        fn shell_paths_preserve_unicode_spaces_and_commas() {
            assert_eq!(
                text(r"\\?\C:\音乐\artist, title.mp3"),
                r"C:\音乐\artist, title.mp3"
            );
            assert_eq!(
                text(r"\\?\UNC\server\music\artist, title.flac"),
                r"\\server\music\artist, title.flac"
            );
            assert_eq!(text(r"C:\Music\normal.mp3"), r"C:\Music\normal.mp3");
            assert!(shell_path(Path::new("bad\0path")).is_err());
        }
    }
}
