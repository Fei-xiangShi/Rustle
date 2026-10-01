//! Source badge component for displaying song origin
//!
//! Renders a small colored badge with icon and label before the artist name.

use iced::widget::{container, row, svg, text};
use iced::{Element, Padding};
use rustle_ui::theme::decoration::{self, Tone};

use crate::app::Message;
use crate::ui::responsive::{TextRole, UiTokens};
use crate::utils::Source;

/// Build a source badge element (icon + label)
pub fn source_badge(source: Source, tokens: UiTokens) -> Element<'static, Message> {
    let (icon_data, label, tone) = match source {
        Source::Local => (crate::ui::icons::HARD_DRIVE, "本地", Tone::Green),
        Source::Cached => (crate::ui::icons::DOWNLOAD, "缓存", Tone::Blue),
        Source::Online => (crate::ui::icons::CLOUD, "在线", Tone::Violet),
    };

    container(
        row![
            svg(svg::Handle::from_memory(icon_data.as_bytes()))
                .width(tokens.size(10.0))
                .height(tokens.size(10.0))
                .style(move |_theme, _status| svg::Style {
                    color: Some(decoration::ink(tone, _theme)),
                }),
            text(label)
                .size(tokens.text(TextRole::Micro))
                .style(move |theme| text::Style {
                    color: Some(decoration::ink(tone, theme))
                }),
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
        background: Some(iced::Background::Color(
            decoration::ink(tone, _theme).scale_alpha(0.2),
        )),
        border: iced::Border {
            color: decoration::ink(tone, _theme).scale_alpha(0.2),
            width: tokens.size(1.0),
            radius: tokens.size(4.0).into(),
        },
        ..Default::default()
    })
    .into()
}
