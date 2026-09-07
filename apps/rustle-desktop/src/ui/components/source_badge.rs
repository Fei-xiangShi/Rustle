//! Source badge component for displaying song origin
//!
//! Renders a small colored badge with icon and label before the artist name.

use iced::widget::{container, row, svg, text};
use iced::{Color, Element, Padding};

use crate::app::Message;
use crate::ui::responsive::{TextRole, UiTokens};
use crate::utils::Source;

/// Colors for source badges
const EMERALD_400: Color = Color::from_rgb(52.0 / 255.0, 211.0 / 255.0, 153.0 / 255.0);
const PURPLE_400: Color = Color::from_rgb(192.0 / 255.0, 132.0 / 255.0, 252.0 / 255.0);

/// Build a source badge element (icon + label)
pub fn source_badge(source: Source, tokens: UiTokens) -> Element<'static, Message> {
    let (icon_data, label, color) = match source {
        Source::Local => (crate::ui::icons::HARD_DRIVE, "本地", EMERALD_400),
        Source::Online => (crate::ui::icons::CLOUD, "在线", PURPLE_400),
    };

    let r = color.r;
    let g = color.g;
    let b = color.b;

    container(
        row![
            svg(svg::Handle::from_memory(icon_data.as_bytes()))
                .width(tokens.size(10.0))
                .height(tokens.size(10.0))
                .style(move |_theme, _status| svg::Style {
                    color: Some(Color::from_rgb(r, g, b)),
                }),
            text(label).size(tokens.text(TextRole::Micro)).color(color),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(tokens.space(4.0)),
    )
    .padding(
        Padding::new(tokens.space(1.0))
            .left(tokens.space(6.0))
            .right(tokens.space(6.0)),
    )
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(Color::from_rgba(r, g, b, 0.2))),
        border: iced::Border {
            color: Color::from_rgba(r, g, b, 0.2),
            width: tokens.size(1.0),
            radius: tokens.size(4.0).into(),
        },
        ..Default::default()
    })
    .into()
}
