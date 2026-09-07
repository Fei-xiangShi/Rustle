//! User detail page.

use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::components::{cover_image, detail_card, detail_description};
use crate::ui::pages::playlist::{self, DetailGradientSnapshot, PlaylistView};
use crate::ui::responsive::{
    CardRole, ResponsiveContext, TextRole, UiTokens, detail_header_metrics,
};
use crate::ui::theme::BOLD_WEIGHT;
use crate::ui::widgets::detail_header;
use crate::ui::{theme, widgets};

pub fn view<'a>(
    user: &'a PlaylistView,
    image_state: &'a ImageState,
    locale: Locale,
    description_expanded: bool,
    gradient_source: Option<DetailGradientSnapshot>,
    gradient_progress: f32,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    view_for_context(
        user,
        image_state,
        locale,
        description_expanded,
        gradient_source,
        gradient_progress,
        context,
    )
}

fn view_for_context<'a>(
    user: &'a PlaylistView,
    image_state: &'a ImageState,
    locale: Locale,
    description_expanded: bool,
    gradient_source: Option<DetailGradientSnapshot>,
    gradient_progress: f32,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let header = build_header(user, image_state, description_expanded, locale, context);
    let body = build_playlist_grid(user, image_state, locale, context);

    let gradient_target = user.gradient_snapshot();
    let gradient_section = container(header).width(Fill).style(move |theme| {
        playlist::detail_gradient_style(theme, gradient_source, gradient_target, gradient_progress)
    });

    column![gradient_section, body]
        .spacing(0)
        .width(Fill)
        .into()
}

fn build_header(
    user: &PlaylistView,
    image_state: &ImageState,
    description_expanded: bool,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let header_metrics = detail_header_metrics(context);
    let avatar_size = header_metrics.artwork_size;
    let avatar = circular_avatar(
        image_state.get(ImageKind::UserAvatar, user.creator_id),
        &user.name,
        avatar_size,
        tokens,
    );

    let title = text(user.name.clone())
        .size(header_metrics.title_size)
        .style(|theme| iced::widget::text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    let stats = text(
        user.profile_stats
            .clone()
            .unwrap_or_else(|| locale.get(Key::ProfileStatsFallback).to_string()),
    )
    .size(tokens.text(TextRole::Title))
    .style(|theme| iced::widget::text::Style {
        color: Some(theme::text_secondary(theme)),
    });

    let intro: Element<'static, Message> = {
        let desc_text = user
            .description
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| locale.get(Key::ProfileNoBio).to_string());
        let has_description = user
            .description
            .as_ref()
            .is_some_and(|d| !d.trim().is_empty());

        let line_count = desc_text.lines().count()
            + desc_text
                .lines()
                .map(|l| l.chars().count().saturating_sub(1) / 55)
                .sum::<usize>();
        let is_long = line_count > 2;

        let desc_widget = text(desc_text)
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph);

        if has_description && is_long {
            if description_expanded {
                let scrollable_desc = crate::ui::widgets::smooth_scroll(
                    scrollable(
                        container(desc_widget)
                            .width(Fill)
                            .padding(Padding::new(tokens.space(4.0)).left(0.0)),
                    )
                    .direction(scrollable::Direction::Vertical(
                        iced::widget::scrollable::Scrollbar::new()
                            .width(tokens.size(4.0))
                            .scroller_width(tokens.size(4.0)),
                    ))
                    .height(tokens.size(150.0))
                    .id(iced::widget::Id::new("user_description_scroll")),
                    SmoothScrollTarget::Native("user_description_scroll"),
                    tokens,
                    Message::SmoothScroll,
                );

                let collapse_btn = detail_description::toggle_button(
                    locale.get(Key::CollapseDescription),
                    Message::ToggleDescriptionExpand,
                    tokens,
                );

                column![scrollable_desc, collapse_btn]
                    .spacing(tokens.space(2.0))
                    .width(Fill)
                    .into()
            } else {
                let clamped_desc = container(desc_widget)
                    .height(detail_description::collapsed_height(tokens))
                    .clip(true)
                    .width(Fill);

                let expand_btn = detail_description::toggle_button(
                    locale.get(Key::ExpandDescription),
                    Message::ToggleDescriptionExpand,
                    tokens,
                );

                column![clamped_desc, expand_btn]
                    .spacing(tokens.space(2.0))
                    .width(Fill)
                    .into()
            }
        } else {
            container(desc_widget)
                .width(detail_description::text_width())
                .into()
        }
    };

    let info = column![
        title,
        Space::new().height(tokens.space(10.0)),
        stats,
        Space::new().height(tokens.space(12.0)),
        intro,
    ]
    .align_x(Alignment::Start)
    .width(Fill);

    detail_header::view(
        avatar,
        info,
        context,
        detail_header::VerticalAlignment::Center,
    )
}

fn build_playlist_grid<'a>(
    user: &'a PlaylistView,
    image_state: &'a ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    if user.user_playlists.is_empty() {
        return container(
            text(locale.get(Key::ProfileNoPlaylists).to_string())
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_secondary(theme)),
                }),
        )
        .padding(tokens.space(48.0))
        .width(Fill)
        .into();
    }

    let card_spacing = context.tokens.space(detail_card::CARD_SPACING);
    let card_metrics = tokens.card(CardRole::Detail);

    let title = text(locale.get(Key::PlaylistTypeLabel).to_string())
        .size(tokens.text(TextRole::TitleLarge))
        .style(|theme| iced::widget::text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    let grid = widgets::responsive_card_columns(card_metrics, 8, move |columns_per_row| {
        let rows = user
            .user_playlists
            .chunks(columns_per_row)
            .map(|chunk| {
                let mut row_items: Vec<Element<'a, Message>> = Vec::new();
                for playlist in chunk {
                    let cover_handle = image_state.get(ImageKind::PlaylistCover, playlist.id);
                    row_items.push(detail_card::view(
                        playlist.name.clone(),
                        playlist.creator.nickname.clone(),
                        cover_handle,
                        ImageKind::PlaylistCover,
                        Message::OpenNcmPlaylist(playlist.id),
                        context,
                    ));
                    row_items.push(Space::new().width(card_spacing).into());
                }
                if !row_items.is_empty() {
                    row_items.pop();
                }
                row(row_items).align_y(Alignment::Start).into()
            })
            .collect::<Vec<Element<'a, Message>>>();

        column(rows).spacing(tokens.space(18.0)).into()
    });

    let rows = column![title, Space::new().height(tokens.space(20.0)), grid]
        .spacing(tokens.space(18.0))
        .padding(
            Padding::new(tokens.space(40.0))
                .left(tokens.space(48.0))
                .right(tokens.space(48.0)),
        );

    widgets::page_scrollable(
        column![rows, Space::new().height(tokens.space(32.0))],
        "playlist_scroll",
        tokens,
        Message::SmoothScroll,
    )
}

fn circular_avatar(
    handle: Option<&iced::widget::image::Handle>,
    fallback_name: &str,
    size: f32,
    tokens: UiTokens,
) -> Element<'static, Message> {
    if handle.is_some() {
        return cover_image::circle(handle, ImageKind::UserAvatar, size, tokens);
    }

    let initial = fallback_name.chars().next().unwrap_or('?').to_string();
    container(
        text(initial)
            .size((size * 0.22).max(tokens.size(24.0)))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_primary(theme)),
            })
            .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT)),
    )
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .center_x(Length::Fixed(size))
    .center_y(Length::Fixed(size))
    .style(move |theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(theme::surface_container(theme))),
        border: iced::Border {
            radius: (size / 2.0).into(),
            width: tokens.size(1.0),
            color: theme::border_color(theme),
        },
        ..Default::default()
    })
    .into()
}
