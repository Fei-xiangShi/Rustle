//! Main application module

pub mod helpers;
mod message;
mod state;
mod update;
mod view;

use iced::{Task, Theme};
use std::sync::Arc;

use crate::error::{AppError, ErrorCode};
use crate::i18n::{Language, Locale};
pub use message::{
    ContextMenuAction, IconId, Message, PlaylistViewPayload, SettingsSection, SidebarId,
};
pub(crate) use state::ImageEntry;
pub use state::{
    App, ContextMenuState, CoreState, DiscoverPageState, DiscoverViewMode, DownloadTab, ImageState,
    LibraryState, LyricsDisplayMode, PlaybackSessionState, Route, SEARCH_PAGE_SIZE,
    SearchPageState, SearchSuggestionsState, SearchTab, SongEditDialogState, UiState, UserInfo,
};
pub use update::song_resolver::ResolvedAudioQuality;

mod subscription_logic {
    pub(super) fn mouse_button_release_message(
        button: iced::mouse::Button,
    ) -> Option<super::Message> {
        match button {
            iced::mouse::Button::Left => Some(super::Message::MouseReleased),
            iced::mouse::Button::Back => Some(super::Message::NavigateBack),
            iced::mouse::Button::Forward => Some(super::Message::NavigateForward),
            _ => None,
        }
    }

    pub fn window_should_throttle(window_hidden: bool) -> bool {
        window_hidden
    }

    pub fn animation_subscription_enabled(
        window_throttled: bool,
        has_animations: bool,
        lyrics_needs_frames: bool,
        audio_engine_needs_frames: bool,
    ) -> bool {
        !window_throttled && (has_animations || lyrics_needs_frames || audio_engine_needs_frames)
    }

    pub fn playback_tick_interval_ms(
        is_playing: bool,
        window_throttled: bool,
        power_saving: bool,
    ) -> Option<u64> {
        if !is_playing {
            None
        } else if window_throttled {
            Some(1000)
        } else if power_saving {
            Some(500)
        } else {
            Some(100)
        }
    }

    #[cfg(test)]
    pub fn needs_animation_subscription(
        has_animations: bool,
        lyrics_needs_frames: bool,
        audio_engine_needs_frames: bool,
    ) -> bool {
        animation_subscription_enabled(
            false,
            has_animations,
            lyrics_needs_frames,
            audio_engine_needs_frames,
        )
    }

    #[cfg(test)]
    pub fn needs_playback_subscription(is_playing: bool) -> bool {
        playback_tick_interval_ms(is_playing, false, false).is_some()
    }

    #[cfg(test)]
    pub fn subscription_decisions(
        has_animations: bool,
        lyrics_needs_frames: bool,
        audio_engine_needs_frames: bool,
        is_playing: bool,
    ) -> (bool, bool) {
        let needs_animation = needs_animation_subscription(
            has_animations,
            lyrics_needs_frames,
            audio_engine_needs_frames,
        );
        let needs_playback = needs_playback_subscription(is_playing);
        (needs_animation, needs_playback)
    }
}

impl App {
    /// Create new application instance
    pub fn new() -> (Self, Task<Message>) {
        crate::observability::set_runtime_phase(crate::observability::RuntimePhase::Application);
        // 1. Load settings first to initialize locale correctly
        let settings = crate::features::Settings::load();

        // Clean up orphan temp files from interrupted downloads in the
        // configured (not default) download directory.
        crate::cache::cleanup_temp_files(&settings);
        let locale = {
            let lang = Language::from_code(&settings.display.language).unwrap_or_default();
            Locale::new(lang)
        };

        // 2. Initialize audio system
        let (audio_thread, audio, audio_chain, audio_listener_task) =
            helpers::init_audio(&settings);

        // 3. Initialize sub-states
        let core = CoreState::new(settings, locale, audio_thread, audio, audio_chain);
        let library = LibraryState::default();
        let playback = PlaybackSessionState::default();
        let ui = UiState::new();

        let app = Self {
            core,
            library,
            playback,
            ui,
        };

        // 4. Open main window
        let (window_id, open_window) =
            iced::window::open(crate::platform::window::window_settings());
        tracing::info!("Opening main window with id: {:?}", window_id);

        // 5. Initialize async tasks
        let open_window_and_init_services = open_window
            .then(move |_| {
                crate::platform::window::native_window_handle(window_id).map(|window_handle| {
                    match helpers::init_mpris(window_handle) {
                        Ok((handle, rx)) => Message::MprisStartedWithHandle(handle, rx),
                        Err(e) => {
                            tracing::warn!("Failed to start media controls: {}", e);
                            Message::Noop
                        }
                    }
                })
            })
            .chain(Task::done(Message::InitializeGlobalHotkeys));

        let init_task = Task::batch([
            open_window_and_init_services,
            // Protocol IPC listener and URI stream
            {
                let (ipc_tx, ipc_rx) = crate::protocol::ipc::ipc_channel();
                crate::platform::protocol::setup_macos_url_handler(ipc_tx.clone());
                crate::protocol::ipc::spawn_ipc_listener(ipc_tx);
                let ipc_stream = Task::run(
                    async_stream::stream! {
                        tokio::pin!(ipc_rx);
                        while let Some(msg) = ipc_rx.recv().await {
                            yield msg;
                        }
                    },
                    |msg| match msg {
                        crate::protocol::ipc::IpcMessage::Uri(uri) => Message::UriReceived(uri),
                        crate::protocol::ipc::IpcMessage::Focus => Message::ShowOrFocusWindow,
                    },
                );
                let pending = Task::done(
                    crate::protocol::ipc::take_pending_startup_uri()
                        .map(Message::UriReceived)
                        .unwrap_or(Message::Noop),
                );
                Task::batch([ipc_stream, pending])
            },
            Task::perform(helpers::init_database(), |result| match result {
                Ok(db) => Message::DatabaseReady(Arc::new(db)),
                Err(e) => Message::DatabaseError(e.into()),
            }),
            Task::perform(helpers::init_cover_cache(), |result| match result {
                Ok(cache) => Message::CoverCacheReady(Arc::new(cache)),
                Err(e) => Message::DatabaseError(AppError::with_message_source(
                    ErrorCode::StorageOpenFailed,
                    "Rustle could not initialize the cover cache",
                    e.to_string(),
                )),
            }),
            Task::perform(helpers::init_font_system(), |font_system| {
                Message::LyricsFontSystemReady(font_system)
            }),
            Task::done(Message::TryAutoLogin(0)),
            Task::done(Message::EnforceCacheLimit),
            audio_listener_task,
        ]);

        (app, init_task)
    }

    /// Application theme for a specific window
    pub fn theme(&self, _window_id: iced::window::Id) -> Theme {
        // Access settings via core state
        rustle_ui::theme::application(self.core.settings.display.dark_mode)
    }

    /// Dynamic window title based on current playback state
    pub fn title(&self, _window_id: iced::window::Id) -> String {
        // Access current song via library state
        if let Some(song) = &self.playback.current_song {
            format!("Rustle - {}", song.title)
        } else {
            "Rustle - Music".to_string()
        }
    }

    /// Subscriptions for animations, playback, keyboard events, and window close
    pub fn subscription(&self) -> iced::Subscription<Message> {
        use iced::keyboard;
        use iced::time::{Duration, Instant};

        let now = Instant::now();

        // Check if power saving mode is enabled
        let power_saving = self.core.settings.display.power_saving_mode;

        // 1. UI animations (disabled in power saving mode)
        let has_animations = if power_saving {
            false
        } else {
            self.ui.has_active_animations(now)
        };

        // 2. Playback state
        let is_playing = self.playback_is_playing();

        // 3. Lyrics page needs continuous updates for smooth scrolling
        let lyrics_needs_frames = if power_saving {
            false
        } else {
            self.ui.lyrics.is_open
        };

        // 4. Audio engine visualization
        let audio_engine_needs_frames = if power_saving {
            false
        } else {
            matches!(self.ui.current_route, Route::AudioEngine) && is_playing
        };

        // 5. Keyboard events
        let window_hidden = self.core.is_window_hidden();
        let keyboard_sub = if !window_hidden {
            keyboard::listen().filter_map(|event| match event {
                keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    Some(Message::KeyPressed(key, modifiers))
                }
                _ => None,
            })
        } else {
            iced::Subscription::none()
        };

        // 6. Window events
        let close_request_sub = iced::window::close_requests().map(|_id| Message::RequestClose);

        let window_throttled = subscription_logic::window_should_throttle(window_hidden);

        // 7. Animation subscription
        // Import finalization must advance even when decorative animations are
        // disabled. Use a lower frame rate in power saving mode.
        let animation_sub =
            if power_saving && !window_throttled && self.ui.importing_playlist.is_some() {
                iced::time::every(Duration::from_millis(33)).map(Message::AnimationTick)
            } else if subscription_logic::animation_subscription_enabled(
                window_throttled,
                has_animations,
                lyrics_needs_frames,
                audio_engine_needs_frames,
            ) {
                iced::window::frames().map(Message::AnimationTick)
            } else {
                iced::Subscription::none()
            };

        // 8. Playback monitoring
        let playback_sub = if let Some(interval) = subscription_logic::playback_tick_interval_ms(
            is_playing,
            window_throttled,
            power_saving,
        ) {
            iced::time::every(Duration::from_millis(interval)).map(|_| Message::PlaybackTick)
        } else {
            iced::Subscription::none()
        };

        // 9. Window lifecycle + resize/focus events
        let opened_sub = iced::window::open_events().map(|_id| Message::InitializeTray);
        let resize_sub =
            iced::window::resize_events().map(|(_id, size)| Message::WindowResized(size));
        let shown_sub = iced::window::shown_events().map(|_id| Message::WindowShown);
        let focus_sub = iced::window::events().filter_map(|(_id, event)| match event {
            iced::window::Event::Focused => Some(Message::WindowFocused),
            iced::window::Event::Unfocused => Some(Message::WindowUnfocused),
            _ => None,
        });

        // 10. Mouse events for sidebar resize and context menus
        let mouse_sub = if !window_hidden {
            iced::event::listen().filter_map(|event| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(button)) => {
                    subscription_logic::mouse_button_release_message(button)
                }
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::MouseMoved(position))
                }
                _ => None,
            })
        } else {
            iced::Subscription::none()
        };

        // 11. Player events - handled via Task::run in initialization, not subscription
        // (see handle_player_event_receiver_ready message)

        // Batch all subscriptions
        iced::Subscription::batch([
            // Maintenance outlives playback/download events and retries entries
            // once their ten-minute grace expires, including while hidden.
            iced::time::every(Duration::from_secs(60)).map(|_| Message::EnforceCacheLimit),
            update::audio_index::subscription(self.core.settings.storage.effective_download_dir()),
            keyboard_sub,
            close_request_sub,
            animation_sub, // Animation updates (vsync rate)
            playback_sub,  // Playback monitoring (100ms intervals)
            opened_sub,
            resize_sub,
            shown_sub,
            focus_sub,
            mouse_sub,
        ])
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new().0
    }
}

#[cfg(test)]
mod tests {
    use super::subscription_logic::*;
    use crate::app::Message;
    use iced::mouse::Button;

    #[test]
    fn side_mouse_buttons_map_to_navigation_history() {
        assert!(matches!(
            mouse_button_release_message(Button::Back),
            Some(Message::NavigateBack)
        ));
        assert!(matches!(
            mouse_button_release_message(Button::Forward),
            Some(Message::NavigateForward)
        ));
    }

    #[test]
    fn unrelated_mouse_buttons_do_not_trigger_navigation() {
        assert!(mouse_button_release_message(Button::Right).is_none());
        assert!(mouse_button_release_message(Button::Middle).is_none());
    }

    #[test]
    fn unfocused_visible_window_is_not_throttled() {
        assert!(
            !window_should_throttle(false),
            "Visible background windows should keep normal frame cadence"
        );
    }

    #[test]
    fn hidden_window_is_throttled_even_if_focused_state_is_stale() {
        assert!(
            window_should_throttle(true),
            "Only hidden/closed-to-tray windows should use the low cadence"
        );
    }

    #[test]
    fn playback_interval_only_uses_low_cadence_when_hidden() {
        assert_eq!(playback_tick_interval_ms(true, false, false), Some(100));
        assert_eq!(playback_tick_interval_ms(true, false, true), Some(500));
        assert_eq!(playback_tick_interval_ms(true, true, false), Some(1000));
        assert_eq!(playback_tick_interval_ms(false, false, false), None);
    }

    mod property_playback_independence {
        use super::*;

        #[test]
        fn playback_active_no_animations() {
            // When playing with no animations, playback subscription should be active
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                false, // lyrics_needs_frames
                false, // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(
                !needs_animation,
                "No animation subscription when no animations"
            );
            assert!(
                needs_playback,
                "Playback subscription must be active when playing"
            );
        }

        #[test]
        fn playback_active_with_lyrics_page() {
            // When playing with lyrics page open, BOTH subscriptions should be active
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                true,  // lyrics_needs_frames (lyrics page open)
                false, // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(needs_animation, "Animation subscription for lyrics page");
            assert!(
                needs_playback,
                "Playback subscription must be active when playing"
            );
        }

        #[test]
        fn playback_active_with_ui_animations() {
            // When playing with UI animations, BOTH subscriptions should be active
            let (needs_animation, needs_playback) = subscription_decisions(
                true,  // has_animations
                false, // lyrics_needs_frames
                false, // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(needs_animation, "Animation subscription for UI animations");
            assert!(
                needs_playback,
                "Playback subscription must be active when playing"
            );
        }

        #[test]
        fn playback_active_with_audio_engine_page() {
            // When playing with audio engine visualization, BOTH subscriptions should be active
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                false, // lyrics_needs_frames
                true,  // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(
                needs_animation,
                "Animation subscription for audio visualization"
            );
            assert!(
                needs_playback,
                "Playback subscription must be active when playing"
            );
        }

        #[test]
        fn playback_active_with_all_animations() {
            // When playing with all animation sources active, BOTH subscriptions should be active
            let (needs_animation, needs_playback) = subscription_decisions(
                true, // has_animations
                true, // lyrics_needs_frames
                true, // audio_engine_needs_frames
                true, // is_playing
            );
            assert!(needs_animation, "Animation subscription for all sources");
            assert!(
                needs_playback,
                "Playback subscription must be active when playing"
            );
        }

        #[test]
        fn playback_inactive_no_subscriptions() {
            // When not playing and no animations, no subscriptions needed
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                false, // lyrics_needs_frames
                false, // audio_engine_needs_frames
                false, // is_playing
            );
            assert!(!needs_animation, "No animation subscription");
            assert!(!needs_playback, "No playback subscription when not playing");
        }

        #[test]
        fn playback_inactive_with_animations() {
            // When not playing but animations active, only animation subscription
            let (needs_animation, needs_playback) = subscription_decisions(
                true,  // has_animations
                false, // lyrics_needs_frames
                false, // audio_engine_needs_frames
                false, // is_playing
            );
            assert!(needs_animation, "Animation subscription for UI animations");
            assert!(!needs_playback, "No playback subscription when not playing");
        }
    }

    mod property_coexistence {
        use super::*;

        #[test]
        fn both_subscriptions_when_lyrics_and_playing() {
            // Critical case: lyrics page open while playing
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                true,  // lyrics_needs_frames
                false, // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(
                needs_animation && needs_playback,
                "Both subscriptions must coexist when lyrics page is open and playing"
            );
        }

        #[test]
        fn both_subscriptions_when_audio_engine_and_playing() {
            // Audio engine visualization while playing
            let (needs_animation, needs_playback) = subscription_decisions(
                false, // has_animations
                false, // lyrics_needs_frames
                true,  // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(
                needs_animation && needs_playback,
                "Both subscriptions must coexist when audio engine page is open and playing"
            );
        }

        #[test]
        fn both_subscriptions_when_ui_animations_and_playing() {
            // UI animations (hover, fade) while playing
            let (needs_animation, needs_playback) = subscription_decisions(
                true,  // has_animations
                false, // lyrics_needs_frames
                false, // audio_engine_needs_frames
                true,  // is_playing
            );
            assert!(
                needs_animation && needs_playback,
                "Both subscriptions must coexist when UI animations are active and playing"
            );
        }

        #[test]
        fn subscriptions_are_independent() {
            // Verify that animation state changes don't affect playback subscription
            // and vice versa

            // Case 1: Animation state changes, playback stays active
            let (_, playback1) = subscription_decisions(false, false, false, true);
            let (_, playback2) = subscription_decisions(true, false, false, true);
            let (_, playback3) = subscription_decisions(false, true, false, true);
            let (_, playback4) = subscription_decisions(false, false, true, true);

            assert!(
                playback1 && playback2 && playback3 && playback4,
                "Playback subscription must be independent of animation state"
            );

            // Case 2: Playback state changes, animation stays active
            let (anim1, _) = subscription_decisions(true, false, false, false);
            let (anim2, _) = subscription_decisions(true, false, false, true);

            assert!(
                anim1 && anim2,
                "Animation subscription must be independent of playback state"
            );
        }
    }

    mod property_playback_isolation {
        use super::*;

        #[test]
        fn playback_only_depends_on_is_playing() {
            // Test all combinations of animation states with is_playing = true
            let animation_combinations = [
                (false, false, false),
                (true, false, false),
                (false, true, false),
                (false, false, true),
                (true, true, false),
                (true, false, true),
                (false, true, true),
                (true, true, true),
            ];

            for (has_anim, lyrics, audio_engine) in animation_combinations {
                let needs_playback = needs_playback_subscription(true);
                assert!(
                    needs_playback,
                    "Playback subscription must be active when is_playing=true, \
                     regardless of animation state (has_anim={}, lyrics={}, audio_engine={})",
                    has_anim, lyrics, audio_engine
                );
            }

            // Test all combinations with is_playing = false
            for (has_anim, lyrics, audio_engine) in animation_combinations {
                let needs_playback = needs_playback_subscription(false);
                assert!(
                    !needs_playback,
                    "Playback subscription must be inactive when is_playing=false, \
                     regardless of animation state (has_anim={}, lyrics={}, audio_engine={})",
                    has_anim, lyrics, audio_engine
                );
            }
        }
    }
}
