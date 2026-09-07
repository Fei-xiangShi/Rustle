//! Shared cover card used by user and artist detail pages.

use iced::widget::{Space, button, column, text};
use iced::{Color, Element};

use crate::app::Message;
use crate::image::ImageKind;
use crate::ui::components::cover_image;
use crate::ui::responsive::{CardMetrics, CardRole, ResponsiveContext, TextRole, UiTokens};
use crate::ui::theme;

/// Horizontal gap shared by the detail-page card grids.
// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
pub const CARD_SPACING: f32 = 20.0;

const COVER_TEXT_SPACING: f32 = 10.0;

/// Render a clickable cover card with a title and secondary label.
///
/// The caller supplies the already-resolved image handle. Image downloads and
/// cache registration remain in the app image pipeline, so both user playlists
/// and artist albums use the exact same rendering and loading path.
/// Render a detail card with dimensions derived from the shared responsive
/// token set.
pub fn view<'a>(
    name: String,
    subtitle: String,
    cover_handle: Option<&'a iced::widget::image::Handle>,
    image_kind: ImageKind,
    on_press: Message,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    view_with_metrics(
        name,
        subtitle,
        cover_handle,
        image_kind,
        on_press,
        context.tokens.card(CardRole::Detail),
        context.tokens,
    )
}

fn view_with_metrics<'a>(
    name: String,
    subtitle: String,
    cover_handle: Option<&'a iced::widget::image::Handle>,
    image_kind: ImageKind,
    on_press: Message,
    metrics: CardMetrics,
    tokens: UiTokens,
) -> Element<'a, Message> {
    let card_width = metrics.width;
    let card_radius = metrics.radius;

    let cover = cover_image::custom(cover_handle, image_kind, card_width, card_radius, tokens);

    button(
        column![
            cover,
            Space::new().height(tokens.space(COVER_TEXT_SPACING)),
            text(name)
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_primary(theme)),
                }),
            text(subtitle)
                .size(tokens.text(TextRole::Body))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_secondary(theme)),
                }),
        ]
        .width(card_width),
    )
    .padding(0)
    .style(|_theme, _status| iced::widget::button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(on_press)
    .into()
}
