//! Shared translucent play button for cover and feature-card overlays.

use iced::widget::{button, svg};
use iced::{Color, Element};

use crate::ui::responsive::{IconRole, UiTokens};
use crate::ui::{icons, theme, widgets};

const PLAY_BUTTON_SIZE: f32 = 48.0;
const PLAY_ICON_MAX_OPACITY: f32 = 0.72;
const HOVER_SCALE: f32 = 1.06;

/// Resolve the fixed card-overlay play target.
pub fn size(tokens: UiTokens) -> f32 {
    tokens.size(PLAY_BUTTON_SIZE)
}

/// Build a fixed-hit-target play button whose reveal belongs to the parent
/// card while its hover color and visual scale animate locally.
pub fn view<'a, Message: Clone + 'a>(
    on_press: Message,
    reveal_progress: f32,
    tokens: UiTokens,
) -> Element<'a, Message> {
    let reveal = reveal_progress.clamp(0.0, 1.0);
    let button_size = size(tokens);
    let icon_size = tokens.icon(IconRole::Large);
    let play = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(icons::PLAY.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(Color::WHITE),
            })
            .opacity(PLAY_ICON_MAX_OPACITY * reveal),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(on_press);

    widgets::hover_surface(play)
        .style(move |_theme, hover_progress| {
            let background = play_background(reveal, hover_progress);
            iced::widget::container::Style {
                background: Some(iced::Background::Color(background)),
                border: iced::Border {
                    radius: (button_size / 2.0).into(),
                    ..Default::default()
                },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.30 * reveal),
                    offset: iced::Vector::new(0.0, tokens.size(4.0)),
                    blur_radius: tokens.size(8.0),
                },
                ..Default::default()
            }
        })
        .scale_on_hover(HOVER_SCALE)
        .into()
}

fn play_background(reveal_progress: f32, hover_progress: f32) -> Color {
    let reveal = reveal_progress.clamp(0.0, 1.0);
    let idle = Color::from_rgba(0.42, 0.43, 0.46, 0.66 * reveal);
    let hovered = Color::from_rgba(0.58, 0.59, 0.62, 0.78 * reveal);
    theme::lerp_color(idle, hovered, hover_progress)
}

#[cfg(test)]
mod tests {
    use super::{PLAY_ICON_MAX_OPACITY, play_background};

    #[test]
    fn shared_play_button_stays_translucent_and_animates_color() {
        let idle = play_background(1.0, 0.0);
        let midpoint = play_background(1.0, 0.5);
        let hovered = play_background(1.0, 1.0);

        assert!(idle.a > 0.0 && idle.a < 1.0);
        assert!(hovered.a > idle.a && hovered.a < 1.0);
        assert!(midpoint.r > idle.r && midpoint.r < hovered.r);
        const { assert!(PLAY_ICON_MAX_OPACITY > 0.0 && PLAY_ICON_MAX_OPACITY < 1.0) };
    }
}
