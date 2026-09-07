//! Queue popup component
//!
//! Shows the current play queue as a popup bubble above the player bar.

use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, svg, text};
use iced::{Alignment, Color, Element, Fill, Length, Padding};

use crate::app::Message;
use crate::database::DbSong;
use crate::i18n::{Key, Locale};
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::responsive::{
    ChromeRole, IconRole, ResponsiveContext, TextRole, bounded_panel_size, top_bar_height,
};
use crate::ui::{icons, theme, widgets};

/// Queue popup width
// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
pub const QUEUE_PANEL_WIDTH: f32 = 360.0;
/// Queue popup max height
pub const QUEUE_PANEL_HEIGHT: f32 = 400.0;
/// Height of each queue item (padding 8*2 + content ~36)
const QUEUE_ITEM_HEIGHT: f32 = 54.0;
/// Scrollable ID for queue panel
pub const QUEUE_SCROLLABLE_ID: &str = "queue_panel_scroll";

/// Calculate the queue offset using the same tokenized panel geometry as the
/// rendered queue. Keeping this policy pure lets the update layer request an
/// initial position without importing widget state or message semantics.
pub fn calculate_scroll_offset(
    queue_len: usize,
    queue_index: Option<usize>,
    context: ResponsiveContext,
) -> f32 {
    let Some(idx) = queue_index else {
        return 0.0;
    };

    if queue_len == 0 {
        return 0.0;
    }

    let tokens = context.tokens;
    let panel_size = bounded_panel_size(
        iced::Size::new(
            tokens.size(QUEUE_PANEL_WIDTH),
            tokens.size(QUEUE_PANEL_HEIGHT),
        ),
        context.viewport,
        tokens.space(16.0),
        tokens.space(12.0),
        top_bar_height(&context) + tokens.chrome(ChromeRole::PlayerBar) + tokens.space(8.0),
    );
    let visible_height = (panel_size.height - tokens.size(60.0)).max(tokens.size(80.0));
    let item_height = tokens.size(QUEUE_ITEM_HEIGHT);
    let total_height = queue_len as f32 * item_height;

    if total_height <= visible_height {
        return 0.0;
    }

    let item_center = idx as f32 * item_height + item_height / 2.0;
    let target_scroll = item_center - visible_height / 2.0;
    let max_scroll = total_height - visible_height;
    let clamped_scroll = target_scroll.clamp(0.0, max_scroll);
    clamped_scroll / max_scroll
}

/// Build the queue popup bubble
pub fn view(
    context: ResponsiveContext,
    queue: &[DbSong],
    queue_index: Option<usize>,
    locale: Locale,
    is_fm_mode: bool,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let panel_size = bounded_panel_size(
        iced::Size::new(
            tokens.size(QUEUE_PANEL_WIDTH),
            tokens.size(QUEUE_PANEL_HEIGHT),
        ),
        context.viewport,
        tokens.space(16.0),
        tokens.space(12.0),
        top_bar_height(&context) + tokens.chrome(ChromeRole::PlayerBar) + tokens.space(8.0),
    );
    let panel_width = panel_size.width;
    let panel_height = panel_size.height;
    let header_height = tokens.size(60.0);

    let header_title = if is_fm_mode {
        "私人FM".to_string()
    } else {
        locale.get(Key::QueueTitle).to_string()
    };

    let header = row![
        text(header_title)
            .size(tokens.text(TextRole::BodyLarge))
            .style(move |theme| text::Style {
                color: Some(theme::text_primary(theme))
            }),
        Space::new().width(Fill),
        text(
            locale
                .get(Key::QueueSongCount)
                .replace("{}", &queue.len().to_string()),
        )
        .size(tokens.text(TextRole::Caption))
        .style(|theme| text::Style {
            color: Some(theme::text_muted(theme))
        }),
        Space::new().width(tokens.space(8.0)),
        button(
            svg(svg::Handle::from_memory(icons::TRASH.as_bytes()))
                .width(tokens.icon(IconRole::Small))
                .height(tokens.icon(IconRole::Small))
                .style(|theme, _status| svg::Style {
                    color: Some(theme::text_muted(theme)),
                })
        )
        .padding(tokens.space(6.0))
        .style(theme::transparent_btn)
        .on_press(Message::ClearQueue),
        button(
            svg(svg::Handle::from_memory(icons::CLOSE.as_bytes()))
                .width(tokens.icon(IconRole::Small))
                .height(tokens.icon(IconRole::Small))
                .style(|theme, _status| svg::Style {
                    color: Some(theme::text_muted(theme)),
                })
        )
        .padding(tokens.space(6.0))
        .style(theme::transparent_btn)
        .on_press(Message::ToggleQueue),
    ]
    .align_y(Alignment::Center)
    .padding(
        Padding::new(tokens.space(12.0))
            .left(tokens.space(16.0))
            .right(tokens.space(12.0)),
    );

    let song_items: Vec<Element<'static, Message>> = queue
        .iter()
        .enumerate()
        .map(|(idx, song)| {
            let is_current = queue_index == Some(idx);
            build_queue_item(song.clone(), idx, is_current, tokens)
        })
        .collect();

    let song_list: Element<'static, Message> = if song_items.is_empty() {
        container(
            text(locale.get(Key::QueueEmpty).to_string())
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        )
        .width(Fill)
        .padding(tokens.space(32.0))
        .center_x(Fill)
        .into()
    } else {
        crate::ui::widgets::smooth_scroll(
            scrollable(
                column(song_items).spacing(tokens.space(2.0)).padding(
                    Padding::new(0.0)
                        .left(tokens.space(8.0))
                        .right(tokens.space(8.0))
                        .bottom(tokens.space(8.0)),
                ),
            )
            .direction(crate::ui::widgets::vertical_scrollbar(tokens))
            .id(iced::widget::Id::new(QUEUE_SCROLLABLE_ID))
            .height(Length::Fixed(
                (panel_height - header_height).max(tokens.size(80.0)),
            )),
            SmoothScrollTarget::Native(QUEUE_SCROLLABLE_ID),
            tokens,
            Message::SmoothScroll,
        )
        .into()
    };

    let content = column![header, song_list,].width(panel_width);

    container(content)
        .width(panel_width)
        .height(panel_height)
        .style(move |theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::surface_elevated(theme))),
            border: iced::Border {
                color: theme::divider(theme),
                width: tokens.size(1.0),
                radius: tokens.size(12.0).into(),
            },
            shadow: iced::Shadow {
                color: theme::overlay_backdrop(theme, 0.5),
                offset: iced::Vector::new(0.0, -tokens.size(4.0)),
                blur_radius: tokens.size(20.0),
            },
            ..Default::default()
        })
        .into()
}

/// Build a single queue item
fn build_queue_item(
    song: DbSong,
    index: usize,
    is_current: bool,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    let duration_secs = song.duration_secs as u64;
    let mins = duration_secs / 60;
    let secs = duration_secs % 60;
    let duration_str = format!("{}:{:02}", mins, secs);

    let indicator: Element<'static, Message> = if is_current {
        svg(svg::Handle::from_memory(icons::PLAYING.as_bytes()))
            .width(tokens.icon(IconRole::Small))
            .height(tokens.icon(IconRole::Small))
            .style(|_theme, _status| svg::Style {
                color: Some(theme::ACCENT_PINK),
            })
            .into()
    } else {
        text(format!("{}", index + 1))
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .into()
    };

    let indicator_size = tokens.size(24.0);
    let indicator_container = container(indicator)
        .width(indicator_size)
        .center_x(indicator_size);

    let title = text(song.title.clone())
        .size(tokens.text(TextRole::Label))
        .style(move |theme| text::Style {
            color: Some(if is_current {
                theme::ACCENT_PINK
            } else {
                theme::text_primary(theme)
            }),
        });

    let artist_text = if song.artist.is_empty() {
        "未知艺术家".to_string()
    } else {
        song.artist.clone()
    };
    let artist = text(artist_text)
        .size(tokens.text(TextRole::Caption))
        .style(move |theme| text::Style {
            color: Some(if is_current {
                Color::from_rgba(
                    theme::ACCENT_PINK.r,
                    theme::ACCENT_PINK.g,
                    theme::ACCENT_PINK.b,
                    0.7,
                )
            } else {
                theme::text_muted(theme)
            }),
        });

    let info = column![title, artist]
        .spacing(tokens.space(2.0))
        .width(Fill);

    let duration = text(duration_str)
        .size(tokens.text(TextRole::Caption))
        .style(|theme| text::Style {
            color: Some(theme::text_muted(theme)),
        });

    let remove_btn = button(
        svg(svg::Handle::from_memory(icons::CLOSE.as_bytes()))
            .width(tokens.icon(IconRole::Small))
            .height(tokens.icon(IconRole::Small))
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_muted(theme)),
            }),
    )
    .padding(tokens.space(4.0))
    .style(theme::transparent_btn)
    .on_press(Message::RemoveFromQueue(index));

    let item_row = row![
        indicator_container,
        Space::new().width(tokens.space(8.0)),
        info,
        duration,
        Space::new().width(tokens.space(4.0)),
        remove_btn,
    ]
    .align_y(Alignment::Center)
    .padding(
        Padding::new(tokens.space(8.0))
            .left(tokens.space(8.0))
            .right(tokens.space(8.0)),
    );

    let song_id = song.id;
    let btn = button(item_row)
        .width(Fill)
        .padding(0)
        .style(|_theme, _status| button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            ..Default::default()
        })
        .on_press(Message::PlayQueueIndex(index));
    let btn = widgets::hover_surface(btn).style(move |theme, progress| {
        let base = if is_current {
            theme::hover_bg(theme)
        } else {
            Color::TRANSPARENT
        };
        iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                base,
                theme::hover_bg(theme),
                progress,
            ))),
            border: iced::Border {
                radius: tokens.size(4.0).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });

    mouse_area(btn)
        .on_right_press(Message::RightClickSong(song_id))
        .into()
}
