//! Unified play mode button widget
//!
//! Provides a reusable play mode toggle button with tooltip.
//! Used by both the player bar and lyrics page.

use iced::widget::{button, container, svg, text, tooltip};
use iced::{Color, Element};

use crate::ui::responsive::{TextRole, UiTokens};
use crate::ui::theme;

/// Size variant for play mode button
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonSize {
    /// Small control used by the player bar.
    Small,
    /// Large control used by full-screen surfaces.
    Large,
    /// Slightly emphasized large control for compact lyrics focus mode.
    LargeEmphasized,
}

impl ButtonSize {
    fn emphasis(self) -> f32 {
        match self {
            Self::Small | Self::Large => 1.0,
            Self::LargeEmphasized => 1.08,
        }
    }

    fn icon_size(self, tokens: UiTokens) -> f32 {
        let reference = 22.0;
        tokens.size(reference * self.emphasis())
    }

    fn padding(self, tokens: UiTokens) -> f32 {
        let reference = if matches!(self, Self::Small) {
            9.0
        } else {
            10.0
        };
        tokens.space(reference * self.emphasis())
    }

    fn radius(self, tokens: UiTokens) -> f32 {
        (self.icon_size(tokens) + self.padding(tokens) * 2.0) / 2.0
    }
}

/// Build the play mode button with tooltip
pub fn view<M: Clone + 'static>(
    play_mode_icon: &'static str,
    play_mode_tooltip: &'static str,
    size: ButtonSize,
    tokens: UiTokens,
    on_press: M,
) -> Element<'static, M> {
    let icon_size = size.icon_size(tokens);
    let padding = size.padding(tokens);
    let radius = size.radius(tokens);

    let button = button(
        svg(svg::Handle::from_memory(play_mode_icon.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(move |_theme, _status| svg::Style {
                color: Some(theme::text_secondary(_theme)),
            }),
    )
    .padding(padding)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(on_press);
    let button = super::hover_surface(button).style(move |theme, progress| container::Style {
        background: Some(iced::Background::Color(theme::hover_bg_alpha(
            theme,
            0.12 * progress,
        ))),
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    });

    tooltip(
        button,
        text(play_mode_tooltip).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Top,
    )
    .gap(tokens.space(4.0))
    .padding(tokens.space(5.0))
    .style(move |theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(
            crate::ui::theme::surface_container(theme),
        )),
        border: iced::Border {
            radius: tokens.size(4.0).into(),
            color: crate::ui::theme::divider(theme),
            width: tokens.size(1.0),
        },
        ..Default::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::ButtonSize;
    use crate::ui::responsive::UiTokens;

    #[test]
    fn hover_backgrounds_are_circular_at_every_size() {
        let tokens = UiTokens::default();
        assert_eq!(ButtonSize::Small.radius(tokens), 20.0);
        assert_eq!(ButtonSize::Large.radius(tokens), 21.0);
    }
}
