//! Lyrics page - full screen lyrics display
//!
//! Layout:
//! - Left panel: Cover art, song title, artist, progress bar, playback controls
//! - Right panel: Scrollable lyrics with current line highlighted

use std::{sync::Arc, time::Duration};

use iced::widget::{
    Sensor, Space, button, column, container, mouse_area, opaque, row, scrollable, shader, svg,
    text, transition,
};
use iced::{Alignment, Animation, Color, Element, Fill, Length, Padding};

use crate::app::{ImageState, LyricsDisplayMode, Message};
use crate::application::lyrics::LyricLine;
use crate::database::DbSong;
use crate::features::PlayMode;
use crate::features::lyrics::engine::{LyricLineData, LyricsEngine};
use crate::ui::components::window_drag_region;
use crate::ui::effects::textured_background::TexturedBackgroundProgram;
use crate::ui::icons;
use crate::ui::overlay;
use crate::ui::responsive::{
    CoverRadiusRole, IconRole, LyricsPageLayout, RadiusRole, ResponsiveContext, TargetRole,
    TextRole, lyrics_media_width, lyrics_page_layout, top_bar_height,
};
use crate::ui::theme::{self, BOLD_WEIGHT};
use crate::ui::widgets::{self, ControlSize, SliderSize};

/// Build the lyrics page view
///
/// `animation_progress`: 0.0 = hidden at bottom, 1.0 = fully visible
/// `cached_engine_lines`: Pre-computed engine lines (Arc for O(1) clone, thread-safe)
/// `power_saving_mode`: When true, use simple text rendering instead of SDF engine
/// `is_liked`: Whether the current song is in user's favorites
/// `download_progress`: Download progress for streaming songs (0.0 to 1.0)
/// `is_fm_mode`: Whether in Personal FM mode
/// `is_maximized`: Whether the application window is currently maximized
pub struct LyricsPageView<'a> {
    pub song: &'a DbSong,
    pub image_state: &'a ImageState,
    pub artist_id: Option<u64>,
    pub is_playing: bool,
    pub position: f32,
    pub duration_secs: f32,
    pub cached_engine_lines: Option<&'a Arc<Vec<LyricLineData>>>,
    pub play_mode: PlayMode,
    pub animation_progress: f32,
    pub bg_colors: &'a crate::utils::DominantColors,
    pub textured_bg_shader: &'a TexturedBackgroundProgram,
    pub lyrics_engine: Option<&'a std::cell::RefCell<LyricsEngine>>,
    pub power_saving_mode: bool,
    pub is_liked: bool,
    pub download_progress: Option<f32>,
    pub is_fm_mode: bool,
    pub is_maximized: bool,
    pub display_mode: LyricsDisplayMode,
    pub context: ResponsiveContext,
}

pub fn view<'a>(view: LyricsPageView<'a>) -> Element<'a, Message> {
    let LyricsPageView {
        song,
        image_state,
        artist_id,
        is_playing,
        position,
        duration_secs,
        cached_engine_lines,
        play_mode,
        animation_progress,
        bg_colors,
        textured_bg_shader,
        lyrics_engine,
        power_saving_mode,
        is_liked,
        download_progress,
        is_fm_mode,
        is_maximized,
        display_mode,
        context,
    } = view;
    let tokens = context.tokens;
    let title_bar_height = top_bar_height(&context);

    let content: Element<'a, Message> = match lyrics_page_layout(context) {
        LyricsPageLayout::Split => {
            let artwork_panel = build_artwork_panel(ArtworkPanelView {
                song,
                image_state,
                artist_id,
                is_playing,
                position,
                duration_secs,
                play_mode,
                is_liked,
                download_progress,
                is_fm_mode,
                context,
                presentation: ArtworkPanelPresentation::Wide,
            });
            let lyrics_panel = if power_saving_mode {
                build_simple_lyrics_panel(
                    cached_engine_lines,
                    position * duration_secs * 1000.0,
                    context,
                )
            } else {
                build_right_panel_engine(
                    cached_engine_lines,
                    lyrics_engine,
                    position * duration_secs * 1000.0,
                    context,
                )
            };

            row![
                container(artwork_panel)
                    .width(Length::FillPortion(4))
                    .height(Fill)
                    .padding(tokens.space(40.0)),
                container(lyrics_panel)
                    .width(Length::FillPortion(6))
                    .height(Fill)
                    .padding(
                        Padding::new(0.0)
                            .left(tokens.space(20.0))
                            .right(tokens.space(40.0)),
                    ),
            ]
            .width(Fill)
            .height(Fill)
            .into()
        }
        LyricsPageLayout::Focus => {
            let target = match display_mode {
                LyricsDisplayMode::Artwork => 0.0,
                LyricsDisplayMode::Lyrics => 1.0,
            };

            transition(
                target,
                || {
                    Animation::new(0.0)
                        .duration(Duration::from_millis(220))
                        .easing(iced::animation::Easing::EaseInOutCubic)
                },
                move |animation, now| {
                    let progress = animation.interpolate(now).clamp(0.0, 1.0);
                    let visible_mode = lyrics_mode_at_progress(progress);
                    let body: Element<'a, Message> = match visible_mode {
                        LyricsDisplayMode::Artwork => {
                            let artwork_panel = build_artwork_panel(ArtworkPanelView {
                                song,
                                image_state,
                                artist_id,
                                is_playing,
                                position,
                                duration_secs,
                                play_mode,
                                is_liked,
                                download_progress,
                                is_fm_mode,
                                context,
                                presentation: ArtworkPanelPresentation::Focus,
                            });
                            let horizontal_padding = tokens.space(24.0);
                            let minimum_vertical_padding = tokens.space(16.0);

                            if artwork_requires_vertical_scroll(context, title_bar_height) {
                                crate::ui::widgets::scaled_scroll(
                                    scrollable(
                                        container(artwork_panel)
                                            .width(Fill)
                                            .center_x(Fill)
                                            .padding(
                                                Padding::new(horizontal_padding)
                                                    .top(minimum_vertical_padding)
                                                    .bottom(minimum_vertical_padding),
                                            ),
                                    )
                                    .direction(crate::ui::widgets::vertical_scrollbar(tokens))
                                    .id(iced::widget::Id::new("lyrics_artwork_scroll"))
                                    .width(Fill)
                                    .height(Fill),
                                    tokens,
                                )
                                .into()
                            } else {
                                column![
                                    Space::new().height(Fill),
                                    container(artwork_panel).width(Fill).center_x(Fill).padding(
                                        Padding::new(horizontal_padding).top(0.0).bottom(0.0),
                                    ),
                                    Space::new().height(Fill),
                                ]
                                .width(Fill)
                                .height(Fill)
                                .into()
                            }
                        }
                        LyricsDisplayMode::Lyrics => {
                            let lyrics_panel = if power_saving_mode {
                                build_simple_lyrics_panel(
                                    cached_engine_lines,
                                    position * duration_secs * 1000.0,
                                    context,
                                )
                            } else {
                                build_right_panel_engine(
                                    cached_engine_lines,
                                    lyrics_engine,
                                    position * duration_secs * 1000.0,
                                    context,
                                )
                            };

                            container(lyrics_panel)
                                .width(Fill)
                                .height(Fill)
                                .padding(
                                    Padding::new(tokens.space(20.0))
                                        .top(tokens.space(12.0))
                                        .bottom(tokens.space(24.0)),
                                )
                                .into()
                        }
                    };
                    let toggle_message = match visible_mode {
                        LyricsDisplayMode::Artwork => Message::ShowLyricsContent,
                        LyricsDisplayMode::Lyrics => Message::ShowLyricsArtwork,
                    };
                    let interactive_body = mouse_area(body).on_press(toggle_message);
                    widgets::foreground_reveal(
                        interactive_body,
                        foreground_reveal_at_progress(progress),
                    )
                },
            )
            .key("lyrics-focus-transition")
            .into()
        }
    };
    let window_button_size = tokens.target(TargetRole::WindowControl);
    let control_radius = window_button_size / 2.0;
    let window_icon_size = tokens.icon(IconRole::WindowControl);
    let back_icon_size = tokens.icon(IconRole::Large);

    // Back button overlay in top-left corner
    let back_button: iced::widget::Button<'_, Message> = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(icons::CHEVRON_DOWN.as_bytes()))
            .width(back_icon_size)
            .height(back_icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(theme::text_primary(_theme)),
            }),
        window_button_size,
    ))
    .width(window_button_size)
    .height(window_button_size)
    .padding(0)
    .style(move |_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        border: iced::Border {
            radius: control_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .on_press(Message::CloseLyricsPage);
    let back_btn: Element<'a, Message> = widgets::hover_surface(back_button)
        .style(move |theme, progress| container::Style {
            background: (progress > 0.0)
                .then(|| iced::Background::Color(theme::hover_bg_alpha(theme, 0.14 * progress))),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    // Top-right buttons - using unified window control styles
    let icon_btn_style = move |_theme: &iced::Theme, status: button::Status| {
        let base = button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            text_color: theme::text_primary(_theme),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            shadow: iced::Shadow::default(),
            snap: true,
        };

        match status {
            button::Status::Hovered => button::Style {
                text_color: theme::text_primary(_theme),
                ..base
            },
            _ => base,
        }
    };

    let close_btn_style = move |_theme: &iced::Theme, status: button::Status| {
        let base = button::Style {
            background: None,
            text_color: theme::text_primary(_theme),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            shadow: iced::Shadow::default(),
            snap: true,
        };

        match status {
            button::Status::Hovered => button::Style {
                text_color: theme::text_primary(_theme),
                ..base
            },
            _ => base,
        }
    };

    let settings_button: iced::widget::Button<'_, Message> =
        button(widgets::centered_button_content(
            svg(svg::Handle::from_memory(icons::SETTINGS.as_bytes()))
                .width(window_icon_size)
                .height(window_icon_size)
                .style(|_theme, _status| svg::Style {
                    color: Some(theme::text_primary(_theme)),
                }),
            window_button_size,
        ))
        .width(window_button_size)
        .height(window_button_size)
        .padding(0)
        .style(icon_btn_style)
        .on_press(Message::OpenSettingsWithCloseLyrics);
    let settings_btn: Element<'a, Message> = widgets::hover_surface(settings_button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.14 * progress,
            ))),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let minimize_button: iced::widget::Button<'_, Message> =
        button(widgets::centered_button_content(
            svg(svg::Handle::from_memory(icons::MINIMIZE.as_bytes()))
                .width(window_icon_size)
                .height(window_icon_size)
                .style(|_theme, _status| svg::Style {
                    color: Some(theme::text_primary(_theme)),
                }),
            window_button_size,
        ))
        .width(window_button_size)
        .height(window_button_size)
        .padding(0)
        .style(icon_btn_style)
        .on_press(Message::WindowMinimize);
    let minimize_btn: Element<'a, Message> = widgets::hover_surface(minimize_button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.14 * progress,
            ))),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let maximize_button: iced::widget::Button<'_, Message> =
        button(widgets::centered_button_content(
            svg(svg::Handle::from_memory(
                icons::maximize_restore(is_maximized).as_bytes(),
            ))
            .width(window_icon_size)
            .height(window_icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(theme::text_primary(_theme)),
            }),
            window_button_size,
        ))
        .width(window_button_size)
        .height(window_button_size)
        .padding(0)
        .style(icon_btn_style)
        .on_press(Message::WindowMaximize);
    let maximize_btn: Element<'a, Message> = widgets::hover_surface(maximize_button)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.14 * progress,
            ))),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let close_button: iced::widget::Button<'_, Message> = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(icons::CLOSE.as_bytes()))
            .width(window_icon_size)
            .height(window_icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(theme::text_primary(_theme)),
            }),
        window_button_size,
    ))
    .width(window_button_size)
    .height(window_button_size)
    .padding(0)
    .style(close_btn_style)
    .on_press(Message::RequestClose);
    let close_btn: Element<'a, Message> = widgets::hover_surface(close_button)
        .style(move |theme, progress| container::Style {
            background: (progress > 0.0).then(|| {
                iced::Background::Color(theme::lerp_color(
                    Color::TRANSPARENT,
                    theme::close_button_hover(theme),
                    progress,
                ))
            }),
            border: iced::Border {
                radius: control_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let top_right_buttons = row![
        settings_btn,
        Space::new().width(tokens.space(4.0)),
        minimize_btn,
        Space::new().width(tokens.space(4.0)),
        maximize_btn,
        Space::new().width(tokens.space(4.0)),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let top_bar = row![back_btn, Space::new().width(Fill), top_right_buttons,]
        .align_y(Alignment::Center)
        .height(Length::Fixed(title_bar_height))
        .padding(
            Padding::new(0.0)
                .left(tokens.space(20.0))
                .right(tokens.space(20.0)),
        );

    let top_chrome = opaque(
        iced::widget::stack![
            window_drag_region::view(context, Message::WindowDrag),
            container(top_bar)
                .width(Fill)
                .height(Length::Fixed(title_bar_height)),
        ]
        .width(Fill)
        .height(Length::Fixed(title_bar_height)),
    );

    // Keep the lyrics surface full-window so the transparent top chrome can sit
    // above lyric text and effects without removing the top-bar region from the
    // renderer's bounds. `opaque` on the chrome captures presses only; it does
    // not paint a background over the lyrics.
    let lyrics_body = container(content).width(Fill).height(Fill).clip(true);
    let foreground = iced::widget::stack![lyrics_body, top_chrome]
        .width(Fill)
        .height(Fill);

    let slide_offset = (1.0 - animation_progress) * tokens.space(30.0);

    let background_layer: Element<'a, Message> = if power_saving_mode {
        let background = bg_colors.primary;
        container(Space::new())
            .width(Fill)
            .height(Fill)
            .style(move |_theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(background)),
                ..Default::default()
            })
            .into()
    } else {
        shader(textured_bg_shader).width(Fill).height(Fill).into()
    };

    let content_with_shader = iced::widget::stack![
        background_layer,
        container(foreground)
            .width(Fill)
            .height(Fill)
            .padding(Padding::new(0.0).top(slide_offset)),
    ]
    .width(Fill)
    .height(Fill);

    // Block all pointer events from reaching the main content behind the lyrics page while
    // preserving the controls and scrollable lyrics inside this full-screen surface.
    overlay::block_mouse_events(
        container(content_with_shader)
            .width(Fill)
            .height(Fill)
            .into(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtworkPanelPresentation {
    Wide,
    Focus,
}

#[inline]
fn lyrics_mode_at_progress(progress: f32) -> LyricsDisplayMode {
    if progress.clamp(0.0, 1.0) < 0.5 {
        LyricsDisplayMode::Artwork
    } else {
        LyricsDisplayMode::Lyrics
    }
}

#[inline]
fn foreground_reveal_at_progress(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    (2.0 * progress - 1.0).abs()
}

#[inline]
fn artwork_requires_vertical_scroll(context: ResponsiveContext, title_bar_height: f32) -> bool {
    let tokens = context.tokens;
    let available_height = (context.height() - title_bar_height).max(0.0);
    let required_height =
        lyrics_media_width(context) + tokens.size(220.0) + 2.0 * tokens.space(16.0);

    required_height > available_height
}

/// Build the player panel with cover, song info, and controls.
struct ArtworkPanelView<'a> {
    song: &'a DbSong,
    image_state: &'a ImageState,
    artist_id: Option<u64>,
    is_playing: bool,
    position: f32,
    duration_secs: f32,
    play_mode: PlayMode,
    is_liked: bool,
    download_progress: Option<f32>,
    is_fm_mode: bool,
    context: ResponsiveContext,
    presentation: ArtworkPanelPresentation,
}

fn build_artwork_panel<'a>(view: ArtworkPanelView<'a>) -> Element<'a, Message> {
    let ArtworkPanelView {
        song,
        image_state,
        artist_id,
        is_playing,
        position,
        duration_secs,
        play_mode,
        is_liked,
        download_progress,
        is_fm_mode,
        context,
        presentation,
    } = view;
    let tokens = context.tokens;
    let current_time = crate::utils::format_time(position * duration_secs);
    let total_time = crate::utils::format_time(duration_secs);
    let media_width = lyrics_media_width(context);

    let (cover_kind, cover_id) = crate::image::song_cover_key_for_source(song.id, &song.file_path)
        .unwrap_or((crate::image::ImageKind::SongCover, 0));
    let cover = crate::ui::components::cover_image::custom(
        image_state.get(cover_kind, cover_id),
        cover_kind,
        media_width,
        tokens.cover_radius(CoverRadiusRole::Hero),
        tokens,
    );

    // Song title
    let title = text(&song.title)
        .size(tokens.text(TextRole::TitleLarge))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    // Artist name
    let artist_action = artist_id
        .map(Message::OpenArtist)
        .or_else(|| Some(Message::OpenArtistByName(song.artist.clone())));
    let artist: Element<'a, Message> = button(
        text(&song.artist)
            .size(tokens.text(TextRole::Subtitle))
            .style(|theme| text::Style {
                color: Some(theme::text_secondary(theme)),
            }),
    )
    .padding(0)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press_maybe(artist_action)
    .into();

    // Progress bar - using unified widget with download progress
    let progress_slider = widgets::progress_slider::view(
        position,
        download_progress,
        SliderSize::Full,
        None,
        tokens,
        Message::SeekPreview,
        Message::SeekRelease,
    );
    let progress_slider: Element<'a, Message> =
        container(progress_slider).width(media_width).into();

    let time_row = row![
        text(current_time)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            }),
        Space::new().width(Fill),
        text(total_time)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            }),
    ];
    let time_row = time_row.width(media_width);

    // Playback controls - using unified widgets
    let control_size = match presentation {
        ArtworkPanelPresentation::Wide => ControlSize::Large,
        ArtworkPanelPresentation::Focus => ControlSize::LargeEmphasized,
    };
    let playback_controls = widgets::playback_controls::view_simple(
        is_playing,
        control_size,
        tokens,
        Some(Message::PrevSong),
        Message::TogglePlayback,
        Message::NextSong,
    );

    // Play mode button - the application boundary maps domain state/actions
    // into the generic widget.
    let play_mode_btn = crate::ui::components::playback_controls::play_mode_button(
        play_mode,
        match presentation {
            ArtworkPanelPresentation::Wide => widgets::PlayModeButtonSize::Large,
            ArtworkPanelPresentation::Focus => widgets::PlayModeButtonSize::LargeEmphasized,
        },
        tokens,
        is_fm_mode,
    );

    // Like button - only for NCM songs (negative ID)
    let like_btn: Element<'a, Message> = if song.id < 0 {
        let ncm_id = (-song.id) as u64;
        let heart_icon = if is_liked {
            icons::HEART
        } else {
            icons::HEART_OUTLINE
        };
        let heart_color = if is_liked {
            theme::ACCENT_PINK
        } else {
            theme::TEXT_SECONDARY
        };
        button(
            svg(svg::Handle::from_memory(heart_icon.as_bytes()))
                .width(tokens.icon(IconRole::Large))
                .height(tokens.icon(IconRole::Large))
                .style(move |_theme, _status| svg::Style {
                    color: Some(heart_color),
                }),
        )
        .padding(tokens.space(10.0))
        .style(move |theme, status| {
            let bg = match status {
                button::Status::Hovered => theme::hover_bg(theme),
                _ => Color::TRANSPARENT,
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::Border {
                    radius: tokens.radius(RadiusRole::Pill).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(Message::ToggleFavorite(ncm_id))
        .into()
    } else {
        // Local songs - show disabled like button
        let disabled_button = button(
            svg(svg::Handle::from_memory(icons::HEART.as_bytes()))
                .width(tokens.icon(IconRole::Large))
                .height(tokens.icon(IconRole::Large))
                .style(|_theme, _status| svg::Style {
                    color: Some(theme::opaque_color(theme::icon_muted(&iced::Theme::Dark))),
                })
                .opacity(0.4_f32),
        )
        .padding(tokens.space(10.0))
        .style(move |_theme, _status| button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Pill).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        // The compact body itself is clickable. Shield this disabled target so
        // pressing it does not bubble into the component toggle surface.
        mouse_area(disabled_button).on_press(Message::Noop).into()
    };

    // The whole action lane shares the exact media width used by the cover,
    // progress rail, and time row. Keep this contract in narrow focus mode as
    // well: flexible gaps compress before any action is removed or wrapped.
    let controls: Element<'a, Message> = container(
        row![
            play_mode_btn,
            Space::new().width(Fill),
            playback_controls,
            Space::new().width(Fill),
            like_btn,
        ]
        .align_y(Alignment::Center)
        .width(Fill),
    )
    .width(Length::Fixed(media_width))
    .into();

    // Content container with max width
    let content = column![
        cover,
        Space::new().height(tokens.space(24.0)),
        title,
        Space::new().height(tokens.space(4.0)),
        artist,
        Space::new().height(tokens.space(24.0)),
        progress_slider,
        Space::new().height(tokens.space(4.0)),
        time_row,
        Space::new().height(tokens.space(20.0)),
        controls,
    ];

    match presentation {
        ArtworkPanelPresentation::Wide => column![
            Space::new().height(Fill),
            content.width(media_width),
            Space::new().height(Fill),
        ]
        .align_x(Alignment::Center)
        .width(Fill)
        .height(Fill)
        .into(),
        ArtworkPanelPresentation::Focus => {
            content.align_x(Alignment::Start).width(media_width).into()
        }
    }
}

/// Build the right panel with the engine
/// Uses pre-computed cached_engine_lines to avoid per-frame conversion
fn build_right_panel_engine<'a>(
    cached_engine_lines: Option<&Arc<Vec<LyricLineData>>>,
    lyrics_engine: Option<&'a std::cell::RefCell<LyricsEngine>>,
    current_time_ms: f32,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    // Check if we have cached engine lines
    let engine_lines = match cached_engine_lines {
        Some(arc) if !arc.is_empty() => arc,
        _ => {
            // No lyrics or empty - show placeholder
            return container(
                column![
                    svg(svg::Handle::from_memory(icons::MUSIC.as_bytes()))
                        .width(tokens.icon(IconRole::Hero))
                        .height(tokens.icon(IconRole::Hero))
                        .style(|_theme: &iced::Theme, _status| svg::Style {
                            color: Some(theme::opaque_color(
                                theme::icon_muted(&iced::Theme::Dark,)
                            )),
                        })
                        .opacity(0.4_f32),
                    Space::new().height(tokens.space(16.0)),
                    text("纯音乐，请欣赏")
                        .size(tokens.text(TextRole::Subtitle))
                        .style(|theme| text::Style {
                            color: Some(theme::text_muted(theme))
                        }),
                ]
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }
    };

    // Use provided engine - if none, show a simple fallback
    if let Some(engine_cell) = lyrics_engine {
        // Build primitive with current animation state
        // Arc clone is O(1) - no data copying needed
        let primitive =
            crate::features::lyrics::engine::pipeline::LyricsEnginePrimitive::from_engine(
                &mut engine_cell.borrow_mut(),
                engine_lines.clone(), // Arc clone is O(1)
                current_time_ms,
            );

        let content = Sensor::new(
            shader(crate::features::lyrics::engine::program::LyricsEngineProgram::new(primitive))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_show(Message::LyricsViewportResized)
        .on_resize(Message::LyricsViewportResized);
        let content = container(content)
            .id(iced::widget::Id::new("lyrics_renderer"))
            .width(Fill)
            .height(Fill);

        mouse_area(content)
            .on_scroll(move |delta| {
                let scroll_amount = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => y * tokens.size(40.0),
                    iced::mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                Message::LyricsScroll(-scroll_amount)
            })
            .into()
    } else {
        // Fallback: show simple text-based lyrics without engine
        build_simple_lyrics_panel_from_engine_lines(engine_lines, current_time_ms, context)
    }
}

/// Simple fallback lyrics panel when engine is not available
/// Uses pre-computed engine lines
fn build_simple_lyrics_panel_from_engine_lines(
    engine_lines: &[LyricLineData],
    current_time_ms: f32,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let current_time = current_time_ms as u64;

    // Find current line
    let current_idx = engine_lines
        .iter()
        .position(|line| line.start_ms <= current_time && line.end_ms > current_time);

    // Clone text to owned strings for the UI elements
    let lines_data: Vec<(bool, String)> = engine_lines
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let is_active = Some(idx) == current_idx;
            (is_active, line.text.clone())
        })
        .collect();

    let lines_column: Element<'static, Message> = column(
        lines_data
            .into_iter()
            .map(|(is_active, line_text)| {
                let opacity = if is_active { 1.0 } else { 0.5 };
                let size = if is_active {
                    tokens.text(TextRole::TitleLarge)
                } else {
                    (tokens.text(TextRole::Title) - tokens.space(2.0))
                        .max(tokens.text(TextRole::Caption))
                };

                text(line_text)
                    .size(size)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, opacity))
                    .into()
            })
            .collect::<Vec<_>>(),
    )
    .spacing(tokens.space(12.0))
    .align_x(Alignment::Start)
    .into();

    container(lines_column)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(tokens.space(20.0))
        .into()
}

/// Simple lyrics panel for power saving mode
/// Uses plain text rendering instead of SDF engine
fn build_simple_lyrics_panel(
    cached_engine_lines: Option<&Arc<Vec<LyricLineData>>>,
    current_time_ms: f32,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    // Check if we have cached engine lines
    let engine_lines = match cached_engine_lines {
        Some(arc) if !arc.is_empty() => arc,
        _ => {
            // No lyrics or empty - show placeholder
            return container(
                column![
                    svg(svg::Handle::from_memory(icons::MUSIC.as_bytes()))
                        .width(tokens.icon(IconRole::Hero))
                        .height(tokens.icon(IconRole::Hero))
                        .style(|_theme: &iced::Theme, _status| svg::Style {
                            color: Some(theme::opaque_color(
                                theme::icon_muted(&iced::Theme::Dark,)
                            )),
                        })
                        .opacity(0.4_f32),
                    Space::new().height(tokens.space(16.0)),
                    text("暂无歌词")
                        .size(tokens.text(TextRole::Subtitle))
                        .style(|theme| text::Style {
                            color: Some(theme::text_muted(theme))
                        }),
                ]
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }
    };

    let current_time = current_time_ms as u64;

    // Find current line index
    let current_idx = engine_lines
        .iter()
        .position(|line| line.start_ms <= current_time && line.end_ms > current_time);

    let (visible_start, visible_end) = if engine_lines.len() <= 9 {
        (0, engine_lines.len())
    } else if let Some(idx) = current_idx {
        let start = idx
            .saturating_sub(4)
            .min(engine_lines.len().saturating_sub(9));
        (start, (start + 9).min(engine_lines.len()))
    } else {
        (0, 9)
    };

    let lines_data: Vec<(bool, String, Option<String>)> = engine_lines[visible_start..visible_end]
        .iter()
        .enumerate()
        .map(|(offset, line)| {
            let idx = visible_start + offset;
            let is_active = Some(idx) == current_idx;
            (is_active, line.text.clone(), line.translated.clone())
        })
        .collect();

    // Power saving mode: render only the current neighborhood.
    let lines_column: Element<'static, Message> = column(
        lines_data
            .into_iter()
            .map(|(is_active, line_text, translated)| {
                let opacity = if is_active { 1.0 } else { 0.35 };
                let size = if is_active {
                    tokens.text(TextRole::Hero)
                } else {
                    tokens.text(TextRole::TitleLarge)
                };
                let weight = if is_active {
                    BOLD_WEIGHT
                } else {
                    iced::font::Weight::Normal
                };

                let main_text = text(line_text)
                    .size(size)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, opacity))
                    .font(iced::Font::DEFAULT.weight(weight));

                if let Some(trans) = translated {
                    column![
                        main_text,
                        text(trans)
                            .size(tokens.text(TextRole::Subtitle))
                            .color(Color::from_rgba(1.0, 1.0, 1.0, opacity * 0.7))
                    ]
                    .spacing(tokens.space(6.0))
                    .into()
                } else {
                    main_text.into()
                }
            })
            .collect::<Vec<_>>(),
    )
    .spacing(tokens.space(24.0))
    .padding(tokens.space(40.0))
    .align_x(Alignment::Start)
    .into();

    container(lines_column)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// Find the current lyric line index based on playback position
pub fn find_current_line(lyrics: &[LyricLine], position_ms: u64) -> Option<usize> {
    if lyrics.is_empty() {
        return None;
    }

    let mut current = None;
    for (idx, line) in lyrics.iter().enumerate() {
        if line.start_ms <= position_ms {
            current = Some(idx);
        } else {
            break;
        }
    }

    current
}

#[cfg(test)]
mod responsive_mode_transition_tests {
    use super::{
        artwork_requires_vertical_scroll, foreground_reveal_at_progress, lyrics_mode_at_progress,
    };
    use crate::app::LyricsDisplayMode;
    use crate::ui::responsive::ResponsiveContext;
    use iced::Size;

    #[test]
    fn component_swap_happens_only_at_the_fully_hidden_midpoint() {
        assert_eq!(lyrics_mode_at_progress(-1.0), LyricsDisplayMode::Artwork);
        assert_eq!(lyrics_mode_at_progress(0.0), LyricsDisplayMode::Artwork);
        assert_eq!(lyrics_mode_at_progress(0.49), LyricsDisplayMode::Artwork);
        assert_eq!(lyrics_mode_at_progress(0.5), LyricsDisplayMode::Lyrics);
        assert_eq!(lyrics_mode_at_progress(1.0), LyricsDisplayMode::Lyrics);
        assert_eq!(lyrics_mode_at_progress(2.0), LyricsDisplayMode::Lyrics);
    }

    #[test]
    fn foreground_contracts_to_the_swap_then_expands_in_both_directions() {
        assert_eq!(foreground_reveal_at_progress(-1.0), 1.0);
        assert_eq!(foreground_reveal_at_progress(0.0), 1.0);
        assert_eq!(foreground_reveal_at_progress(0.25), 0.5);
        assert_eq!(foreground_reveal_at_progress(0.5), 0.0);
        assert_eq!(foreground_reveal_at_progress(0.75), 0.5);
        assert_eq!(foreground_reveal_at_progress(1.0), 1.0);
        assert_eq!(foreground_reveal_at_progress(2.0), 1.0);
    }

    #[test]
    fn artwork_scroll_is_enabled_only_when_the_focus_content_cannot_fit() {
        for viewport in [
            Size::new(2_560.0, 1_440.0),
            Size::new(1_280.0, 1_440.0),
            Size::new(960.0, 1_080.0),
            Size::new(768.0, 1_024.0),
            Size::new(720.0, 800.0),
            Size::new(560.0, 800.0),
        ] {
            let context = ResponsiveContext::from_viewport(viewport);
            assert!(
                !artwork_requires_vertical_scroll(context, context.tokens.size(64.0)),
                "artwork should fit without a scroll surface at {viewport:?}"
            );
        }

        let compact_landscape = ResponsiveContext::from_viewport(Size::new(960.0, 540.0));
        assert!(artwork_requires_vertical_scroll(
            compact_landscape,
            compact_landscape.tokens.size(64.0)
        ));
    }
}
