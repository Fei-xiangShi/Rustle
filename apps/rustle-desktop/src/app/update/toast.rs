//! Toast message handlers

use iced::Task;

use crate::app::message::Message;
use crate::app::state::App;
use crate::ui::widgets::Toast;

impl App {
    fn show_toast(&mut self, toast: Toast, hide_after_secs: u64) -> Task<Message> {
        self.ui.toast = Some(toast);
        self.ui.toast_visible = true;

        Task::perform(
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(hide_after_secs)).await;
            },
            |_| Message::HideToast,
        )
    }

    /// Handle toast-related messages
    pub fn handle_toast(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::ShowInfoToast(msg) => Some(self.show_toast(Toast::info(msg.clone()), 3)),
            Message::ShowSuccessToast(msg) => Some(self.show_toast(Toast::success(msg.clone()), 3)),
            Message::ShowWarningToast(msg) => Some(self.show_toast(Toast::warning(msg.clone()), 4)),
            Message::ShowErrorToast(msg) => Some(self.show_toast(Toast::error(msg.clone()), 4)),
            Message::HideToast => {
                self.ui.toast_visible = false;
                Some(Task::none())
            }
            _ => None,
        }
    }

    /// Create a task that shows an error toast
    pub fn toast_error(msg: impl Into<String>) -> Task<Message> {
        Task::done(Message::ShowErrorToast(msg.into()))
    }

    /// Create a task that shows a warning toast
    pub fn toast_warning(msg: impl Into<String>) -> Task<Message> {
        Task::done(Message::ShowWarningToast(msg.into()))
    }

    /// Create a task that shows a success toast
    pub fn toast_success(msg: impl Into<String>) -> Task<Message> {
        Task::done(Message::ShowSuccessToast(msg.into()))
    }

    /// Create a task that shows an info toast
    pub fn toast_info(msg: impl Into<String>) -> Task<Message> {
        Task::done(Message::ShowInfoToast(msg.into()))
    }
}
