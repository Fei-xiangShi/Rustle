// src/app/update/navigation.rs
//! Navigation message handlers

use iced::Size;
use iced::Task;

use crate::app::helpers::open_folder_dialog;
use crate::app::message::Message;
use crate::app::state::{App, NavigationEntry};

impl App {
    fn set_sidebar_drawer_open(&mut self, open: bool) {
        let animate = !self.core.settings.display.power_saving_mode;
        self.ui.set_sidebar_drawer_open(open, animate);
    }

    /// Handle navigation-related messages
    pub fn handle_navigation(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::Noop => Some(Task::none()),

            Message::NavigateBack => {
                if let Some(NavigationEntry::Route(route)) = self.ui.nav_history.go_back() {
                    Some(self.navigate_to_route(route, false))
                } else {
                    Some(Task::none())
                }
            }

            Message::NavigateForward => {
                if let Some(NavigationEntry::Route(route)) = self.ui.nav_history.go_forward() {
                    Some(self.navigate_to_route(route, false))
                } else {
                    Some(Task::none())
                }
            }

            Message::Navigate(_) => {
                let Some(route) = self.route_for_message(message) else {
                    return Some(Task::none());
                };

                self.set_sidebar_drawer_open(false);

                Some(self.navigate_to_route(route, true))
            }

            Message::LibrarySelect(_)
            | Message::OpenSettings
            | Message::OpenSettingsWithCloseLyrics
            | Message::OpenAudioEngine
            | Message::OpenUser(_)
            | Message::OpenAlbum(_) => {
                let Some(route) = self.route_for_message(message) else {
                    return Some(Task::none());
                };
                self.set_sidebar_drawer_open(false);
                Some(self.navigate_to_route(route, true))
            }

            Message::ToggleSidebarDrawer => {
                self.set_sidebar_drawer_open(!self.ui.sidebar_drawer_open);
                self.ui.sidebar_dragging = false;
                Some(Task::none())
            }

            Message::CloseSidebarDrawer => {
                self.set_sidebar_drawer_open(false);
                self.ui.sidebar_dragging = false;
                Some(Task::none())
            }

            Message::SearchChanged(query) => {
                self.ui.search_query = query.clone();
                Some(Task::none())
            }

            Message::PlayHero => {
                tracing::info!("Playing Global Hits 2024");
                Some(Task::none())
            }

            Message::ImportLocalPlaylist => {
                tracing::info!("Import local playlist");
                Some(Task::perform(open_folder_dialog(), Message::FolderSelected))
            }

            Message::MouseMoved(position) => {
                self.core.mouse_position = *position;
                if self.ui.has_blocking_pointer_overlay() {
                    // The event subscription is independent from Iced's widget event capture.
                    // Do not continue a drag that started beneath a modal/popup while that layer
                    // is visible.
                    self.ui.sidebar_dragging = false;
                    return Some(Task::none());
                }
                // Update sidebar width if dragging
                if self.ui.sidebar_dragging {
                    // Persisted sidebar widths use 1080P reference pixels.
                    const MIN_WIDTH: f32 = 240.0;
                    const MAX_WIDTH: f32 = 440.0;
                    let tokens = crate::ui::responsive::ResponsiveContext::from_viewport(
                        Size::new(self.core.window_width, self.core.window_height),
                    )
                    .tokens;
                    // `sidebar_width` is stored in reference-design units so
                    // the same persisted intent scales with a 2K viewport.
                    self.ui.sidebar_width = tokens
                        .reference_pixels(position.x)
                        .clamp(MIN_WIDTH, MAX_WIDTH);
                }
                Some(Task::none())
            }

            Message::MouseReleased => {
                // 结束侧边栏拖动
                if self.ui.sidebar_dragging {
                    self.ui.sidebar_dragging = false;
                }
                Some(Task::none())
            }

            _ => None,
        }
    }
}
