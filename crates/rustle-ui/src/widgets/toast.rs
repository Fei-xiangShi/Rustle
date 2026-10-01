//! Toast notification widget
//!
//! Modern dark minimalist toast notifications.
//! Follows Shadcn UI / Spotify style: dark surface with accent color accents.

use iced::widget::{Space, container, row, svg, text};
use iced::{Alignment, Element, Padding};

use crate::responsive::{RadiusRole, TextRole, UiTokens};
use crate::{icons, theme};

/// Toast notification style
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastStyle {
    Success,
    Error,
    Warning,
    Info,
}

impl ToastStyle {
    /// Get the accent color for this style (used for icon/indicator only)
    pub fn accent_color(&self, theme: &iced::Theme) -> iced::Color {
        match self {
            ToastStyle::Success => theme::success(theme),
            ToastStyle::Error => theme::danger(theme),
            ToastStyle::Warning => theme::warning(theme),
            ToastStyle::Info => theme::info(theme),
        }
    }

    /// Get the icon SVG for this style
    pub fn icon_svg(&self) -> &'static str {
        match self {
            ToastStyle::Success => icons::CHECK,
            ToastStyle::Error => icons::ERROR,
            ToastStyle::Warning => icons::WARNING,
            ToastStyle::Info => icons::INFO,
        }
    }
}

/// Toast notification data
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub style: ToastStyle,
}

impl Toast {
    pub fn new(message: impl Into<String>, style: ToastStyle) -> Self {
        Self {
            message: message.into(),
            style,
        }
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastStyle::Success)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, ToastStyle::Error)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, ToastStyle::Warning)
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, ToastStyle::Info)
    }
}

/// Build a toast notification widget
///
/// Modern dark minimalist style:
/// - Dark gray background (blends with dark mode)
/// - Subtle border for depth
/// - Accent color only on icon (not background)
/// - Soft shadow for floating effect
pub fn view_toast<'a, Message: 'a>(toast: &Toast, tokens: UiTokens) -> Element<'a, Message> {
    let style = toast.style;
    let icon = toast.style.icon_svg();
    let message = toast.message.clone();

    // Left accent bar (thin vertical line)
    let accent_bar = container(
        Space::new()
            .width(tokens.size(3.0))
            .height(tokens.size(20.0)),
    )
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(style.accent_color(_theme))),
        border: iced::Border {
            radius: tokens.size(2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    });

    // Icon with accent color
    let icon_widget = svg(svg::Handle::from_memory(icon.as_bytes()))
        .width(tokens.size(14.0))
        .height(tokens.size(14.0))
        .style(move |_theme, _status| svg::Style {
            color: Some(style.accent_color(_theme)),
        });

    // Message text
    let message_widget = text(message)
        .size(tokens.text(TextRole::Label))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    // Toast content
    let content = row![
        accent_bar,
        Space::new().width(tokens.space(12.0)),
        icon_widget,
        Space::new().width(tokens.space(10.0)),
        message_widget,
    ]
    .align_y(Alignment::Center)
    .padding(
        Padding::new(tokens.space(14.0))
            .left(tokens.space(12.0))
            .right(tokens.space(20.0)),
    );

    // Toast container with dark surface style
    container(content)
        .style(move |theme| iced::widget::container::Style {
            // Surface elevated background
            background: Some(iced::Background::Color(theme::surface_elevated(theme))),
            // Subtle border for depth
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                width: tokens.size(1.0),
                color: theme::border_color(theme),
            },
            // Soft shadow for floating effect
            shadow: iced::Shadow {
                color: theme::shadow_color(theme),
                offset: iced::Vector::new(0.0, tokens.size(4.0)),
                blur_radius: tokens.size(12.0),
            },
            ..Default::default()
        })
        .into()
}
