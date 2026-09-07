//! Shared shaped image entry-point for covers, avatars, banners, and thumbnails.
//!
//! Every cover, avatar, banner, and thumbnail flows through one of the two
//! functions below.  There is no other image widget in the codebase.
//!
//! # Quick reference
//!
//! | Situation | Use |
//! |---|---|
//! | Custom pixel size / radius | `custom(handle, kind, px, radius)` |
//! | Circular avatar | `circle(handle, kind, px)` |
//!
//! All functions accept `Option<&image::Handle>` from `ImageState`; resolving,
//! downloading, and local-path registration happen in the update layer.

use iced::Element;
use iced::widget::{container, image, svg};

use crate::image::ImageKind;
use crate::ui::responsive::UiTokens;
use crate::ui::widgets;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render with a custom pixel size and border radius.
pub fn custom(
    handle: Option<&image::Handle>,
    kind: ImageKind,
    px: f32,
    radius: f32,
    tokens: UiTokens,
) -> Element<'static, crate::app::Message> {
    shaped(handle, kind, px, radius, tokens)
}

/// Render a circular image (avatar, artist portrait).
pub fn circle(
    handle: Option<&image::Handle>,
    kind: ImageKind,
    px: f32,
    tokens: UiTokens,
) -> Element<'static, crate::app::Message> {
    shaped(handle, kind, px, px / 2.0, tokens)
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn shaped(
    handle: Option<&image::Handle>,
    kind: ImageKind,
    px: f32,
    radius: f32,
    tokens: UiTokens,
) -> Element<'static, crate::app::Message> {
    let svg_data = placeholder_svg(kind);
    let icon = (px * 0.4).min(tokens.size(64.0));
    let placeholder = container(
        svg(svg::Handle::from_memory(svg_data.as_bytes()))
            .width(icon)
            .height(icon),
    )
    .width(px)
    .height(px)
    .center_x(px)
    .center_y(px)
    .style(move |t| iced::widget::container::Style {
        background: Some(iced::Background::Color(
            crate::ui::theme::surface_container(t),
        )),
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let image: Element<'static, crate::app::Message> = widgets::crossfade_image(handle.cloned())
        .width(px)
        .height(px)
        .content_fit(iced::ContentFit::Cover)
        .border_radius(radius)
        .into();

    iced::widget::stack![placeholder, image].into()
}

fn placeholder_svg(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::SongCover
        | ImageKind::LocalSongCover
        | ImageKind::PlaylistCover
        | ImageKind::LocalPlaylistCover
        | ImageKind::AlbumCover
        | ImageKind::VideoCover
        | ImageKind::RadioCover => crate::ui::icons::MUSIC,
        ImageKind::ArtistCover | ImageKind::UserAvatar | ImageKind::VipBadge => {
            crate::ui::icons::USER
        }
    }
}
