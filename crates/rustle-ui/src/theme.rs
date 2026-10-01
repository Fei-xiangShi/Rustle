//! Theme system for the music streaming application
//! Supports both dark and light modes with consistent color palette

use iced::font::Weight;
use iced::widget::{button, container, pick_list, scrollable};
use iced::{Background, Border, Color, Shadow, Theme, Vector};
pub mod decoration;
pub mod palette;
pub use palette::{application, colors};

/// Bold UI weight tuned for the native platform's default sans-serif family.
#[cfg(target_os = "macos")]
pub const BOLD_WEIGHT: Weight = Weight::Semibold;

#[cfg(not(target_os = "macos"))]
pub const BOLD_WEIGHT: Weight = Weight::Bold;

/// Medium UI weight tuned for the native platform's default sans-serif family.
#[cfg(target_os = "macos")]
pub const MEDIUM_WEIGHT: Weight = Weight::Medium;

#[cfg(not(target_os = "macos"))]
pub const MEDIUM_WEIGHT: Weight = Weight::Normal;

// ============================================================================
// Typography
// ============================================================================

/// 1080P reference sizes; production callers resolve these through
/// `UiTokens::text` instead of passing them directly to iced.
pub const TEXT_SIZE_MICRO: f32 = 10.0;
pub const TEXT_SIZE_CAPTION: f32 = 12.0;
pub const TEXT_SIZE_LABEL: f32 = 13.0;
pub const TEXT_SIZE_BODY: f32 = 14.0;
pub const TEXT_SIZE_BODY_LARGE: f32 = 16.0;
pub const TEXT_SIZE_SUBTITLE: f32 = 18.0;
pub const TEXT_SIZE_TITLE: f32 = 24.0;
pub const TEXT_SIZE_TITLE_LARGE: f32 = 28.0;
pub const TEXT_SIZE_HERO: f32 = 32.0;
pub const TEXT_SIZE_DISPLAY: f32 = 48.0;

// ============================================================================
// Layout
// ============================================================================

/// 1080P reference height resolved by the shared chrome token.
pub const TOP_BAR_HEIGHT: f32 = 68.0;
pub const TOP_BAR_BACKGROUND_ALPHA: f32 = 0.78;
/// 1080P reference width of Iced's default vertical scrollbar at the right edge.
pub const TOP_BAR_SCROLLBAR_GUTTER_WIDTH: f32 = 10.0;

/// Root-rem-resolved dimensions shared by reusable theme style callbacks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeMetrics {
    pub small_radius: f32,
    pub medium_radius: f32,
    pub large_radius: f32,
    pub pill_radius: f32,
    pub border_width: f32,
    pub popup_shadow_offset_y: f32,
    pub popup_shadow_blur: f32,
}

// ============================================================================
// Color Palette - Dynamic based on theme
// ============================================================================

/// Check if theme is dark mode
fn is_dark(theme: &Theme) -> bool {
    theme.palette().is_dark
}

/// Public function to check if theme is dark mode
pub fn is_dark_theme(theme: &Theme) -> bool {
    is_dark(theme)
}

/// Get background color based on theme
pub fn background(theme: &Theme) -> Color {
    colors(theme).canvas
}

/// Translucent background used by the fixed top-bar overlay.
pub fn top_bar_background(theme: &Theme) -> Color {
    let mut color = background(theme);
    color.a = TOP_BAR_BACKGROUND_ALPHA;
    color
}

/// Get sidebar color based on theme
pub fn sidebar_bg(theme: &Theme) -> Color {
    colors(theme).sidebar
}

/// Get surface color based on theme
pub fn surface(theme: &Theme) -> Color {
    colors(theme).surface
}

/// Get border color based on theme
pub fn border_color(theme: &Theme) -> Color {
    colors(theme).border
}

/// Get muted text color based on theme
pub fn text_muted(theme: &Theme) -> Color {
    colors(theme).muted
}

/// Get secondary text color based on theme
pub fn text_secondary(theme: &Theme) -> Color {
    colors(theme).secondary
}

/// Get primary text color based on theme
pub fn text_primary(theme: &Theme) -> Color {
    colors(theme).text
}

/// Return the RGB portion of a color without carrying its alpha channel.
///
/// SVG color filters do not consistently apply the alpha component as
/// transparency across renderers. Callers that need translucent SVGs should
/// use this for the tint and the SVG widget's `opacity` field for alpha.
#[inline]
pub fn opaque_color(color: Color) -> Color {
    crate::color::with_alpha(color, 1.0)
}

/// Theme-resolved cyan-teal brand and paired foreground.
pub fn accent(theme: &Theme) -> Color {
    colors(theme).accent
}
pub fn accent_hover(theme: &Theme) -> Color {
    colors(theme).accent_hover
}
pub fn on_accent(theme: &Theme) -> Color {
    colors(theme).on_accent
}
pub fn accent_subtle(theme: &Theme) -> Color {
    colors(theme).accent_subtle
}
pub fn disabled(theme: &Theme) -> Color {
    text_muted(theme).scale_alpha(0.5)
}
/// Neutral OKLCH endpoint L=1, for artwork foregrounds and white overlays.
pub fn white(alpha: f32) -> Color {
    crate::color::with_alpha(Color::WHITE, alpha)
}
/// Neutral OKLCH endpoint L=0, for scrims and shadows.
pub fn black(alpha: f32) -> Color {
    crate::color::with_alpha(Color::BLACK, alpha)
}

/// Dynamic surface hover color based on theme
pub fn surface_hover(theme: &Theme) -> Color {
    colors(theme).hover
}

// ============================================================================
// Container Styles
// ============================================================================

/// Main content area background
pub fn main_content(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(background(theme))),
        text_color: Some(text_primary(theme)),
        ..Default::default()
    }
}

/// Sidebar background
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(sidebar_bg(theme))),
        text_color: Some(text_primary(theme)),
        ..Default::default()
    }
}

/// Login popup container
pub fn login_popup(theme: &Theme, metrics: ThemeMetrics) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface(theme))),
        text_color: Some(text_primary(theme)),
        border: Border {
            radius: metrics.large_radius.into(),
            width: metrics.border_width,
            color: border_color(theme),
        },
        shadow: Shadow {
            color: black(0.5),
            offset: Vector::new(0.0, metrics.popup_shadow_offset_y),
            blur_radius: metrics.popup_shadow_blur,
        },
        ..Default::default()
    }
}

// ============================================================================
// Button Styles
// ============================================================================

/// Primary button style
pub fn primary_button(
    theme: &Theme,
    status: button::Status,
    metrics: ThemeMetrics,
) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(accent(theme))),
        text_color: on_accent(theme),
        border: Border {
            radius: metrics.pill_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(accent_hover(theme))),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(colors(theme).accent_pressed)),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(accent_subtle(theme))),
            text_color: disabled(theme),
            ..base
        },
        _ => base,
    }
}

/// Secondary button - transparent with border
pub fn secondary_button(
    theme: &Theme,
    status: button::Status,
    metrics: ThemeMetrics,
) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_primary(theme),
        border: Border {
            radius: metrics.pill_radius.into(),
            width: metrics.border_width,
            color: border_color(theme),
        },
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(surface(theme))),
            border: Border {
                color: text_muted(theme),
                ..base.border
            },
            ..base
        },
        _ => base,
    }
}

/// Text button (no background, just text color change on hover)
pub fn text_button(theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_secondary(theme),
        border: Border::default(),
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            text_color: text_primary(theme),
            ..base
        },
        _ => base,
    }
}

/// Danger button (red for destructive actions)
pub fn danger_button(
    theme: &Theme,
    status: button::Status,
    metrics: ThemeMetrics,
) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(danger(theme))),
        text_color: palette::on_color(danger(theme)),
        border: Border {
            radius: metrics.pill_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(danger_hover(theme))),
            ..base
        },
        _ => base,
    }
}

/// Hover background color based on theme
pub fn hover_bg(theme: &Theme) -> Color {
    if is_dark(theme) {
        white(0.12)
    } else {
        black(0.08)
    }
}

/// Play button hover color - slightly lighter/darker than text_primary
pub fn play_button_hover(theme: &Theme) -> Color {
    lerp_color(text_primary(theme), text_secondary(theme), 0.25)
}

/// Surface elevated color (for cards, popups)
pub fn surface_elevated(theme: &Theme) -> Color {
    colors(theme).raised
}

/// Surface container color (for input fields, panels)
pub fn surface_container(theme: &Theme) -> Color {
    colors(theme).surface
}

/// Danger/error color
pub fn danger(theme: &Theme) -> Color {
    colors(theme).danger
}

/// Danger hover color
pub fn danger_hover(theme: &Theme) -> Color {
    colors(theme).danger_hover
}

/// Success color
pub fn success(theme: &Theme) -> Color {
    colors(theme).success
}

/// Warning color
pub fn warning(theme: &Theme) -> Color {
    colors(theme).warning
}

/// Info color
pub fn info(theme: &Theme) -> Color {
    colors(theme).info
}

/// Divider/separator color
pub fn divider(theme: &Theme) -> Color {
    if is_dark(theme) {
        white(0.1)
    } else {
        black(0.1)
    }
}

/// Overlay backdrop color
pub fn overlay_backdrop(theme: &Theme, opacity: f32) -> Color {
    if is_dark(theme) {
        black(opacity)
    } else {
        black(opacity * 0.7)
    }
}

/// Navigation menu item - inactive
pub fn nav_item(theme: &Theme, status: button::Status, metrics: ThemeMetrics) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_muted(theme),
        border: Border {
            radius: metrics.medium_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(hover_bg(theme))),
            text_color: text_primary(theme),
            ..base
        },
        _ => base,
    }
}

/// Transparent button - no background, no hover effect (for icon buttons with custom hover)
pub fn transparent_btn(theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_primary(theme),
        border: Border::default(),
        ..Default::default()
    }
}

/// Danger button (for destructive actions)
pub fn button_danger(
    theme: &Theme,
    status: button::Status,
    metrics: ThemeMetrics,
) -> button::Style {
    let base = match status {
        button::Status::Hovered => danger_hover(theme),
        _ => danger(theme),
    };

    button::Style {
        background: Some(Background::Color(base)),
        text_color: palette::on_color(danger(theme)),
        border: Border {
            radius: metrics.small_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ============================================================================
// Text Input Styles
// ============================================================================

// ============================================================================
// Scrollable Styles
// ============================================================================

// ============================================================================
// Pick List (Dropdown) Styles
// ============================================================================

/// Unified dropdown style - semi-transparent background with rounded corners
pub fn settings_pick_list(
    theme: &Theme,
    status: pick_list::Status,
    metrics: ThemeMetrics,
) -> pick_list::Style {
    let bg = if is_dark(theme) {
        match status {
            pick_list::Status::Active => white(0.08),
            pick_list::Status::Hovered => white(0.12),
            pick_list::Status::Opened { .. } => white(0.15),
            pick_list::Status::Disabled => white(0.04),
        }
    } else {
        match status {
            pick_list::Status::Active => black(0.05),
            pick_list::Status::Hovered => black(0.08),
            pick_list::Status::Opened { .. } => black(0.1),
            pick_list::Status::Disabled => black(0.03),
        }
    };

    let border_color = if is_dark(theme) {
        white(0.1)
    } else {
        black(0.15)
    };

    pick_list::Style {
        text_color: text_primary(theme),
        placeholder_color: text_muted(theme),
        handle_color: text_secondary(theme),
        background: Background::Color(bg),
        border: Border {
            radius: metrics.medium_radius.into(),
            width: metrics.border_width,
            color: border_color,
        },
    }
}

/// Unified dropdown menu style - dark background with rounded corners
pub fn settings_pick_list_menu(theme: &Theme, metrics: ThemeMetrics) -> iced::overlay::menu::Style {
    let (bg, selected_bg, border_color) = if is_dark(theme) {
        (surface(theme), white(0.1), white(0.1))
    } else {
        (surface(theme), black(0.08), black(0.1))
    };

    iced::overlay::menu::Style {
        text_color: text_primary(theme),
        background: Background::Color(bg),
        border: Border {
            radius: metrics.medium_radius.into(),
            width: metrics.border_width,
            color: border_color,
        },
        selected_text_color: text_primary(theme),
        selected_background: Background::Color(selected_bg),
        shadow: Shadow::default(),
    }
}

// ============================================================================
// Scrollable Styles
// ============================================================================

/// Scrollbar style for main content
pub fn dark_scrollable(
    theme: &Theme,
    _status: scrollable::Status,
    metrics: ThemeMetrics,
) -> scrollable::Style {
    let scrollbar = scrollable::Rail {
        background: Some(Background::Color(Color::TRANSPARENT)),
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(border_color(theme)),
            border: Border {
                radius: metrics.small_radius.into(),
                ..Default::default()
            },
        },
    };

    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: scrollbar,
        horizontal_rail: scrollbar,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(surface(theme)),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: text_muted(theme),
        },
    }
}

// ============================================================================
// Theme-aware color helpers for components
// ============================================================================

/// Panel background (queue panel, popups)
pub fn panel_bg(theme: &Theme) -> Color {
    colors(theme).raised
}

/// Panel border color
pub fn panel_border(theme: &Theme) -> Color {
    colors(theme).border
}

/// Shadow color for panels
pub fn shadow_color(theme: &Theme) -> Color {
    if is_dark(theme) {
        black(0.5)
    } else {
        black(0.15)
    }
}

/// Player bar background
pub fn player_bar_bg(theme: &Theme) -> Color {
    sidebar_bg(theme)
}

/// Top border color used by the player bar and matching sidebar footer separator
pub fn player_bar_border(theme: &Theme) -> Color {
    colors(theme).border
}

/// Header text color (slightly dimmed)
pub fn header_text(theme: &Theme) -> Color {
    colors(theme).secondary
}

/// Dimmed text color (for indices, durations)
pub fn dimmed_text(theme: &Theme) -> Color {
    colors(theme).muted
}

/// Icon color (muted)
pub fn icon_muted(theme: &Theme) -> Color {
    colors(theme).muted
}

/// Hover background with alpha
pub fn hover_bg_alpha(theme: &Theme, alpha: f32) -> Color {
    if is_dark(theme) {
        white(alpha)
    } else {
        black(alpha * 0.7)
    }
}

/// Interpolate between two colors for animated UI state transitions.
pub fn lerp_color(from: Color, to: Color, progress: f32) -> Color {
    crate::color::mix(from, to, progress)
}

/// Animated text color based on progress (for hover animations)
pub fn animated_text(theme: &Theme, progress: f32) -> Color {
    lerp_color(text_secondary(theme), text_primary(theme), progress)
}

/// Animated brightness for sidebar items
pub fn animated_brightness(theme: &Theme, progress: f32) -> Color {
    lerp_color(text_secondary(theme), text_primary(theme), progress)
}

/// Close button hover (muted red adapted to the active theme)
pub fn close_button_hover(theme: &Theme) -> Color {
    crate::color::with_alpha(lerp_color(surface(theme), danger(theme), 0.22), 0.92)
}

/// Spectrum meter colors
pub fn spectrum_green() -> Color {
    palette::DARK.success
}

pub fn spectrum_yellow() -> Color {
    palette::DARK.warning
}

pub fn spectrum_red() -> Color {
    palette::DARK.danger
}

/// Settings page title color
pub fn settings_title(theme: &Theme) -> Color {
    colors(theme).text
}

/// Settings page label color
pub fn settings_label(theme: &Theme) -> Color {
    colors(theme).text
}

/// Settings page description color
pub fn settings_desc(theme: &Theme) -> Color {
    colors(theme).muted
}

/// Settings page value color
pub fn settings_value(theme: &Theme) -> Color {
    colors(theme).secondary
}

/// Settings section title color
pub fn settings_section_title(theme: &Theme) -> Color {
    colors(theme).secondary
}

/// Settings inactive tab color
pub fn settings_inactive_tab(theme: &Theme) -> Color {
    colors(theme).muted
}

/// Settings inactive underline color
pub fn settings_inactive_underline(theme: &Theme) -> Color {
    colors(theme).border
}

/// Settings input background color
pub fn settings_input_bg(theme: &Theme) -> Color {
    colors(theme).surface
}

/// Settings input border color
pub fn settings_input_border(theme: &Theme) -> Color {
    colors(theme).control_border
}

/// Settings input border hover color
pub fn settings_input_border_hover(theme: &Theme) -> Color {
    colors(theme).accent
}

/// Shortcut key background color
pub fn shortcut_key_bg(theme: &Theme) -> Color {
    colors(theme).accent_subtle
}

/// Shortcut background color
pub fn shortcut_bg(theme: &Theme) -> Color {
    colors(theme).surface
}
