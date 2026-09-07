//! Message update handlers - thin dispatcher delegating to submodules

pub mod audio_preload_manager;
mod context_menu;
mod database;
mod discord;
mod discover;
mod images;
mod import;
mod keyboard;
mod lyrics;
pub mod lyrics_preload_manager;
pub mod lyrics_render_manager;
mod mpris;
mod navigation;
mod ncm;
mod overlay;
pub mod page_loader;
mod playback;
mod player_controller;
mod playlist;
mod preload;
pub mod preload_coordinator;
mod protocol;
mod queue;
pub mod queue_navigator;
mod router;
mod scroll;
mod search;
mod settings;
pub mod song_resolver;
mod toast;
mod tray;
mod window;

use iced::Task;

use super::{App, Message};

impl App {
    /// Handle messages by delegating to appropriate submodule handlers
    pub fn update(&mut self, message: Message) -> Task<Message> {
        macro_rules! handle {
            ($handler:ident) => {
                if let Some(task) = self.$handler(&message) {
                    return self.with_image_tasks(&message, task);
                }
            };
        }

        // Try each handler in order until one handles the message
        handle!(handle_navigation);
        handle!(handle_context_menu);
        handle!(handle_overlay);
        handle!(handle_database);
        handle!(handle_import);
        handle!(handle_toast);
        handle!(handle_download);
        handle!(handle_playback);
        handle!(handle_scroll);
        handle!(handle_playlist);
        handle!(handle_queue);
        handle!(handle_settings);
        handle!(handle_window);
        handle!(handle_tray);
        handle!(handle_mpris);
        handle!(handle_discord);
        handle!(handle_keyboard);
        handle!(handle_lyrics);
        handle!(handle_image);
        handle!(handle_ncm);
        handle!(handle_discover);
        handle!(handle_search);
        handle!(handle_preload);
        handle!(handle_protocol);

        // Default: no task
        Task::none()
    }

    fn with_image_tasks(&mut self, message: &Message, task: Task<Message>) -> Task<Message> {
        let image_tasks = self.collect_image_tasks_after_message(message);
        self.ui
            .playlist_page
            .sync_gradient_animation(self.core.settings.display.power_saving_mode);
        Task::batch([task, image_tasks])
    }
}
