//! Window behavior abstraction
//!
//! Provides unified window behavior functions across platforms.
//! Handles platform-specific differences in show/hide/minimize behavior.

#[cfg(target_os = "linux")]
use crate::APP_ID;
use iced::Task;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub fn initialize_process() {
    #[cfg(target_os = "windows")]
    {
        windows::initialize_process();
    }
}

pub fn set_window_mode<Message: Send + 'static>(mode: iced::window::Mode) -> Task<Message> {
    #[cfg(target_os = "windows")]
    {
        windows::set_window_mode(mode)
    }
    #[cfg(target_os = "linux")]
    {
        linux::set_window_mode(mode)
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_window_mode(mode)
    }
}

pub fn focus_window<Message: Send + 'static>() -> Task<Message> {
    #[cfg(target_os = "windows")]
    {
        windows::focus_window()
    }
    #[cfg(target_os = "linux")]
    {
        linux::focus_window()
    }
    #[cfg(target_os = "macos")]
    {
        macos::focus_window()
    }
}

pub fn drag_resize<Message: Send + 'static>(direction: iced::window::Direction) -> Task<Message> {
    iced::window::latest().and_then(move |id| iced::window::drag_resize(id, direction))
}

pub fn needs_manual_resize_handles() -> bool {
    cfg!(target_os = "windows")
}

pub fn is_wayland_backend() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_wayland_backend()
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

pub fn native_window_handle(id: iced::window::Id) -> Task<Option<usize>> {
    #[cfg(target_os = "windows")]
    {
        windows::native_window_handle(id)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = id;
        Task::done(None)
    }
}

/// Get platform-specific window settings
pub fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        // Windows' borderless client area needs the tighter default width for
        // a complete six-card playlist row after shell insets are reserved.
        #[cfg(target_os = "windows")]
        size: iced::Size::new(1440.0, 900.0),
        // Preserve the existing default on platforms whose compositor already
        // provides the expected playlist density.
        #[cfg(not(target_os = "windows"))]
        size: iced::Size::new(1560.0, 900.0),
        exit_on_close_request: false,
        decorations: false,
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..Default::default()
        },
        #[cfg(target_os = "macos")]
        platform_specific: iced::window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        #[cfg(target_os = "windows")]
        platform_specific: Default::default(),
        ..Default::default()
    }
}
