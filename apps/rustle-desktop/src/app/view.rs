// src/app/view.rs
//! Application view rendering

use iced::mouse::Interaction;
use iced::widget::{
    Space, button, column, container, mouse_area, responsive, row, scrollable, stack, text,
};
use iced::{Alignment, Color, Element, Fill, Length};

use super::message::Message;
use super::{App, Route};
use crate::image::ImageKind;
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::components::cover_image;
use crate::ui::overlay::{self, ModalKind, OverlayKind};
use crate::ui::responsive::{
    ChromeRole, CoverRadiusRole, RadiusRole, ResponsiveContext, TargetRole, TextRole,
};
use crate::ui::theme;
use crate::ui::{components, pages, widgets};

impl App {
    fn current_song_artist_id(&self) -> Option<u64> {
        self.playback.current_artist_id
    }

    /// Build the view for a specific window
    pub fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        responsive(move |size| {
            self.view_with_context(window_id, ResponsiveContext::from_viewport(size))
        })
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn view_with_context(
        &self,
        _window_id: iced::window::Id,
        context: ResponsiveContext,
    ) -> Element<'_, Message> {
        // Check if lyrics page is open or animating
        let lyrics_progress = self.ui.lyrics.animation.progress();
        let lyrics_animating = self.ui.lyrics.animation.is_animating();
        let lyrics_overlay: Element<'_, Message> =
            if self.ui.lyrics.is_open || lyrics_animating || lyrics_progress > 0.01 {
                if let Some(song) = &self.playback.current_song {
                    let runtime = self.playback_runtime();
                    // Get playback info - same logic as player bar for consistency
                    let (is_playing, position, duration) =
                        if runtime.has_loaded_audio && runtime.info.duration.as_secs_f32() > 0.0 {
                            (
                                runtime.is_playing(),
                                runtime.info.position.as_secs_f32()
                                    / runtime.info.duration.as_secs_f32().max(1.0),
                                runtime.info.duration.as_secs_f32(),
                            )
                        } else {
                            // Player exists but no file loaded yet (e.g., NCM song still resolving)
                            // Use saved state for display
                            let saved_pos = self
                                .playback
                                .saved_state
                                .as_ref()
                                .map(|s| s.position_secs as f32)
                                .unwrap_or(0.0);
                            let song_duration = song.duration_secs.max(1) as f32;
                            (false, saved_pos / song_duration, song_duration)
                        };

                    // Use preview position while seeking, otherwise use actual position
                    let display_position = self.ui.seek_preview_position.unwrap_or(position);

                    pages::lyrics::view(pages::lyrics::LyricsPageView {
                        song,
                        image_state: &self.ui.image_state,
                        artist_id: self.current_song_artist_id(),
                        is_playing,
                        position: display_position,
                        duration_secs: duration,
                        cached_engine_lines: self.ui.lyrics.cached_engine_lines.as_ref(),
                        play_mode: self.core.settings.play_mode,
                        animation_progress: lyrics_progress,
                        bg_colors: &self.ui.lyrics.bg_colors,
                        textured_bg_shader: &self.ui.lyrics.textured_bg_shader,
                        lyrics_engine: self.ui.lyrics.engine.as_ref(),
                        power_saving_mode: self.core.settings.display.power_saving_mode,
                        is_liked: if song.id < 0 {
                            let ncm_id = (-song.id) as u64;
                            self.core
                                .user_info
                                .as_ref()
                                .map(|u| u.like_songs.contains(&ncm_id))
                                .unwrap_or(false)
                        } else {
                            false
                        },
                        download_progress: self.playback_cache_progress(),
                        is_fm_mode: self.is_fm_mode(),
                        is_maximized: self.core.window_maximized,
                        display_mode: self.ui.lyrics.display_mode,
                        context,
                    })
                } else {
                    Space::new().width(0).height(0).into()
                }
            } else {
                Space::new().width(0).height(0).into()
            };

        // Left sidebar
        let sidebar = components::sidebar::view(
            components::sidebar::SidebarView {
                current_route: &self.ui.current_route,
                locale: self.core.locale,
                is_logged_in: self.core.is_logged_in,
                importing_playlist: self.ui.importing_playlist.as_ref(),
                playlists: &self.library.playlists,
                user_playlists: &self.ui.home.user_playlists,
                image_state: &self.ui.image_state,
                sidebar_animations: &self.ui.sidebar_animations,
                sidebar_width: self.ui.sidebar_width,
                my_playlists_expanded: self.ui.my_playlists_expanded,
                collected_playlists_expanded: self.ui.collected_playlists_expanded,
                context,
            },
            false,
        );

        // Sidebar resize handle (draggable divider)
        let resize_handle =
            components::sidebar_resize_handle::view(context, self.ui.sidebar_dragging);

        // Determine main content: playlist page or nav page
        let liked_songs = self.core.user_info.as_ref().map(|u| &u.like_songs);

        let current_user_id = self.core.user_info.as_ref().map(|u| u.user_id);

        let current_playing_id = self.playback.current_song.as_ref().map(|s| s.id);

        let active_personal_fm_cover = if self.is_fm_mode() {
            self.playback
                .current_song
                .as_ref()
                .and_then(|song| crate::image::song_cover_key_for_source(song.id, &song.file_path))
                .and_then(|(kind, id)| self.ui.image_state.get(kind, id))
        } else {
            None
        };

        let main_content = match &self.ui.current_route {
            Route::Playlist(_)
            | Route::NcmPlaylist(_)
            | Route::Album(_)
            | Route::RecentlyPlayed => {
                if let Some(playlist) = &self.ui.playlist_page.current {
                    pages::playlist::view(pages::playlist::DetailPageView {
                        detail: playlist,
                        image_state: &self.ui.image_state,
                        song_animations: &self.ui.playlist_page.song_animations,
                        icon_animations: &self.ui.playlist_page.icon_animations,
                        search_animation: &self.ui.playlist_page.search_animation,
                        search_expanded: self.ui.playlist_page.search_expanded,
                        search_query: &self.ui.playlist_page.search_query,
                        liked_songs,
                        locale: self.core.locale,
                        scroll_state: self.ui.playlist_page.scroll_state.clone(),
                        current_user_id,
                        current_playing_id,
                        description_expanded: self.ui.playlist_page.description_expanded,
                        gradient_source: self.ui.playlist_page.gradient_source(),
                        gradient_progress: self.ui.playlist_page.gradient_animation.progress(),
                        context,
                    })
                } else {
                    pages::playlist::gradient_placeholder(self.ui.playlist_page.gradient_source())
                }
            }
            Route::User(_) => {
                if let Some(playlist) = &self.ui.playlist_page.current {
                    pages::user::view(
                        playlist,
                        &self.ui.image_state,
                        self.core.locale,
                        self.ui.playlist_page.description_expanded,
                        self.ui.playlist_page.gradient_source(),
                        self.ui.playlist_page.gradient_animation.progress(),
                        context,
                    )
                } else {
                    pages::playlist::gradient_placeholder(self.ui.playlist_page.gradient_source())
                }
            }
            Route::Artist(_) => {
                if let Some(playlist) = &self.ui.playlist_page.current {
                    pages::artist::view(pages::playlist::DetailPageView {
                        detail: playlist,
                        image_state: &self.ui.image_state,
                        song_animations: &self.ui.playlist_page.song_animations,
                        icon_animations: &self.ui.playlist_page.icon_animations,
                        search_animation: &self.ui.playlist_page.search_animation,
                        search_expanded: self.ui.playlist_page.search_expanded,
                        search_query: &self.ui.playlist_page.search_query,
                        liked_songs,
                        locale: self.core.locale,
                        scroll_state: self.ui.playlist_page.scroll_state.clone(),
                        current_user_id,
                        current_playing_id,
                        description_expanded: self.ui.playlist_page.description_expanded,
                        gradient_source: self.ui.playlist_page.gradient_source(),
                        gradient_progress: self.ui.playlist_page.gradient_animation.progress(),
                        context,
                    })
                } else {
                    pages::playlist::gradient_placeholder(self.ui.playlist_page.gradient_source())
                }
            }
            Route::Search { .. } => pages::search::view(
                &self.ui.search,
                &self.ui.image_state,
                self.core.locale,
                context,
            ),
            Route::Discover(mode) => {
                let _ = mode;
                pages::discover::view(
                    &self.ui.discover,
                    &self.ui.image_state,
                    self.core.locale,
                    active_personal_fm_cover,
                    context,
                )
            }
            Route::Radio => pages::discover::view(
                &self.ui.discover,
                &self.ui.image_state,
                self.core.locale,
                active_personal_fm_cover,
                context,
            ),
            Route::Downloads => crate::ui::components::download_panel::download_panel(
                self.core.locale,
                &self.core.download_manager,
                self.ui.download_tab,
                &self.ui.image_state,
                context,
            ),
            Route::Settings(section) => {
                let _ = section;
                pages::settings::view(pages::settings::SettingsPageView {
                    settings: &self.core.settings,
                    audio_devices: self.audio_output_devices(),
                    font_families: self.lyrics_font_families(),
                    active_section: self.ui.active_settings_section,
                    locale: self.core.locale,
                    editing_keybinding: self.ui.editing_keybinding,
                    is_logged_in: self.core.is_logged_in,
                    user_info: self.core.user_info.as_ref(),
                    image_state: &self.ui.image_state,
                    cache_stats: self.ui.cache_stats.as_ref(),
                    context,
                })
            }
            Route::AudioEngine => pages::audio_engine::view(
                &self.core.settings,
                self.core.locale,
                Some(self.playback_analysis_data()),
                context,
            ),
        };

        let top_bar = components::window_controls::view(components::window_controls::TopBarView {
            context,
            locale: self.core.locale,
            can_go_back: self.ui.nav_history.can_go_back(),
            can_go_forward: self.ui.nav_history.can_go_forward(),
            search_query: &self.ui.search_query,
            is_logged_in: self.core.is_logged_in,
            user_info: self.core.user_info.as_ref(),
            image_state: &self.ui.image_state,
            show_background: !self.ui.current_route.has_gradient_background(),
            is_maximized: self.core.window_maximized,
        });
        let controls_overlay = container(top_bar).width(Fill).padding(0);

        // Right panel with content and window controls overlay
        let right_panel = container(
            stack![main_content, controls_overlay,]
                .width(Fill)
                .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(theme::main_content);

        // Build player bar (always visible, full width at bottom)
        let runtime = self.playback_runtime();
        let (is_playing, position, duration, volume) =
            if runtime.has_loaded_audio && runtime.info.duration.as_secs_f32() > 0.0 {
                (
                    runtime.is_playing(),
                    runtime.display_position.as_secs_f32(),
                    runtime.info.duration.as_secs_f32().max(1.0),
                    runtime.info.volume,
                )
            } else {
                let saved_pos = self
                    .playback
                    .saved_state
                    .as_ref()
                    .map(|s| s.position_secs as f32)
                    .unwrap_or(0.0);
                let saved_vol = self
                    .playback
                    .saved_state
                    .as_ref()
                    .map(|s| s.volume as f32)
                    .unwrap_or(1.0);
                let song_duration = self
                    .playback
                    .current_song
                    .as_ref()
                    .map(|s| s.duration_secs as f32)
                    .unwrap_or(1.0)
                    .max(1.0);
                (false, saved_pos, song_duration, saved_vol)
            };

        let display_position = if let Some(preview) = self.ui.seek_preview_position {
            preview
        } else {
            position / duration
        };

        let is_buffering = self.playback_is_buffering();
        let is_fm_mode = self.is_fm_mode();
        let is_first_song = self
            .playback
            .current_index
            .map(|idx| idx == 0)
            .unwrap_or(true);

        let current_song_cover = self.playback.current_song.as_ref().and_then(|song| {
            let (kind, id) = crate::image::song_cover_key_for_source(song.id, &song.file_path)?;
            self.ui.image_state.get(kind, id)
        });

        let current_favorite = self.playback.current_song.as_ref().and_then(|song| {
            (song.id < 0).then(|| {
                let ncm_id = (-song.id) as u64;
                let is_liked = self
                    .core
                    .user_info
                    .as_ref()
                    .is_some_and(|user| user.like_songs.contains(&ncm_id));
                (ncm_id, is_liked)
            })
        });

        let progress_colors = self.playback.current_song.as_ref().and_then(|song| {
            self.playback
                .preload_coordinator
                .background_colors(song.id)
                .map(|colors| {
                    let to_color = |[r, g, b, a]: [f32; 4]| iced::Color::from_rgba(r, g, b, a);
                    [to_color(colors.0), to_color(colors.1), to_color(colors.2)]
                })
        });

        let player_bar = components::player_bar::view(components::player_bar::PlayerBarView {
            context,
            current_song: self.playback.current_song.as_ref(),
            current_artists: &self.playback.current_artists,
            is_playing,
            position: display_position,
            duration_secs: duration,
            volume,
            play_mode: self.core.settings.play_mode,
            current_favorite,
            progress_colors,
            is_buffering,
            download_progress: self.playback_cache_progress(),
            is_fm_mode,
            is_first_song,
            current_song_cover,
            current_quality: self.playback.current_quality.as_ref(),
        });

        // Queue overlay - full width, positioned above player bar
        let queue_overlay: Element<'_, Message> = if self.ui.queue_visible {
            let queue_popup = components::queue_panel::view(
                context,
                &self.playback.queue,
                self.playback.current_index,
                self.core.locale,
                is_fm_mode,
            );

            overlay::block_mouse_events(
                mouse_area(
                    container(
                        column![
                            Space::new().height(Fill),
                            container(queue_popup)
                                .width(Fill)
                                .align_x(Alignment::End)
                                .padding(
                                    iced::Padding::new(0.0)
                                        .right(context.tokens.space(20.0))
                                        .bottom(context.tokens.space(8.0))
                                ),
                            Space::new().height(context.tokens.chrome(ChromeRole::PlayerBar)),
                        ]
                        .width(Fill)
                        .height(Fill),
                    )
                    .width(Fill)
                    .height(Fill),
                )
                .interaction(Interaction::Idle)
                .on_press(Message::ToggleQueue)
                .into(),
            )
        } else {
            Space::new().width(0).height(0).into()
        };

        // Content row: sidebar + resize handle + right panel (fills remaining vertical space)
        let content_row = row![sidebar, resize_handle, right_panel]
            .width(Fill)
            .height(Fill);

        // Main layout: content row on top, full-width player bar at bottom
        let main_layout: Element<'_, Message> = stack![
            column![content_row, player_bar].width(Fill).height(Fill),
            queue_overlay,
        ]
        .width(Fill)
        .height(Fill)
        .into();

        // Compact profiles keep the rail (or no sidebar on the narrowest
        // profile) in the content row and promote the complete navigation
        // tree to a blocking drawer when requested.
        let sidebar_drawer_overlay: Element<'_, Message> =
            if context.profile.uses_navigation_drawer() && self.ui.sidebar_drawer_visible() {
                components::sidebar::drawer_view(
                    components::sidebar::SidebarView {
                        current_route: &self.ui.current_route,
                        locale: self.core.locale,
                        is_logged_in: self.core.is_logged_in,
                        importing_playlist: self.ui.importing_playlist.as_ref(),
                        playlists: &self.library.playlists,
                        user_playlists: &self.ui.home.user_playlists,
                        image_state: &self.ui.image_state,
                        sidebar_animations: &self.ui.sidebar_animations,
                        sidebar_width: self.ui.sidebar_width,
                        my_playlists_expanded: self.ui.my_playlists_expanded,
                        collected_playlists_expanded: self.ui.collected_playlists_expanded,
                        context,
                    },
                    self.ui.sidebar_drawer_progress(),
                )
            } else {
                Space::new().width(0).height(0).into()
            };

        // Build overlays - always use consistent stack structure to preserve scroll

        // Toast overlay (empty space if not visible)
        // Intentionally pointer-transparent: notifications should not disturb controls beneath.
        let toast_overlay: Element<'_, Message> = if self.ui.toast_visible {
            if let Some(toast) = &self.ui.toast {
                let toast_widget = widgets::view_toast(toast, context.tokens);
                container(toast_widget)
                    .width(Fill)
                    .padding(context.tokens.space(20.0))
                    .align_x(Alignment::Center)
                    .into()
            } else {
                Space::new().width(0).height(0).into()
            }
        } else {
            Space::new().width(0).height(0).into()
        };

        // Login popup overlay (independent — not managed by overlay stack)
        let login_popup_overlay = components::login_popup::view(
            context,
            self.ui.home.login_popup_open,
            self.ui.home.qr_code_path.as_ref(),
            self.ui.home.qr_status.as_deref(),
            self.core.user_info.as_ref(),
            self.core.is_logged_in,
            self.core.locale,
        );

        // Context menu overlay — positioned at cursor (independent)
        let context_menu_overlay: Element<'_, Message> =
            if let Some(ref menu) = self.ui.context_menu {
                components::context_menu::view(menu, self.core.locale, context)
            } else {
                Space::new().width(0).height(0).into()
            };

        // ── Unified modal overlay slots ──
        let locale = self.core.locale;
        let modal_slot = |idx: usize, context: ResponsiveContext| -> Element<'_, Message> {
            match self.ui.overlay_stack.get(idx) {
                Some(entry) => match &entry.kind {
                    OverlayKind::Modal(kind, config) => {
                        let content: Element<'_, Message> = match kind {
                            ModalKind::SongEdit(edit) => {
                                let title = locale.get(crate::i18n::Key::SongEditTitle).to_string();
                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(
                                        context,
                                        components::song_info_dialog::view_edit_body(
                                            edit, locale, context,
                                        ),
                                    ),
                                    overlay::modal_footer(
                                        context,
                                        vec![
                                            cancel_btn(locale, context),
                                            accent_btn(
                                                locale
                                                    .get(crate::i18n::Key::SongEditSave)
                                                    .to_string(),
                                                Message::SaveSongEdits(edit.song_id),
                                                context,
                                            ),
                                        ],
                                    ),
                                )
                            }
                            ModalKind::PlaylistEdit {
                                playlist_id: _,
                                name,
                                description,
                                cover_path,
                                watch_enabled,
                                watch_available,
                                watch_path,
                            } => {
                                let body = components::edit_dialog::view_body(
                                    components::edit_dialog::PlaylistEditorView {
                                        name,
                                        description,
                                        cover_path: cover_path.as_deref(),
                                        watch_available: *watch_available,
                                        watch_enabled: *watch_enabled,
                                        watch_path: watch_path.as_deref(),
                                        locale,
                                        context,
                                    },
                                );
                                let title =
                                    locale.get(crate::i18n::Key::EditPlaylistTitle).to_string();
                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(context, body),
                                    overlay::modal_footer(
                                        context,
                                        vec![
                                            cancel_btn(locale, context),
                                            save_btn(locale, context),
                                        ],
                                    ),
                                )
                            }
                            ModalKind::DeleteConfirm {
                                playlist_id: _,
                                playlist_name,
                            } => {
                                let title = locale
                                    .get(crate::i18n::Key::DeletePlaylistTitle)
                                    .to_string();
                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(
                                        context,
                                        text(format!(
                                            "确定要删除歌单「{}」吗？此操作无法撤销。",
                                            playlist_name
                                        ))
                                        .size(
                                            context
                                                .tokens
                                                .text(crate::ui::responsive::TextRole::BodyLarge),
                                        )
                                        .into(),
                                    ),
                                    overlay::modal_footer(
                                        context,
                                        vec![
                                            cancel_btn(locale, context),
                                            delete_btn(locale, context),
                                        ],
                                    ),
                                )
                            }
                            ModalKind::DownloadConfirm {
                                playlist_id: _,
                                playlist_name,
                                song_count,
                            } => {
                                let title = locale
                                    .get(crate::i18n::Key::DownloadPlaylistTitle)
                                    .to_string();
                                let template =
                                    locale.get(crate::i18n::Key::DownloadPlaylistConfirm);
                                let body = template
                                    .replace("{name}", playlist_name)
                                    .replace("{count}", &song_count.to_string());
                                let confirm_label = locale
                                    .get(crate::i18n::Key::DownloadPlaylistConfirmBtn)
                                    .to_string();
                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(
                                        context,
                                        text(body)
                                            .size(
                                                context.tokens.text(
                                                    crate::ui::responsive::TextRole::BodyLarge,
                                                ),
                                            )
                                            .into(),
                                    ),
                                    overlay::modal_footer(
                                        context,
                                        vec![
                                            cancel_btn(locale, context),
                                            accent_btn(
                                                confirm_label,
                                                Message::ConfirmDownloadPlaylist,
                                                context,
                                            ),
                                        ],
                                    ),
                                )
                            }
                            ModalKind::ExitConfirm { remember_choice } => {
                                let body = components::exit_dialog::view_body(
                                    *remember_choice,
                                    locale,
                                    context,
                                );
                                let title =
                                    locale.get(crate::i18n::Key::ExitDialogTitle).to_string();
                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(context, body),
                                    overlay::modal_footer(
                                        context,
                                        vec![
                                            cancel_btn(locale, context),
                                            exit_btn(locale, context),
                                            minimize_btn(locale, context),
                                        ],
                                    ),
                                )
                            }
                            ModalKind::PlaylistPicker {
                                song_id,
                                ncm_playlists,
                            } => {
                                let song_id = *song_id;
                                let title = locale
                                    .get(crate::i18n::Key::PlaylistPickerTitle)
                                    .to_string();

                                let list: Element<'_, Message> = match ncm_playlists {
                                    // NCM: show online playlists with music icon
                                    Some(pls) if !pls.is_empty() => {
                                        let mut col = column![].spacing(context.tokens.space(4.0));
                                        for pl in pls {
                                            let pl_name = pl.name.clone();
                                            let pl_id = pl.id;
                                            let cover_el = cover_image::custom(
                                                self.ui
                                                    .image_state
                                                    .get(ImageKind::PlaylistCover, pl_id),
                                                ImageKind::PlaylistCover,
                                                context.tokens.size(40.0),
                                                context
                                                    .tokens
                                                    .cover_radius(CoverRadiusRole::Thumbnail),
                                                context.tokens,
                                            );
                                            col = col.push(
                                                button(
                                                    row![
                                                        cover_el,
                                                        Space::new().width(context.tokens.space(12.0)),
                                                        text(pl_name).size(context.tokens.text(crate::ui::responsive::TextRole::Body)),
                                                        Space::new().width(Fill),
                                                    ]
                                                    .align_y(Alignment::Center),
                                                )
                                                .padding([
                                                    context.tokens.space(8.0),
                                                    context.tokens.space(14.0),
                                                ])
                                                .width(Fill)
                                                .style(move |theme, status| {
                                                    let bg =
                                                        matches!(status, button::Status::Hovered)
                                                            .then_some(iced::Background::Color(
                                                                crate::ui::theme::hover_bg(theme),
                                                            ));
                                                    button::Style {
                                                        background: bg,
                                                        text_color: crate::ui::theme::text_primary(
                                                            theme,
                                                        ),
                                                        border: iced::Border {
                                                            radius: context
                                                                .tokens
                                                                .radius(crate::ui::responsive::RadiusRole::Medium)
                                                                .into(),
                                                            ..Default::default()
                                                        },
                                                        ..Default::default()
                                                    }
                                                })
                                                .on_press(Message::AddToNcmPlaylist(
                                                    song_id.unsigned_abs(),
                                                    pl_id,
                                                )),
                                            );
                                        }
                                        widgets::smooth_scroll(
                                            scrollable(container(col).width(Fill))
                                                .height(Length::Fixed(context.tokens.size(300.0)))
                                                .direction(widgets::vertical_scrollbar(
                                                    context.tokens,
                                                ))
                                                .id(iced::widget::Id::new(
                                                    "playlist_picker_scroll",
                                                )),
                                            SmoothScrollTarget::Native("playlist_picker_scroll"),
                                            context.tokens,
                                            Message::SmoothScroll,
                                        )
                                        .into()
                                    }
                                    _ => {
                                        let playlists = self.library.playlists.clone();
                                        if playlists.is_empty() {
                                            text("No playlists available")
                                                .size(context.tokens.text(TextRole::Body))
                                                .style(|theme| text::Style {
                                                    color: Some(crate::ui::theme::text_muted(
                                                        theme,
                                                    )),
                                                })
                                                .into()
                                        } else {
                                            let mut col =
                                                column![].spacing(context.tokens.space(4.0));
                                            for pl in &playlists {
                                                let pl_id = pl.id;
                                                let pl_name = pl.name.clone();
                                                let cover_handle =
                                                    u64::try_from(pl_id).ok().and_then(|id| {
                                                        self.ui
                                                            .image_state
                                                            .get(ImageKind::LocalPlaylistCover, id)
                                                    });
                                                let thumb = cover_image::custom(
                                                    cover_handle,
                                                    ImageKind::LocalPlaylistCover,
                                                    context.tokens.size(40.0),
                                                    context
                                                        .tokens
                                                        .cover_radius(CoverRadiusRole::Thumbnail),
                                                    context.tokens,
                                                );
                                                col = col.push(
                                                    button(
                                                        row![
                                                            thumb,
                                                            Space::new().width(context.tokens.space(12.0)),
                                                            text(pl_name).size(context.tokens.text(crate::ui::responsive::TextRole::Body)),
                                                            Space::new().width(Fill),
                                                        ]
                                                        .align_y(Alignment::Center),
                                                    )
                                                    .padding([
                                                        context.tokens.space(8.0),
                                                        context.tokens.space(14.0),
                                                    ])
                                                    .width(Fill)
                                                    .style(move |theme, status| {
                                                        let bg = matches!(
                                                            status,
                                                            button::Status::Hovered
                                                        )
                                                        .then_some(iced::Background::Color(
                                                            crate::ui::theme::hover_bg(theme),
                                                        ));
                                                        button::Style {
                                                            background: bg,
                                                            text_color:
                                                                crate::ui::theme::text_primary(
                                                                    theme,
                                                                ),
                                                            border: iced::Border {
                                                            radius: context
                                                                .tokens
                                                                .radius(crate::ui::responsive::RadiusRole::Medium)
                                                                .into(),
                                                                ..Default::default()
                                                            },
                                                            ..Default::default()
                                                        }
                                                    })
                                                    .on_press(Message::PlaylistPickerConfirm(
                                                        song_id, pl_id,
                                                    )),
                                                );
                                            }
                                            widgets::smooth_scroll(
                                                scrollable(container(col).width(Fill))
                                                    .height(Length::Fixed(
                                                        context.tokens.size(300.0),
                                                    ))
                                                    .direction(widgets::vertical_scrollbar(
                                                        context.tokens,
                                                    ))
                                                    .id(iced::widget::Id::new(
                                                        "playlist_picker_scroll",
                                                    )),
                                                SmoothScrollTarget::Native(
                                                    "playlist_picker_scroll",
                                                ),
                                                context.tokens,
                                                Message::SmoothScroll,
                                            )
                                            .into()
                                        }
                                    }
                                };

                                overlay::modal_section(
                                    context,
                                    title,
                                    Message::DismissTopModal,
                                    overlay::modal_body(context, list),
                                    overlay::modal_footer(
                                        context,
                                        vec![cancel_btn(locale, context)],
                                    ),
                                )
                            }
                        };
                        overlay::modal_view(context, config, content, Message::DismissTopModal)
                    }
                },
                _ => Space::new().width(0).height(0).into(),
            }
        };

        // Keep borderless-window resize handles from sitting above a blocking overlay.
        // Toasts are deliberately excluded so they remain fully pointer-transparent.
        let resize_handles: Element<'_, Message> = if self.ui.has_blocking_pointer_overlay() {
            Space::new().width(0).height(0).into()
        } else {
            components::window_resize_handles::view(context)
        };

        stack![
            main_layout,
            sidebar_drawer_overlay,
            lyrics_overlay,
            toast_overlay,
            login_popup_overlay,
            modal_slot(0, context),
            modal_slot(1, context),
            modal_slot(2, context),
            context_menu_overlay,
            resize_handles,
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }
}

// ── Modal button helpers ──

fn cancel_btn(
    locale: crate::i18n::Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(locale.get(crate::i18n::Key::Cancel).to_string())
            .size(tokens.text(TextRole::BodyLarge)),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: theme::text_secondary(theme),
        ..Default::default()
    })
    .on_press(Message::DismissTopModal);
    widgets::hover_surface(button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                Color::TRANSPARENT,
                theme::hover_bg(theme),
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                width: tokens.size(1.0),
                color: theme::divider(theme),
            },
            ..Default::default()
        })
        .into()
}

fn save_btn(locale: crate::i18n::Locale, context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(locale.get(crate::i18n::Key::Save).to_string())
            .size(tokens.text(TextRole::BodyLarge))
            .color(Color::BLACK),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: Color::BLACK,
        ..Default::default()
    })
    .on_press(Message::SavePlaylistEdits);
    widgets::hover_surface(button)
        .style(move |_theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                theme::ACCENT_PINK,
                theme::ACCENT_PINK_HOVER,
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn delete_btn(
    locale: crate::i18n::Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(locale.get(crate::i18n::Key::Delete).to_string())
            .size(tokens.text(TextRole::BodyLarge))
            .color(Color::WHITE),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: Color::WHITE,
        ..Default::default()
    })
    .on_press(Message::ConfirmDeletePlaylist);
    widgets::hover_surface(button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                theme::danger(theme),
                theme::danger_hover(theme),
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn exit_btn(locale: crate::i18n::Locale, context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(locale.get(crate::i18n::Key::ExitDialogExit).to_string())
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| text::Style {
                color: Some(theme::text_primary(theme)),
            }),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: theme::text_primary(theme),
        ..Default::default()
    })
    .on_press(Message::ConfirmExit);
    widgets::hover_surface(button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                theme::surface_container(theme),
                theme::surface_hover(theme),
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                width: tokens.size(1.0),
                color: theme::divider(theme),
            },
            ..Default::default()
        })
        .into()
}

fn minimize_btn(
    locale: crate::i18n::Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(locale.get(crate::i18n::Key::ExitDialogMinimize).to_string())
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| text::Style {
                color: Some(theme::background(theme)),
            }),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: theme::background(theme),
        ..Default::default()
    })
    .on_press(Message::MinimizeToTray);
    widgets::hover_surface(button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                theme::text_primary(theme),
                theme::play_button_hover(theme),
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn accent_btn(
    label: String,
    msg: Message,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let button = button(
        text(label)
            .size(tokens.text(TextRole::BodyLarge))
            .color(Color::WHITE),
    )
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(8.0), tokens.space(20.0)])
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: Color::WHITE,
        ..Default::default()
    })
    .on_press(msg);
    widgets::hover_surface(button)
        .style(move |_theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                theme::ACCENT_PINK,
                theme::ACCENT_PINK_HOVER,
                progress,
            ))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
