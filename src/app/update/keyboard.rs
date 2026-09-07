// src/app/update/keyboard.rs
//! Keyboard and action message handlers

use iced::Task;
use iced::keyboard::Key;

use crate::app::message::Message;
use crate::app::state::{App, Route};
use crate::features::Action;
use crate::ui::overlay::OverlayKind;

impl App {
    /// Handle keyboard-related messages
    pub fn handle_keyboard(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::KeyPressed(key, modifiers) => {
                // Escape key: dismiss topmost overlay if escape_close allows it
                if *key == Key::Named(iced::keyboard::key::Named::Escape) && modifiers.is_empty() {
                    let can_dismiss = self
                        .ui
                        .overlay_stack
                        .last()
                        .map(|entry| match &entry.kind {
                            OverlayKind::Modal(_, config) => config.escape_close,
                        })
                        .unwrap_or(false);
                    if can_dismiss {
                        return Some(Task::done(Message::DismissTopModal));
                    }

                    if self.ui.sidebar_drawer_visible() {
                        return Some(Task::done(Message::CloseSidebarDrawer));
                    }
                }

                // If editing a keybinding, capture the key press for that
                if self.ui.editing_keybinding.is_some() {
                    return Some(
                        self.update(Message::KeybindingKeyPressed(key.clone(), *modifiers)),
                    );
                }

                // Otherwise, check for keybinding actions
                if let Some(action) = crate::platform::keybindings::find_global_action(
                    &self.core.settings.keybindings,
                    key,
                    modifiers,
                ) && self
                    .core
                    .global_hotkeys
                    .as_ref()
                    .is_some_and(|service| service.is_registered(action))
                {
                    // The native global event owns this press. Skipping the local
                    // path prevents one focused-window press from firing twice.
                    return Some(Task::none());
                }

                if let Some(action) = crate::platform::keybindings::find_action(
                    &self.core.settings.keybindings,
                    key,
                    modifiers,
                ) {
                    return Some(self.update(Message::ExecuteAction(action)));
                }
                Some(Task::none())
            }

            Message::GlobalHotkeyPressed(id) => {
                if self.ui.editing_keybinding.is_some() {
                    self.ui.global_hotkey_seen_while_recording = Some(*id);
                    // Shortcut recording owns keyboard input and must not also
                    // execute a previously registered global action.
                    return Some(Task::none());
                }

                if let Some((suppressed_id, deadline)) = self.ui.suppressed_recording_hotkey {
                    if iced::time::Instant::now() <= deadline && suppressed_id == *id {
                        self.ui.suppressed_recording_hotkey = None;
                        return Some(Task::none());
                    }
                    if iced::time::Instant::now() > deadline {
                        self.ui.suppressed_recording_hotkey = None;
                    }
                }

                let action = self
                    .core
                    .global_hotkeys
                    .as_ref()
                    .and_then(|service| service.action_for_id(*id))
                    .filter(|registered_action| {
                        self.core
                            .settings
                            .keybindings
                            .global_binding(registered_action)
                            .and_then(crate::platform::global_hotkeys::hotkey_for_binding)
                            .is_some_and(|hotkey| hotkey.id() == *id)
                    });

                action
                    .map(|action| self.execute_action(action))
                    .or_else(|| {
                        tracing::warn!(id, "Ignoring unknown global hotkey event");
                        Some(Task::none())
                    })
            }

            Message::ExecuteAction(action) => Some(self.execute_action(*action)),

            _ => None,
        }
    }

    /// Execute a keybinding action
    fn execute_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::PlayPause => {
                return self.update(Message::TogglePlayback);
            }
            Action::NextTrack => {
                return self.update(Message::NextSong);
            }
            Action::PrevTrack => {
                return self.update(Message::PrevSong);
            }
            Action::VolumeUp => {
                let current = self.playback_info().volume;
                self.set_output_volume((current + 0.05).min(1.0), true);
            }
            Action::VolumeDown => {
                let current = self.playback_info().volume;
                self.set_output_volume((current - 0.05).max(0.0), true);
            }
            Action::VolumeMute => {
                let current = self.playback_info().volume;
                if current > 0.0 {
                    // Save current volume before muting
                    self.core.volume_before_mute = Some(current);
                    self.set_output_volume(0.0, true);
                } else {
                    // Restore previous volume or default to 0.5
                    let restore_vol = self.core.volume_before_mute.unwrap_or(0.5);
                    self.set_output_volume(restore_vol, true);
                    self.core.volume_before_mute = None;
                }
            }
            Action::SeekForward => {
                self.seek_by_offset(std::time::Duration::from_secs(10), true);
            }
            Action::SeekBackward => {
                self.seek_by_offset(std::time::Duration::from_secs(10), false);
            }
            Action::GoHome => {
                return self.navigate_to_route(
                    Route::Discover(crate::app::state::DiscoverViewMode::Overview),
                    true,
                );
            }
            Action::FocusSearch => {
                return iced::widget::operation::focus(iced::widget::Id::new(
                    crate::ui::components::search_bar::TOP_BAR_SEARCH_INPUT_ID,
                ));
            }
            Action::ToggleQueue => {
                self.ui.queue_visible = !self.ui.queue_visible;
            }
            Action::ToggleFullscreen => {
                let mode = if self.core.window_restore_mode == iced::window::Mode::Fullscreen {
                    iced::window::Mode::Windowed
                } else {
                    iced::window::Mode::Fullscreen
                };
                self.core.window_restore_mode = mode;

                if self.core.is_window_hidden() {
                    return Task::none();
                }

                self.core.window_operation_pending = true;
                return crate::platform::window::set_window_mode(mode)
                    .chain(Task::done(Message::WindowOperationComplete));
            }
        }
        Task::none()
    }
}
