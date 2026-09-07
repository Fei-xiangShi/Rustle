//! Windows window behavior implementation
//!
//! Windows requires special handling: minimize before hide, restore before show

use crate::platform::APP_ID;
use iced::Task;

pub fn initialize_process() {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SetCurrentProcessExplicitAppUserModelID(appid: *const u16) -> i32;
    }

    let app_id: Vec<u16> = format!("{}\0", APP_ID).encode_utf16().collect();
    // SAFETY: app_id is NUL-terminated and remains allocated for this
    // synchronous process-initialization call. The API only borrows it.
    let result = unsafe { SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr()) };

    if result < 0 {
        tracing::warn!(
            "Failed to set Windows AppUserModelID: HRESULT={:#x}",
            result
        );
    }
}

pub fn native_window_handle(id: iced::window::Id) -> Task<Option<usize>> {
    use iced::window::raw_window_handle::RawWindowHandle;

    iced::window::run(id, |window| match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
            _ => None,
        },
        Err(_) => None,
    })
}

pub fn set_window_mode<Message: Send + 'static>(mode: iced::window::Mode) -> Task<Message> {
    iced::window::latest().and_then(move |id| match mode {
        iced::window::Mode::Hidden => Task::batch([
            iced::window::minimize(id, true),
            iced::window::set_mode(id, iced::window::Mode::Hidden),
        ]),
        iced::window::Mode::Windowed => Task::batch([
            iced::window::set_mode(id, iced::window::Mode::Windowed),
            iced::window::minimize(id, false),
            iced::window::gain_focus(id),
        ]),
        iced::window::Mode::Fullscreen => Task::batch([
            iced::window::set_mode(id, iced::window::Mode::Fullscreen),
            iced::window::minimize(id, false),
            iced::window::gain_focus(id),
        ]),
    })
}

pub fn focus_window<Message: Send + 'static>() -> Task<Message> {
    iced::window::latest().and_then(|id| {
        Task::batch([
            iced::window::minimize(id, false),
            iced::window::gain_focus(id),
        ])
    })
}
