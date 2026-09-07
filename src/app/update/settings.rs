//! Settings update handlers

use crate::app::SettingsSection;
use crate::app::message::Message;
use crate::app::state::{App, Route};
use crate::cache;
use crate::features::keybindings::{KeyBinding, KeyCode, ModifierSet, ShortcutScope};
use crate::i18n::{Key as I18nKey, Language, Locale};
use iced::Task;
use iced::advanced::widget::operation::{self as widget_op, Operation, Outcome, Scrollable};
use iced::time::Instant;
use iced::widget::Id;
use iced::{Rectangle, Size};

use crate::ui::animation::SmoothScrollTarget;

fn match_section(id: &Id) -> Option<SettingsSection> {
    let all = [
        SettingsSection::Account,
        SettingsSection::Playback,
        SettingsSection::Display,
        SettingsSection::System,
        SettingsSection::Network,
        SettingsSection::Storage,
        SettingsSection::Shortcuts,
        SettingsSection::About,
    ];
    all.into_iter().find(|s| id == &s.widget_id())
}

/// Custom operation that traverses the widget tree and measures
/// the Y position of each settings section container relative to
/// the scrollable content origin.
fn measure_section_positions() -> impl Operation<Vec<(SettingsSection, f32)>> {
    struct MeasurePositions {
        positions: Vec<(SettingsSection, f32)>,
        /// Absolute Y of the scrollable content origin (captured from scrollable callback)
        content_origin_y: Option<f32>,
    }

    impl Operation<Vec<(SettingsSection, f32)>> for MeasurePositions {
        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<Vec<(SettingsSection, f32)>>),
        ) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            content_bounds: Rectangle,
            _translation: iced::Vector,
            _state: &mut dyn Scrollable,
        ) {
            if id == Some(&Id::new("settings_scroll")) {
                self.content_origin_y = Some(content_bounds.y);
            }
        }

        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            if let (Some(origin_y), Some(id)) = (self.content_origin_y, id)
                && let Some(section) = match_section(id)
            {
                // bounds.y is absolute (window-relative); subtract content
                // origin to get scrollable-content-relative position
                self.positions.push((section, bounds.y - origin_y));
            }
        }

        fn finish(&self) -> Outcome<Vec<(SettingsSection, f32)>> {
            Outcome::Some(self.positions.clone())
        }
    }

    MeasurePositions {
        positions: Vec::new(),
        content_origin_y: None,
    }
}

impl App {
    fn apply_recorded_keybinding(
        &mut self,
        action: crate::features::Action,
        scope: ShortcutScope,
        binding: KeyBinding,
    ) -> Task<Message> {
        self.finish_keybinding_recording(&binding);

        let cleared = matches!(binding.key, KeyCode::Delete | KeyCode::Backspace);
        let bindings = (!cleared).then_some(binding).into_iter().collect();

        match scope {
            ShortcutScope::Local => {
                self.core.settings.keybindings.set(action, bindings);
                Task::done(Message::SaveSettings)
            }
            ShortcutScope::Global => {
                self.update_global_keybinding(action, bindings.into_iter().next())
            }
        }
    }

    fn finish_keybinding_recording(&mut self, binding: &KeyBinding) {
        let already_seen = self.ui.global_hotkey_seen_while_recording.take();
        self.ui.editing_keybinding = None;

        let registered_id = crate::platform::global_hotkeys::hotkey_for_binding(binding)
            .map(|hotkey| hotkey.id())
            .filter(|id| {
                self.core
                    .global_hotkeys
                    .as_ref()
                    .is_some_and(|service| service.action_for_id(*id).is_some())
            });

        if let Some(id) = registered_id
            && already_seen != Some(id)
        {
            self.ui.suppressed_recording_hotkey = Some((
                id,
                iced::time::Instant::now() + std::time::Duration::from_millis(500),
            ));
        }
    }

    fn update_global_keybinding(
        &mut self,
        action: crate::features::Action,
        new_binding: Option<KeyBinding>,
    ) -> Task<Message> {
        let old_binding = self
            .core
            .settings
            .keybindings
            .global_binding(&action)
            .cloned();

        let registration_result: Result<(), crate::error::AppError> = if let Some(service) =
            &mut self.core.global_hotkeys
        {
            service
                .replace(action, new_binding.as_ref())
                .map_err(crate::error::AppError::from)
        } else if new_binding.is_none() {
            Ok(())
        } else {
            Err(crate::platform::global_hotkeys::GlobalHotkeyError::unsupported_session().into())
        };

        if let Err(error) = registration_result {
            tracing::warn!(
                code = %error.code(),
                recovery = ?error.recovery(),
                "Global shortcut update was rejected"
            );
            return Self::toast_error(format!(
                "{}: {}",
                self.core.locale.get(I18nKey::SettingsGlobalShortcutError),
                error.user_summary()
            ));
        }

        self.core
            .settings
            .keybindings
            .set_global(action, new_binding.clone().into_iter().collect());

        if let Err(error) = self.core.settings.save() {
            self.core
                .settings
                .keybindings
                .set_global(action, old_binding.clone().into_iter().collect());
            let save_error = crate::error::AppError::with_source(
                crate::error::ErrorCode::StorageQueryFailed,
                "Rustle could not save the shortcut settings",
                error,
            );
            if let Some(service) = &mut self.core.global_hotkeys
                && let Err(rollback_error) = service.replace(action, old_binding.as_ref())
            {
                let rollback_error = crate::error::AppError::from(rollback_error);
                tracing::error!(
                    code = %rollback_error.code(),
                    recovery = ?rollback_error.recovery(),
                    "Failed to restore global shortcut after settings save failure"
                );
            }

            tracing::error!(
                code = %save_error.code(),
                recovery = ?save_error.recovery(),
                "Failed to persist global shortcut settings"
            );
            return Self::toast_error(format!(
                "{}: {}",
                self.core.locale.get(I18nKey::SettingsGlobalShortcutError),
                save_error.user_summary()
            ));
        }

        Task::none()
    }

    /// Look up the scroll position of a section (from measured or seed positions)
    pub(super) fn section_scroll_position(&self, section: SettingsSection) -> f32 {
        self.ui
            .section_positions
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, p)| *p)
            .unwrap_or(0.0)
    }

    /// Get which section corresponds to a scroll Y offset (for tab highlight)
    fn section_at_position(&self, y_offset: f32) -> SettingsSection {
        let activation_tolerance = crate::ui::responsive::ResponsiveContext::from_viewport(
            Size::new(self.core.window_width, self.core.window_height),
        )
        .tokens
        .size(20.0);
        let mut current = SettingsSection::Account;
        for (section, pos) in &self.ui.section_positions {
            if y_offset >= *pos - activation_tolerance {
                current = *section;
            } else {
                break;
            }
        }
        current
    }

    pub(super) fn sync_settings_section_route(&mut self, section: SettingsSection) {
        self.ui.active_settings_section = section;
        if matches!(self.ui.current_route, Route::Settings(_)) {
            self.ui.current_route = Route::Settings(section);
            self.ui
                .nav_history
                .replace_current(crate::app::state::NavigationEntry::Route(
                    self.ui.current_route.clone(),
                ));
        }
    }
}

impl App {
    pub(super) fn refresh_cache_stats(&mut self) {
        let stats = cache::calculate_cache_stats();
        self.ui.cache_stats = Some(stats);
    }

    /// Handle settings-related messages
    pub fn handle_settings(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::UpdateCloseBehavior(behavior) => {
                self.core.settings.close_behavior = *behavior;
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateFadeInOut(enabled) => {
                self.core.settings.playback.fade_in_out = *enabled;
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateAutomixEnabled(enabled) => {
                self.core.settings.playback.automix_enabled = *enabled;
                if *enabled {
                    if let Some(song) = self.playback.current_song.clone() {
                        self.schedule_automix_analysis_window(&song);
                    }
                } else {
                    self.clear_scheduled_transition_state();
                }
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateVolumeNormalization(enabled) => {
                self.core.settings.playback.volume_normalization = *enabled;
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateMusicQuality(quality) => {
                self.core.settings.playback.music_quality = *quality;
                // Update NcmClient's quality setting
                if let Some(client) = &self.core.ncm_client {
                    client.set_quality(quality.to_api_rate());
                }
                tracing::info!("Music quality changed to: {:?}", quality);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateEqualizerEnabled(enabled) => {
                self.core.settings.playback.equalizer_enabled = *enabled;
                self.set_audio_equalizer_enabled(*enabled);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateEqualizerPreset(preset) => {
                use crate::features::EqualizerPreset;
                self.core.settings.playback.equalizer_preset = *preset;
                // Apply preset values (unless custom)
                if *preset != EqualizerPreset::Custom {
                    let values = preset.values();
                    self.core.settings.playback.equalizer_values = values;
                    self.set_audio_equalizer_gains(values);
                }
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateEqualizerValues(values) => {
                self.core.settings.playback.equalizer_values = *values;
                // When manually adjusting, switch to custom preset
                self.core.settings.playback.equalizer_preset =
                    crate::features::EqualizerPreset::Custom;
                self.set_audio_equalizer_gains(*values);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateEqualizerPreamp(preamp) => {
                self.core.settings.playback.equalizer_preamp = *preamp;
                self.set_audio_preamp(*preamp);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateSpectrumDecay(decay) => {
                self.core.settings.playback.spectrum_decay = *decay;
                self.set_audio_analysis_decay(*decay);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateSpectrumBarsMode(bars_mode) => {
                self.core.settings.playback.spectrum_bars_mode = *bars_mode;
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateDarkMode(enabled) => {
                self.core.settings.display.dark_mode = *enabled;
                tracing::info!("Dark mode: {}", enabled);
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateAppLanguage(language) => {
                let lang = Language::from_code(language).unwrap_or_default();
                self.core.settings.display.language = lang.code().to_string();
                self.core.locale = Locale::new(lang);
                self.update_tray_and_mpris_current(self.playback_is_playing());
                tracing::info!("Language changed to: {}", lang.code());
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateLyricsFontFamily(family) => {
                let changed =
                    self.core.settings.lyrics.lyrics_font_family.as_deref() != family.as_deref();
                if changed {
                    self.core.settings.lyrics.lyrics_font_family = family.clone();

                    // Update engine's text shaper so synchronous shaping uses the new font
                    if let Some(engine_cell) = &self.ui.lyrics.engine
                        && let Some(font_system) = &self.ui.lyrics.shared_font_system
                    {
                        let mut engine = engine_cell.borrow_mut();
                        engine.set_font_family(family.clone(), font_system.clone());
                    }

                    // Invalidate all caches and trigger re-shape
                    self.ui.lyrics.cached_shaped_lines = None;
                    self.playback.lyrics_render_manager =
                        crate::app::update::lyrics_render_manager::LyricsRenderManager::default();
                    let reshape_task = self.request_lyrics_shaping_for_current_viewport();

                    tracing::info!("Lyrics font family changed to {:?}", family);

                    return Some(Task::batch([
                        Task::perform(async { Message::SaveSettings }, |m| m),
                        reshape_task,
                    ]));
                }
                Some(Task::none())
            }
            Message::UpdatePowerSavingMode(enabled) => {
                self.core.settings.display.power_saving_mode = *enabled;
                self.sync_audio_analysis_state();
                tracing::info!("Power saving mode: {}", enabled);

                let smooth_scroll_task = if *enabled {
                    self.settle_smooth_scroll()
                } else {
                    Task::none()
                };

                let lyrics_viewport_task = if *enabled {
                    if self.ui.lyrics.is_open {
                        self.ui.lyrics.animation.settle_at(1.0);
                    }
                    self.flush_pending_lyrics_viewport_after_animation()
                } else {
                    Task::none()
                };

                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    lyrics_viewport_task,
                    smooth_scroll_task,
                ]))
            }
            Message::UpdateMaxCacheMb(size_mb) => {
                self.core.settings.storage.max_cache_mb = *size_mb;
                // Save settings and enforce the new cache limit
                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    Task::done(Message::EnforceCacheLimit),
                ]))
            }
            Message::UpdateDownloadQuality(quality) => {
                self.core.settings.storage.download_quality = *quality;
                Some(Task::done(Message::SaveSettings))
            }
            Message::UpdateDownloadDir(dir) => {
                self.core.settings.storage.download_dir = dir.clone();
                if let Some(path) = dir {
                    let _ = std::fs::create_dir_all(path);
                }
                Some(Task::done(Message::SaveSettings))
            }
            Message::UpdateDownloadDirDialog => Some(Task::perform(
                crate::app::helpers::open_folder_dialog(),
                |result| {
                    Message::UpdateDownloadDir(result.map(|p| p.to_string_lossy().to_string()))
                },
            )),
            Message::ClearCache => Some(Task::perform(
                async {
                    let result = cache::clear_all_cache();
                    Message::CacheCleared(result.files_deleted, result.bytes_freed)
                },
                |m| m,
            )),
            Message::CacheCleared(files, bytes) => {
                tracing::info!(
                    "Cache cleared: {} files, {} MB freed",
                    files,
                    bytes / (1024 * 1024)
                );
                // Recalculate cache stats after clearing
                Some(Task::perform(async { Message::RefreshCacheStats }, |m| m))
            }
            Message::RefreshCacheStats => {
                self.refresh_cache_stats();
                Some(Task::none())
            }
            Message::EnforceCacheLimit => {
                let max_mb = self.core.settings.storage.max_cache_mb;
                Some(Task::perform(
                    async move {
                        let result = cache::enforce_cache_limit(max_mb);
                        if result.files_deleted > 0 {
                            tracing::info!(
                                "Cache limit enforced: {} files deleted, {} MB freed",
                                result.files_deleted,
                                result.mb_freed()
                            );
                        }
                        Message::RefreshCacheStats
                    },
                    |m| m,
                ))
            }
            Message::UpdateAudioOutputDevice(device) => {
                self.core.settings.system.audio_output_device = device.clone();
                self.switch_audio_output_device(device.clone());
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateDiscordEnabled(enabled) => {
                self.core.settings.system.discord_enabled = *enabled;
                self.core.discord_presence.set_enabled(*enabled);
                if !*enabled {
                    // Clear presence immediately when disabled
                    return Some(Task::run(
                        futures_util::stream::once(async move {
                            crate::platform::discord::clear_activity_oneshot().await;
                        }),
                        |_| Message::SaveSettings,
                    ));
                }
                Some(Task::perform(async { Message::SaveSettings }, |m| m))
            }
            Message::UpdateProxyType(proxy_type) => {
                self.core.settings.network.proxy_type = *proxy_type;
                tracing::info!("Proxy type changed to: {:?}", proxy_type);
                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    Task::done(Message::ApplyProxySettings),
                ]))
            }
            Message::UpdateProxyHost(host) => {
                self.core.settings.network.proxy_host = host.clone();
                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    Task::done(Message::ApplyProxySettings),
                ]))
            }
            Message::UpdateProxyPort(port_str) => {
                if let Ok(port) = port_str.parse::<u16>() {
                    self.core.settings.network.proxy_port = port;
                    Some(Task::batch([
                        Task::perform(async { Message::SaveSettings }, |m| m),
                        Task::done(Message::ApplyProxySettings),
                    ]))
                } else if port_str.is_empty() {
                    self.core.settings.network.proxy_port = 0;
                    Some(Task::none())
                } else {
                    Some(Task::none())
                }
            }
            Message::UpdateProxyUsername(username) => {
                self.core.settings.network.proxy_username = if username.is_empty() {
                    None
                } else {
                    Some(username.clone())
                };
                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    Task::done(Message::ApplyProxySettings),
                ]))
            }
            Message::UpdateProxyPassword(password) => {
                self.core.settings.network.proxy_password = if password.is_empty() {
                    None
                } else {
                    Some(password.clone())
                };
                Some(Task::batch([
                    Task::perform(async { Message::SaveSettings }, |m| m),
                    Task::done(Message::ApplyProxySettings),
                ]))
            }
            Message::ApplyProxySettings => {
                if let Some(client) = &mut self.core.ncm_client {
                    if let Some(proxy_url) = self.core.settings.network.proxy_url() {
                        match client.set_proxy(proxy_url.clone()) {
                            Ok(()) => tracing::info!("Proxy applied: {}", proxy_url),
                            Err(e) => tracing::error!("Failed to apply proxy: {}", e),
                        }
                    } else {
                        tracing::info!("Proxy disabled");
                        // When proxy is disabled, recreate client without proxy
                        // and sync quality setting
                        let quality = self.core.settings.playback.music_quality.to_api_rate();
                        if let Some(cookie) = crate::api::NcmClient::load_cookie_from_file() {
                            *client = crate::api::NcmClient::from_cookie(cookie);
                        } else {
                            *client = crate::api::NcmClient::new();
                        }
                        client.set_quality(quality);
                    }
                }
                Some(Task::none())
            }
            Message::ScrollToSection(section) => {
                self.sync_settings_section_route(*section);
                let target_y = self.section_scroll_position(*section);
                let delta = target_y - self.ui.settings_scroll_offset;
                let target = SmoothScrollTarget::Native("settings_scroll");

                if self.core.settings.display.power_saving_mode {
                    Some(self.apply_smooth_scroll_delta(target, delta))
                } else {
                    self.ui
                        .smooth_scroll
                        .request_programmatic(target, delta, Instant::now());
                    Some(Task::none())
                }
            }
            Message::SettingsScrolled(y_offset) => {
                self.ui.settings_scroll_offset = *y_offset;
                // Only update active tab highlight — do NOT overwrite measured positions
                let section = self.section_at_position(*y_offset);
                self.sync_settings_section_route(section);
                Some(Task::none())
            }
            Message::SectionPositionsMeasured(positions) => {
                // Replace seed positions with actual measured positions
                for (section, pos) in positions {
                    self.ui.section_positions.retain(|(s, _)| *s != *section);
                    self.ui.section_positions.push((*section, *pos));
                }
                self.ui
                    .section_positions
                    .sort_by_key(|(_, p)| (*p * 100.0) as i32);
                self.ui.positions_measured = true;
                tracing::info!(
                    "Settings section positions measured: {:?}",
                    self.ui.section_positions
                );
                Some(Task::none())
            }
            Message::MeasureSectionPositions => Some(iced_runtime::task::widget(widget_op::map(
                measure_section_positions(),
                Message::SectionPositionsMeasured,
            ))),
            Message::StartEditingKeybinding(action, scope) => {
                self.ui.global_hotkey_seen_while_recording = None;
                self.ui.suppressed_recording_hotkey = None;
                self.ui.editing_keybinding = Some((*action, *scope));
                Some(Task::none())
            }
            Message::CancelEditingKeybinding => {
                self.ui.global_hotkey_seen_while_recording = None;
                self.ui.editing_keybinding = None;
                Some(Task::none())
            }
            Message::KeybindingKeyPressed(key, modifiers) => {
                if let Some((action, scope)) = self.ui.editing_keybinding {
                    // Convert iced key to our KeyCode
                    if let Some(key_code) = crate::platform::keybindings::key_to_keycode(key) {
                        let binding = KeyBinding {
                            modifiers: ModifierSet {
                                ctrl: modifiers.control(),
                                cmd: modifiers.logo(),
                                alt: modifiers.alt(),
                                shift: modifiers.shift(),
                            },
                            key: key_code,
                        };
                        return Some(self.apply_recorded_keybinding(action, scope, binding));
                    }
                }
                Some(Task::none())
            }
            Message::GlobalHotkeyPressed(id) => {
                let (action, scope) = self.ui.editing_keybinding?;

                let binding = self
                    .core
                    .global_hotkeys
                    .as_ref()
                    .and_then(|service| service.action_for_id(*id))
                    .and_then(|registered_action| {
                        self.core
                            .settings
                            .keybindings
                            .global_binding(&registered_action)
                            .cloned()
                    })
                    .filter(|binding| {
                        crate::platform::global_hotkeys::hotkey_for_binding(binding)
                            .is_some_and(|hotkey| hotkey.id() == *id)
                    });

                if let Some(binding) = binding {
                    self.ui.global_hotkey_seen_while_recording = Some(*id);
                    return Some(self.apply_recorded_keybinding(action, scope, binding));
                }

                // Never execute a native action while the recorder owns input,
                // even if an inconsistent stale event cannot be reconstructed.
                Some(Task::none())
            }
            Message::SaveSettings => {
                if let Err(e) = self.core.settings.save() {
                    tracing::error!("Failed to save settings: {}", e);
                } else {
                    tracing::info!("Settings saved successfully");
                }
                Some(Task::none())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::Key;

    #[test]
    fn converts_character_space_to_space_keycode() {
        assert_eq!(
            crate::platform::keybindings::key_to_keycode(&Key::Character(" ".into())),
            Some(KeyCode::Space)
        );
    }
}
