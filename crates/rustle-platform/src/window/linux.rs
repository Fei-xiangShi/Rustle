//! Linux window behavior implementation

use std::env;

use iced::Task;

pub fn is_wayland_backend() -> bool {
    env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var("XDG_SESSION_TYPE")
            .is_ok_and(|session_type| session_type.eq_ignore_ascii_case("wayland"))
}

pub fn set_window_mode<Message: Send + 'static>(mode: iced::window::Mode) -> Task<Message> {
    iced::window::latest().and_then(move |id| {
        if is_wayland_backend() {
            return iced::window::set_mode(id, mode);
        }

        match mode {
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
        }
    })
}

pub fn focus_window<Message: Send + 'static>() -> Task<Message> {
    iced::window::latest().and_then(|id| iced::window::gain_focus(id))
}
