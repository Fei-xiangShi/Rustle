//! System tray abstraction.
//!
//! The platform backends intentionally have different ownership models:
//! Linux delegates to the StatusNotifierItem service, Windows owns Win32
//! objects on the Iced/Winit event-loop thread, and macOS owns its status item
//! on that same UI thread.

pub use crate::application::tray::{
    TrayAvailability, TrayCommand, TrayPresentation, TrayWindowCommand,
};
use std::sync::OnceLock;
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

const COMMAND_CHANNEL_CAPACITY: usize = 32;

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
pub async fn initialize(presentation: TrayPresentation) -> anyhow::Result<TrayResult> {
    let (handle, rx) = linux::start_linux_tray(presentation, COMMAND_CHANNEL_CAPACITY).await?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| anyhow::anyhow!("system tray was already initialized"))?;
    tracing::info!("System tray started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(target_os = "windows")]
pub fn initialize(presentation: TrayPresentation) -> anyhow::Result<TrayResult> {
    let (handle, rx) = windows::start_windows_tray(presentation, COMMAND_CHANNEL_CAPACITY)?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| anyhow::anyhow!("system tray was already initialized"))?;
    tracing::info!("Windows system tray started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(target_os = "macos")]
pub fn initialize(presentation: TrayPresentation) -> anyhow::Result<TrayResult> {
    let (handle, rx) = macos::start_macos_tray(presentation, COMMAND_CHANNEL_CAPACITY)?;
    TRAY_HANDLE
        .set(handle)
        .map_err(|_| anyhow::anyhow!("system tray was already initialized"))?;
    tracing::info!("macOS status item started");
    Ok(std::sync::Arc::new(tokio::sync::Mutex::new(rx)))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn initialize(_presentation: TrayPresentation) -> anyhow::Result<TrayResult> {
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
