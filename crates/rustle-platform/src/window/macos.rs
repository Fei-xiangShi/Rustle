//! macOS window behavior implementation

use iced::Task;

pub fn set_window_mode<Message: Send + 'static>(mode: iced::window::Mode) -> Task<Message> {
    iced::window::latest().and_then(move |id| match mode {
        iced::window::Mode::Hidden => iced::window::set_mode(id, iced::window::Mode::Hidden),
        iced::window::Mode::Windowed => Task::batch([
            iced::window::set_mode(id, iced::window::Mode::Windowed),
            iced::window::gain_focus(id),
        ]),
        iced::window::Mode::Fullscreen => Task::batch([
            iced::window::set_mode(id, iced::window::Mode::Fullscreen),
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
