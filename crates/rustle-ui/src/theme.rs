//! Theme system for the music streaming application
//! Supports both dark and light modes with consistent color palette

use iced::color;
use iced::font::Weight;
use iced::widget::{button, container, pick_list, scrollable};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

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
    matches!(
        theme,
        Theme::Dark
            | Theme::Dracula
            | Theme::Nord
            | Theme::SolarizedDark
            | Theme::GruvboxDark
            | Theme::CatppuccinMocha
            | Theme::TokyoNight
            | Theme::TokyoNightStorm
            | Theme::TokyoNightLight
            | Theme::KanagawaWave
            | Theme::KanagawaDragon
            | Theme::KanagawaLotus
            | Theme::Moonfly
            | Theme::Nightfly
            | Theme::Oxocarbon
    )
}

/// Public function to check if theme is dark mode
pub fn is_dark_theme(theme: &Theme) -> bool {
    is_dark(theme)
}

// Dark mode colors
mod dark {
    use super::*;
    pub const BACKGROUND: Color = color!(0x000000);
    pub const SIDEBAR: Color = color!(0x121212);
    pub const SURFACE: Color = color!(0x1a1a1a);
    pub const BORDER: Color = color!(0x282828);
    pub const SURFACE_LIGHT: Color = color!(0x333333);
    pub const TEXT_MUTED: Color = color!(0x888888);
    pub const TEXT_SECONDARY: Color = color!(0xb3b3b3);
    pub const TEXT_PRIMARY: Color = color!(0xffffff);
}

// Light mode colors
mod light {
    use super::*;
    pub const BACKGROUND: Color = color!(0xffffff);
    pub const SIDEBAR: Color = color!(0xf5f5f5);
    pub const SURFACE: Color = color!(0xeeeeee);
    pub const BORDER: Color = color!(0xdddddd);
    pub const TEXT_MUTED: Color = color!(0x777777);
    pub const TEXT_SECONDARY: Color = color!(0x555555);
    pub const TEXT_PRIMARY: Color = color!(0x1a1a1a);
}

/// Get background color based on theme
pub fn background(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::BACKGROUND
    } else {
        light::BACKGROUND
    }
}

/// Translucent background used by the fixed top-bar overlay.
pub fn top_bar_background(theme: &Theme) -> Color {
    let mut color = background(theme);
    color.a = TOP_BAR_BACKGROUND_ALPHA;
    color
}

/// Get sidebar color based on theme
pub fn sidebar_bg(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::SIDEBAR
    } else {
        light::SIDEBAR
    }
}

/// Get surface color based on theme
pub fn surface(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::SURFACE
    } else {
        light::SURFACE
    }
}

/// Get border color based on theme
pub fn border_color(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::BORDER
    } else {
        light::BORDER
    }
}

/// Get muted text color based on theme
pub fn text_muted(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::TEXT_MUTED
    } else {
        light::TEXT_MUTED
    }
}

/// Get secondary text color based on theme
pub fn text_secondary(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::TEXT_SECONDARY
    } else {
        light::TEXT_SECONDARY
    }
}

/// Get primary text color based on theme
pub fn text_primary(theme: &Theme) -> Color {
    if is_dark(theme) {
        dark::TEXT_PRIMARY
    } else {
        light::TEXT_PRIMARY
    }
}

/// Return the RGB portion of a color without carrying its alpha channel.
///
/// SVG color filters do not consistently apply the alpha component as
/// transparency across renderers. Callers that need translucent SVGs should
/// use this for the tint and the SVG widget's `opacity` field for alpha.
#[inline]
pub fn opaque_color(color: Color) -> Color {
    Color::from_rgb(color.r, color.g, color.b)
}

// Legacy constants for backward compatibility (dark mode defaults)
pub const BLACK: Color = dark::BACKGROUND;
pub const BORDER_GRAY: Color = dark::BORDER;
pub const SURFACE_LIGHT: Color = dark::SURFACE_LIGHT;
pub const TEXT_MUTED: Color = dark::TEXT_MUTED;
pub const TEXT_SECONDARY: Color = dark::TEXT_SECONDARY;
pub const TEXT_PRIMARY: Color = dark::TEXT_PRIMARY;
/// Disabled text color (for inactive buttons)
pub const TEXT_DISABLED: Color = Color::from_rgba(0.5, 0.5, 0.5, 0.5);

/// Neon pink accent color (same for both modes)
pub const ACCENT_PINK: Color = color!(0xff1493);

/// Hover state for accent
pub const ACCENT_PINK_HOVER: Color = color!(0xff69b4);

/// Primary accent color
pub const ACCENT: Color = color!(0x1e90ff);

/// Hover state for primary accent
pub const ACCENT_HOVER: Color = color!(0x4169e1);

/// Surface secondary color
pub const SURFACE_SECONDARY: Color = color!(0x1a1a1a);

/// Dynamic surface hover color based on theme
pub fn surface_hover(theme: &Theme) -> Color {
    if is_dark(theme) {
        color!(0x2a2a2a)
    } else {
        color!(0xe0e0e0)
    }
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
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
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
    _theme: &Theme,
    status: button::Status,
    metrics: ThemeMetrics,
) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(ACCENT)),
        text_color: Color::WHITE,
        border: Border {
            radius: metrics.pill_radius.into(),
            ..Default::default()
        },
        ..Default::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(ACCENT_HOVER)),
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
        text_color: Color::WHITE,
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
        Color::from_rgba(1.0, 1.0, 1.0, 0.12)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.08)
    }
}

/// Play button hover color - slightly lighter/darker than text_primary
pub fn play_button_hover(theme: &Theme) -> Color {
    if is_dark(theme) {
        // Dark mode: white button, hover slightly gray
        Color::from_rgb(0.9, 0.9, 0.9)
    } else {
        // Light mode: dark button, hover slightly lighter
        Color::from_rgb(0.25, 0.25, 0.25)
    }
}

/// Surface elevated color (for cards, popups)
pub fn surface_elevated(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.12, 0.12, 0.14)
    } else {
        Color::from_rgb(0.96, 0.96, 0.98)
    }
}

/// Surface container color (for input fields, panels)
pub fn surface_container(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.15, 0.15, 0.15)
    } else {
        Color::from_rgb(0.92, 0.92, 0.92)
    }
}

/// Danger/error color
pub fn danger(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.9, 0.3, 0.3)
    } else {
        Color::from_rgb(0.8, 0.2, 0.2)
    }
}

/// Danger hover color
pub fn danger_hover(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(1.0, 0.4, 0.4)
    } else {
        Color::from_rgb(0.9, 0.3, 0.3)
    }
}

/// Success color
pub fn success(_theme: &Theme) -> Color {
    Color::from_rgb(0.3, 0.8, 0.5)
}

/// Warning color
pub fn warning(_theme: &Theme) -> Color {
    Color::from_rgb(0.95, 0.75, 0.3)
}

/// Info color
pub fn info(_theme: &Theme) -> Color {
    Color::from_rgb(0.4, 0.7, 0.95)
}

/// Divider/separator color
pub fn divider(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.1)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.1)
    }
}

/// Overlay backdrop color
pub fn overlay_backdrop(theme: &Theme, opacity: f32) -> Color {
    if is_dark(theme) {
        Color::from_rgba(0.0, 0.0, 0.0, opacity)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, opacity * 0.7)
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
        text_color: Color::WHITE,
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
            pick_list::Status::Active => Color::from_rgba(1.0, 1.0, 1.0, 0.08),
            pick_list::Status::Hovered => Color::from_rgba(1.0, 1.0, 1.0, 0.12),
            pick_list::Status::Opened { .. } => Color::from_rgba(1.0, 1.0, 1.0, 0.15),
            pick_list::Status::Disabled => Color::from_rgba(1.0, 1.0, 1.0, 0.04),
        }
    } else {
        match status {
            pick_list::Status::Active => Color::from_rgba(0.0, 0.0, 0.0, 0.05),
            pick_list::Status::Hovered => Color::from_rgba(0.0, 0.0, 0.0, 0.08),
            pick_list::Status::Opened { .. } => Color::from_rgba(0.0, 0.0, 0.0, 0.1),
            pick_list::Status::Disabled => Color::from_rgba(0.0, 0.0, 0.0, 0.03),
        }
    };

    let border_color = if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.1)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.15)
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
        (
            Color::from_rgb(0.15, 0.15, 0.15),
            Color::from_rgba(1.0, 1.0, 1.0, 0.1),
            Color::from_rgba(1.0, 1.0, 1.0, 0.1),
        )
    } else {
        (
            Color::from_rgb(0.98, 0.98, 0.98),
            Color::from_rgba(0.0, 0.0, 0.0, 0.08),
            Color::from_rgba(0.0, 0.0, 0.0, 0.1),
        )
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
    if is_dark(theme) {
        Color::from_rgb(0.12, 0.12, 0.14)
    } else {
        Color::from_rgb(0.96, 0.96, 0.97)
    }
}

/// Panel border color
pub fn panel_border(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.08)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.1)
    }
}

/// Shadow color for panels
pub fn shadow_color(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(0.0, 0.0, 0.0, 0.5)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.15)
    }
}

/// Player bar background
pub fn player_bar_bg(theme: &Theme) -> Color {
    sidebar_bg(theme)
}

/// Top border color used by the player bar and matching sidebar footer separator
pub fn player_bar_border(theme: &Theme) -> Color {
    if is_dark(theme) {
        color!(0x2d2d2d)
    } else {
        color!(0xd8d8d8)
    }
}

/// Header text color (slightly dimmed)
pub fn header_text(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.6)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.7)
    }
}

/// Dimmed text color (for indices, durations)
pub fn dimmed_text(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.5)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.6)
    }
}

/// Icon color (muted)
pub fn icon_muted(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.4)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.4)
    }
}

/// Hover background with alpha
pub fn hover_bg_alpha(theme: &Theme, alpha: f32) -> Color {
    if is_dark(theme) {
        Color::from_rgba(1.0, 1.0, 1.0, alpha)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, alpha * 0.7)
    }
}

/// Interpolate between two colors for animated UI state transitions.
pub fn lerp_color(from: Color, to: Color, progress: f32) -> Color {
    let progress = progress.clamp(0.0, 1.0);
    Color::from_rgba(
        from.r + (to.r - from.r) * progress,
        from.g + (to.g - from.g) * progress,
        from.b + (to.b - from.b) * progress,
        from.a + (to.a - from.a) * progress,
    )
}

/// Animated text color based on progress (for hover animations)
pub fn animated_text(theme: &Theme, progress: f32) -> Color {
    if is_dark(theme) {
        let alpha = 0.55 + 0.45 * progress;
        Color::from_rgba(1.0, 1.0, 1.0, alpha)
    } else {
        let alpha = 0.55 + 0.45 * progress;
        Color::from_rgba(0.0, 0.0, 0.0, alpha)
    }
}

/// Animated brightness for sidebar items
pub fn animated_brightness(theme: &Theme, progress: f32) -> Color {
    if is_dark(theme) {
        let idle = color!(0x99a1af);
        Color::from_rgb(
            idle.r + (1.0 - idle.r) * progress,
            idle.g + (1.0 - idle.g) * progress,
            idle.b + (1.0 - idle.b) * progress,
        )
    } else {
        let brightness = 0.5 - 0.3 * progress;
        Color::from_rgb(brightness, brightness, brightness)
    }
}

/// Close button hover (muted red adapted to the active theme)
pub fn close_button_hover(theme: &Theme) -> Color {
    let mut color = if is_dark(theme) {
        color!(0x4a3032)
    } else {
        color!(0xead6d7)
    };
    color.a = 0.72;
    color
}

/// Spectrum meter colors
pub fn spectrum_green() -> Color {
    Color::from_rgb(0.2, 0.8, 0.4)
}

pub fn spectrum_yellow() -> Color {
    Color::from_rgb(0.9, 0.8, 0.2)
}

pub fn spectrum_red() -> Color {
    Color::from_rgb(0.95, 0.3, 0.3)
}

/// Settings page title color
pub fn settings_title(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.95, 0.95, 0.95)
    } else {
        Color::from_rgb(0.1, 0.1, 0.1)
    }
}

/// Settings page label color
pub fn settings_label(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.9, 0.9, 0.9)
    } else {
        Color::from_rgb(0.15, 0.15, 0.15)
    }
}

/// Settings page description color
pub fn settings_desc(_theme: &Theme) -> Color {
    Color::from_rgb(0.5, 0.5, 0.5)
}

/// Settings page value color
pub fn settings_value(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.7, 0.7, 0.7)
    } else {
        Color::from_rgb(0.35, 0.35, 0.35)
    }
}

/// Settings section title color
pub fn settings_section_title(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.7, 0.7, 0.7)
    } else {
        Color::from_rgb(0.35, 0.35, 0.35)
    }
}

/// Settings inactive tab color
pub fn settings_inactive_tab(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.6, 0.6, 0.6)
    } else {
        Color::from_rgb(0.45, 0.45, 0.45)
    }
}

/// Settings inactive underline color
pub fn settings_inactive_underline(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.25, 0.25, 0.25)
    } else {
        Color::from_rgb(0.8, 0.8, 0.8)
    }
}

/// Settings input background color
pub fn settings_input_bg(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.15, 0.15, 0.15)
    } else {
        Color::from_rgb(0.95, 0.95, 0.95)
    }
}

/// Settings input border color
pub fn settings_input_border(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.3, 0.3, 0.3)
    } else {
        Color::from_rgb(0.75, 0.75, 0.75)
    }
}

/// Settings input border hover color
pub fn settings_input_border_hover(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.4, 0.4, 0.4)
    } else {
        Color::from_rgb(0.6, 0.6, 0.6)
    }
}

/// Shortcut key background color
pub fn shortcut_key_bg(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.3, 0.15, 0.2)
    } else {
        Color::from_rgb(0.95, 0.85, 0.9)
    }
}

/// Shortcut background color
pub fn shortcut_bg(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb(0.2, 0.2, 0.2)
    } else {
        Color::from_rgb(0.9, 0.9, 0.9)
    }
}
