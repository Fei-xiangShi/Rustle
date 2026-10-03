//! Window control buttons and navigation bar
//! Positioned at top of the application with navigation on left, search in center, and controls on right

use iced::widget::{Space, button, container, mouse_area, opaque, row, stack, svg, text, tooltip};
use iced::{Alignment, Color, ContentFit, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message, UserInfo};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::components::{
    cover_image,
    search_bar::{self, SearchBarStyle},
    window_drag_region,
};
use crate::ui::responsive::{
    IconRole, LayoutProfile, ResponsiveContext, TargetRole, TextRole, top_bar_height,
};
use crate::ui::{theme, widgets};

const ACCOUNT_AVATAR_NAME_GAP: f32 = 5.0;
const ACCOUNT_NAME_BADGE_GAP: f32 = 6.0;

pub struct TopBarView<'a> {
    pub context: ResponsiveContext,
    pub locale: Locale,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub search_suggestions: &'a crate::app::SearchSuggestionsState,
    pub search_query: &'a str,
    pub is_logged_in: bool,
    pub user_info: Option<&'a UserInfo>,
    pub image_state: &'a ImageState,
    pub show_background: bool,
    pub is_maximized: bool,
}

/// Build the complete top bar with navigation buttons on left, search bar in center, user info and window controls on right
pub fn view<'a>(view: TopBarView<'a>) -> Element<'a, Message> {
    let TopBarView {
        context,
        locale,
        can_go_back,
        can_go_forward,
        search_query,
        search_suggestions,
        is_logged_in,
        user_info,
        image_state,
        show_background,
        is_maximized,
    } = view;
    let tokens = context.tokens;
    let button_size = tokens.target(TargetRole::WindowControl);
    let control_radius = button_size / 2.0;
    let icon_size = tokens.icon(IconRole::TopBarAction);
    let nav_icon_size = tokens.icon(IconRole::TopBarNavigation);
    let top_height = top_bar_height(&context);
    let compact_actions = context.profile.is_compact();
    let back_icon_opacity: f32 = if can_go_back { 1.0 } else { 0.5 };
    let forward_icon_opacity: f32 = if can_go_forward { 1.0 } else { 0.5 };

    // Navigation buttons (left side)
    let back_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(BACK_ICON.as_bytes()))
            .width(nav_icon_size)
            .height(nav_icon_size)
            .style(move |theme, _status| svg::Style {
                color: Some(if can_go_back {
                    theme::text_secondary(theme)
                } else {
                    theme::opaque_color(theme::disabled(theme))
                }),
            })
            .opacity(back_icon_opacity),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| nav_button_style(theme, status, can_go_back, control_radius))
    .on_press_maybe(if can_go_back {
        Some(Message::NavigateBack)
    } else {
        None
    });
    let back_btn = tooltip(
        animated_nav_button(back_button.into(), can_go_back, control_radius),
        text(locale.get(Key::Back)).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    let forward_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(FORWARD_ICON.as_bytes()))
            .width(nav_icon_size)
            .height(nav_icon_size)
            .style(move |theme, _status| svg::Style {
                color: Some(if can_go_forward {
                    theme::text_secondary(theme)
                } else {
                    theme::opaque_color(theme::disabled(theme))
                }),
            })
            .opacity(forward_icon_opacity),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| nav_button_style(theme, status, can_go_forward, control_radius))
    .on_press_maybe(if can_go_forward {
        Some(Message::NavigateForward)
    } else {
        None
    });
    let forward_btn = tooltip(
        animated_nav_button(forward_button.into(), can_go_forward, control_radius),
        text(locale.get(Key::Forward)).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    // Keep back and forward as separate circular controls.
    let nav_buttons = container(
        row![back_btn, forward_btn]
            .spacing(tokens.space(10.0))
            .align_y(Alignment::Center),
    )
    .padding(Padding::new(0.0).left(tokens.space(20.0)));

    // User info (avatar + username + API-backed membership badge)
    let avatar_size = tokens.size(28.0);
    let avatar_handle = is_logged_in
        .then(|| user_info.and_then(|info| image_state.get(ImageKind::UserAvatar, info.user_id)))
        .flatten();
    let avatar_elem =
        cover_image::circle(avatar_handle, ImageKind::UserAvatar, avatar_size, tokens);

    let user_text: Element<'_, Message> = if is_logged_in {
        if let Some(info) = user_info {
            if let Some(icon_url) = info.vip.badge_url()
                && let Some(handle) = image_state.get(
                    ImageKind::VipBadge,
                    crate::image::vip_badge_key(info.user_id, info.vip.tier(), icon_url),
                )
            {
                let vip_badge = widgets::crossfade_image(Some(handle.clone()))
                    // Match the adjacent nickname. Width remains Shrink,
                    // so Iced derives it from the official image aspect ratio.
                    .height(tokens.text(TextRole::Subtitle))
                    .content_fit(ContentFit::Contain);
                row![
                    text(info.nickname.clone())
                        .size(tokens.text(TextRole::Subtitle))
                        .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT))
                        .style(|theme| text::Style {
                            color: Some(theme::text_primary(theme))
                        }),
                    Space::new().width(tokens.space(ACCOUNT_NAME_BADGE_GAP)),
                    vip_badge,
                ]
                .align_y(Alignment::Center)
                .into()
            } else {
                text(info.nickname.clone())
                    .size(tokens.text(TextRole::Subtitle))
                    .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT))
                    .style(|theme| text::Style {
                        color: Some(theme::text_primary(theme)),
                    })
                    .into()
            }
        } else {
            text("未登录")
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .into()
        }
    } else {
        text("未登录")
            .size(tokens.text(TextRole::BodyLarge))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .into()
    };

    let user_content: Element<'_, Message> = if compact_actions {
        // The avatar-only presentation keeps a complete square hit target.
        container(avatar_elem).center(button_size).into()
    } else {
        // The full account row is one interactive surface, so the avatar only
        // needs its visual width. Avoid hidden horizontal slack from centering
        // 28px artwork inside a 42px square before applying the visible gap.
        let avatar = container(avatar_elem)
            .width(avatar_size)
            .height(button_size)
            .center_x(avatar_size)
            .center_y(button_size);
        row![
            avatar,
            Space::new().width(tokens.space(ACCOUNT_AVATAR_NAME_GAP)),
            user_text
        ]
        .height(button_size)
        .align_y(Alignment::Center)
        .into()
    };
    let account_action = if is_logged_in {
        user_info.map_or(Message::OpenSettings, |info| {
            Message::OpenUser(info.user_id)
        })
    } else {
        Message::ToggleLoginPopup
    };
    let user_info_surface = mouse_area(user_content)
        .on_press(account_action)
        .interaction(iced::mouse::Interaction::Pointer);

    let user_info_widget: Element<'a, Message> = if compact_actions {
        tooltip(
            user_info_surface,
            text(if is_logged_in {
                user_info
                    .map(|info| info.nickname.clone())
                    .unwrap_or_else(|| locale.get(Key::NotLoggedIn).to_string())
            } else {
                locale.get(Key::NotLoggedIn).to_string()
            })
            .size(tokens.text(TextRole::Caption)),
            tooltip::Position::Bottom,
        )
        .padding(tokens.space(5.0))
        .into()
    } else {
        user_info_surface.into()
    };

    // Window control buttons (right side)
    let settings_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(
            crate::ui::icons::SETTINGS.as_bytes(),
        ))
        .width(icon_size)
        .height(icon_size)
        .style(|theme, _status| svg::Style {
            color: Some(theme::text_secondary(theme)),
        }),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| window_button_style(theme, status, control_radius))
    .on_press(Message::OpenSettings);
    let settings_btn = tooltip(
        animated_window_button(settings_button.into(), control_radius),
        text(locale.get(Key::Settings)).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    let minimize_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(MINIMIZE_ICON.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_secondary(theme)),
            }),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| window_button_style(theme, status, control_radius))
    .on_press(Message::WindowMinimize);
    let minimize_btn = tooltip(
        animated_window_button(minimize_button.into(), control_radius),
        text(locale.get(Key::Minimize)).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    let maximize_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(
            crate::ui::icons::maximize_restore(is_maximized).as_bytes(),
        ))
        .width(icon_size)
        .height(icon_size)
        .style(|theme, _status| svg::Style {
            color: Some(theme::text_secondary(theme)),
        }),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| window_button_style(theme, status, control_radius))
    .on_press(Message::WindowMaximize);
    let maximize_btn = tooltip(
        animated_window_button(maximize_button.into(), control_radius),
        text(locale.get(if is_maximized {
            Key::Restore
        } else {
            Key::Maximize
        }))
        .size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    let close_button = button(widgets::centered_button_content(
        svg(svg::Handle::from_memory(CLOSE_ICON.as_bytes()))
            .width(icon_size)
            .height(icon_size)
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_secondary(theme)),
            }),
        button_size,
    ))
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, status| close_button_style(theme, status, control_radius))
    .on_press(Message::RequestClose);
    let close_btn = tooltip(
        animated_close_button(close_button.into(), control_radius),
        text(locale.get(Key::Close)).size(tokens.text(TextRole::Caption)),
        tooltip::Position::Bottom,
    )
    .padding(tokens.space(5.0));

    // Search bar (left, after nav buttons)
    let desktop_search_bar = search_bar::view(
        search_query,
        locale,
        SearchBarStyle::top_bar(&context, 0.0),
        search_suggestions,
        context,
    );

    let menu_button: Element<'a, Message> = if context.profile == LayoutProfile::Narrow {
        let button = button(widgets::centered_button_content(
            svg(svg::Handle::from_memory(crate::ui::icons::LIST.as_bytes()))
                .width(icon_size)
                .height(icon_size)
                .style(|theme, _status| svg::Style {
                    color: Some(theme::text_secondary(theme)),
                }),
            button_size,
        ))
        .width(button_size)
        .height(button_size)
        .padding(0)
        .style(move |theme, status| window_button_style(theme, status, control_radius))
        .on_press(Message::ToggleSidebarDrawer);
        tooltip(
            animated_window_button(button.into(), control_radius),
            text(locale.get(Key::NavigationMenu)).size(tokens.text(TextRole::Caption)),
            tooltip::Position::Bottom,
        )
        .padding(tokens.space(5.0))
        .into()
    } else {
        Space::new().width(0).height(0).into()
    };
    // Navigation, search, account, settings, and native window controls are a
    // single chrome group at every profile. Search is the only flexible lane;
    // compact profiles already reduce account identity to the avatar before
    // any functional control needs to move or disappear.
    let flexible_gap = if context.profile.is_compact() {
        tokens.space(6.0)
    } else {
        tokens.space(14.0)
    };
    let action_gap = if context.profile.is_compact() {
        tokens.space(5.0)
    } else {
        tokens.space(8.0)
    };
    let controls: Element<'a, Message> = container(
        row![
            menu_button,
            nav_buttons,
            Space::new().width(flexible_gap),
            desktop_search_bar,
            Space::new().width(flexible_gap),
            user_info_widget,
            Space::new().width(action_gap),
            settings_btn,
            Space::new().width(action_gap),
            minimize_btn,
            Space::new().width(action_gap),
            maximize_btn,
            Space::new().width(action_gap),
            close_btn,
        ]
        .align_y(Alignment::Center)
        .width(Fill),
    )
    .width(Fill)
    .height(Length::Fixed(top_height))
    .align_y(Alignment::Center)
    .padding(Padding::new(0.0).right(tokens.space(16.0)))
    .into();

    // Native Iced scrollbars do not support a top-only track inset. Cover the
    // scrollbar gutter inside the window chrome so the scroll thumb starts
    // visually below the top bar while page content can still pass underneath.
    let scrollbar_gutter: Element<'a, Message> = if show_background {
        container(
            container(Space::new())
                .width(tokens.size(theme::TOP_BAR_SCROLLBAR_GUTTER_WIDTH))
                .height(Fill)
                .style(|active_theme| container::Style {
                    background: Some(iced::Background::Color(theme::background(active_theme))),
                    ..Default::default()
                }),
        )
        .width(Fill)
        .height(Length::Fixed(top_height))
        .align_x(Alignment::End)
        .into()
    } else {
        Space::new().into()
    };

    // Complete top bar layout: nav + search on left, user info + window controls on right.
    // The bottom layer provides dragging only in empty space; controls stay interactive above it.
    opaque(
        container(
            stack![
                window_drag_region::view(context, Message::WindowDrag),
                controls,
                scrollbar_gutter
            ]
            .width(Fill)
            .height(Length::Fixed(top_height)),
        )
        .width(Fill)
        .height(Length::Fixed(top_height))
        .style(move |active_theme| container::Style {
            background: show_background
                .then(|| iced::Background::Color(theme::top_bar_background(active_theme))),
            ..Default::default()
        }),
    )
}

/// Navigation button style (back/forward)
fn nav_button_style(
    theme: &iced::Theme,
    status: button::Status,
    enabled: bool,
    radius: f32,
) -> button::Style {
    let base = button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: if enabled {
            theme::text_secondary(theme)
        } else {
            theme::disabled(theme)
        },
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        shadow: iced::Shadow::default(),
        snap: true,
    };

    if !enabled {
        return base;
    }

    match status {
        button::Status::Hovered => button::Style {
            text_color: theme::text_primary(theme),
            ..base
        },
        _ => base,
    }
}

/// Window button style (settings, minimize, maximize)
fn window_button_style(theme: &iced::Theme, status: button::Status, radius: f32) -> button::Style {
    let base = button::Style {
        background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
        text_color: theme::text_secondary(theme),
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        shadow: iced::Shadow::default(),
        snap: true,
    };

    match status {
        button::Status::Hovered => button::Style {
            text_color: theme::text_primary(theme),
            ..base
        },
        _ => base,
    }
}

/// Close button style (red on hover)
fn close_button_style(theme: &iced::Theme, status: button::Status, radius: f32) -> button::Style {
    let base = button::Style {
        background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
        text_color: theme::text_secondary(theme),
        border: iced::Border {
            radius: radius.into(),
            ..Default::default()
        },
        shadow: iced::Shadow::default(),
        snap: true,
    };

    match status {
        button::Status::Hovered => button::Style {
            text_color: theme::text_primary(theme),
            ..base
        },
        _ => base,
    }
}

fn animated_nav_button<'a>(
    content: Element<'a, Message>,
    enabled: bool,
    radius: f32,
) -> Element<'a, Message> {
    widgets::hover_surface(content)
        .enabled(enabled)
        .style(move |theme, progress| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.08 + 0.06 * progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn animated_window_button<'a>(content: Element<'a, Message>, radius: f32) -> Element<'a, Message> {
    widgets::hover_surface(content)
        .style(move |theme, progress| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                0.14 * progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn animated_close_button<'a>(content: Element<'a, Message>, radius: f32) -> Element<'a, Message> {
    widgets::hover_surface(content)
        .style(move |theme, progress| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                Color::TRANSPARENT,
                theme::close_button_hover(theme),
                progress,
            ))),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

// Navigation icons - clean chevron style
const BACK_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
    <polyline points="15 18 9 12 15 6"/>
</svg>"#;

const FORWARD_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
    <polyline points="9 18 15 12 9 6"/>
</svg>"#;

// Window control icons
const MINIMIZE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
    <line x1="5" y1="12" x2="19" y2="12"/>
</svg>"#;

const CLOSE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <line x1="6" y1="6" x2="18" y2="18"/>
    <line x1="6" y1="18" x2="18" y2="6"/>
</svg>"#;

#[cfg(test)]
mod tests {
    use super::{ACCOUNT_AVATAR_NAME_GAP, ACCOUNT_NAME_BADGE_GAP};

    #[test]
    fn avatar_name_gap_is_tighter_than_name_badge_gap() {
        const { assert!(ACCOUNT_AVATAR_NAME_GAP < ACCOUNT_NAME_BADGE_GAP) };
    }
}
