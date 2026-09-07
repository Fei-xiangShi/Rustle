// src/app/update/tray.rs
//! System tray message handlers

use iced::Task;

use crate::app::message::Message;
use crate::app::state::App;
use crate::application::tray::{
    TrayAvailability, TrayCommand, TrayPresentation, TrayState, TrayWindowCommand,
};

fn initialize_tray_task(language: crate::i18n::Language) -> Task<Message> {
    let presentation = TrayPresentation::new(
        &TrayState::default(),
        crate::app::helpers::tray_labels(language),
    );

    #[cfg(target_os = "linux")]
    {
        Task::perform(
            crate::platform::tray::initialize(presentation),
            |result| match result {
                Ok(receiver) => Message::TrayStarted(receiver),
                Err(error) => {
                    tracing::warn!(%error, "Failed to start system tray");
                    Message::TrayUnavailable(error.to_string())
                }
            },
        )
    }

    #[cfg(not(target_os = "linux"))]
    {
        Task::done(match crate::platform::tray::initialize(presentation) {
            Ok(receiver) => Message::TrayStarted(receiver),
            Err(error) => {
                tracing::warn!(%error, "Failed to start system tray");
                Message::TrayUnavailable(error.to_string())
            }
        })
    }
}

fn tray_window_message(command: TrayWindowCommand) -> Message {
    match command {
        TrayWindowCommand::Toggle => Message::ToggleWindow,
        TrayWindowCommand::PrimaryActivation => {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                Message::ShowOrFocusWindow
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                Message::ToggleWindow
            }
        }
    }
}

impl App {
    /// Handle tray-related messages
    pub fn handle_tray(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::InitializeTray => {
                if self.core.tray_initialization_requested {
                    return Some(Task::none());
                }
                self.core.tray_initialization_requested = true;
                Some(initialize_tray_task(self.core.locale.language))
            }

            Message::TrayStarted(rx) => {
                tracing::info!("Tray service started");
                self.core.tray_availability = TrayAvailability::Available;
                self.update_tray_and_mpris_current(self.playback_is_playing());
                let rx = rx.clone();
                Some(Task::run(
                    async_stream::stream! {
                        loop {
                            let cmd = rx.lock().await.recv().await;
                            if let Some(cmd) = cmd {
                                yield cmd;
                            } else {
                                break;
                            }
                        }
                    },
                    Message::TrayCommand,
                ))
            }

            Message::TrayUnavailable(error) => {
                let first_failure = !matches!(
                    self.core.tray_availability,
                    TrayAvailability::Unavailable(_)
                );
                self.core.tray_availability = TrayAvailability::Unavailable(error.clone());

                let mut tasks = Vec::new();
                if self.core.is_window_hidden() {
                    tasks.push(self.update(Message::ShowWindow));
                }
                if first_failure {
                    tasks.push(Self::toast_warning(
                        self.core
                            .locale
                            .get(crate::i18n::Key::TrayUnavailable)
                            .to_string(),
                    ));
                }
                Some(Task::batch(tasks))
            }

            Message::TrayCommand(cmd) => {
                match cmd {
                    TrayCommand::Window(window_command) => {
                        let message = tray_window_message(*window_command);
                        return Some(self.update(message));
                    }
                    TrayCommand::PlayPause => {
                        return Some(self.update(Message::TogglePlayback));
                    }
                    TrayCommand::NextTrack => {
                        return Some(self.update(Message::NextSong));
                    }
                    TrayCommand::PrevTrack => {
                        return Some(self.update(Message::PrevSong));
                    }
                    TrayCommand::SetPlayMode(mode) => {
                        self.core.settings.play_mode = *mode;
                        let _ = self.core.settings.save();
                        // Clear shuffle cache and re-calculate for new mode
                        self.clear_shuffle_cache();
                        self.cache_shuffle_indices();
                        self.refresh_preload_window();
                        let preload_task = self.preload_adjacent_tracks_with_ncm();
                        let is_playing = self.playback_is_playing();
                        self.update_tray_and_mpris_current(is_playing);
                        return Some(preload_task);
                    }
                    TrayCommand::ToggleFavorite => {
                        // Toggle favorite for current NCM song
                        if let Some(song) = &self.playback.current_song
                            && song.id < 0
                        {
                            let ncm_id = (-song.id) as u64;
                            return Some(self.update(Message::ToggleFavorite(ncm_id)));
                        }
                    }
                    TrayCommand::Quit => {
                        return Some(self.update(Message::ConfirmExit));
                    }
                    TrayCommand::AvailabilityChanged(availability) => {
                        let became_unavailable =
                            matches!(availability, TrayAvailability::Unavailable(_))
                                && !matches!(
                                    self.core.tray_availability,
                                    TrayAvailability::Unavailable(_)
                                );
                        self.core.tray_availability = availability.clone();

                        if matches!(availability, TrayAvailability::Unavailable(_)) {
                            let mut tasks = Vec::new();
                            if self.core.is_window_hidden() {
                                tasks.push(self.update(Message::ShowWindow));
                            }
                            if became_unavailable {
                                tasks.push(Self::toast_warning(
                                    self.core
                                        .locale
                                        .get(crate::i18n::Key::TrayUnavailable)
                                        .to_string(),
                                ));
                            }
                            return Some(Task::batch(tasks));
                        }
                    }
                }
                Some(Task::none())
            }

            _ => None,
        }
    }
}
