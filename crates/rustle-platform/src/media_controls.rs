//! Media controls abstraction
//!
//! Provides a unified interface for system media controls across platforms.
//!
//! - Linux: Uses MPRIS D-Bus interface (mpris-server)
//! - Windows: Uses System Media Transport Controls (SMTC) via souvlaki
//! - macOS: Uses MPNowPlayingInfoCenter via souvlaki
//! - WASM: No-op (not available)

use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod souvlaki_impl;

// ============ Common Types (all platforms) ============

/// Commands that can be sent from media controls to the application
#[derive(Debug, Clone)]
pub enum MediaCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek(i64),
    SetPosition(String, i64),
    SetVolume(f64),
    Raise,
    Quit,
    /// Open a URI (rustle:// or other supported scheme)
    OpenUri(String),
}

/// Track metadata for media controls
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaMetadata {
    pub track_id: Option<String>,
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artists: Vec<String>,
    pub length_us: Option<i64>,
    pub art_url: Option<String>,
}

/// Playback status
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum MediaPlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// Media controls state
#[derive(Debug, Clone, Default)]
pub struct MediaState {
    pub status: MediaPlaybackStatus,
    pub metadata: MediaMetadata,
    pub position_us: i64,
    #[cfg(target_os = "linux")]
    pub volume: f64,
    #[cfg(target_os = "linux")]
    pub can_go_next: bool,
    #[cfg(target_os = "linux")]
    pub can_go_previous: bool,
    #[cfg(target_os = "linux")]
    pub can_play: bool,
    #[cfg(target_os = "linux")]
    pub can_pause: bool,
    #[cfg(target_os = "linux")]
    pub can_seek: bool,
}

// ============ Platform-specific Handle ============

/// Handle to control media controls from the application
#[derive(Debug, Clone)]
pub struct MediaHandle {
    #[cfg(target_os = "linux")]
    inner: linux::LinuxMediaHandle,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    inner: souvlaki_impl::SouvlakiMediaHandle,
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    _phantom: (),
}

impl MediaHandle {
    /// Update media controls state
    pub fn update(&self, state: MediaState) {
        #[cfg(target_os = "linux")]
        self.inner.update(state);
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        self.inner.update(state);
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        let _ = state; // Suppress unused warning
    }
}

// ============ Platform-specific Initialization ============

/// Start media controls service (Linux - MPRIS)
#[cfg(target_os = "linux")]
pub fn start_media_controls_with(
    _window_handle: Option<usize>,
    spawn_worker: crate::runtime::WorkerSpawner,
) -> (MediaHandle, mpsc::UnboundedReceiver<MediaCommand>) {
    let (inner, rx) = linux::start(spawn_worker);
    (MediaHandle { inner }, rx)
}

/// Start media controls service (Windows/macOS - souvlaki)
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn start_media_controls_with(
    window_handle: Option<usize>,
    _spawn_worker: crate::runtime::WorkerSpawner,
) -> (MediaHandle, mpsc::UnboundedReceiver<MediaCommand>) {
    let (inner, rx) = souvlaki_impl::start(window_handle);
    (MediaHandle { inner }, rx)
}

/// Start media controls service (no-op on unsupported platforms)
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn start_media_controls_with(
    _window_handle: Option<usize>,
    _spawn_worker: crate::runtime::WorkerSpawner,
) -> (MediaHandle, mpsc::UnboundedReceiver<MediaCommand>) {
    let (_tx, rx) = mpsc::unbounded_channel();
    (MediaHandle { _phantom: () }, rx)
}

/// Check if media controls are available on this platform
pub fn is_available() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "windows",
        target_os = "macos"
    ))
}
