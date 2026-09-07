//! Login popup component with QR code display
//!
//! Displays QR code for NCM login with status messages.

use iced::mouse::Interaction;
use iced::widget::{Space, button, column, container, mouse_area, row, svg, text};
use iced::{Alignment, Color, Element, Fill, Padding};
use std::path::PathBuf;

use crate::app::Message;
use crate::app::UserInfo;
use crate::i18n::{Key, Locale};
use crate::ui::responsive::{IconRole, ResponsiveContext, TextRole, bounded_panel_size};
use crate::ui::theme::BOLD_WEIGHT;
use crate::ui::{overlay, theme, widgets};

// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
const POPUP_WIDTH: f32 = 320.0;
const POPUP_HEIGHT: f32 = 400.0;
const QR_SIZE: f32 = 200.0;

/// Build the login popup view
pub fn view<'a>(
    context: ResponsiveContext,
    is_open: bool,
    qr_code_path: Option<&'a PathBuf>,
    qr_status: Option<&'a str>,
    user_info: Option<&'a UserInfo>,
    is_logged_in: bool,
    locale: Locale,
) -> Element<'a, Message> {
    if !is_open {
        return Space::new().width(0).height(0).into();
    }

    let tokens = context.tokens;
    let popup_size = bounded_panel_size(
        iced::Size::new(tokens.size(POPUP_WIDTH), tokens.size(POPUP_HEIGHT)),
        context.viewport,
        tokens.space(16.0),
        tokens.space(16.0),
        0.0,
    );

    let content: Element<'_, Message> = if is_logged_in {
        // Show user info and logout button
        if let Some(user) = user_info {
            view_logged_in(user, locale, tokens)
        } else {
            view_qr_login(qr_code_path, qr_status, locale, tokens)
        }
    } else {
        view_qr_login(qr_code_path, qr_status, locale, tokens)
    };

    // Popup container with mouse_area to prevent click-through to backdrop
    let popup = mouse_area(
        container(content)
            .width(popup_size.width)
            .height(popup_size.height)
            .padding(tokens.space(24.0))
            .style(move |theme| theme::login_popup(theme, tokens.theme_metrics())),
    )
    .interaction(Interaction::Idle);

    // Backdrop - clicking outside popup closes it
    let backdrop = mouse_area(
        container(Space::new().width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .style(|theme| container::Style {
                background: Some(theme::overlay_backdrop(theme, 0.5).into()),
                ..Default::default()
            }),
    )
    .interaction(Interaction::Idle)
    .on_press(Message::ToggleLoginPopup);

    // Stack popup on backdrop and block all pointer events from reaching layers below
    overlay::block_mouse_events(
        iced::widget::stack![
            backdrop,
            container(popup)
                .width(Fill)
                .height(Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        ]
        .width(Fill)
        .height(Fill)
        .into(),
    )
}

/// View for QR code login
fn view_qr_login<'a>(
    qr_code_path: Option<&'a PathBuf>,
    qr_status: Option<&'a str>,
    locale: Locale,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'a, Message> {
    let title = text(locale.get(Key::LoginScanQr).to_string())
        .size(tokens.text(TextRole::Title))
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    let qr_display: Element<'_, Message> = if let Some(path) = qr_code_path {
        container(
            widgets::crossfade_image(Some(iced::widget::image::Handle::from_path(path.clone())))
                .width(tokens.size(QR_SIZE))
                .height(tokens.size(QR_SIZE)),
        )
        .style(move |_theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: iced::Border {
                radius: tokens.size(8.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .padding(tokens.space(8.0))
        .into()
    } else {
        container(
            text(locale.get(Key::LoginGeneratingQr).to_string())
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::text_secondary(theme)),
                }),
        )
        .width(tokens.size(QR_SIZE))
        .height(tokens.size(QR_SIZE))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(theme::SURFACE_SECONDARY.into()),
            border: iced::Border {
                radius: tokens.size(8.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };

    let status_text = text(qr_status.unwrap_or("请使用网易云音乐App扫码"))
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_secondary(theme)),
        });

    let refresh_button = button(
        row![
            svg(svg::Handle::from_memory(
                crate::ui::icons::REFRESH.as_bytes()
            ))
            .width(tokens.icon(IconRole::Small))
            .height(tokens.icon(IconRole::Small))
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_primary(theme)),
            }),
            Space::new().width(tokens.space(8.0)),
            text(locale.get(Key::LoginRefreshQr).to_string())
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::text_primary(theme))
                }),
        ]
        .align_y(Alignment::Center),
    )
    .padding(
        Padding::new(tokens.space(8.0))
            .left(tokens.space(16.0))
            .right(tokens.space(16.0)),
    )
    .style(move |theme, status| theme::secondary_button(theme, status, tokens.theme_metrics()))
    .on_press(Message::RequestQrCode);

    column![
        title,
        Space::new().height(tokens.space(24.0)),
        qr_display,
        Space::new().height(tokens.space(16.0)),
        status_text,
        Space::new().height(tokens.space(16.0)),
        refresh_button,
    ]
    .spacing(0)
    .align_x(Alignment::Center)
    .width(Fill)
    .into()
}

/// View for logged in user
fn view_logged_in(
    user: &UserInfo,
    locale: Locale,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'_, Message> {
    let title = text(locale.get(Key::LoginLoggedIn).to_string())
        .size(tokens.text(TextRole::Title))
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    let username = text(&user.nickname)
        .size(tokens.text(TextRole::Subtitle))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    let uid_text = text(format!("UID: {}", user.user_id))
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_secondary(theme)),
        });

    let logout_button = button(
        row![
            svg(svg::Handle::from_memory(
                crate::ui::icons::LOGOUT.as_bytes()
            ))
            .width(tokens.icon(IconRole::Small))
            .height(tokens.icon(IconRole::Small))
            .style(|_theme, _status| svg::Style {
                color: Some(Color::WHITE),
            }),
            Space::new().width(tokens.space(8.0)),
            text(locale.get(Key::LoginLogout).to_string())
                .size(tokens.text(TextRole::Body))
                .color(Color::WHITE),
        ]
        .align_y(Alignment::Center),
    )
    .padding(
        Padding::new(tokens.space(12.0))
            .left(tokens.space(20.0))
            .right(tokens.space(20.0)),
    )
    .style(move |theme, status| theme::danger_button(theme, status, tokens.theme_metrics()))
    .on_press(Message::Logout);

    column![
        title,
        Space::new().height(tokens.space(32.0)),
        username,
        Space::new().height(tokens.space(8.0)),
        uid_text,
        Space::new().height(tokens.space(32.0)),
        logout_button,
    ]
    .spacing(0)
    .align_x(Alignment::Center)
    .width(Fill)
    .into()
}
