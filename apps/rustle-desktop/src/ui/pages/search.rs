//! Search results page
//!
//! Displays search results for songs, artists, albums, and playlists
//! with tabbed navigation and pagination.

use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message, SearchPageState, SearchTab};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::animation::{SmoothScrollEvent, SmoothScrollTarget};
use crate::ui::components::cover_image;
use crate::ui::responsive::{LayoutProfile, ResponsiveContext, TextRole, playlist_card_metrics};
use crate::ui::theme::BOLD_WEIGHT;
use crate::ui::{theme, widgets};

use crate::ui::primitives::virtual_list::VirtualList;

/// Page size for pagination
const PAGE_SIZE: u32 = 50;
// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
const SONG_ROW_HEIGHT: f32 = 64.0;

/// Build the search results page view
pub fn view<'a>(
    state: &'a SearchPageState,
    image_state: &'a ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    view_for_context(state, image_state, locale, context)
}

fn view_for_context<'a>(
    state: &'a SearchPageState,
    image_state: &'a ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    if state.keyword.is_empty() {
        return empty_search_state(locale, context);
    }

    let title = text(&state.keyword)
        .size(tokens.text(TextRole::Hero))
        .style(|theme| iced::widget::text::Style {
            color: Some(theme::text_primary(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));
    let related = text(format!(" {}", locale.get(Key::SearchRelated)))
        .size(tokens.text(TextRole::TitleLarge))
        .style(|theme| iced::widget::text::Style {
            color: Some(theme::text_muted(theme)),
        });
    let title_row: Element<'a, Message> = if matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        column![title.width(Fill).wrapping(Wrapping::WordOrGlyph), related]
            .spacing(tokens.space(4.0))
            .into()
    } else {
        row![
            title
                .width(Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End),
            related
        ]
        .align_y(Alignment::Center)
        .into()
    };
    let title_and_tabs: Element<'a, Message> = if matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        column![
            title_row,
            Space::new().height(tokens.space(16.0)),
            search_tabs(state.active_tab, locale, context)
        ]
        .into()
    } else {
        column![
            title_row,
            Space::new().height(tokens.space(24.0)),
            search_tabs(state.active_tab, locale, context)
        ]
        .into()
    };

    // Header section (title + tabs), with a stacked title in portrait layouts.
    let header_section = container(title_and_tabs)
        .width(Fill)
        .padding(
            Padding::new(tokens.space(40.0))
                .top(tokens.space(70.0))
                .right(tokens.space(32.0))
                .bottom(tokens.space(20.0))
                .left(tokens.space(32.0)),
        )
        .style(|theme| container::Style {
            background: Some(Background::Color(theme::background(theme))),
            ..Default::default()
        });

    // Content area
    let content: Element<'a, Message> = if state.loading {
        loading_state(context)
    } else {
        match state.active_tab {
            SearchTab::Songs => {
                if state.tracks.is_empty() {
                    empty_results_state(&state.keyword, context)
                } else {
                    // Use VirtualList for high performance song list
                    let song_count = state.tracks.len();
                    let songs = &state.tracks;
                    let song_animations = &state.song_animations;
                    let current_page = state.current_page;
                    let song_row_height = tokens.size(SONG_ROW_HEIGHT);

                    let table_header = search_table_header(context);

                    let virtual_list =
                        VirtualList::new(song_count, song_row_height, tokens, move |index| {
                            if index >= songs.len() {
                                return Space::new().height(song_row_height).into();
                            }

                            let song = &songs[index];
                            let hover_progress = song_animations.get_progress(&song.id);
                            let index_num = current_page * PAGE_SIZE + index as u32 + 1;
                            let duration_secs = song.duration_ms / 1000;
                            let duration_str =
                                format!("{}:{:02}", duration_secs / 60, duration_secs % 60);
                            let quality_label = song
                                .quality_options
                                .iter()
                                .max_by_key(|option| option.level.priority())
                                .map(|option| option.level.short_name().to_string());
                            let availability_label = song.availability.label();
                            let availability_restricted = song.availability.is_restricted();
                            let quality_badge: Element<'static, Message> = quality_label
                                .map(|label| -> Element<'static, Message> {
                                    text(label)
                                        .size(tokens.text(TextRole::Caption))
                                        .style(|_theme| iced::widget::text::Style {
                                            color: Some(theme::accent(_theme)),
                                        })
                                        .into()
                                })
                                .unwrap_or_else(|| -> Element<'static, Message> {
                                    Space::new().width(0).into()
                                });
                            let availability_badge: Element<'static, Message> =
                                if availability_label.is_empty() {
                                    Space::new().width(0).into()
                                } else {
                                    text(availability_label)
                                        .size(tokens.text(TextRole::Caption))
                                        .style(move |theme| iced::widget::text::Style {
                                            color: Some(if availability_restricted {
                                                theme::accent(theme)
                                            } else {
                                                theme::text_muted(theme)
                                            }),
                                        })
                                        .into()
                                };

                            search_song_row(
                                song,
                                index_num,
                                duration_str,
                                quality_badge,
                                availability_badge,
                                hover_progress,
                                context,
                            )
                        })
                        .keyed_by(move |index| {
                            songs
                                .get(index)
                                .map(|song| (song.id, current_page, index))
                                .unwrap_or((0, current_page, index))
                        })
                        .state(state.scroll_state.clone())
                        .on_item_hover(move |index| {
                            if index < songs.len() {
                                Message::HoverSearchSong(Some(songs[index].id))
                            } else {
                                Message::HoverSearchSong(None)
                            }
                        })
                        .on_empty_area(Message::HoverSearchSong(None))
                        .on_smooth_scroll(|delta| {
                            Message::SmoothScroll(SmoothScrollEvent::Requested {
                                target: SmoothScrollTarget::SearchSongs,
                                delta,
                            })
                        })
                        .on_smooth_scroll_cancel(Message::SmoothScroll(
                            SmoothScrollEvent::Cancelled {
                                target: SmoothScrollTarget::SearchSongs,
                            },
                        ))
                        .height(Length::Fill);

                    let list_section = column![
                        table_header,
                        Space::new().height(tokens.space(8.0)),
                        container(virtual_list).height(Fill).width(Fill),
                    ]
                    .padding(Padding::new(tokens.space(32.0)).top(0.0));

                    if state.total_count > PAGE_SIZE {
                        column![
                            list_section.height(Fill),
                            Space::new().height(tokens.space(16.0)),
                            pagination(state, context),
                            Space::new().height(tokens.space(32.0)),
                        ]
                        .height(Fill)
                        .into()
                    } else {
                        column![
                            list_section.height(Fill),
                            Space::new().height(tokens.space(32.0)),
                        ]
                        .height(Fill)
                        .into()
                    }
                }
            }
            SearchTab::Albums
            | SearchTab::Artists
            | SearchTab::Playlists
            | SearchTab::Videos
            | SearchTab::Radios => {
                let is_empty = match state.active_tab {
                    SearchTab::Artists => state.artists.is_empty(),
                    SearchTab::Albums => state.albums.is_empty(),
                    SearchTab::Playlists => state.playlists.is_empty(),
                    SearchTab::Videos => state.videos.is_empty(),
                    SearchTab::Radios => state.radios.is_empty(),
                    SearchTab::Songs => true,
                };
                let content = if is_empty {
                    empty_results_state(&state.keyword, context)
                } else {
                    let grid = grid_results(state, image_state, state.active_tab, context);
                    let mut col = column![grid];

                    if state.total_count > PAGE_SIZE {
                        col = col
                            .push(Space::new().height(tokens.space(24.0)))
                            .push(pagination(state, context));
                    }
                    col = col.push(Space::new().height(tokens.space(40.0)));

                    col.padding(Padding::new(tokens.space(32.0)).top(0.0))
                        .into()
                };

                widgets::page_scrollable(content, "search_scroll", tokens, Message::SmoothScroll)
            }
        }
    };

    container(column![header_section, content].width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(theme::main_content)
        .into()
}

fn search_song_row<'a>(
    song: &'a crate::api::Track,
    index_num: u32,
    duration: String,
    quality_badge: Element<'static, Message>,
    availability_badge: Element<'static, Message>,
    hover_progress: f32,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let badges = row![quality_badge, availability_badge]
        .spacing(tokens.space(6.0))
        .width(Fill);

    let row_content: Element<'a, Message> = if matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        let metadata = format!("{} · {}", song.artist_names(), song.album.name);
        row![
            text(format!("{:02}", index_num))
                .size(tokens.text(TextRole::Label))
                .width(tokens.size(36.0))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
            column![
                text(song.title.as_str())
                    .size(tokens.text(TextRole::BodyLarge))
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .style(move |theme| iced::widget::text::Style {
                        color: Some(theme::animated_text(theme, hover_progress)),
                    }),
                badges,
                text(metadata)
                    .size(tokens.text(TextRole::Caption))
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .style(|theme| iced::widget::text::Style {
                        color: Some(theme::text_secondary(theme)),
                    }),
            ]
            .spacing(tokens.space(3.0))
            .width(Fill),
            text(duration)
                .size(tokens.text(TextRole::Label))
                .width(tokens.size(60.0))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        ]
        .spacing(tokens.space(8.0))
        .align_y(Alignment::Center)
        .into()
    } else {
        row![
            text(format!("{:02}", index_num))
                .size(tokens.text(TextRole::Label))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .width(tokens.size(40.0)),
            column![
                text(song.title.as_str())
                    .size(tokens.text(TextRole::Body))
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .style(move |theme| iced::widget::text::Style {
                        color: Some(theme::animated_text(theme, hover_progress)),
                    }),
                badges,
            ]
            .width(Fill),
            text(song.artist_names())
                .size(tokens.text(TextRole::Label))
                .width(Length::FillPortion(2))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_secondary(theme)),
                }),
            text(song.album.name.as_str())
                .size(tokens.text(TextRole::Label))
                .width(Length::FillPortion(2))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
            text(duration)
                .size(tokens.text(TextRole::Label))
                .width(tokens.size(60.0))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        ]
        .spacing(tokens.space(12.0))
        .align_y(Alignment::Center)
        .into()
    };

    button(row_content)
        .style(move |theme, status| {
            song_row_style(
                theme,
                status,
                hover_progress,
                tokens.radius(crate::ui::responsive::RadiusRole::Small),
            )
        })
        .on_press(Message::PlaySearchSong(song.id))
        .width(Fill)
        .padding(
            Padding::new(tokens.space(10.0))
                .left(tokens.space(12.0))
                .right(tokens.space(12.0)),
        )
        .into()
}

/// Search tabs component
fn search_tabs(
    active_tab: SearchTab,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let tabs = [
        (SearchTab::Songs, Key::SearchTabSongs),
        (SearchTab::Artists, Key::SearchTabArtists),
        (SearchTab::Albums, Key::SearchTabAlbums),
        (SearchTab::Playlists, Key::SearchTabPlaylists),
        (SearchTab::Videos, Key::SearchTabVideos),
        (SearchTab::Radios, Key::SearchTabRadios),
    ];

    let tab_buttons: Vec<Element<'static, Message>> = tabs
        .iter()
        .map(|(tab, label_key)| {
            let is_active = active_tab == *tab;
            let tab_clone = *tab;

            let tab_button = button(
                container(
                    text(locale.get(*label_key).to_string())
                        .size(tokens.text(TextRole::Body))
                        .style(move |theme| iced::widget::text::Style {
                            color: Some(if is_active {
                                theme::accent(theme)
                            } else {
                                theme::settings_inactive_tab(theme)
                            }),
                        }),
                )
                .width(Fill)
                .center_x(Fill),
            )
            .style(move |theme, status| {
                let hover_bg = match status {
                    button::Status::Hovered => {
                        Some(Background::Color(theme::hover_bg_alpha(theme, 0.05)))
                    }
                    _ => None,
                };
                button::Style {
                    background: hover_bg,
                    text_color: theme::text_primary(theme),
                    border: Border::default(),
                    ..Default::default()
                }
            })
            .on_press(Message::SearchTabChanged(tab_clone))
            .padding([tokens.space(12.0), 0.0])
            .width(Fill);

            let underline = container(Space::new().height(tokens.size(2.0)))
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(Background::Color(if is_active {
                        theme::accent(theme)
                    } else {
                        theme::settings_inactive_underline(theme)
                    })),
                    ..Default::default()
                });

            container(column![tab_button, underline].spacing(0).width(Fill))
                .width(tokens.size(100.0))
                .into()
        })
        .collect();

    widgets::scaled_scroll(
        scrollable(row(tab_buttons).spacing(0))
            .direction(widgets::hidden_horizontal_scrollbar())
            .id(iced::widget::Id::new("search_tabs_scroll"))
            .width(Fill),
        tokens,
    )
    .into()
}

/// Search table header
fn search_table_header(context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    let mut items: Vec<Element<'static, Message>> = vec![
        text("#")
            .size(tokens.text(TextRole::Caption))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .width(tokens.size(40.0))
            .into(),
        text("标题")
            .size(tokens.text(TextRole::Caption))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .width(Fill)
            .into(),
    ];
    if !matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        items.push(
            text("歌手")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .width(Length::FillPortion(2))
                .into(),
        );
        items.push(
            text("专辑")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .width(Length::FillPortion(2))
                .into(),
        );
    }
    items.push(
        text("时长")
            .size(tokens.text(TextRole::Caption))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .width(tokens.size(60.0))
            .into(),
    );
    row(items)
        .spacing(tokens.space(12.0))
        .padding(
            Padding::new(tokens.space(8.0))
                .left(tokens.space(12.0))
                .right(tokens.space(12.0)),
        )
        .into()
}

/// Song row style with hover animation
fn song_row_style(
    theme: &iced::Theme,
    status: button::Status,
    hover_progress: f32,
    radius: f32,
) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => {
            theme::hover_bg_alpha(theme, 0.08 + 0.04 * hover_progress)
        }
        _ => theme::hover_bg_alpha(theme, 0.04 * hover_progress),
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::text_primary(theme),
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Grid view for albums and playlists
fn grid_results<'a>(
    state: &'a SearchPageState,
    image_state: &'a ImageState,
    tab: SearchTab,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let items: Vec<GridItemRef<'a>> = match tab {
        SearchTab::Albums => state.albums.iter().map(GridItemRef::Album).collect(),
        SearchTab::Artists => state.artists.iter().map(GridItemRef::Artist).collect(),
        SearchTab::Playlists => state.playlists.iter().map(GridItemRef::Playlist).collect(),
        SearchTab::Videos => state.videos.iter().map(GridItemRef::Video).collect(),
        SearchTab::Radios => state.radios.iter().map(GridItemRef::Radio).collect(),
        _ => return Space::new().into(),
    };
    let kind = search_image_kind(tab);

    let tokens = context.tokens;
    let card_metrics = playlist_card_metrics(context);
    let card_width = card_metrics.width;
    let card_spacing = card_metrics.gap;
    let row_spacing = tokens.space(32.0);

    widgets::responsive_card_columns(card_metrics, usize::MAX, move |columns| {
        let mut rows: Vec<Element<'a, Message>> = Vec::new();

        for chunk in items.chunks(columns) {
            let mut row_items: Vec<Element<'a, Message>> = Vec::new();

            for item in chunk {
                let hover_progress = state.card_animations.get_progress(&item.id());
                let item_id = item.id();
                let item_tab = tab;

                let cover_handle = image_state.get(kind, item_id);
                let card = grid_card(GridCardView {
                    item: *item,
                    cover_handle,
                    kind,
                    hover_progress,
                    item_id,
                    tab: item_tab,
                    card_width,
                    card_radius: card_metrics.radius,
                    tokens,
                });
                row_items.push(card);
            }

            let mut spaced_row = Vec::with_capacity(row_items.len().saturating_mul(2));
            for (index, item) in row_items.into_iter().enumerate() {
                if index > 0 {
                    spaced_row.push(Space::new().width(card_spacing).into());
                }
                spaced_row.push(item);
            }
            rows.push(row(spaced_row).into());
            rows.push(Space::new().height(row_spacing).into());
        }

        column(rows).into()
    })
}

/// Grid card for album/playlist
struct GridCardView<'a> {
    item: GridItemRef<'a>,
    cover_handle: Option<&'a iced::widget::image::Handle>,
    kind: ImageKind,
    hover_progress: f32,
    item_id: u64,
    tab: SearchTab,
    card_width: f32,
    card_radius: f32,
    tokens: crate::ui::responsive::UiTokens,
}

fn grid_card<'a>(view: GridCardView<'a>) -> Element<'a, Message> {
    let GridCardView {
        item,
        cover_handle,
        kind,
        hover_progress,
        item_id,
        tab,
        card_width,
        card_radius,
        tokens,
    } = view;
    let has_cover = cover_handle.is_some();

    let cover: Element<'a, Message> = container(cover_image::custom(
        cover_handle,
        kind,
        card_width,
        card_radius,
        tokens,
    ))
    .width(card_width)
    .height(card_width)
    .style(move |theme| {
        if has_cover {
            cover_card_style(theme, hover_progress, card_radius, tokens)
        } else {
            cover_placeholder_style(theme, hover_progress, card_radius, tokens)
        }
    })
    .into();

    let card_content = column![
        cover,
        Space::new().height(tokens.space(8.0)),
        text(item.name())
            .size(tokens.text(TextRole::Body))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_primary(theme)),
            })
            .width(card_width),
        text(item.subtitle())
            .size(tokens.text(TextRole::Caption))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .width(card_width),
    ]
    .width(card_width);

    let card_btn = button(card_content)
        .padding(0)
        .style(|_theme, _status| button::Style {
            background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
            ..Default::default()
        })
        .on_press(Message::OpenSearchResult(item_id, tab));

    iced::widget::mouse_area(card_btn)
        .on_enter(Message::HoverSearchCard(Some(item_id)))
        .on_exit(Message::HoverSearchCard(None))
        .into()
}

#[derive(Clone, Copy)]
enum GridItemRef<'a> {
    Album(&'a crate::api::AlbumSummary),
    Artist(&'a crate::api::ArtistSummary),
    Playlist(&'a crate::api::PlaylistSummary),
    Video(&'a crate::api::VideoSummary),
    Radio(&'a crate::api::RadioSummary),
}

impl<'a> GridItemRef<'a> {
    fn id(self) -> u64 {
        match self {
            Self::Album(item) => item.id,
            Self::Artist(item) => item.id,
            Self::Playlist(item) => item.id,
            Self::Video(item) => item.id,
            Self::Radio(item) => item.id,
        }
    }

    fn name(self) -> &'a str {
        match self {
            Self::Album(item) => &item.name,
            Self::Artist(item) => &item.name,
            Self::Playlist(item) => &item.name,
            Self::Video(item) => &item.name,
            Self::Radio(item) => &item.name,
        }
    }

    fn subtitle(self) -> String {
        match self {
            Self::Album(item) => item.artist_names(),
            Self::Artist(_) => "歌手".to_string(),
            Self::Playlist(item) => item.creator.nickname.clone(),
            Self::Video(item) => item.artist_name.clone(),
            Self::Radio(item) => {
                if item.category.is_empty() {
                    item.creator.nickname.clone()
                } else {
                    item.category.clone()
                }
            }
        }
    }
}

fn search_image_kind(tab: SearchTab) -> ImageKind {
    match tab {
        SearchTab::Artists => ImageKind::ArtistCover,
        SearchTab::Albums => ImageKind::AlbumCover,
        SearchTab::Playlists => ImageKind::PlaylistCover,
        SearchTab::Videos => ImageKind::VideoCover,
        SearchTab::Radios => ImageKind::RadioCover,
        SearchTab::Songs => ImageKind::SongCover,
    }
}

/// Cover placeholder style
fn cover_placeholder_style(
    theme: &iced::Theme,
    hover_progress: f32,
    radius: f32,
    tokens: crate::ui::responsive::UiTokens,
) -> container::Style {
    cover_base_style(
        theme,
        hover_progress,
        iced::Background::Color(theme::surface(theme)),
        radius,
        tokens,
    )
}

fn cover_card_style(
    theme: &iced::Theme,
    hover_progress: f32,
    radius: f32,
    tokens: crate::ui::responsive::UiTokens,
) -> container::Style {
    cover_base_style(
        theme,
        hover_progress,
        iced::Background::Color(iced::Color::TRANSPARENT),
        radius,
        tokens,
    )
}

fn cover_base_style(
    theme: &iced::Theme,
    hover_progress: f32,
    background: iced::Background,
    radius: f32,
    tokens: crate::ui::responsive::UiTokens,
) -> container::Style {
    let shadow_blur = tokens.size(8.0 + 8.0 * hover_progress);
    let shadow_alpha = if theme::is_dark_theme(theme) {
        0.2 + 0.2 * hover_progress
    } else {
        0.08 + 0.08 * hover_progress
    };
    let scale_offset = -tokens.size(2.0 * hover_progress);

    container::Style {
        background: Some(background),
        border: iced::Border {
            radius: radius.into(),
            width: tokens.size(1.0),
            color: theme::border_color(theme),
        },
        shadow: iced::Shadow {
            color: rustle_ui::theme::black(shadow_alpha),
            offset: iced::Vector::new(0.0, tokens.size(4.0) + scale_offset),
            blur_radius: shadow_blur,
        },
        ..Default::default()
    }
}

/// Pagination component
fn pagination<'a>(state: &'a SearchPageState, context: ResponsiveContext) -> Element<'a, Message> {
    let tokens = context.tokens;
    let total_pages = state.total_count.div_ceil(PAGE_SIZE);
    let current_page = state.current_page;

    let mut items: Vec<Element<'a, Message>> = Vec::new();

    // Previous button
    let prev_btn = button(text("上一页").size(tokens.text(TextRole::Label)))
        .padding(
            Padding::new(tokens.space(8.0))
                .left(tokens.space(16.0))
                .right(tokens.space(16.0)),
        )
        .style(move |theme, status| theme::secondary_button(theme, status, tokens.theme_metrics()))
        .on_press_maybe(if current_page > 0 {
            Some(Message::SearchPageChanged(current_page - 1))
        } else {
            None
        });
    items.push(prev_btn.into());

    // Page info
    items.push(
        text(format!("{} / {}", current_page + 1, total_pages))
            .size(tokens.text(TextRole::Body))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_secondary(theme)),
            })
            .into(),
    );

    // Next button
    let next_btn = button(text("下一页").size(tokens.text(TextRole::Label)))
        .padding(
            Padding::new(tokens.space(8.0))
                .left(tokens.space(16.0))
                .right(tokens.space(16.0)),
        )
        .style(move |theme, status| theme::secondary_button(theme, status, tokens.theme_metrics()))
        .on_press_maybe(if current_page + 1 < total_pages {
            Some(Message::SearchPageChanged(current_page + 1))
        } else {
            None
        });
    items.push(next_btn.into());

    container(
        row(items)
            .spacing(tokens.space(16.0))
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .align_x(Alignment::Center)
    .into()
}

/// Loading state
fn loading_state<'a>(context: ResponsiveContext) -> Element<'a, Message> {
    let tokens = context.tokens;
    container(
        text("搜索中...")
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| iced::widget::text::Style {
                color: Some(theme::text_muted(theme)),
            }),
    )
    .width(Fill)
    .height(tokens.size(200.0))
    .center_x(Fill)
    .center_y(tokens.size(200.0))
    .into()
}

/// Empty search state (no keyword entered)
fn empty_search_state<'a>(_locale: Locale, context: ResponsiveContext) -> Element<'a, Message> {
    let tokens = context.tokens;
    container(
        column![
            text("🔍").size(tokens.text(TextRole::Display)),
            Space::new().height(tokens.space(16.0)),
            text("输入关键词开始搜索")
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        ]
        .align_x(Alignment::Center),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .style(theme::main_content)
    .into()
}

/// Empty results state
fn empty_results_state<'a>(keyword: &str, context: ResponsiveContext) -> Element<'a, Message> {
    let tokens = context.tokens;
    container(
        column![
            text("🔍").size(tokens.text(TextRole::Display)),
            Space::new().height(tokens.space(16.0)),
            text(format!("未找到 \"{}\" 的相关结果", keyword))
                .size(tokens.text(TextRole::BodyLarge))
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(|theme| iced::widget::text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        ]
        .align_x(Alignment::Center),
    )
    .width(Fill)
    .height(tokens.size(200.0))
    .center_x(Fill)
    .center_y(tokens.size(200.0))
    .into()
}

#[cfg(test)]
mod tests {
    use iced::Size;

    use crate::ui::responsive::{
        ResponsiveContext, calculate_grid_columns_clamped, playlist_card_metrics,
    };

    fn visible_column_count(available_width: f32, context: ResponsiveContext) -> usize {
        let metrics = playlist_card_metrics(context);
        calculate_grid_columns_clamped(available_width, metrics.width, metrics.gap, usize::MAX)
    }

    #[test]
    fn search_card_results_use_complete_columns_at_all_validation_viewports() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), 1_650.0, 8),
            (Size::new(2_560.0, 1_440.0), 2_093.0, 8),
            (Size::new(960.0, 1_080.0), 828.0, 5),
            (Size::new(1_280.0, 1_440.0), 1_104.0, 5),
            (Size::new(768.0, 1_024.0), 642.0, 3),
            (Size::new(720.0, 800.0), 601.0, 3),
            (Size::new(960.0, 540.0), 841.0, 5),
            (Size::new(560.0, 800.0), 502.0, 3),
        ];

        for (viewport, available_width, expected_columns) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            assert_eq!(
                visible_column_count(available_width, context),
                expected_columns,
                "unexpected complete search-card columns for {viewport:?}"
            );
        }
    }
}
