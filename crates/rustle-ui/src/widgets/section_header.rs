//! Section header widget
//!
//! Displays a section title with optional "See All" link.
//! This is a reusable widget that does not depend on application-specific types.
//!
//! # Design
//!
//! Uses generic Message type to allow reuse across different contexts.

use iced::widget::{Space, row, text};
use iced::{Alignment, Element, Fill, Padding, mouse};

use crate::responsive::{RadiusRole, TextRole, UiTokens};
use crate::theme::{self, BOLD_WEIGHT};
use crate::widgets;

/// Create a section header element
///
/// # Arguments
/// * `title` - The section title text
/// * `see_all_text` - Text for the "See All" button
/// * `on_see_all` - Optional message to send when "See All" is clicked
pub fn view<'a, Message: Clone + 'a>(
    title: &'a str,
    see_all_text: &'a str,
    on_see_all: Option<Message>,
    tokens: UiTokens,
) -> Element<'a, Message> {
    let title_text = text(title)
        .size(tokens.text(TextRole::TitleLarge))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    let see_all_btn: Element<'a, Message> = if let Some(msg) = on_see_all {
        let action = iced::widget::mouse_area(
            iced::widget::container(
                row![
                    text(see_all_text).size(tokens.text(TextRole::Body)),
                    Space::new().width(tokens.space(4.0)),
                    text("›").size(tokens.text(TextRole::Subtitle)),
                ]
                .align_y(Alignment::Center),
            )
            .padding(
                Padding::new(tokens.space(6.0))
                    .left(tokens.space(10.0))
                    .right(tokens.space(10.0)),
            ),
        )
        .on_press(msg)
        .interaction(mouse::Interaction::Pointer);

        widgets::hover_surface(action)
            .style(
                move |active_theme, progress| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::hover_bg_alpha(
                        active_theme,
                        0.10 * progress,
                    ))),
                    border: iced::Border {
                        radius: tokens.radius(RadiusRole::Pill).into(),
                        ..Default::default()
                    },
                    text_color: Some(theme::lerp_color(
                        theme::text_secondary(active_theme),
                        theme::text_primary(active_theme),
                        progress,
                    )),
                    ..Default::default()
                },
            )
            .into()
    } else {
        Space::new().width(0).into()
    };

    row![title_text, Space::new().width(Fill), see_all_btn,]
        .align_y(Alignment::Center)
        .into()
}
