//! Right-click context menu — glass-morphism floating panel
//! Theme-aware: adapts to dark/light via theme functions in style closures

use iced::widget::{button, column, container, row, svg, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};

use crate::app::{ContextMenuAction, ContextMenuState, Message};
use crate::i18n::{Key, Locale};
use crate::ui::responsive::{
    IconRole, RadiusRole, ResponsiveContext, TargetRole, TextRole, bounded_height, bounded_width,
};
use crate::ui::{icons, overlay, theme, widgets};
use crate::utils::Source;

// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
const MENU_WIDTH: f32 = 240.0;
const PANEL_PADDING: f32 = 8.0;
const ITEM_HORIZONTAL_PADDING: f32 = 12.0;
const DIVIDER_VERTICAL_PADDING: f32 = 4.0;
const DIVIDER_COUNT: f32 = 4.0;

/// Render the menu using the same logical viewport and root rem as the shell.
pub fn view(
    menu: &ContextMenuState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'_, Message> {
    let tokens = context.tokens;
    let source = menu.source;
    let is_liked = menu.is_liked;
    let can_show_folder = source != Source::Online;
    let can_download = source != Source::Local;
    let can_edit = source == Source::Local;

    let always_items: usize = 7;
    let mut item_count = always_items;
    if can_show_folder {
        item_count += 1;
    }
    if can_download {
        item_count += 1;
    }
    if can_edit {
        item_count += 1;
    }

    let menu_width = bounded_width(
        tokens.size(MENU_WIDTH),
        context.width(),
        tokens.space(PANEL_PADDING),
    );
    let item_height = tokens.target(TargetRole::Icon);
    let divider_height = tokens.space(1.0 + DIVIDER_VERTICAL_PADDING * 2.0);
    let menu_height = item_count as f32 * item_height
        + DIVIDER_COUNT * divider_height
        + tokens.space(PANEL_PADDING * 2.0);
    let menu_height = bounded_height(menu_height, context.height(), tokens.space(PANEL_PADDING));
    let edge_gap = tokens.space(4.0);

    let x = if menu.x + menu_width > context.width() {
        (menu.x - menu_width - edge_gap).max(0.0)
    } else {
        menu.x.max(0.0)
    }
    .min((context.width() - menu_width).max(0.0));
    let y = if menu.y + menu_height > context.height() {
        (menu.y - menu_height - edge_gap).max(0.0)
    } else {
        menu.y.max(0.0)
    }
    .min((context.height() - menu_height).max(0.0));

    let backdrop = iced::widget::mouse_area(
        container(iced::widget::Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                ..Default::default()
            }),
    )
    .on_press(Message::CloseContextMenu);

    use ContextMenuAction::*;
    let id = menu.song_id;
    macro_rules! ic {
        ($s:expr) => {
            svg::Handle::from_memory($s.as_bytes())
        };
    }

    let mut items: Vec<Element<'_, Message>> = Vec::new();
    items.push(item(
        ic!(icons::PLAY),
        locale.get(Key::ContextMenuPlayNow),
        Style::Normal,
        msg(PlayNow, id),
        tokens,
    ));
    items.push(item(
        ic!(icons::SKIP_NEXT),
        locale.get(Key::ContextMenuPlayNext),
        Style::Normal,
        msg(PlayNext, id),
        tokens,
    ));
    items.push(div(tokens));
    let (heart_icon, heart_label) = if is_liked {
        (
            ic!(icons::HEART),
            locale.get(Key::ContextMenuRemoveFavorites),
        )
    } else {
        (
            ic!(icons::HEART_OUTLINE),
            locale.get(Key::ContextMenuAddFavorites),
        )
    };
    items.push(item(
        heart_icon,
        heart_label,
        Style::Normal,
        msg(AddToFavorites, id),
        tokens,
    ));
    items.push(item(
        ic!(icons::PLUS),
        locale.get(Key::ContextMenuAddPlaylist),
        Style::Normal,
        msg(AddToPlaylist, id),
        tokens,
    ));
    items.push(div(tokens));
    items.push(item(
        ic!(icons::USER),
        locale.get(Key::ContextMenuViewArtist),
        Style::Normal,
        msg(ViewArtist, id),
        tokens,
    ));
    items.push(item(
        ic!(icons::DISC),
        locale.get(Key::ContextMenuViewAlbum),
        Style::Normal,
        msg(ViewAlbum, id),
        tokens,
    ));
    items.push(div(tokens));
    if can_show_folder {
        items.push(item(
            ic!(icons::FOLDER),
            locale.get(Key::ContextMenuShowInFolder),
            Style::Normal,
            msg(ShowInFolder, id),
            tokens,
        ));
    }
    if can_download {
        items.push(item(
            ic!(icons::DOWNLOAD),
            locale.get(Key::ContextMenuDownload),
            Style::Normal,
            msg(Download, id),
            tokens,
        ));
    }
    if can_edit {
        items.push(item(
            ic!(icons::EDIT),
            locale.get(Key::ContextMenuEditTags),
            Style::Accent,
            msg(EditSongTags, id),
            tokens,
        ));
    }
    items.push(div(tokens));
    items.push(item(
        ic!(icons::TRASH),
        locale.get(Key::ContextMenuRemoveFromList),
        Style::Danger,
        msg(RemoveFromList, id),
        tokens,
    ));

    let panel = container(
        column(items)
            .spacing(0)
            .padding(tokens.space(PANEL_PADDING)),
    )
    .width(menu_width)
    .style(move |t| container::Style {
        background: Some(Background::Color(glass(t))),
        border: Border {
            radius: tokens.radius(RadiusRole::Medium).into(),
            width: tokens.size(1.0),
            color: glass_border(t),
        },
        ..Default::default()
    });

    let positioned = container(panel).padding(Padding::new(0.0).top(y).left(x));
    overlay::block_mouse_events(
        container(iced::widget::stack([backdrop.into(), positioned.into()]))
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

fn msg(a: ContextMenuAction, id: i64) -> Message {
    Message::ContextMenuAction(a, id)
}

enum Style {
    Normal,
    Accent,
    Danger,
}

impl Style {
    fn is_accent(&self) -> bool {
        matches!(self, Style::Accent)
    }
    fn is_danger(&self) -> bool {
        matches!(self, Style::Danger)
    }
}

fn item<'a>(
    icon: iced::widget::svg::Handle,
    label: &str,
    style: Style,
    msg: Message,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'a, Message> {
    let lb = label.to_string();
    let is_accent = style.is_accent();
    let is_danger = style.is_danger();
    let button = button(
        container(
            row![
                svg(icon)
                    .width(tokens.icon(IconRole::Small))
                    .height(tokens.icon(IconRole::Small))
                    .style(move |t, _| svg::Style {
                        color: Some(if is_accent {
                            purple()
                        } else if is_danger {
                            theme::ACCENT_PINK
                        } else {
                            theme::text_muted(t)
                        }),
                    }),
                iced::widget::Space::new().width(tokens.space(12.0)),
                text(lb)
                    .size(tokens.text(TextRole::Body))
                    .style(move |t| text::Style {
                        color: Some(if is_accent {
                            accent_text()
                        } else if is_danger {
                            theme::ACCENT_PINK
                        } else {
                            theme::text_primary(t)
                        }),
                    }),
            ]
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .padding(item_content_padding(tokens)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(tokens.target(TargetRole::Icon)))
    .padding(0)
    .style(|_theme, _status| button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(msg);

    widgets::hover_surface(button)
        .style(move |theme, progress| {
            let hover = if is_accent {
                purple_bg()
            } else if is_danger {
                red_bg()
            } else {
                theme::surface_hover(theme)
            };
            container::Style {
                background: Some(Background::Color(theme::lerp_color(
                    Color::TRANSPARENT,
                    hover,
                    progress,
                ))),
                border: Border {
                    radius: tokens.radius(RadiusRole::Small).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

fn purple() -> Color {
    Color::from_rgba(0.659, 0.333, 0.969, 1.0)
}
fn accent_text() -> Color {
    Color::from_rgba(0.847, 0.706, 0.996, 1.0)
}
fn purple_bg() -> Color {
    Color::from_rgba(0.659, 0.333, 0.969, 0.12)
}
fn red_bg() -> Color {
    Color::from_rgba(1.0, 0.08, 0.29, 0.10)
}

fn div<'a>(tokens: crate::ui::responsive::UiTokens) -> Element<'a, Message> {
    container(
        container(
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(tokens.space(1.0)),
        )
        .width(Length::Fill)
        .height(Length::Fixed(tokens.space(1.0)))
        .style(|t| container::Style {
            background: Some(Background::Color(glass_border(t))),
            ..Default::default()
        }),
    )
    .width(Length::Fill)
    .padding(divider_padding(tokens))
    .into()
}

fn item_content_padding(tokens: crate::ui::responsive::UiTokens) -> Padding {
    Padding::new(0.0)
        .left(tokens.space(ITEM_HORIZONTAL_PADDING))
        .right(tokens.space(ITEM_HORIZONTAL_PADDING))
}

fn divider_padding(tokens: crate::ui::responsive::UiTokens) -> Padding {
    Padding::new(tokens.space(DIVIDER_VERTICAL_PADDING))
        .left(0.0)
        .right(0.0)
}

// ── Theme-aware helpers ────────────────────────────

fn glass(t: &iced::Theme) -> Color {
    let s = theme::surface(t);
    Color::from_rgba(s.r, s.g, s.b, 0.92)
}

fn glass_border(t: &iced::Theme) -> Color {
    if theme::is_dark_theme(t) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.07)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.08)
    }
}

#[cfg(test)]
mod tests {
    use super::{divider_padding, item_content_padding};
    use crate::ui::responsive::UiTokens;

    #[test]
    fn options_own_horizontal_padding_while_dividers_keep_full_width() {
        let tokens = UiTokens::default();
        let item = item_content_padding(tokens);
        let divider = divider_padding(tokens);

        assert!(item.left > 0.0);
        assert_eq!(item.left, item.right);
        assert_eq!(divider.left, 0.0);
        assert_eq!(divider.right, 0.0);
        assert!(divider.top > 0.0);
        assert_eq!(divider.top, divider.bottom);
    }
}
