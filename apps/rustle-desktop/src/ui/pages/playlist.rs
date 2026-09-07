//! Playlist detail page
//!
//! Shows playlist info with gradient background extracted from cover,
//! and song list with hover effects.
//!
//! Uses virtual list for efficient rendering of large playlists.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, svg, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message};
use crate::i18n::{Key, Locale};
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::components::detail_description;
use crate::ui::components::playlist_view::{self, PlaylistColumns, SongItem};
use crate::ui::responsive::{
    CoverRadiusRole, IconRole, RadiusRole, ResponsiveContext, TargetRole, TextRole, UiTokens,
    detail_header_metrics,
};
use crate::ui::theme::BOLD_WEIGHT;
use crate::ui::widgets::{VirtualListState, detail_header};
use crate::ui::{icons, theme};
use crate::utils::ColorPalette;

/// Playlist data for display
#[derive(Debug, Clone)]
pub struct PlaylistView {
    pub kind: DetailPageKind,
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub profile_stats: Option<String>,
    pub artist_tab: ArtistPageTab,
    pub artist_albums: Vec<crate::api::AlbumSummary>,
    pub user_playlists: Vec<crate::api::PlaylistSummary>,
    pub cover_path: Option<String>,
    pub owner: String,
    pub owner_artist_id: Option<u64>,
    pub owner_avatar_path: Option<String>,
    /// Creator user ID (for NCM playlists, 0 for local)
    pub creator_id: u64,
    pub song_count: u32,
    pub total_duration: String,
    pub like_count: String,
    pub songs: Vec<PlaylistSongView>,
    /// Extracted color palette from cover
    pub palette: Option<ColorPalette>,
    /// Whether this is a local playlist (no like count, no download)
    pub is_local: bool,
    /// Whether the current user has subscribed to this playlist
    pub is_subscribed: bool,
    /// Root folder backing this local library playlist, if any.
    pub watched_folder_path: Option<String>,
    /// Whether the local library root is actively monitored.
    pub watch_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailPageKind {
    Playlist,
    Album,
    User,
    Artist,
}

/// A cover-derived gradient that can survive detail-page navigation.
///
/// The page kind is retained because playlist/album headers and user/artist
/// headers intentionally use slightly different color treatments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailGradientSnapshot {
    pub kind: DetailPageKind,
    pub primary: Color,
}

impl PlaylistView {
    pub fn gradient_snapshot(&self) -> Option<DetailGradientSnapshot> {
        self.palette.as_ref().map(|palette| DetailGradientSnapshot {
            kind: self.kind,
            primary: palette.primary,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtistPageTab {
    #[default]
    TopSongs,
    Albums,
}

/// Song item in playlist (alias for SongItem)
pub type PlaylistSongView = SongItem;

pub struct DetailPageView<'a> {
    pub detail: &'a PlaylistView,
    pub image_state: &'a ImageState,
    pub song_animations: &'a crate::ui::animation::HoverAnimations<i64>,
    pub icon_animations: &'a crate::ui::animation::HoverAnimations<crate::app::IconId>,
    pub search_animation: &'a crate::ui::animation::SingleHoverAnimation,
    pub search_expanded: bool,
    pub search_query: &'a str,
    pub liked_songs: Option<&'a HashSet<u64>>,
    pub locale: Locale,
    pub scroll_state: Rc<RefCell<VirtualListState>>,
    pub current_user_id: Option<u64>,
    pub current_playing_id: Option<i64>,
    pub description_expanded: bool,
    pub gradient_source: Option<DetailGradientSnapshot>,
    pub gradient_progress: f32,
    pub context: ResponsiveContext,
}

/// Build the playlist detail page
pub fn view<'a>(view: DetailPageView<'a>) -> Element<'a, Message> {
    view_for_context(view)
}

fn view_for_context<'a>(view: DetailPageView<'a>) -> Element<'a, Message> {
    let DetailPageView {
        detail: playlist,
        image_state,
        song_animations,
        icon_animations,
        search_animation,
        search_expanded,
        search_query,
        liked_songs,
        locale,
        scroll_state,
        current_user_id,
        current_playing_id,
        description_expanded,
        gradient_source,
        gradient_progress,
        context,
    } = view;
    let controls = build_controls(DetailControls {
        playlist,
        icon_animations,
        search_animation,
        search_expanded,
        search_query,
        locale,
        current_user_id,
        context,
    });
    let header = build_header(
        playlist,
        image_state,
        locale,
        description_expanded,
        controls,
        context,
    );

    // Filter songs by index so the full playlist is not cloned on every view rebuild.
    let filtered_indices = playlist_view::filter_song_indices(&playlist.songs, search_query);

    let gradient_target = playlist.gradient_snapshot();

    let gradient_section = container(header).width(Fill).style(move |theme| {
        detail_gradient_style(theme, gradient_source, gradient_target, gradient_progress)
    });

    // Build song list header using the reusable component
    let columns = if playlist.is_local {
        PlaylistColumns::local()
    } else {
        PlaylistColumns::online()
    }
    .for_context(context);
    let song_list_header = playlist_view::build_header(locale, columns, context);

    // Use virtual list for song rows
    let song_list = playlist_view::build_list(playlist_view::SongListView {
        songs: &playlist.songs,
        filtered_indices,
        image_state,
        song_animations,
        liked_songs,
        columns,
        scroll_state,
        current_playing_id,
        context,
    });

    let content = column![gradient_section, song_list_header, song_list,]
        .spacing(0)
        .width(Fill);

    content.into()
}

/// Show the retained gradient while a detail route is waiting for its page
/// model. Most routes publish a skeleton immediately, but recently played is
/// populated asynchronously and can otherwise flash the themed background.
pub fn gradient_placeholder(
    gradient_source: Option<DetailGradientSnapshot>,
) -> Element<'static, Message> {
    container(Space::new().width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(move |theme| detail_gradient_style(theme, gradient_source, None, 0.0))
        .into()
}

#[derive(Debug, Clone, Copy)]
struct DetailGradientColors {
    top: Color,
    middle: Color,
    middle_stop: f32,
}

pub(crate) fn detail_gradient_style(
    iced_theme: &iced::Theme,
    source: Option<DetailGradientSnapshot>,
    target: Option<DetailGradientSnapshot>,
    progress: f32,
) -> iced::widget::container::Style {
    let bottom = theme::background(iced_theme);

    let Some(target) = target else {
        return match source {
            Some(source) => {
                gradient_container_style(detail_gradient_colors(iced_theme, source, bottom), bottom)
            }
            None => iced::widget::container::Style {
                background: Some(iced::Background::Color(bottom)),
                ..Default::default()
            },
        };
    };

    let target = detail_gradient_colors(iced_theme, target, bottom);
    let source = source
        .map(|source| detail_gradient_colors(iced_theme, source, bottom))
        .unwrap_or(DetailGradientColors {
            top: bottom,
            middle: bottom,
            middle_stop: target.middle_stop,
        });
    let progress = progress.clamp(0.0, 1.0);
    let colors = DetailGradientColors {
        top: fade_gradient_color(target.top, source.top, progress),
        middle: fade_gradient_color(target.middle, source.middle, progress),
        middle_stop: source.middle_stop + (target.middle_stop - source.middle_stop) * progress,
    };

    gradient_container_style(colors, bottom)
}

fn detail_gradient_colors(
    iced_theme: &iced::Theme,
    snapshot: DetailGradientSnapshot,
    bottom: Color,
) -> DetailGradientColors {
    match snapshot.kind {
        DetailPageKind::Playlist | DetailPageKind::Album => {
            let primary = snapshot.primary;
            let top = Color::from_rgb(
                (primary.r * 1.1 + 0.05).min(1.0),
                (primary.g * 1.05 + 0.03).min(1.0),
                (primary.b * 1.08 + 0.04).min(1.0),
            );
            let top = if theme::is_dark_theme(iced_theme) {
                top
            } else {
                let average = (top.r + top.g + top.b) / 3.0;
                let desaturation = 0.4;
                let lighten = 0.3;
                Color::from_rgb(
                    ((top.r * (1.0 - desaturation) + average * desaturation) + lighten).min(1.0),
                    ((top.g * (1.0 - desaturation) + average * desaturation) + lighten).min(1.0),
                    ((top.b * (1.0 - desaturation) + average * desaturation) + lighten).min(1.0),
                )
            };

            DetailGradientColors {
                top,
                middle: Color::from_rgb(
                    top.r * 0.6 + bottom.r * 0.4,
                    top.g * 0.55 + bottom.g * 0.4,
                    top.b * 0.58 + bottom.b * 0.4,
                ),
                middle_stop: 0.55,
            }
        }
        DetailPageKind::User | DetailPageKind::Artist => {
            let primary = snapshot.primary;
            DetailGradientColors {
                top: Color::from_rgb(
                    (primary.r * 1.08 + 0.04).min(1.0),
                    (primary.g * 1.06 + 0.03).min(1.0),
                    (primary.b * 1.08 + 0.04).min(1.0),
                ),
                middle: Color::from_rgb(
                    primary.r * 0.58 + bottom.r * 0.42,
                    primary.g * 0.58 + bottom.g * 0.42,
                    primary.b * 0.58 + bottom.b * 0.42,
                ),
                middle_stop: 0.58,
            }
        }
    }
}

fn gradient_container_style(
    colors: DetailGradientColors,
    bottom: Color,
) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(iced::Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                .add_stop(0.0, colors.top)
                .add_stop(colors.middle_stop, colors.middle)
                .add_stop(1.0, bottom),
        ))),
        ..Default::default()
    }
}

pub(crate) fn fade_gradient_color(target: Color, source: Color, progress: f32) -> Color {
    let progress = progress.clamp(0.0, 1.0);
    Color::from_rgba(
        source.r + (target.r - source.r) * progress,
        source.g + (target.g - source.g) * progress,
        source.b + (target.b - source.b) * progress,
        source.a + (target.a - source.a) * progress,
    )
}

#[cfg(test)]
mod gradient_tests {
    use super::*;

    fn assert_color_close(actual: Color, expected: Color) {
        const EPSILON: f32 = 0.000_001;
        assert!((actual.r - expected.r).abs() < EPSILON);
        assert!((actual.g - expected.g).abs() < EPSILON);
        assert!((actual.b - expected.b).abs() < EPSILON);
        assert!((actual.a - expected.a).abs() < EPSILON);
    }

    #[test]
    fn gradient_fade_starts_at_background_and_ends_at_target() {
        let background = Color::from_rgb(0.1, 0.2, 0.3);
        let target = Color::from_rgb(0.7, 0.6, 0.5);

        assert_color_close(fade_gradient_color(target, background, 0.0), background);
        assert_color_close(fade_gradient_color(target, background, 1.0), target);
        assert_color_close(
            fade_gradient_color(target, background, 0.5),
            Color::from_rgb(0.4, 0.4, 0.4),
        );
    }

    #[test]
    fn detail_gradient_transition_starts_at_retained_colors() {
        let iced_theme = iced::Theme::Dark;
        let bottom = theme::background(&iced_theme);
        let retained = DetailGradientSnapshot {
            kind: DetailPageKind::Playlist,
            primary: Color::from_rgb(0.2, 0.4, 0.7),
        };
        let target = DetailGradientSnapshot {
            kind: DetailPageKind::Artist,
            primary: Color::from_rgb(0.8, 0.25, 0.15),
        };
        let retained_colors = detail_gradient_colors(&iced_theme, retained, bottom);
        let target_colors = detail_gradient_colors(&iced_theme, target, bottom);

        let at_start = DetailGradientColors {
            top: fade_gradient_color(target_colors.top, retained_colors.top, 0.0),
            middle: fade_gradient_color(target_colors.middle, retained_colors.middle, 0.0),
            middle_stop: retained_colors.middle_stop,
        };
        let at_end = DetailGradientColors {
            top: fade_gradient_color(target_colors.top, retained_colors.top, 1.0),
            middle: fade_gradient_color(target_colors.middle, retained_colors.middle, 1.0),
            middle_stop: target_colors.middle_stop,
        };

        assert_color_close(at_start.top, retained_colors.top);
        assert_color_close(at_start.middle, retained_colors.middle);
        assert_color_close(at_end.top, target_colors.top);
        assert_color_close(at_end.middle, target_colors.middle);
        assert_eq!(at_start.middle_stop, retained_colors.middle_stop);
        assert_eq!(at_end.middle_stop, target_colors.middle_stop);
    }
}

/// Build the playlist header
fn build_header<'a>(
    playlist: &PlaylistView,
    image_state: &ImageState,
    locale: Locale,
    description_expanded: bool,
    controls: Element<'a, Message>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let header_metrics = detail_header_metrics(context);
    let cover_size = header_metrics.artwork_size;
    let cover_handle = playlist_header_cover_handle(playlist, image_state);
    let cover: Element<'static, Message> = crate::ui::components::cover_image::custom(
        cover_handle,
        crate::image::ImageKind::PlaylistCover,
        cover_size,
        tokens.cover_radius(CoverRadiusRole::Hero),
        tokens,
    );

    // Playlist type label - larger font
    let type_label_text = match playlist.kind {
        DetailPageKind::Playlist => locale.get(Key::PlaylistTypeLabel).to_string(),
        DetailPageKind::Album => locale.get(Key::AlbumTypeLabel).to_string(),
        DetailPageKind::User => locale.get(Key::UserTypeLabel).to_string(),
        DetailPageKind::Artist => locale.get(Key::ArtistTypeLabel).to_string(),
    };
    let type_label = text(type_label_text)
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    let description_text = meaningful_description(playlist.description.as_deref());
    let title_size = if description_text.is_some() {
        header_metrics.title_size
    } else {
        header_metrics.prominent_title_size
    };

    // Keep the compact title when it shares the identity lane with a real
    // description. Without a description, the title owns that full region.
    let title: Element<'static, Message> = text(playlist.name.clone())
        .size(title_size)
        .line_height(iced::widget::text::LineHeight::Relative(1.0))
        .width(Fill)
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        .into();

    // Description — clamped to 2 lines by default, expandable when long
    let identity_region: Element<'static, Message> = if let Some(desc) = description_text {
        let desc_text = desc.to_string();
        // Rough estimate: count newlines + approximate line wraps by char count
        let line_count = desc_text.lines().count()
            + desc_text
                .lines()
                .map(|l| l.chars().count().saturating_sub(1) / 55)
                .sum::<usize>();
        let is_long = line_count > 2;

        let desc_widget = text(desc_text)
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| text::Style {
                color: Some(theme::text_secondary(theme)),
            });

        let description: Element<'static, Message> = if is_long {
            if description_expanded {
                // Expanded: scrollable description with "收起" button
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
                    .id(iced::widget::Id::new("playlist_description_scroll")),
                    SmoothScrollTarget::Native("playlist_description_scroll"),
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
                // Collapsed: clamped to 2 lines with "展开" button
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
            desc_widget.into()
        };

        column![
            title,
            Space::new().height(tokens.space(6.0)),
            container(description).height(Fill).clip(true),
        ]
        .spacing(0)
        .width(Fill)
        .height(Fill)
        .into()
    } else {
        container(title)
            .width(Fill)
            .height(Fill)
            .align_y(Alignment::Center)
            .into()
    };

    // Owner avatar - use real avatar if available, otherwise show first letter
    let owner_name = playlist.owner.clone();
    let owner_avatar: Element<'static, Message> =
        if let Some(handle) = playlist_owner_avatar_handle(playlist, image_state) {
            crate::ui::components::cover_image::circle(
                Some(handle),
                crate::image::ImageKind::UserAvatar,
                tokens.size(24.0),
                tokens,
            )
        } else {
            build_owner_avatar_placeholder(&owner_name, tokens)
        };

    // Owner and stats - better spacing and brighter colors
    let song_count = playlist.song_count;
    let duration = playlist.total_duration.clone();
    let is_local = playlist.is_local;
    let like_count = playlist.like_count.clone();
    let owner_artist_id = playlist.owner_artist_id;

    // Build stats row - use proper dot separator with spacing
    let owner_label = text(owner_name.clone())
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    let owner_action =
        if playlist.kind == DetailPageKind::Playlist && !is_local && playlist.creator_id != 0 {
            Some(Message::OpenUser(playlist.creator_id))
        } else {
            owner_artist_id.map(Message::OpenArtist)
        };

    let owner_info: Element<'static, Message> = if let Some(action) = owner_action {
        container(
            button(
                row![
                    owner_avatar,
                    Space::new().width(tokens.space(8.0)),
                    owner_label
                ]
                .align_y(Alignment::Center),
            )
            .padding(
                Padding::new(tokens.space(4.0))
                    .left(0.0)
                    .right(tokens.space(8.0)),
            )
            .style(move |_theme, _status| button::Style {
                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                border: iced::Border {
                    radius: tokens.radius(RadiusRole::Pill).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .on_press(action),
        )
        .into()
    } else {
        row![
            owner_avatar,
            Space::new().width(tokens.space(8.0)),
            owner_label
        ]
        .align_y(Alignment::Center)
        .into()
    };

    let mut stats_items: Vec<Element<'static, Message>> = vec![owner_info];

    // Only show like count for non-local playlists
    if !is_local && !like_count.is_empty() {
        stats_items.push(Space::new().width(tokens.space(6.0)).into());
        stats_items.push(
            text("·")
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::header_text(theme)),
                })
                .into(),
        );
        stats_items.push(Space::new().width(tokens.space(6.0)).into());
        stats_items.push(
            text(locale.get(Key::PlaylistLikes).replace("{}", &like_count))
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::text_secondary(theme)),
                })
                .into(),
        );
    }

    // Song count and duration - brighter, with proper spacing
    stats_items.push(Space::new().width(tokens.space(6.0)).into());
    stats_items.push(
        text("·")
            .size(tokens.text(TextRole::Body))
            .style(|theme| text::Style {
                color: Some(theme::header_text(theme)),
            })
            .into(),
    );
    stats_items.push(Space::new().width(tokens.space(6.0)).into());
    stats_items.push(
        text(
            locale
                .get(Key::PlaylistSongCount)
                .replace("{}", &song_count.to_string()),
        )
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_secondary(theme)),
        })
        .into(),
    );
    stats_items.push(Space::new().width(tokens.space(6.0)).into());
    stats_items.push(
        text("·")
            .size(tokens.text(TextRole::Body))
            .style(|theme| text::Style {
                color: Some(theme::header_text(theme)),
            })
            .into(),
    );
    stats_items.push(Space::new().width(tokens.space(6.0)).into());
    stats_items.push(
        text(duration)
            .size(tokens.text(TextRole::Body))
            .style(|theme| text::Style {
                color: Some(theme::text_secondary(theme)),
            })
            .into(),
    );

    let stats = row(stats_items).align_y(Alignment::Center);

    // Match the reference hierarchy: type and title are separate identity
    // levels, while metadata plus actions/search stay pinned beside the cover.
    let info = column![
        type_label,
        Space::new().height(tokens.space(6.0)),
        identity_region,
        Space::new().height(tokens.space(6.0)),
        stats,
        Space::new().height(tokens.space(8.0)),
        controls,
    ]
    .spacing(0)
    .width(Fill)
    .height(Length::Fixed(cover_size));

    detail_header::view(cover, info, context, detail_header::VerticalAlignment::End)
}

#[inline]
fn meaningful_description(description: Option<&str>) -> Option<&str> {
    description.filter(|description| !description.trim().is_empty())
}

fn playlist_page_cover_handle<'a>(
    playlist: &PlaylistView,
    image_state: &'a ImageState,
) -> Option<&'a iced::widget::image::Handle> {
    if playlist.is_local {
        return u64::try_from(playlist.id)
            .ok()
            .and_then(|id| image_state.get(crate::image::ImageKind::LocalPlaylistCover, id));
    }

    match playlist.kind {
        DetailPageKind::Playlist => playlist
            .id
            .checked_neg()
            .and_then(|id| u64::try_from(id).ok())
            .and_then(|id| image_state.get(crate::image::ImageKind::PlaylistCover, id)),
        DetailPageKind::Album => playlist
            .id
            .checked_sub(i64::MIN / 4)
            .and_then(|id| u64::try_from(id).ok())
            .and_then(|id| image_state.get(crate::image::ImageKind::AlbumCover, id)),
        DetailPageKind::Artist => playlist
            .id
            .checked_sub(i64::MIN)
            .and_then(|id| u64::try_from(id).ok())
            .and_then(|id| image_state.get(crate::image::ImageKind::ArtistCover, id)),
        DetailPageKind::User => playlist
            .owner_artist_id
            .and_then(|id| image_state.get(crate::image::ImageKind::ArtistCover, id)),
    }
}

fn playlist_header_cover_handle<'a>(
    playlist: &PlaylistView,
    image_state: &'a ImageState,
) -> Option<&'a iced::widget::image::Handle> {
    playlist_page_cover_handle(playlist, image_state).or_else(|| {
        playlist.songs.iter().find_map(|song| {
            let (kind, id) = song.cover_key?;
            image_state.get(kind, id)
        })
    })
}

fn playlist_owner_avatar_handle<'a>(
    playlist: &PlaylistView,
    image_state: &'a ImageState,
) -> Option<&'a iced::widget::image::Handle> {
    if playlist.kind == DetailPageKind::Playlist && playlist.creator_id != 0 {
        return image_state.get(crate::image::ImageKind::UserAvatar, playlist.creator_id);
    }
    playlist
        .owner_artist_id
        .and_then(|id| image_state.get(crate::image::ImageKind::ArtistCover, id))
}

struct CapsuleAction {
    label: String,
    icon_svg: &'static str,
    icon_id: crate::app::IconId,
    on_press: Message,
    emphasized: bool,
    selected: bool,
    hover_progress: f32,
    tokens: UiTokens,
}

fn capsule_action_button(action: CapsuleAction) -> Element<'static, Message> {
    let CapsuleAction {
        label,
        icon_svg,
        icon_id,
        on_press,
        emphasized,
        selected,
        hover_progress,
        tokens,
    } = action;
    let height = tokens.target(TargetRole::Control);
    let icon_size = tokens.icon(IconRole::Medium);
    let horizontal_padding = tokens.space(14.0);
    let content_gap = tokens.space(7.0);
    let vertical_padding = ((height - icon_size) / 2.0).max(0.0);

    let icon = svg(svg::Handle::from_memory(icon_svg.as_bytes()))
        .width(icon_size)
        .height(icon_size)
        .style(move |theme, _status| svg::Style {
            color: Some(if emphasized {
                theme::BLACK
            } else if selected {
                theme::ACCENT_PINK
            } else {
                theme::lerp_color(
                    theme::text_secondary(theme),
                    theme::text_primary(theme),
                    hover_progress,
                )
            }),
        });
    let label = text(label)
        .size(tokens.text(TextRole::Body))
        .line_height(iced::widget::text::LineHeight::Relative(1.0))
        .height(icon_size)
        .align_y(iced::alignment::Vertical::Center)
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        .style(move |theme| text::Style {
            color: Some(if emphasized {
                theme::BLACK
            } else {
                theme::lerp_color(
                    theme::text_secondary(theme),
                    theme::text_primary(theme),
                    hover_progress,
                )
            }),
        });
    let content = row![icon, Space::new().width(content_gap), label]
        .height(icon_size)
        .align_y(Alignment::Center);

    let control = button(content)
        .height(Length::Fixed(height))
        .padding(
            Padding::new(vertical_padding)
                .left(horizontal_padding)
                .right(horizontal_padding),
        )
        .style(move |theme, status| {
            let background = if emphasized {
                emphasized_capsule_background(
                    hover_progress,
                    matches!(status, button::Status::Pressed),
                )
            } else {
                let mut idle = theme::text_primary(theme);
                idle.a = 0.10;
                let mut hovered = theme::text_primary(theme);
                hovered.a = if matches!(status, button::Status::Pressed) {
                    0.24
                } else {
                    0.18
                };
                theme::lerp_color(idle, hovered, hover_progress)
            };

            button::Style {
                background: Some(iced::Background::Color(background)),
                border: iced::Border {
                    radius: tokens.radius(RadiusRole::Pill).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(on_press);

    container(
        mouse_area(control)
            .on_enter(Message::HoverIcon(Some(icon_id)))
            .on_exit(Message::HoverIcon(None)),
    )
    .height(Length::Fixed(height))
    .align_y(Alignment::Center)
    .into()
}

fn emphasized_capsule_background(hover_progress: f32, pressed: bool) -> Color {
    if pressed {
        Color::from_rgb(0.86, 0.86, 0.86)
    } else {
        theme::lerp_color(
            Color::WHITE,
            Color::from_rgb(0.92, 0.92, 0.92),
            hover_progress,
        )
    }
}

/// Build the control buttons (play, like, download, etc.)
pub(crate) struct DetailControls<'a> {
    pub playlist: &'a PlaylistView,
    pub icon_animations: &'a crate::ui::animation::HoverAnimations<crate::app::IconId>,
    pub search_animation: &'a crate::ui::animation::SingleHoverAnimation,
    pub search_expanded: bool,
    pub search_query: &'a str,
    pub locale: Locale,
    pub current_user_id: Option<u64>,
    pub context: ResponsiveContext,
}

pub(crate) fn build_controls<'a>(view: DetailControls<'a>) -> Element<'a, Message> {
    let DetailControls {
        playlist,
        icon_animations,
        search_animation,
        search_expanded,
        search_query,
        locale,
        current_user_id,
        context,
    } = view;
    use crate::app::IconId;

    let tokens = context.tokens;

    let is_local = playlist.is_local;
    let is_artist = playlist.kind == DetailPageKind::Artist;
    let is_album = playlist.kind == DetailPageKind::Album;
    let is_user = playlist.kind == DetailPageKind::User;
    let playlist_id = playlist.id;
    let is_own_playlist = current_user_id == Some(playlist.creator_id);
    let is_subscribed = playlist.is_subscribed;

    // Helper to get icon color based on animation (using gray levels instead of opacity)
    let get_icon_color = |icon_id: IconId| -> Color {
        let base = 0.5_f32; // Default dimmed (gray)
        let bright = 1.0_f32; // Hover bright (white)
        let value = icon_animations.interpolate_f32(&icon_id, base, bright);
        Color::from_rgb(value, value, value)
    };

    let action_height = tokens.target(TargetRole::Control);
    let play_btn = capsule_action_button(CapsuleAction {
        label: locale.get(Key::PlaylistPlayAll).to_string(),
        icon_svg: icons::PLAY,
        icon_id: IconId::PlayButton,
        on_press: Message::PlayPlaylist(playlist_id),
        emphasized: true,
        selected: false,
        hover_progress: icon_animations.get_progress(&IconId::PlayButton),
        tokens,
    });

    // Build controls row
    let mut action_items: Vec<Element<'a, Message>> = vec![play_btn];

    if is_artist || is_user || is_album {
        // Artist page keeps only play and search controls.
    } else if is_local && playlist_id != -1 {
        // For local playlists (but not recently played), show edit button with animated color
        let edit_color = get_icon_color(IconId::Edit);
        let edit_btn = mouse_area(
            button(crate::ui::widgets::centered_button_content(
                svg(svg::Handle::from_memory(icons::EDIT.as_bytes()))
                    .width(tokens.icon(IconRole::Medium))
                    .height(tokens.icon(IconRole::Medium))
                    .style(move |_theme, _status| svg::Style {
                        color: Some(edit_color),
                    }),
                action_height,
            ))
            .style(theme::transparent_btn)
            .width(action_height)
            .height(action_height)
            .padding(0)
            .on_press(Message::EditPlaylist(playlist_id)),
        )
        .on_enter(Message::HoverIcon(Some(IconId::Edit)))
        .on_exit(Message::HoverIcon(None));

        action_items.push(edit_btn.into());

        // Delete button for local playlists
        let delete_color = get_icon_color(IconId::Delete);
        let delete_btn = mouse_area(
            button(crate::ui::widgets::centered_button_content(
                svg(svg::Handle::from_memory(icons::TRASH.as_bytes()))
                    .width(tokens.icon(IconRole::Medium))
                    .height(tokens.icon(IconRole::Medium))
                    .style(move |_theme, _status| svg::Style {
                        color: Some(delete_color),
                    }),
                action_height,
            ))
            .style(theme::transparent_btn)
            .width(action_height)
            .height(action_height)
            .padding(0)
            .on_press(Message::RequestDeletePlaylist(playlist_id)),
        )
        .on_enter(Message::HoverIcon(Some(IconId::Delete)))
        .on_exit(Message::HoverIcon(None));

        action_items.push(delete_btn.into());
    } else if !is_local {
        // For cloud playlists, show like button only if not own playlist
        if !is_own_playlist {
            let like_label = if is_subscribed {
                Key::PlaylistUnsubscribe
            } else {
                Key::PlaylistSubscribe
            };
            let heart_icon = if is_subscribed {
                icons::HEART
            } else {
                icons::HEART_OUTLINE
            };
            let like_btn = capsule_action_button(CapsuleAction {
                label: locale.get(like_label).to_string(),
                icon_svg: heart_icon,
                icon_id: IconId::Like,
                on_press: Message::TogglePlaylistSubscribe(playlist_id),
                emphasized: false,
                selected: is_subscribed,
                hover_progress: icon_animations.get_progress(&IconId::Like),
                tokens,
            });

            action_items.push(like_btn);
        }

        let download_btn = capsule_action_button(CapsuleAction {
            label: locale.get(Key::PlaylistDownload).to_string(),
            icon_svg: icons::DOWNLOAD,
            icon_id: IconId::Download,
            on_press: Message::RequestDownloadPlaylist(
                playlist_id,
                playlist.name.clone(),
                playlist.song_count,
            ),
            emphasized: false,
            selected: false,
            hover_progress: icon_animations.get_progress(&IconId::Download),
            tokens,
        });

        action_items.push(download_btn);
    }

    // Animated search component - expands from right to left
    let search_progress = search_animation.progress();
    let search_color = get_icon_color(IconId::Search);
    let search_target_size = action_height;
    let search_icon_size = tokens.icon(IconRole::Medium);

    // Animation: width goes from 36 (just icon) to 250 (full input)
    let min_width = search_target_size;
    let max_width = tokens.size(250.0_f32);
    let current_width = min_width + (max_width - min_width) * search_progress;

    // Input opacity: fade in as it expands
    let input_opacity = search_progress;

    let search_query_owned = search_query.to_string();

    let search_component: Element<'a, Message> = if search_expanded || search_progress > 0.01 {
        // Expanded or animating - show input with search icon
        let search_icon = button(crate::ui::widgets::centered_button_content(
            svg(svg::Handle::from_memory(icons::SEARCH.as_bytes()))
                .width(search_icon_size)
                .height(search_icon_size)
                .style(move |_theme, _status| svg::Style {
                    color: Some(search_color),
                }),
            search_target_size,
        ))
        .style(theme::transparent_btn)
        .width(search_target_size)
        .height(search_target_size)
        .padding(0)
        .on_press(Message::TogglePlaylistSearch);

        // Text input - only show when animation is far enough
        let input_element: Element<'a, Message> = if search_progress > 0.3 {
            text_input("", search_query_owned)
                .id(iced::widget::Id::new("playlist_search_input"))
                .on_input(Message::PlaylistSearchChanged)
                .on_submit(Message::PlaylistSearchSubmit)
                .padding(
                    Padding::new(tokens.space(8.0))
                        .left(0.0)
                        .right(tokens.space(8.0)),
                )
                .size(tokens.text(TextRole::Body))
                .width(Fill)
                .style(move |_theme, _status| text_input::Style {
                    background: iced::Background::Color(Color::TRANSPARENT),
                    border: iced::Border::default(),
                    placeholder: Color::from_rgba(1.0, 1.0, 1.0, 0.5 * input_opacity),
                    value: Color::from_rgba(1.0, 1.0, 1.0, input_opacity),
                    selection: theme::ACCENT_PINK,
                })
                .into()
        } else {
            Space::new().width(Fill).into()
        };

        let search_row = row![
            search_icon,
            Space::new().width(tokens.space(8.0)),
            input_element,
        ]
        .align_y(Alignment::Center);

        // Container with animated width and rounded background
        let bg_alpha = 0.15 * search_progress;
        let search_container = container(search_row)
            .width(current_width)
            .height(search_target_size)
            .padding(
                Padding::new(0.0)
                    .left(tokens.space(8.0))
                    .right(tokens.space(4.0)),
            )
            .center_y(search_target_size)
            .style(move |_theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    1.0, 1.0, 1.0, bg_alpha,
                ))),
                border: iced::Border {
                    radius: tokens.radius(RadiusRole::Pill).into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        // Wrap in mouse_area to detect when mouse leaves - triggers blur check
        mouse_area(search_container)
            .on_exit(Message::PlaylistSearchBlur)
            .into()
    } else {
        // Collapsed - just show search button
        mouse_area(
            button(crate::ui::widgets::centered_button_content(
                svg(svg::Handle::from_memory(icons::SEARCH.as_bytes()))
                    .width(search_icon_size)
                    .height(search_icon_size)
                    .style(move |_theme, _status| svg::Style {
                        color: Some(search_color),
                    }),
                search_target_size,
            ))
            .style(theme::transparent_btn)
            .width(search_target_size)
            .height(search_target_size)
            .padding(0)
            .on_press(Message::TogglePlaylistSearch),
        )
        .on_enter(Message::HoverIcon(Some(IconId::Search)))
        .on_exit(Message::HoverIcon(None))
        .into()
    };

    let actions = row(action_items)
        .align_y(Alignment::Center)
        .spacing(tokens.space(8.0));
    let utilities = row![search_component]
        .align_y(Alignment::Center)
        .spacing(0)
        .width(Length::Fit);
    let (action_lane_width, utility_lane_width) = control_bar_lane_widths();

    // Iced lays out main-axis Fit children before Fill children. Keeping the
    // utility lane intrinsic reserves its complete width first; the left lane
    // then receives all remaining space and naturally pushes the tools to the
    // right without a profile-specific second row.
    container(
        row![
            container(actions).align_left(action_lane_width),
            container(utilities).align_right(utility_lane_width),
        ]
        .spacing(tokens.space(8.0))
        .align_y(Alignment::Center)
        .width(Fill),
    )
    .width(Fill)
    .height(Length::Fixed(action_height))
    .align_y(Alignment::Center)
    .into()
}

#[inline]
const fn control_bar_lane_widths() -> (Length, Length) {
    (Length::Fill, Length::Fit)
}

#[cfg(test)]
mod control_bar_tests {
    use super::{control_bar_lane_widths, emphasized_capsule_background, meaningful_description};
    use iced::{Color, Length};

    #[test]
    fn utility_lane_keeps_intrinsic_width_while_actions_fill_the_remainder() {
        assert_eq!(control_bar_lane_widths(), (Length::Fill, Length::Fit));
    }

    #[test]
    fn only_non_whitespace_text_reserves_the_description_region() {
        assert_eq!(meaningful_description(None), None);
        assert_eq!(meaningful_description(Some("")), None);
        assert_eq!(meaningful_description(Some(" \n\t ")), None);
        assert_eq!(
            meaningful_description(Some("  playlist introduction  ")),
            Some("  playlist introduction  ")
        );
    }

    #[test]
    fn emphasized_capsule_starts_white_and_darkens_on_interaction() {
        assert_eq!(emphasized_capsule_background(0.0, false), Color::WHITE);
        assert_ne!(emphasized_capsule_background(1.0, false), Color::WHITE);
        assert_ne!(emphasized_capsule_background(0.0, true), Color::WHITE);
    }
}

/// Build owner avatar placeholder (first letter on pink background)
fn build_owner_avatar_placeholder(
    owner_name: &str,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    let size = tokens.size(24.0);
    let first_char = owner_name.chars().next().unwrap_or('R');
    container(
        text(first_char.to_string())
            .size(tokens.text(TextRole::Micro))
            .color(theme::BLACK)
            .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT)),
    )
    .width(size)
    .height(size)
    .center_x(size)
    .center_y(size)
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_PINK_HOVER)),
        border: iced::Border {
            radius: (size / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
