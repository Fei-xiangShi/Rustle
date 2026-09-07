// src/app/update/window.rs
//! Window and exit dialog message handlers

use iced::Task;

use crate::app::message::Message;
use crate::app::state::{App, WindowVisibilityState};
use crate::features::CloseBehavior;
use crate::platform::tray::TrayAvailability;
use crate::platform::window;
use crate::ui::overlay::{ModalKind, OverlayKind};

impl App {
    pub(super) fn sync_window_maximized_task() -> Task<Message> {
        iced::window::latest()
            .and_then(|id| iced::window::is_maximized(id).map(Message::WindowMaximized))
    }

    pub(super) fn sync_window_maximized_after_resize_task() -> Task<Message> {
        #[cfg(target_os = "macos")]
        {
            // Winit queries maximization for a borderless AppKit window by temporarily
            // changing its style mask. The resulting surface resize event would call this
            // query again, creating an endless resize/style-mask feedback loop.
            Task::none()
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::sync_window_maximized_task()
        }
    }

    fn is_window_hidden(&self) -> bool {
        self.core.is_window_hidden()
    }

    fn is_window_visible_or_showing(&self) -> bool {
        matches!(
            self.core.window_visibility,
            WindowVisibilityState::Visible | WindowVisibilityState::Showing
        )
    }

    fn current_visible_mode(&self) -> iced::window::Mode {
        if self.is_window_visible_or_showing() {
            self.core.window_restore_mode
        } else {
            iced::window::Mode::Hidden
        }
    }

    fn begin_hide_window(&mut self) -> Task<Message> {
        let backend_available = crate::platform::tray::is_available();
        if !tray_allows_hiding(&self.core.tray_availability, backend_available) {
            tracing::warn!(
                availability = ?self.core.tray_availability,
                backend_available,
                "Refusing to hide the last window because the system tray is unavailable"
            );
            return Self::toast_warning(
                self.core
                    .locale
                    .get(crate::i18n::Key::TrayUnavailable)
                    .to_string(),
            );
        }

        if self.core.window_operation_pending
            || self.core.window_visibility == WindowVisibilityState::Hidden
            || self.core.window_visibility == WindowVisibilityState::Hiding
        {
            return Task::none();
        }

        tracing::info!(
            backend = if window::is_wayland_backend() {
                "wayland"
            } else {
                "x11"
            },
            "Hiding window to tray"
        );

        self.core.window_visibility = WindowVisibilityState::Hiding;
        self.core.window_operation_pending = true;

        window::set_window_mode(iced::window::Mode::Hidden)
            .chain(Task::done(Message::WindowOperationComplete))
    }

    fn begin_show_window(&mut self) -> Task<Message> {
        if self.core.window_operation_pending
            || self.core.window_visibility == WindowVisibilityState::Visible
            || self.core.window_visibility == WindowVisibilityState::Showing
        {
            return Task::none();
        }

        tracing::info!(
            backend = if window::is_wayland_backend() {
                "wayland"
            } else {
                "x11"
            },
            "Showing window"
        );

        self.core.window_visibility = WindowVisibilityState::Showing;
        self.core.window_operation_pending = true;

        if window::is_wayland_backend() {
            window::set_window_mode(self.core.window_restore_mode)
        } else {
            window::set_window_mode(self.core.window_restore_mode)
                .chain(Task::done(Message::WindowOperationComplete))
        }
    }

    fn toggle_window_task(&mut self) -> Task<Message> {
        if self.is_window_hidden() {
            self.begin_show_window()
        } else {
            self.begin_hide_window()
        }
    }

    fn begin_show_or_focus_window(&mut self) -> Task<Message> {
        if self.is_window_hidden() {
            self.begin_show_window()
        } else if self.core.window_focused {
            Task::none()
        } else {
            window::focus_window()
        }
    }

    /// Handle window-related messages
    pub fn handle_window(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::RequestClose => {
                match self.core.settings.close_behavior {
                    CloseBehavior::Ask => {
                        use crate::ui::overlay::{
                            ModalConfig, ModalKind, OverlayEntry, OverlayKind,
                        };
                        self.ui
                            .overlay_stack
                            .push(OverlayEntry::new(OverlayKind::Modal(
                                ModalKind::ExitConfirm {
                                    remember_choice: false,
                                },
                                ModalConfig::default().width(380.0).no_backdrop_dismiss(),
                            )));
                    }
                    CloseBehavior::Exit => {
                        return Some(iced::exit());
                    }
                    CloseBehavior::MinimizeToTray => {
                        return Some(self.begin_hide_window());
                    }
                }
                Some(Task::none())
            }

            Message::ConfirmExit => {
                let remember = self.exit_remember_from_overlay();
                if remember {
                    self.core.settings.close_behavior = CloseBehavior::Exit;
                    let _ = self.core.settings.save();
                }
                Some(iced::exit())
            }

            Message::MinimizeToTray => {
                let remember = self.exit_remember_from_overlay();
                if remember {
                    self.core.settings.close_behavior = CloseBehavior::MinimizeToTray;
                    let _ = self.core.settings.save();
                }
                self.ui.overlay_stack.clear();
                Some(self.begin_hide_window())
            }

            Message::ExitDialogRememberChanged(checked) => {
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::ExitConfirm { remember_choice: r }, _) =
                        &mut entry.kind
                {
                    *r = *checked;
                }
                Some(Task::none())
            }

            Message::ToggleWindow => Some(self.toggle_window_task()),

            Message::ShowWindow => Some(self.begin_show_window()),

            Message::ShowOrFocusWindow => Some(self.begin_show_or_focus_window()),

            Message::WindowMinimize => {
                Some(iced::window::latest().and_then(|id| iced::window::minimize(id, true)))
            }

            Message::WindowMaximize => Some(iced::window::latest().and_then(|id| {
                iced::window::toggle_maximize(id).chain(Self::sync_window_maximized_task())
            })),

            Message::WindowMaximized(maximized) => {
                self.core.window_maximized = *maximized;
                Some(Task::none())
            }

            Message::WindowDrag => Some(iced::window::latest().and_then(iced::window::drag)),

            Message::WindowResize(direction) => {
                let direction = *direction;
                Some(window::drag_resize(direction))
            }

            Message::InitializeGlobalHotkeys => {
                let mut receiver = crate::platform::global_hotkeys::install_event_handler();
                match crate::platform::global_hotkeys::GlobalHotkeyService::new(
                    &self.core.settings.keybindings,
                ) {
                    Ok(service) => {
                        self.core.global_hotkeys = Some(service);
                        Some(Task::run(
                            async_stream::stream! {
                                while let Some(id) = receiver.recv().await {
                                    yield id;
                                }
                            },
                            Message::GlobalHotkeyPressed,
                        ))
                    }
                    Err(error) => {
                        let error = crate::error::AppError::from(error);
                        tracing::warn!(
                            code = %error.code(),
                            recovery = ?error.recovery(),
                            "Global hotkeys are unavailable"
                        );
                        Some(Task::none())
                    }
                }
            }

            Message::WindowShown => {
                let visibility_task = if self.core.window_visibility
                    == WindowVisibilityState::Showing
                    && self.core.window_operation_pending
                    && window::is_wayland_backend()
                {
                    Task::done(Message::WindowOperationComplete)
                } else {
                    Task::none()
                };

                Some(Task::batch([
                    visibility_task,
                    Self::sync_window_maximized_task(),
                ]))
            }

            Message::WindowFocused => {
                self.core.window_focused = true;
                self.sync_audio_analysis_state();
                Some(Task::none())
            }

            Message::WindowUnfocused => {
                self.core.window_focused = false;
                self.sync_audio_analysis_state();
                Some(Task::none())
            }

            Message::WindowOperationComplete => {
                self.core.window_operation_pending = false;
                self.core.window_visibility =
                    finalize_window_visibility(self.current_visible_mode());
                self.sync_audio_analysis_state();
                Some(Task::none())
            }

            // Sidebar resize
            Message::SidebarResizeStart => {
                self.ui.sidebar_dragging = true;
                Some(Task::none())
            }

            Message::SidebarResizeEnd => {
                self.ui.sidebar_dragging = false;
                Some(Task::none())
            }

            _ => None,
        }
    }

    fn exit_remember_from_overlay(&self) -> bool {
        self.ui
            .overlay_stack
            .last()
            .and_then(|e| match &e.kind {
                OverlayKind::Modal(ModalKind::ExitConfirm { remember_choice }, _) => {
                    Some(*remember_choice)
                }
                _ => None,
            })
            .unwrap_or(false)
    }
}

fn tray_allows_hiding(availability: &TrayAvailability, backend_available: bool) -> bool {
    availability.is_available() && backend_available
}

fn finalize_window_visibility(window_mode: iced::window::Mode) -> WindowVisibilityState {
    if window_mode == iced::window::Mode::Hidden {
        WindowVisibilityState::Hidden
    } else {
        WindowVisibilityState::Visible
    }
}

#[cfg(test)]
mod tests {
    use super::{finalize_window_visibility, tray_allows_hiding};
    use crate::app::state::WindowVisibilityState;
    use crate::platform::tray::TrayAvailability;

    #[test]
    fn hiding_requires_a_confirmed_tray_recovery_surface() {
        assert!(!tray_allows_hiding(&TrayAvailability::Starting, true));
        assert!(!tray_allows_hiding(
            &TrayAvailability::Unavailable("registration failed".into()),
            true,
        ));
        assert!(!tray_allows_hiding(&TrayAvailability::Available, false));
        assert!(tray_allows_hiding(&TrayAvailability::Available, true));
    }

    #[test]
    fn finalize_window_operation_visibility() {
        assert_eq!(
            finalize_window_visibility(iced::window::Mode::Hidden),
            WindowVisibilityState::Hidden,
        );
        assert_eq!(
            finalize_window_visibility(iced::window::Mode::Windowed),
            WindowVisibilityState::Visible,
        );
        assert_eq!(
            finalize_window_visibility(iced::window::Mode::Fullscreen),
            WindowVisibilityState::Visible,
        );
    }
}
