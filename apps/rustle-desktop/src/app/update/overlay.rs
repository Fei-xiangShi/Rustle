//! Unified overlay update handler — manages overlay_stack lifecycle.

use iced::Task;

use super::{App, Message};

impl App {
    pub fn handle_overlay(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::DismissTopModal => {
                self.dismiss_top_overlay();
                Some(Task::none())
            }

            // When song edits are saved, dismiss the overlay and clean up legacy state
            Message::SaveSongEdits(_) => {
                self.dismiss_top_overlay();
                self.ui.song_edit_dialog = None;
                None // Let the existing handler also process this
            }

            _ => None,
        }
    }

    /// Dismiss the topmost overlay in the stack.
    fn dismiss_top_overlay(&mut self) {
        self.ui.overlay_stack.pop();
    }
}
