//! Unified playback control widgets
//!
//! Provides reusable playback controls (prev, play/pause, next) with consistent styling.
//! Used by both the player bar and lyrics page.

use iced::widget::{Space, button, container, row, svg};
use iced::{Alignment, Color, Element, Padding};

use crate::responsive::UiTokens;
use crate::{icons, theme};

/// Size variant for playback controls
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlSize {
    /// Small controls for player chrome.
    Small,
    /// Large controls for full-screen surfaces.
    Large,
    /// Slightly emphasized large controls for the compact lyrics focus mode.
    LargeEmphasized,
}

impl ControlSize {
    fn emphasis(self) -> f32 {
        match self {
            Self::Small | Self::Large => 1.0,
            Self::LargeEmphasized => 1.08,
        }
    }

    fn play_button_size(self, tokens: UiTokens) -> f32 {
        let reference = if self.is_small() { 44.0 } else { 64.0 };
        tokens.size(reference * self.emphasis())
    }

    fn play_icon_size(self, tokens: UiTokens) -> f32 {
        let reference = if self.is_small() { 22.0 } else { 28.0 };
        tokens.size(reference * self.emphasis())
    }

    fn skip_icon_size(self, tokens: UiTokens) -> f32 {
        self.play_icon_size(tokens)
    }

    fn skip_button_padding(self, tokens: UiTokens) -> f32 {
        let reference = if self.is_small() { 9.0 } else { 12.0 };
        tokens.space(reference * self.emphasis())
    }

    fn skip_button_radius(self, tokens: UiTokens) -> f32 {
        let reference = if self.is_small() { 22.0 } else { 26.0 };
        tokens.size(reference * self.emphasis())
    }

    fn spacing(self, tokens: UiTokens) -> f32 {
        let reference = if self.is_small() { 10.0 } else { 16.0 };
        tokens.space(reference * self.emphasis())
    }

    fn is_small(self) -> bool {
        matches!(self, Self::Small)
    }
}

/// Build the play/pause button with buffering state
pub fn play_button_with_buffering<M: Clone + 'static>(
    is_playing: bool,
    is_buffering: bool,
    size: ControlSize,
    tokens: UiTokens,
    on_press: M,
) -> Element<'static, M> {
    // Only show loading icon when playing AND buffering
    let show_loading = is_playing && is_buffering;
    let play_icon = if show_loading {
        icons::LOADING
    } else if is_playing {
        icons::PAUSE
    } else {
        icons::PLAY
    };

    let btn_size = size.play_button_size(tokens);
    let icon_size = size.play_icon_size(tokens);
    let inner_padding = (btn_size - icon_size) / 2.0;
    let icon_opacity: f32 = if show_loading { 0.4 } else { 1.0 };
    // Offset to visually center the triangle (play icon is not symmetric)
    let offset = if is_playing || show_loading {
        0.0
    } else if size.is_small() {
        tokens.size(2.0)
    } else {
        tokens.size(3.0)
    };

    let btn = button(
        container(
            svg(svg::Handle::from_memory(play_icon.as_bytes()))
                .width(icon_size)
                .height(icon_size)
                .style(move |theme, _status| svg::Style {
                    // Icon color should contrast with button background
                    color: Some(if show_loading {
                        theme::opaque_color(theme::icon_muted(theme))
                    } else {
                        theme::background(theme)
                    }),
                })
                .opacity(icon_opacity),
        )
        .padding(Padding {
            top: inner_padding,
            bottom: inner_padding,
            left: inner_padding + offset,
            right: inner_padding - offset,
        }),
    )
    .padding(0)
    .width(btn_size)
    .height(btn_size)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(on_press);

    // Always enable button - user can pause even during buffering
    super::hover_surface(btn)
        .enabled(!show_loading)
        .style(move |theme, progress| {
            let background = if show_loading {
                theme::surface_container(theme)
            } else {
                theme::lerp_color(
                    theme::text_primary(theme),
                    theme::play_button_hover(theme),
                    progress,
                )
            };
            container::Style {
                background: Some(iced::Background::Color(background)),
                border: iced::Border {
                    radius: (btn_size / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

/// Build the previous song button
pub fn prev_button<M: Clone + 'static>(
    size: ControlSize,
    tokens: UiTokens,
    disabled: bool,
    on_press: Option<M>,
) -> Element<'static, M> {
    let icon_size = size.skip_icon_size(tokens);
    let padding = size.skip_button_padding(tokens);
    let radius = size.skip_button_radius(tokens);
    let icon_opacity: f32 = if disabled { 0.5 } else { 1.0 };

    let btn = button(
        svg(svg::Handle::from_memory(icons::SKIP_PREV.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(move |_theme, _status| svg::Style {
                color: Some(if disabled {
                    theme::opaque_color(theme::disabled(_theme))
                } else {
                    theme::text_secondary(_theme)
                }),
            })
            .opacity(icon_opacity),
    )
    .padding(padding)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    });

    let btn = if disabled {
        btn
    } else {
        btn.on_press_maybe(on_press)
    };

    super::hover_surface(btn)
        .enabled(!disabled)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.12 * progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Build the next song button
pub fn next_button<M: Clone + 'static>(
    size: ControlSize,
    tokens: UiTokens,
    on_press: M,
) -> Element<'static, M> {
    let icon_size = size.skip_icon_size(tokens);
    let padding = size.skip_button_padding(tokens);
    let radius = size.skip_button_radius(tokens);

    let btn = button(
        svg(svg::Handle::from_memory(icons::SKIP_NEXT.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(theme::text_secondary(_theme)),
            }),
    )
    .padding(padding)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(on_press);

    super::hover_surface(btn)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.12 * progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Build the favorite button used by the player bar.
///
/// `favorite` contains the current favorite state. The caller supplies the
/// application action separately; `None` keeps the button visible but disabled.
pub fn favorite_button<M: Clone + 'static>(
    size: ControlSize,
    tokens: UiTokens,
    favorite: Option<bool>,
    on_press: Option<M>,
) -> Element<'static, M> {
    let icon_size = size.skip_icon_size(tokens);
    let padding = size.skip_button_padding(tokens);
    let radius = size.skip_button_radius(tokens);
    let is_liked = favorite.unwrap_or(false);
    let enabled = favorite.is_some();
    let icon_opacity: f32 = if enabled { 1.0 } else { 0.4 };
    let heart_icon = if is_liked {
        icons::HEART
    } else {
        icons::HEART_OUTLINE
    };

    let btn = button(
        svg(svg::Handle::from_memory(heart_icon.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(move |theme, _status| svg::Style {
                color: Some(if !enabled {
                    theme::opaque_color(theme::icon_muted(theme))
                } else if is_liked {
                    theme::accent(theme)
                } else {
                    theme::text_secondary(theme)
                }),
            })
            .opacity(icon_opacity),
    )
    .padding(padding)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    });

    let btn = btn.on_press_maybe(on_press);

    super::hover_surface(btn)
        .enabled(enabled)
        .style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.12 * progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Build the player bar controls, including play mode and favorite actions.
pub struct PlayerBarControls<M> {
    pub is_playing: bool,
    pub is_buffering: bool,
    pub size: ControlSize,
    pub tokens: UiTokens,
    pub prev_disabled: bool,
    pub play_mode_button: Element<'static, M>,
    pub prev_action: Option<M>,
    pub play_action: M,
    pub next_action: M,
    pub favorite: Option<bool>,
    pub favorite_action: Option<M>,
}

pub fn view_player_bar<M: Clone + 'static>(controls: PlayerBarControls<M>) -> Element<'static, M> {
    let PlayerBarControls {
        is_playing,
        is_buffering,
        size,
        tokens,
        prev_disabled,
        play_mode_button,
        prev_action,
        play_action,
        next_action,
        favorite,
        favorite_action,
    } = controls;
    let spacing = size.spacing(tokens);

    row![
        play_mode_button,
        Space::new().width(spacing),
        prev_button(size, tokens, prev_disabled, prev_action),
        Space::new().width(spacing),
        play_button_with_buffering(is_playing, is_buffering, size, tokens, play_action),
        Space::new().width(spacing),
        next_button(size, tokens, next_action),
        Space::new().width(spacing),
        favorite_button(size, tokens, favorite, favorite_action),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// Build the complete playback controls row with buffering state
pub struct PlaybackControls<M> {
    pub is_playing: bool,
    pub is_buffering: bool,
    pub size: ControlSize,
    pub tokens: UiTokens,
    pub prev_disabled: bool,
    pub prev_action: Option<M>,
    pub play_action: M,
    pub next_action: M,
}

pub fn view<M: Clone + 'static>(controls: PlaybackControls<M>) -> Element<'static, M> {
    let PlaybackControls {
        is_playing,
        is_buffering,
        size,
        tokens,
        prev_disabled,
        prev_action,
        play_action,
        next_action,
    } = controls;
    let spacing = size.spacing(tokens);

    row![
        prev_button(size, tokens, prev_disabled, prev_action),
        Space::new().width(spacing),
        play_button_with_buffering(is_playing, is_buffering, size, tokens, play_action),
        Space::new().width(spacing),
        next_button(size, tokens, next_action),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// Build the complete playback controls row (prev, play, next)
pub fn view_simple<M: Clone + 'static>(
    is_playing: bool,
    size: ControlSize,
    tokens: UiTokens,
    prev_action: Option<M>,
    play_action: M,
    next_action: M,
) -> Element<'static, M> {
    view(PlaybackControls {
        is_playing,
        is_buffering: false,
        size,
        tokens,
        prev_disabled: prev_action.is_none(),
        prev_action,
        play_action,
        next_action,
    })
}
