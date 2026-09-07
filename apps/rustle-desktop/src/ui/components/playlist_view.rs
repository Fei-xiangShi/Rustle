//! Reusable playlist view component
//!
//! A generic song list component with virtual scrolling, hover animations,
//! and customizable columns. Can be used for playlists, albums, search results, etc.
//!
//! Performance optimizations:
//! - Pre-computed display strings (no format! in render loop)
//! - Pre-loaded image handles (no disk IO in render loop)
//! - Cached SVG handles (no repeated parsing)

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;

use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{Space, button, column, container, mouse_area, row, svg, text};
use iced::{Alignment, Color, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message};
use crate::i18n::{Key, Locale};
use crate::ui::animation::{SmoothScrollEvent, SmoothScrollTarget};
use crate::ui::responsive::{
    CoverRadiusRole, LayoutProfile, ResponsiveContext, TargetRole, TextRole, UiTokens,
};
use crate::ui::theme::BOLD_WEIGHT;
use crate::ui::widgets::{VirtualList, VirtualListState};
use crate::ui::{icons, theme, widgets};
use crate::utils::Source;

/// Song row height constant for virtual list
// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
pub const SONG_ROW_HEIGHT: f32 = 62.0;
/// Favorite glyph size is intentionally smaller than its interaction target.
const FAVORITE_GLYPH_SIZE: f32 = 16.0;

/// Resolve the virtual-list row height from the current root rem.
#[inline]
fn song_row_height(tokens: UiTokens) -> f32 {
    tokens.size(SONG_ROW_HEIGHT)
}

#[inline]
fn favorite_glyph_size(tokens: UiTokens) -> f32 {
    tokens.size(FAVORITE_GLYPH_SIZE)
}

#[inline]
fn favorite_crossfade(progress: f32) -> (f32, f32) {
    let favorite_opacity = progress.clamp(0.0, 1.0);
    (1.0 - favorite_opacity, favorite_opacity)
}

/// Pre-cached SVG handles to avoid repeated parsing in render loop
static PLAY_ICON_HANDLE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(icons::PLAY.as_bytes()));
static HEART_ICON_HANDLE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(icons::HEART.as_bytes()));
static HEART_OUTLINE_ICON_HANDLE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(icons::HEART_OUTLINE.as_bytes()));
static CLOCK_ICON_HANDLE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(icons::CLOCK.as_bytes()));

/// Maximum lengths for display text truncation
const MAX_TITLE_LEN: usize = 28;
const MAX_ARTIST_LEN: usize = 25;

/// Song item data for display in the list
/// All display strings and image handles are pre-computed for performance
#[derive(Debug, Clone)]
pub struct SongItem {
    pub id: i64,
    /// Stable image-cache identity. Persisted NCM rows may have a positive DB
    /// ID, so the view must not re-derive this from `id` alone.
    pub cover_key: Option<(crate::image::ImageKind, u64)>,
    /// Remote cover URL when the row represents an online song.
    /// Local songs resolve their cover through the local image cache.
    pub cover_url: Option<String>,
    /// Pre-formatted index string to avoid format! in render loop
    pub index_str: String,
    /// Original title for search/filter
    pub title: String,
    /// Original artist for search/filter
    pub artist: String,
    /// Pre-normalized searchable text to avoid lowercasing on every view rebuild.
    search_text: String,
    /// Pre-truncated display title
    pub display_title: String,
    /// Pre-truncated display artist
    pub display_artist: String,
    /// Pre-truncated display album
    pub display_album: String,
    pub duration: String,
    pub added_date: String,
    /// Song source for display badge
    pub source: Source,
}

pub struct SongItemData {
    pub id: i64,
    pub cover_key: Option<(crate::image::ImageKind, u64)>,
    pub cover_url: Option<String>,
    pub index: usize,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: String,
    pub added_date: String,
    pub source: Source,
}

impl SongItem {
    /// Create a new SongItem with pre-computed display values.
    /// Cover resolution is deferred to `image::resolve`.
    pub fn new(data: SongItemData) -> Self {
        let SongItemData {
            id,
            cover_key,
            cover_url,
            index,
            title,
            artist,
            album,
            duration,
            added_date,
            source,
        } = data;
        let display_title = truncate_string(&title, MAX_TITLE_LEN);
        let display_artist = truncate_string(&artist, MAX_ARTIST_LEN);
        let display_album = album.clone();
        let index_str = index.to_string();
        let search_text = format!(
            "{}\n{}\n{}",
            title.to_lowercase(),
            artist.to_lowercase(),
            album.to_lowercase()
        );

        Self {
            id,
            cover_key,
            cover_url,
            index_str,
            title,
            artist,
            search_text,
            display_title,
            display_artist,
            display_album,
            duration,
            added_date,
            source,
        }
    }
}

pub struct SongListView<'a> {
    pub songs: &'a [SongItem],
    pub filtered_indices: Option<Vec<usize>>,
    pub image_state: &'a ImageState,
    pub song_animations: &'a crate::ui::animation::HoverAnimations<i64>,
    pub liked_songs: Option<&'a HashSet<u64>>,
    pub columns: PlaylistColumns,
    pub scroll_state: Rc<RefCell<VirtualListState>>,
    pub current_playing_id: Option<i64>,
    pub context: ResponsiveContext,
}

/// Truncate string with ellipsis if too long
#[inline]
fn truncate_string(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_len {
        let mut result: String = s.chars().take(max_len).collect();
        result.push('…');
        result
    } else {
        s.to_string()
    }
}

/// Configuration for playlist view columns
#[derive(Debug, Clone, Copy)]
pub struct PlaylistColumns {
    /// Show the like button column (for online playlists)
    pub show_like: bool,
    /// Show the added date column (for local playlists)
    pub show_added_date: bool,
    /// Show album column
    pub show_album: bool,
    /// Use a compact detail-row composition while retaining secondary data.
    pub compact: bool,
}

impl Default for PlaylistColumns {
    fn default() -> Self {
        Self {
            show_like: true,
            show_added_date: false,
            show_album: true,
            compact: false,
        }
    }
}

impl PlaylistColumns {
    /// Configuration for local playlists (with added date, no like button)
    pub fn local() -> Self {
        Self {
            show_like: false,
            show_added_date: true,
            show_album: true,
            compact: false,
        }
    }

    /// Configuration for online/cloud playlists (with like button, no added date)
    pub fn online() -> Self {
        Self {
            show_like: true,
            show_added_date: false,
            show_album: true,
            compact: false,
        }
    }

    /// Adapt the table columns to the available composition profile.
    pub fn for_context(self, context: ResponsiveContext) -> Self {
        let compact = matches!(
            context.profile,
            LayoutProfile::Compact | LayoutProfile::Tablet | LayoutProfile::Narrow
        );
        Self { compact, ..self }
    }
}

/// Build the song list header row
pub fn build_header(
    locale: Locale,
    columns: PlaylistColumns,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let mut header_items: Vec<Element<'static, Message>> = vec![
        container(
            text(locale.get(Key::PlaylistHeaderNumber))
                .size(tokens.text(TextRole::Label))
                .style(|theme| text::Style {
                    color: Some(theme::header_text(theme)),
                }),
        )
        .width(tokens.size(48.0))
        .center_x(tokens.size(48.0))
        .into(),
        Space::new().width(tokens.space(44.0)).into(), // Cover space
        Space::new().width(tokens.space(14.0)).into(),
        text(locale.get(Key::PlaylistHeaderTitle))
            .size(tokens.text(TextRole::Label))
            .style(|theme| text::Style {
                color: Some(theme::header_text(theme)),
            })
            .into(),
        Space::new().width(Fill).into(),
    ];

    if columns.show_album && !columns.compact {
        header_items.push(
            container(
                text(locale.get(Key::PlaylistHeaderAlbum))
                    .size(tokens.text(TextRole::Label))
                    .style(|theme| text::Style {
                        color: Some(theme::header_text(theme)),
                    }),
            )
            .width(tokens.size(200.0))
            .into(),
        );
    }

    if columns.show_added_date && !columns.compact {
        header_items.push(
            container(
                text(locale.get(Key::PlaylistHeaderAddedDate))
                    .size(tokens.text(TextRole::Label))
                    .style(|theme| text::Style {
                        color: Some(theme::header_text(theme)),
                    }),
            )
            .width(tokens.size(90.0))
            .into(),
        );
    }

    // Duration/clock icon column - use cached handle
    header_items.push(
        container(
            svg(CLOCK_ICON_HANDLE.clone())
                .width(tokens.icon(crate::ui::responsive::IconRole::Small))
                .height(tokens.icon(crate::ui::responsive::IconRole::Small))
                .style(|theme, _status| svg::Style {
                    color: Some(theme::opaque_color(theme::header_text(theme))),
                })
                .opacity(0.6_f32),
        )
        .width(tokens.size(50.0))
        .center_x(tokens.size(50.0))
        .into(),
    );

    let header = row(header_items).align_y(Alignment::Center).padding(
        Padding::new(tokens.space(14.0))
            .left(tokens.space(20.0))
            .right(tokens.space(24.0)),
    );

    let header_container = container(header).width(Fill);

    // Divider line
    let divider = container(Space::new().height(tokens.size(1.0)))
        .width(Fill)
        .padding(
            Padding::new(0.0)
                .left(tokens.space(20.0))
                .right(tokens.space(20.0)),
        )
        .style(|theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::divider(theme))),
            ..Default::default()
        });

    column![
        header_container,
        divider,
        Space::new().height(tokens.space(8.0)),
    ]
    .spacing(0)
    .width(Fill)
    .into()
}

/// Build the virtual song list
pub fn build_list<'a>(view: SongListView<'a>) -> Element<'a, Message> {
    let SongListView {
        songs,
        filtered_indices,
        image_state,
        song_animations,
        liked_songs,
        columns,
        scroll_state,
        current_playing_id,
        context,
    } = view;
    let tokens = context.tokens;
    let row_height = song_row_height(tokens);
    let filtered_indices = filtered_indices.map(Rc::new);
    let song_count = filtered_indices
        .as_ref()
        .map_or(songs.len(), |idx| idx.len());

    if song_count == 0 {
        return container(
            text("暂无歌曲")
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::dimmed_text(theme)),
                }),
        )
        .width(Fill)
        .padding(Padding::new(tokens.space(32.0)))
        .center_x(Fill)
        .into();
    }

    let indices_for_builder = filtered_indices.clone();
    let indices_for_hover = filtered_indices.clone();
    let item_builder = move |index: usize| -> Element<'a, Message> {
        let song_index = indices_for_builder
            .as_ref()
            .and_then(|indices| indices.get(index).copied())
            .unwrap_or(index);

        let Some(song) = songs.get(song_index) else {
            return Space::new().height(row_height).into();
        };

        let is_playing = current_playing_id == Some(song.id);
        let animation_progress = song_animations.get_progress(&song.id);
        container(build_song_row(
            song,
            image_state,
            is_playing,
            animation_progress,
            liked_songs,
            columns,
            tokens,
        ))
        .padding(
            Padding::new(tokens.space(1.0))
                .left(tokens.space(12.0))
                .right(tokens.space(12.0)),
        )
        .into()
    };

    let indices_for_key = filtered_indices.clone();
    let indices_for_visible = filtered_indices.clone();
    let songs_for_visible = songs;
    let image_generation = image_state.generation;

    VirtualList::new(song_count, row_height, tokens, item_builder)
        .keyed_by(move |index| {
            let song_index = indices_for_key
                .as_ref()
                .and_then(|indices| indices.get(index).copied())
                .unwrap_or(index);

            songs
                .get(song_index)
                .map(|song| (song.id, song_index))
                .unwrap_or((i64::MIN, index))
        })
        .state(scroll_state)
        .width(Length::Fill)
        .height(Length::Fill)
        .spacing(0.0)
        .on_empty_area(Message::HoverSong(None))
        .on_item_hover(move |index| {
            let song_index = indices_for_hover
                .as_ref()
                .and_then(|indices| indices.get(index).copied())
                .unwrap_or(index);
            let song_id = songs.get(song_index).map(|s| s.id);
            Message::HoverSong(song_id)
        })
        .on_smooth_scroll(|delta| {
            Message::SmoothScroll(SmoothScrollEvent::Requested {
                target: SmoothScrollTarget::PlaylistSongs,
                delta,
            })
        })
        .on_smooth_scroll_cancel(Message::SmoothScroll(SmoothScrollEvent::Cancelled {
            target: SmoothScrollTarget::PlaylistSongs,
        }))
        .on_visible_range(move |(start, end)| {
            let mut images = Vec::new();
            for index in start..end {
                let song_index = indices_for_visible
                    .as_ref()
                    .and_then(|indices| indices.get(index).copied())
                    .unwrap_or(index);
                let Some(song) = songs_for_visible.get(song_index) else {
                    continue;
                };
                let Some((kind, id)) = song.cover_key else {
                    continue;
                };
                let Some(url) = song.cover_url.as_deref() else {
                    continue;
                };
                if !url.is_empty() {
                    images.push((kind, id, url.to_string()));
                }
            }
            Message::ImageViewportChanged(image_generation, images)
        })
        .visible_range_token(image_generation)
        .into()
}

/// Build a single song row with hover effect
/// Optimized: No disk IO, no string allocations, uses pre-cached handles
fn build_song_row(
    song: &SongItem,
    image_state: &ImageState,
    is_playing: bool,
    animation_progress: f32,
    liked_songs: Option<&HashSet<u64>>,
    columns: PlaylistColumns,
    tokens: UiTokens,
) -> Element<'static, Message> {
    let song_id = song.id;

    // Clone strings for 'static lifetime (these are pre-computed, so cheap)
    let index_str = song.index_str.clone();
    let display_title = song.display_title.clone();
    let display_artist = song.display_artist.clone();
    let display_album = song.display_album.clone();
    let duration = song.duration.clone();
    let added_date = song.added_date.clone();

    // --- Index or play icon (fixed slot width; content is re-diffed by VirtualList) ---
    // Keep both states mounted and cross-fade them. SVG opacity must be applied
    // through the widget's opacity field; alpha in `svg::Style::color` is only
    // a tint in the renderer and is not a reliable transparency control.
    let hover_progress = animation_progress.clamp(0.0, 1.0);
    let (duration_opacity, favorite_opacity) = favorite_crossfade(hover_progress);
    let hovered_icon = svg(PLAY_ICON_HANDLE.clone())
        .width(tokens.icon(crate::ui::responsive::IconRole::Small))
        .height(tokens.icon(crate::ui::responsive::IconRole::Small))
        .style(|theme, _status| svg::Style {
            color: Some(theme::text_primary(theme)),
        })
        .opacity(hover_progress);
    let index_content: Element<'static, Message> = if is_playing {
        let playing_icon = svg(PLAY_ICON_HANDLE.clone())
            .width(tokens.icon(crate::ui::responsive::IconRole::Small))
            .height(tokens.icon(crate::ui::responsive::IconRole::Small))
            .style(|_theme, _status| svg::Style {
                color: Some(theme::ACCENT_PINK),
            })
            .opacity(1.0 - hover_progress);

        iced::widget::stack![playing_icon, hovered_icon].into()
    } else {
        let index = text(index_str)
            .size(tokens.text(TextRole::BodyLarge))
            .style(move |theme| text::Style {
                color: Some(theme::dimmed_text(theme).scale_alpha(1.0 - hover_progress)),
            });

        iced::widget::stack![index, hovered_icon].into()
    };

    // --- Song cover (use pre-loaded handle, no disk IO) ---
    let cover_handle = song
        .cover_key
        .and_then(|(kind, id)| image_state.get(kind, id));
    let cover = crate::ui::components::cover_image::custom(
        cover_handle,
        crate::image::ImageKind::SongCover,
        tokens.size(44.0),
        tokens.cover_radius(CoverRadiusRole::Thumbnail),
        tokens,
    );

    // --- Title info (use pre-truncated strings) ---
    let compact_secondary = if columns.compact {
        let mut secondary = display_artist.clone();
        if !display_album.is_empty() {
            secondary.push_str(" · ");
            secondary.push_str(&display_album);
        }
        if columns.show_added_date && !added_date.is_empty() {
            secondary.push_str(" · ");
            secondary.push_str(&added_date);
        }
        Some(secondary)
    } else {
        None
    };

    let secondary_text = compact_secondary.unwrap_or(display_artist);
    let title_info = column![
        text(display_title)
            .size(tokens.text(TextRole::BodyLarge))
            .width(Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(move |theme| text::Style {
                color: Some(if is_playing {
                    theme::ACCENT_PINK
                } else {
                    theme::text_primary(theme)
                })
            })
            .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT)),
        row![
            super::source_badge::source_badge(song.source, tokens),
            text(secondary_text)
                .size(tokens.text(TextRole::Label))
                .width(Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(move |theme| text::Style {
                    color: Some(theme::animated_text(theme, animation_progress))
                }),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(tokens.space(6.0))
        .width(Fill),
    ]
    .spacing(tokens.space(3.0));

    // --- Like button handling ---
    let ncm_song_id = if song_id < 0 {
        (-song_id) as u64
    } else {
        song_id as u64
    };
    let is_liked = liked_songs.is_some_and(|songs| songs.contains(&ncm_song_id));

    // Duration and favorite share one fixed trailing target at every profile.
    // Both widgets stay mounted while opacity alone performs the cross-fade,
    // preserving virtual-row geometry and touch/pointer event identity.
    let duration_or_like: Element<'static, Message> = if columns.show_like {
        let favorite_target_size = tokens.target(TargetRole::Icon);
        let favorite_glyph_size = favorite_glyph_size(tokens);
        let duration_text: Element<'static, Message> = container(
            text(duration)
                .size(tokens.text(TextRole::Body))
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(move |theme| text::Style {
                    color: Some(
                        theme::animated_text(theme, animation_progress * 0.8)
                            .scale_alpha(duration_opacity),
                    ),
                }),
        )
        .center(favorite_target_size)
        .into();

        let heart_handle = if is_liked {
            HEART_ICON_HANDLE.clone()
        } else {
            HEART_OUTLINE_ICON_HANDLE.clone()
        };
        let heart_button = button(
            container(
                svg(heart_handle)
                    .width(favorite_glyph_size)
                    .height(favorite_glyph_size)
                    .style(move |theme, _status| svg::Style {
                        color: Some(if is_liked {
                            theme::ACCENT_PINK
                        } else {
                            theme::text_primary(theme)
                        }),
                    })
                    .opacity(favorite_opacity),
            )
            .center(favorite_target_size),
        )
        .width(favorite_target_size)
        .height(favorite_target_size)
        .padding(0)
        .style(transparent_button)
        .on_press(Message::ToggleFavorite(ncm_song_id));
        let heart: Element<'static, Message> = widgets::hover_surface(heart_button)
            .style(move |theme, progress| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::hover_bg_alpha(
                    theme,
                    0.14 * favorite_opacity * progress,
                ))),
                border: iced::Border {
                    radius: (favorite_target_size / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .scale_on_hover(1.06)
            .into();

        container(iced::widget::stack![duration_text, heart])
            .width(tokens.size(50.0))
            .center_x(tokens.size(50.0))
            .into()
    } else {
        text(duration)
            .size(tokens.text(TextRole::Body))
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(move |theme| text::Style {
                color: Some(theme::animated_text(theme, animation_progress * 0.8)),
            })
            .into()
    };

    // --- Build row content (flattened structure) ---
    let mut row_items: Vec<Element<'static, Message>> = vec![
        container(index_content)
            .width(tokens.size(48.0))
            .center_x(tokens.size(48.0))
            .into(),
        cover,
        Space::new().width(tokens.space(14.0)).into(),
        title_info.width(Fill).into(),
    ];

    if columns.show_album && !columns.compact {
        row_items.push(
            text(display_album)
                .size(tokens.text(TextRole::Body))
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(move |theme| text::Style {
                    color: Some(theme::animated_text(theme, animation_progress)),
                })
                .width(tokens.size(200.0))
                .into(),
        );
    }

    if columns.show_added_date && !columns.compact {
        row_items.push(
            text(added_date)
                .size(tokens.text(TextRole::Body))
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(move |theme| text::Style {
                    color: Some(theme::animated_text(theme, animation_progress)),
                })
                .width(tokens.size(90.0))
                .into(),
        );
    }

    let duration_slot_width = tokens.size(50.0);
    row_items.push(
        container(duration_or_like)
            .width(duration_slot_width)
            .center_x(duration_slot_width)
            .into(),
    );

    let row_content = row(row_items).align_y(Alignment::Center).padding(
        Padding::new(tokens.space(8.0))
            .left(tokens.space(8.0))
            .right(tokens.space(12.0)),
    );

    // --- Outer button with animated background ---
    let btn = button(row_content)
        .width(Fill)
        .padding(0)
        .style(move |theme, _status| {
            let bg_color = if animation_progress > 0.001 {
                theme::hover_bg_alpha(theme, 0.12 * animation_progress)
            } else {
                Color::TRANSPARENT
            };
            button::Style {
                background: Some(iced::Background::Color(bg_color)),
                border: iced::Border {
                    radius: tokens
                        .radius(crate::ui::responsive::RadiusRole::Small)
                        .into(),
                    ..Default::default()
                },
                text_color: theme::text_primary(theme),
                ..Default::default()
            }
        })
        .on_press(Message::PlaySong(song_id));

    // Hover is now handled by VirtualList's on_item_hover for reliable tracking
    mouse_area(btn)
        .on_right_press(Message::RightClickSong(song_id))
        .into()
}

fn transparent_button(
    _theme: &iced::Theme,
    _status: button::Status,
) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: None,
        ..Default::default()
    }
}

/// Filter songs by search query (title, artist, album)
pub fn filter_song_indices(songs: &[SongItem], query: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return None;
    }

    let query_lower = query.to_lowercase();
    Some(
        songs
            .iter()
            .enumerate()
            .filter_map(|(index, song)| song.search_text.contains(&query_lower).then_some(index))
            .collect(),
    )
}

#[cfg(test)]
mod favorite_tests {
    use super::{FAVORITE_GLYPH_SIZE, favorite_crossfade, favorite_glyph_size};
    use crate::ui::responsive::{ResponsiveContext, TargetRole};
    use iced::Size;

    #[test]
    fn favorite_glyph_stays_smaller_than_its_hit_target() {
        let viewports = [
            Size::new(1_920.0, 1_080.0),
            Size::new(2_560.0, 1_440.0),
            Size::new(960.0, 1_080.0),
            Size::new(768.0, 1_024.0),
            Size::new(720.0, 800.0),
            Size::new(960.0, 540.0),
            Size::new(560.0, 800.0),
        ];

        for viewport in viewports {
            let context = ResponsiveContext::from_viewport(viewport);
            assert!(
                favorite_glyph_size(context.tokens) < context.tokens.target(TargetRole::Icon),
                "favorite glyph must not consume its hit target at {viewport:?}"
            );
            assert!(
                favorite_glyph_size(context.tokens) >= context.tokens.size(FAVORITE_GLYPH_SIZE),
                "favorite glyph must preserve the intended visual size at {viewport:?}"
            );
        }
    }

    #[test]
    fn favorite_glyph_uses_the_small_icon_reference_size() {
        assert_eq!(FAVORITE_GLYPH_SIZE, 16.0);
    }

    #[test]
    fn favorite_crossfade_hides_the_glyph_at_rest() {
        assert_eq!(favorite_crossfade(0.0), (1.0, 0.0));
        assert_eq!(favorite_crossfade(0.5), (0.5, 0.5));
        assert_eq!(favorite_crossfade(1.0), (0.0, 1.0));
        assert_eq!(favorite_crossfade(-1.0), (1.0, 0.0));
        assert_eq!(favorite_crossfade(2.0), (0.0, 1.0));
    }
}
