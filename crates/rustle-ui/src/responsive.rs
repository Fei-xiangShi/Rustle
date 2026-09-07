//! Pure responsive layout policy for the Rustle UI.
//!
//! This module deliberately deals only in logical viewport geometry and UI
//! values. It does not know about application state, routes, messages, or
//! business data. View code can build a [`ResponsiveContext`] from iced's
//! layout size and pass the resulting tokens to the component that owns the
//! relevant interaction.

use iced::Size;

use crate::theme;

/// Height of the reference design rectangle in logical pixels.
pub const REFERENCE_HEIGHT: f32 = 1_080.0;
/// Root font size of the reference design in logical pixels.
pub const ROOT_REM_REFERENCE_PX: f32 = 16.0;

/// Lowest root-unit scale. The floor keeps compact layouts readable while
/// profile changes handle the loss of available composition space.
pub const REM_SCALE_FLOOR: f32 = 0.9;
/// Highest root-unit scale used by the 2K composition.
pub const REM_SCALE_CEILING: f32 = 4.0 / 3.0;

/// The narrow profile begins below this logical width.
pub const NARROW_MAX_WIDTH: f32 = 640.0;
/// The tablet profile begins at this logical width when it is not narrow.
pub const TABLET_MIN_WIDTH: f32 = 640.0;
/// The compact profile begins at this logical width.
pub const COMPACT_MIN_WIDTH: f32 = 840.0;
/// The standard profile begins at this logical width.
pub const STANDARD_MIN_WIDTH: f32 = 1_120.0;
/// The expanded profile requires this logical width.
pub const EXPANDED_MIN_WIDTH: f32 = 1_440.0;
/// The expanded profile also requires enough height for desktop chrome.
pub const EXPANDED_MIN_HEIGHT: f32 = 820.0;
/// A portrait-like aspect ratio promotes a width-based profile to Tablet.
pub const TABLET_MAX_ASPECT_RATIO: f32 = 1.15;

/// Minimum width used by bounded composition policies.
pub const MIN_USABLE_CONTENT_WIDTH: f32 = 200.0;
/// Minimum hit target for an interactive control.
pub const MIN_INTERACTION_TARGET: f32 = 36.0;
/// Half-window playlist-card presentation begins at this logical width.
const HALF_WINDOW_PLAYLIST_MIN_WIDTH: f32 = 900.0;
/// Reference minimum width for one three-column shortcut table. This is a
/// composition threshold, not a fixed cell width; cells still share whatever
/// width the rendered table receives.
const SHORTCUT_TABLE_MIN_WIDTH: f32 = 520.0;
/// Reference horizontal inset used by the settings page body.
const SETTINGS_CONTENT_HORIZONTAL_INSET: f32 = 64.0;
/// Reference gap between the two shortcut tables.
const SHORTCUT_TABLE_GAP: f32 = 24.0;

/// A named composition profile selected from logical viewport geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProfile {
    /// Full desktop lanes and expanded content composition.
    Expanded,
    /// Desktop composition with bounded content and fewer columns.
    Standard,
    /// Reduced chrome and compact desktop composition.
    Compact,
    /// Stacked content with drawer/rail-oriented chrome.
    Tablet,
    /// Single-column emergency composition with explicit overflow.
    Narrow,
}

impl LayoutProfile {
    /// Whether this profile is intended for desktop lane composition.
    #[inline]
    pub const fn is_desktop(self) -> bool {
        matches!(self, Self::Expanded | Self::Standard)
    }

    /// Whether this profile needs a stacked or overflow-oriented composition.
    #[inline]
    pub const fn is_compact(self) -> bool {
        matches!(self, Self::Compact | Self::Tablet | Self::Narrow)
    }

    /// Whether this profile uses a navigation drawer instead of the full tree.
    #[inline]
    pub const fn uses_navigation_drawer(self) -> bool {
        matches!(self, Self::Compact | Self::Tablet | Self::Narrow)
    }
}

/// Presentation selected for the application navigation surface.
///
/// The presentation is deliberately independent from route state. All three
/// variants consume the same navigation model and callbacks at the component
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPresentation {
    Full,
    Rail,
    Drawer,
    Hidden,
}

/// Select the navigation presentation for a profile and drawer state.
#[inline]
pub fn sidebar_presentation(profile: LayoutProfile, drawer_open: bool) -> SidebarPresentation {
    match profile {
        LayoutProfile::Expanded | LayoutProfile::Standard => SidebarPresentation::Full,
        LayoutProfile::Compact => {
            if drawer_open {
                SidebarPresentation::Drawer
            } else {
                SidebarPresentation::Rail
            }
        }
        LayoutProfile::Tablet => {
            if drawer_open {
                SidebarPresentation::Drawer
            } else {
                SidebarPresentation::Rail
            }
        }
        LayoutProfile::Narrow => {
            if drawer_open {
                SidebarPresentation::Drawer
            } else {
                SidebarPresentation::Hidden
            }
        }
    }
}

/// Return the finite, positive part of a logical dimension.
#[inline]
fn positive_dimension(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// Return a finite non-negative value for geometry calculations.
#[inline]
fn non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// Compute the aspect ratio of a logical viewport.
///
/// Degenerate or invalid dimensions return `0.0`, allowing the classifier to
/// choose a conservative compact profile instead of propagating `NaN` or an
/// infinite ratio into layout decisions.
#[inline]
pub fn viewport_aspect_ratio(viewport: Size) -> f32 {
    let width = positive_dimension(viewport.width);
    let height = positive_dimension(viewport.height);

    if width > 0.0 && height > 0.0 {
        width / height
    } else {
        0.0
    }
}

/// Compute the unclamped root-unit scale against the 1080P reference height.
///
/// Width deliberately does not participate. It selects the composition
/// profile, while the root unit keeps a full-height half-screen layout at the
/// same visual scale as the corresponding full-screen layout.
#[inline]
pub fn raw_rem_scale(viewport: Size) -> f32 {
    let height = positive_dimension(viewport.height);

    if height > 0.0 {
        height / REFERENCE_HEIGHT
    } else {
        0.0
    }
}

/// Select a layout profile from the logical viewport.
///
/// Width breakpoints are centralized here. The aspect-ratio check runs before
/// the width tiers so a portrait-like window cannot accidentally render a
/// desktop row merely because one axis is wide. Expanded is additionally
/// height-gated; narrower tiers intentionally remain usable in short windows.
#[inline]
pub fn classify_profile(viewport: Size) -> LayoutProfile {
    let width = positive_dimension(viewport.width);
    let height = positive_dimension(viewport.height);
    let aspect_ratio = viewport_aspect_ratio(viewport);

    if width < NARROW_MAX_WIDTH {
        LayoutProfile::Narrow
    } else if aspect_ratio < TABLET_MAX_ASPECT_RATIO {
        LayoutProfile::Tablet
    } else if width >= EXPANDED_MIN_WIDTH && height >= EXPANDED_MIN_HEIGHT {
        LayoutProfile::Expanded
    } else if width >= STANDARD_MIN_WIDTH {
        LayoutProfile::Standard
    } else if width >= COMPACT_MIN_WIDTH {
        LayoutProfile::Compact
    } else if width >= TABLET_MIN_WIDTH {
        LayoutProfile::Tablet
    } else {
        LayoutProfile::Narrow
    }
}

/// Responsive root `rem` used to resolve application-owned visual metrics.
///
/// Iced accepts logical pixels for fixed lengths and text sizes. `RootRem`
/// provides the missing application-level root unit: one rem is 16 logical
/// pixels at 1080P and scales with the logical viewport height.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct RootRem(f32);

impl RootRem {
    /// Clamp an arbitrary root scale to the supported responsive range.
    ///
    /// Non-finite values are treated as invalid and resolve to the compact
    /// floor rather than poisoning every derived token with `NaN`.
    #[inline]
    pub fn from_scale(scale: f32) -> Self {
        let scale = if scale.is_finite() {
            scale
        } else {
            REM_SCALE_FLOOR
        };

        Self(scale.clamp(REM_SCALE_FLOOR, REM_SCALE_CEILING))
    }

    /// Derive the root unit from iced's logical viewport height.
    #[inline]
    pub fn from_viewport(viewport: Size) -> Self {
        Self::from_scale(raw_rem_scale(viewport))
    }

    /// Return the root-unit scale relative to the 1080P reference.
    #[inline]
    pub fn scale(self) -> f32 {
        self.0
    }

    /// Return the logical pixel size of one rem.
    #[inline]
    pub fn logical_pixels(self) -> f32 {
        ROOT_REM_REFERENCE_PX * self.0
    }

    /// Resolve a number of rem units to logical pixels.
    #[inline]
    pub fn resolve(self, rem: f32) -> f32 {
        if rem.is_finite() {
            rem * self.logical_pixels()
        } else {
            0.0
        }
    }
}

impl Default for RootRem {
    #[inline]
    fn default() -> Self {
        Self::from_scale(1.0)
    }
}

/// Immutable responsive inputs and semantic UI tokens for one layout pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsiveContext {
    /// Logical size supplied by iced's layout system.
    pub viewport: Size,
    /// Width divided by height, or `0.0` for invalid/degenerate geometry.
    pub aspect_ratio: f32,
    /// Named composition profile for this viewport.
    pub profile: LayoutProfile,
    /// Root rem used for Rustle-owned UI dimensions.
    pub root_rem: RootRem,
    /// Scaled semantic UI values.
    pub tokens: UiTokens,
}

impl ResponsiveContext {
    /// Build a context from iced's logical layout size.
    #[inline]
    pub fn new(viewport: Size) -> Self {
        let root_rem = RootRem::from_viewport(viewport);

        Self {
            viewport,
            aspect_ratio: viewport_aspect_ratio(viewport),
            profile: classify_profile(viewport),
            root_rem,
            tokens: UiTokens::new(root_rem),
        }
    }

    /// Alias that makes the logical-size boundary explicit at call sites.
    #[inline]
    pub fn from_viewport(viewport: Size) -> Self {
        Self::new(viewport)
    }

    /// Return a sanitized logical width for pure geometry policies.
    #[inline]
    pub fn width(&self) -> f32 {
        positive_dimension(self.viewport.width)
    }

    /// Return a sanitized logical height for pure geometry policies.
    #[inline]
    pub fn height(&self) -> f32 {
        positive_dimension(self.viewport.height)
    }
}

impl From<Size> for ResponsiveContext {
    #[inline]
    fn from(viewport: Size) -> Self {
        Self::new(viewport)
    }
}

/// Semantic typography roles used by [`UiTokens`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    Micro,
    Caption,
    Label,
    Body,
    BodyLarge,
    Subtitle,
    Title,
    TitleLarge,
    Hero,
    Display,
}

impl TextRole {
    #[inline]
    fn reference(self) -> (f32, f32) {
        match self {
            Self::Micro => (theme::TEXT_SIZE_MICRO, 10.0),
            Self::Caption => (theme::TEXT_SIZE_CAPTION, 11.0),
            Self::Label => (theme::TEXT_SIZE_LABEL, 12.0),
            Self::Body => (theme::TEXT_SIZE_BODY, 13.0),
            Self::BodyLarge => (theme::TEXT_SIZE_BODY_LARGE, 14.0),
            Self::Subtitle => (theme::TEXT_SIZE_SUBTITLE, 16.0),
            Self::Title => (theme::TEXT_SIZE_TITLE, 20.0),
            Self::TitleLarge => (theme::TEXT_SIZE_TITLE_LARGE, 24.0),
            Self::Hero => (theme::TEXT_SIZE_HERO, 24.0),
            Self::Display => (theme::TEXT_SIZE_DISPLAY, 32.0),
        }
    }
}

/// Semantic icon-size roles used by [`UiTokens`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconRole {
    Small,
    Medium,
    Large,
    Sidebar,
    TopBarNavigation,
    TopBarAction,
    TopBarSearch,
    WindowControl,
    Hero,
}

impl IconRole {
    #[inline]
    fn reference(self) -> (f32, f32) {
        match self {
            Self::Small => (16.0, 14.0),
            Self::Medium => (20.0, 18.0),
            Self::Large => (24.0, 20.0),
            Self::Sidebar => (24.0, 20.0),
            // These are visual sizes inside separate interaction targets. They
            // intentionally match the pre-regression semantic roles now that
            // fixed buttons no longer stretch their children.
            Self::TopBarNavigation => (22.0, 20.0),
            Self::TopBarAction => (18.0, 16.0),
            Self::TopBarSearch => (18.0, 16.0),
            Self::WindowControl => (17.0, 15.0),
            Self::Hero => (32.0, 24.0),
        }
    }
}

/// Semantic interaction-target roles used by [`UiTokens`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRole {
    Icon,
    Control,
    WindowControl,
}

impl TargetRole {
    #[inline]
    fn reference(self) -> (f32, f32) {
        match self {
            Self::Icon => (36.0, MIN_INTERACTION_TARGET),
            Self::Control => (40.0, MIN_INTERACTION_TARGET),
            Self::WindowControl => (42.0, MIN_INTERACTION_TARGET),
        }
    }
}

/// Semantic surface-radius roles used by [`UiTokens`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadiusRole {
    Small,
    Medium,
    Large,
    Pill,
}

/// Semantic corner radii for square and rectangular artwork.
///
/// Circular avatars remain driven by their dimensions, while the player-bar
/// cover deliberately keeps its component-local radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverRadiusRole {
    Thumbnail,
    Card,
    Hero,
}

impl CoverRadiusRole {
    #[inline]
    fn reference(self) -> f32 {
        match self {
            Self::Thumbnail => 10.0,
            Self::Card => 18.0,
            Self::Hero => 24.0,
        }
    }
}

impl RadiusRole {
    #[inline]
    fn reference(self) -> f32 {
        match self {
            Self::Small => 4.0,
            Self::Medium => 8.0,
            Self::Large => 16.0,
            Self::Pill => 24.0,
        }
    }
}

/// Shared card dimension roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardRole {
    Playlist,
    Detail,
    Feature,
}

/// Scaled geometry for a card family.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardMetrics {
    pub width: f32,
    pub height: f32,
    pub gap: f32,
    pub radius: f32,
}

/// Complete playlist-grid geometry for one measured layout pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaylistGridLayout {
    pub metrics: CardMetrics,
    pub columns: usize,
}

/// Shared geometry for the horizontal identity header used by playlist,
/// album, user, and artist detail pages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailHeaderMetrics {
    pub artwork_size: f32,
    pub gap: f32,
    pub horizontal_padding: f32,
    pub top_padding: f32,
    pub bottom_padding: f32,
    pub title_size: f32,
    pub prominent_title_size: f32,
}

/// Resolve the detail-family header metrics without coupling the policy to a
/// route, business model, or application message.
///
/// Every profile keeps the same left-artwork/right-information composition.
/// Narrower profiles compact the visual metrics instead of treating a
/// portrait-like aspect ratio as proof that the row cannot fit.
#[inline]
pub fn detail_header_metrics(context: ResponsiveContext) -> DetailHeaderMetrics {
    let tokens = context.tokens;
    let (artwork, gap, horizontal_padding, top_padding, bottom_padding, title, prominent_title) =
        match context.profile {
            LayoutProfile::Expanded => (224.0, 28.0, 40.0, 76.0, 20.0, 44.0, 52.0),
            LayoutProfile::Standard => (224.0, 28.0, 40.0, 76.0, 20.0, 40.0, 48.0),
            LayoutProfile::Compact => (200.0, 24.0, 28.0, 62.0, 18.0, 36.0, 46.0),
            LayoutProfile::Tablet => (184.0, 20.0, 24.0, 58.0, 16.0, 34.0, 44.0),
            LayoutProfile::Narrow => (152.0, 16.0, 16.0, 52.0, 14.0, 30.0, 36.0),
        };

    DetailHeaderMetrics {
        artwork_size: tokens.size(artwork),
        gap: tokens.space(gap),
        horizontal_padding: tokens.space(horizontal_padding),
        top_padding: tokens.space(top_padding),
        bottom_padding: tokens.space(bottom_padding),
        title_size: tokens.size(title).max(tokens.text(TextRole::TitleLarge)),
        prominent_title_size: tokens
            .size(prominent_title)
            .max(tokens.text(TextRole::TitleLarge)),
    }
}

/// Responsive composition used by the full-screen lyrics surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsPageLayout {
    /// Established wide artwork/lyrics `4:6` split.
    Split,
    /// Same-page artwork/lyrics component switch for insufficient width.
    Focus,
}

/// Preserve the established split whenever the desktop content lanes fit.
#[inline]
pub fn lyrics_page_layout(context: ResponsiveContext) -> LyricsPageLayout {
    if context.profile.is_desktop() {
        LyricsPageLayout::Split
    } else {
        LyricsPageLayout::Focus
    }
}

/// Resolve the single square-media width shared by the lyrics artwork and its
/// progress/time lane.
///
/// The preferred size follows the responsive root rem while width and height
/// caps reserve enough room for full-screen chrome, metadata, and playback
/// controls. Short-height layouts remain height-capped, while full-height
/// Tablet layouts keep the same root-rem target as the rest of the UI.
#[inline]
pub fn lyrics_media_width(context: ResponsiveContext) -> f32 {
    let tokens = context.tokens;
    let (horizontal_gutter, reserved_vertical) = match context.profile {
        LayoutProfile::Expanded => (80.0, 320.0),
        LayoutProfile::Standard => (64.0, 320.0),
        LayoutProfile::Compact => (48.0, 260.0),
        LayoutProfile::Tablet => (40.0, 260.0),
        LayoutProfile::Narrow => (32.0, 260.0),
    };
    let preferred = match context.profile {
        LayoutProfile::Expanded => tokens.size(480.0),
        LayoutProfile::Standard => tokens.size(460.0),
        LayoutProfile::Compact => tokens.size(340.0),
        LayoutProfile::Tablet => tokens.size(500.0),
        LayoutProfile::Narrow => tokens.size(400.0),
    };
    let width_cap =
        (positive_dimension(context.width()) - 2.0 * tokens.space(horizontal_gutter)).max(0.0);
    let height_cap =
        (positive_dimension(context.height()) - tokens.size(reserved_vertical)).max(0.0);
    preferred.min(width_cap).min(height_cap)
}

impl CardRole {
    #[inline]
    fn reference(self) -> CardMetrics {
        match self {
            Self::Playlist => CardMetrics {
                width: 168.0,
                height: 228.0,
                gap: 20.0,
                radius: 18.0,
            },
            Self::Detail => CardMetrics {
                width: 200.0,
                height: 200.0,
                gap: 20.0,
                radius: 18.0,
            },
            Self::Feature => CardMetrics {
                width: 320.0,
                height: 204.0,
                gap: 24.0,
                radius: 20.0,
            },
        }
    }
}

/// Resolve playlist-card geometry for the current composition.
///
/// Half-window card width follows the horizontal family instead of root-rem
/// height so full-height and short-height variants keep the same covers and
/// five-column density. Expanded layouts continuously tighten the enlarged
/// reference card rhythm as the root rem grows from 1080P toward 2K,
/// preserving an eight-card desktop row even when a maximized window is
/// slightly shorter than the display because of compositor chrome. Smaller
/// portrait tablets retain their established card size.
#[inline]
pub fn playlist_card_metrics(context: ResponsiveContext) -> CardMetrics {
    const HALF_WINDOW_START_WIDTH: f32 = 960.0;
    const HALF_WINDOW_END_WIDTH: f32 = 1_280.0;
    const HALF_WINDOW_START_CARD_WIDTH: f32 = 151.2;
    const HALF_WINDOW_END_CARD_WIDTH: f32 = 179.2;
    const EXPANDED_CARD_WIDTH: f32 = 174.0;
    const TWO_K_EXPANDED_CARD_WIDTH: f32 = 178.0;
    const PLAYLIST_FOOTER_HEIGHT: f32 = 60.0;
    const EXPANDED_CARD_GAP: f32 = 24.0;
    const HALF_WINDOW_CARD_GAP: f32 = 18.0;
    const TWO_K_EXPANDED_CARD_GAP: f32 = 19.0;

    let tokens = context.tokens;
    let metrics = tokens.card(CardRole::Playlist);
    let use_half_window_metrics = uses_half_window_playlist_presentation(context);

    if use_half_window_metrics {
        // The sidebar presentation changes when only a half-window's height
        // changes (for example, 1280x1440 Tablet vs 1280x720 Standard). Keep
        // cover width driven by the shared horizontal family so that height
        // alone cannot make the grid jump to smaller cards.
        let width_progress = ((context.width() - HALF_WINDOW_START_WIDTH)
            / (HALF_WINDOW_END_WIDTH - HALF_WINDOW_START_WIDTH))
            .clamp(0.0, 1.0);
        let width = HALF_WINDOW_START_CARD_WIDTH
            + (HALF_WINDOW_END_CARD_WIDTH - HALF_WINDOW_START_CARD_WIDTH) * width_progress;
        CardMetrics {
            width,
            height: width + tokens.size(PLAYLIST_FOOTER_HEIGHT),
            gap: tokens.space(HALF_WINDOW_CARD_GAP),
            radius: metrics.radius,
        }
    } else if context.profile == LayoutProfile::Expanded {
        let density_progress =
            ((context.root_rem.scale() - 1.0) / (REM_SCALE_CEILING - 1.0)).clamp(0.0, 1.0);
        let reference_width = EXPANDED_CARD_WIDTH
            + (TWO_K_EXPANDED_CARD_WIDTH - EXPANDED_CARD_WIDTH) * density_progress;
        let reference_gap =
            EXPANDED_CARD_GAP + (TWO_K_EXPANDED_CARD_GAP - EXPANDED_CARD_GAP) * density_progress;
        let width = tokens.size(reference_width);
        CardMetrics {
            width,
            height: width + tokens.size(PLAYLIST_FOOTER_HEIGHT),
            gap: tokens.space(reference_gap),
            radius: metrics.radius,
        }
    } else {
        metrics
    }
}

/// Resolve playlist-card geometry against the grid's actual local width.
///
/// Full-height half-window layouts use a collapsed navigation rail and
/// therefore have a wider content lane than their short-height counterparts.
/// Fit exactly five cards across that measured lane instead of forcing the
/// short-height card width into both shell presentations.
#[inline]
pub fn playlist_card_metrics_for_width(
    context: ResponsiveContext,
    available_width: f32,
) -> CardMetrics {
    const HALF_WINDOW_COLUMNS: f32 = 5.0;

    let mut metrics = playlist_card_metrics(context);
    if context.profile == LayoutProfile::Tablet && uses_half_window_playlist_presentation(context) {
        let gaps = metrics.gap * (HALF_WINDOW_COLUMNS - 1.0);
        let fitted_width =
            (positive_dimension(available_width) - gaps).max(0.0) / HALF_WINDOW_COLUMNS;
        if fitted_width > 0.0 {
            let footer_height = (metrics.height - metrics.width).max(0.0);
            metrics.width = fitted_width;
            metrics.height = fitted_width + footer_height;
        }
    }

    metrics
}

/// Resolve playlist-grid columns and fit complete desktop rows to their local
/// measured width.
///
/// Expanded desktop keeps the established six-card default composition and
/// grows to at most eight cards as the actual content lane becomes wider.
/// Fitting the selected row after column selection removes the platform- and
/// DPI-dependent sliver of unused space without changing the half-window
/// presentation, which continues to use its existing five-card policy.
#[inline]
pub fn playlist_grid_layout_for_width(
    context: ResponsiveContext,
    available_width: f32,
    max_columns: usize,
) -> PlaylistGridLayout {
    let mut metrics = playlist_card_metrics_for_width(context, available_width);
    let columns =
        calculate_grid_columns_clamped(available_width, metrics.width, metrics.gap, max_columns);

    if context.profile == LayoutProfile::Expanded && columns > 0 {
        let available_width = positive_dimension(available_width);
        let gaps = metrics.gap * (columns.saturating_sub(1) as f32);
        let fitted_width = ((available_width - gaps) / columns as f32).max(0.0);
        if fitted_width > 0.0 {
            let footer_height = (metrics.height - metrics.width).max(0.0);
            metrics.width = fitted_width;
            metrics.height = fitted_width + footer_height;
        }
    }

    PlaylistGridLayout { metrics, columns }
}

#[inline]
fn uses_half_window_playlist_presentation(context: ResponsiveContext) -> bool {
    context.profile != LayoutProfile::Expanded && context.width() >= HALF_WINDOW_PLAYLIST_MIN_WIDTH
}

/// Shared application-chrome dimension roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeRole {
    TopBar,
    PlayerBar,
    Sidebar,
    SidebarRail,
    ResizeHandle,
}

impl ChromeRole {
    #[inline]
    fn reference(self) -> f32 {
        match self {
            Self::TopBar => theme::TOP_BAR_HEIGHT,
            Self::PlayerBar => 88.0,
            Self::Sidebar => 280.0,
            Self::SidebarRail => 68.0,
            Self::ResizeHandle => 6.0,
        }
    }
}

/// Return the top-bar height for the current composition profile.
#[inline]
pub fn top_bar_height(context: &ResponsiveContext) -> f32 {
    context.tokens.chrome(ChromeRole::TopBar)
}

/// Immutable semantic UI dimensions derived from a responsive [`RootRem`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiTokens {
    root_rem: RootRem,
}

impl UiTokens {
    /// Create tokens for a responsive root unit.
    #[inline]
    pub fn new(root_rem: RootRem) -> Self {
        Self { root_rem }
    }

    /// Return the root rem that produced these tokens.
    #[inline]
    pub fn root_rem(&self) -> RootRem {
        self.root_rem
    }

    /// Resolve an explicit rem value to iced logical pixels.
    #[inline]
    pub fn rem(&self, units: f32) -> f32 {
        non_negative(self.root_rem.resolve(units))
    }

    /// Resolve a reference-design pixel value through the root rem.
    #[inline]
    pub fn px(&self, reference_pixels: f32) -> f32 {
        self.rem(reference_pixels / ROOT_REM_REFERENCE_PX)
    }

    /// Convert rendered logical pixels back into 1080P reference pixels.
    ///
    /// This is used only for state, such as a user-resized sidebar width,
    /// that is intentionally persisted in reference-design units.
    #[inline]
    pub fn reference_pixels(&self, logical_pixels: f32) -> f32 {
        if logical_pixels.is_finite() {
            logical_pixels / self.root_rem.scale()
        } else {
            0.0
        }
    }

    /// Resolve a reference spacing value, treating invalid values as zero.
    #[inline]
    pub fn space(&self, base: f32) -> f32 {
        self.px(base)
    }

    /// Alias for callers that use generic size terminology.
    #[inline]
    pub fn size(&self, base: f32) -> f32 {
        self.space(base)
    }

    /// Resolve a named typography role with its readability floor.
    #[inline]
    pub fn text(&self, role: TextRole) -> f32 {
        let (base, floor) = role.reference();
        self.px(base).max(floor)
    }

    /// Resolve a named icon role with its minimum raster size.
    #[inline]
    pub fn icon(&self, role: IconRole) -> f32 {
        let (base, floor) = role.reference();
        self.px(base).max(floor)
    }

    /// Resolve a named interaction target while preserving the global minimum.
    #[inline]
    pub fn target(&self, role: TargetRole) -> f32 {
        let (base, floor) = role.reference();
        self.px(base).max(floor)
    }

    /// Resolve a named surface radius.
    #[inline]
    pub fn radius(&self, role: RadiusRole) -> f32 {
        self.space(role.reference())
    }

    /// Resolve a named artwork radius.
    #[inline]
    pub fn cover_radius(&self, role: CoverRadiusRole) -> f32 {
        self.space(role.reference())
    }

    /// Resolve shared card geometry.
    #[inline]
    pub fn card(&self, role: CardRole) -> CardMetrics {
        let reference = role.reference();
        CardMetrics {
            width: self.size(reference.width),
            height: self.size(reference.height),
            gap: self.space(reference.gap),
            radius: self.size(reference.radius),
        }
    }

    /// Resolve a shared chrome dimension.
    #[inline]
    pub fn chrome(&self, role: ChromeRole) -> f32 {
        self.size(role.reference())
    }

    /// Resolve dimensions used by shared theme style callbacks.
    #[inline]
    pub fn theme_metrics(&self) -> theme::ThemeMetrics {
        theme::ThemeMetrics {
            small_radius: self.radius(RadiusRole::Small),
            medium_radius: self.radius(RadiusRole::Medium),
            large_radius: self.radius(RadiusRole::Large),
            pill_radius: self.radius(RadiusRole::Pill),
            border_width: self.size(1.0),
            popup_shadow_offset_y: self.size(8.0),
            popup_shadow_blur: self.size(24.0),
        }
    }
}

impl Default for UiTokens {
    #[inline]
    fn default() -> Self {
        Self::new(RootRem::default())
    }
}

/// Clamp a preferred width to a viewport while reserving one gutter on both
/// sides. Invalid inputs resolve to zero.
#[inline]
pub fn bounded_width(preferred: f32, viewport_width: f32, gutter: f32) -> f32 {
    let preferred = non_negative(preferred);
    let available = (positive_dimension(viewport_width) - 2.0 * non_negative(gutter)).max(0.0);
    preferred.min(available)
}

/// Clamp a preferred height to a viewport while reserving one gutter on both
/// sides. Invalid inputs resolve to zero.
#[inline]
pub fn bounded_height(preferred: f32, viewport_height: f32, gutter: f32) -> f32 {
    let preferred = non_negative(preferred);
    let available = (positive_dimension(viewport_height) - 2.0 * non_negative(gutter)).max(0.0);
    preferred.min(available)
}

/// Clamp a panel to a viewport, reserving chrome from the available height.
#[inline]
pub fn bounded_panel_size(
    preferred: Size,
    viewport: Size,
    horizontal_gutter: f32,
    vertical_gutter: f32,
    reserved_height: f32,
) -> Size {
    let available_height =
        (positive_dimension(viewport.height) - non_negative(reserved_height)).max(0.0);

    Size::new(
        bounded_width(preferred.width, viewport.width, horizontal_gutter),
        bounded_height(preferred.height, available_height, vertical_gutter),
    )
}

/// Calculate how many complete cards fit into a content width.
#[inline]
pub fn calculate_grid_columns(content_width: f32, card_width: f32, spacing: f32) -> usize {
    let content_width = positive_dimension(content_width);
    let card_width = positive_dimension(card_width);
    let spacing = non_negative(spacing);

    if content_width <= 0.0 || card_width <= 0.0 {
        return 1;
    }

    let denominator = card_width + spacing;
    let numerator = content_width + spacing;
    if !denominator.is_finite() || !numerator.is_finite() || denominator <= 0.0 {
        return 1;
    }

    (numerator / denominator).floor().max(1.0) as usize
}

/// Calculate grid columns and clamp the result to a positive maximum.
#[inline]
pub fn calculate_grid_columns_clamped(
    content_width: f32,
    card_width: f32,
    spacing: f32,
    max_columns: usize,
) -> usize {
    calculate_grid_columns(content_width, card_width, spacing).clamp(1, max_columns.max(1))
}

/// Responsive arrangement for the pair of shortcut tables.
///
/// Each table keeps one horizontal three-equal-column row contract. Only the
/// relationship between the two complete tables changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutTablesLayout {
    SideBySide,
    Stacked,
}

/// Choose whether two complete shortcut tables fit beside each other after
/// accounting for the current navigation chrome and settings-page inset.
#[inline]
pub fn shortcut_tables_layout(context: ResponsiveContext) -> ShortcutTablesLayout {
    let chrome_width = match sidebar_presentation(context.profile, false) {
        SidebarPresentation::Full => context.tokens.chrome(ChromeRole::Sidebar),
        SidebarPresentation::Rail => context.tokens.chrome(ChromeRole::SidebarRail),
        SidebarPresentation::Drawer | SidebarPresentation::Hidden => 0.0,
    };
    let content_width =
        (context.width() - chrome_width - context.tokens.space(SETTINGS_CONTENT_HORIZONTAL_INSET))
            .max(context.tokens.size(MIN_USABLE_CONTENT_WIDTH));
    let required_width = context.tokens.size(SHORTCUT_TABLE_MIN_WIDTH) * 2.0
        + context.tokens.space(SHORTCUT_TABLE_GAP);

    if content_width >= required_width {
        ShortcutTablesLayout::SideBySide
    } else {
        ShortcutTablesLayout::Stacked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn root_rem_matches_reference_and_2k_viewports_at_full_and_half_width() {
        assert_approx(
            RootRem::from_viewport(Size::new(1_920.0, 1_080.0)).scale(),
            1.0,
        );
        assert_approx(
            RootRem::from_viewport(Size::new(2_560.0, 1_440.0)).scale(),
            4.0 / 3.0,
        );
        assert_approx(
            RootRem::from_viewport(Size::new(960.0, 1_080.0)).scale(),
            1.0,
        );
        assert_approx(
            RootRem::from_viewport(Size::new(1_280.0, 1_440.0)).scale(),
            4.0 / 3.0,
        );
        assert_approx(
            RootRem::from_viewport(Size::new(2_560.0, 1_440.0)).logical_pixels(),
            64.0 / 3.0,
        );
    }

    #[test]
    fn root_rem_is_clamped_and_invalid_values_are_safe() {
        assert_eq!(RootRem::from_scale(0.1).scale(), REM_SCALE_FLOOR);
        assert_eq!(RootRem::from_scale(10.0).scale(), REM_SCALE_CEILING);
        assert_eq!(RootRem::from_scale(f32::NAN).scale(), REM_SCALE_FLOOR);
        assert_eq!(
            RootRem::from_viewport(Size::new(0.0, 0.0)).scale(),
            REM_SCALE_FLOOR
        );
        assert_eq!(raw_rem_scale(Size::new(f32::INFINITY, 0.0)), 0.0);
    }

    #[test]
    fn profile_boundaries_are_centralized_and_deterministic() {
        assert_eq!(
            classify_profile(Size::new(1_440.0, 820.0)),
            LayoutProfile::Expanded
        );
        assert_eq!(
            classify_profile(Size::new(1_439.0, 820.0)),
            LayoutProfile::Standard
        );
        assert_eq!(
            classify_profile(Size::new(1_120.0, 700.0)),
            LayoutProfile::Standard
        );
        assert_eq!(
            classify_profile(Size::new(1_119.0, 700.0)),
            LayoutProfile::Compact
        );
        assert_eq!(
            classify_profile(Size::new(840.0, 720.0)),
            LayoutProfile::Compact
        );
        assert_eq!(
            classify_profile(Size::new(839.0, 720.0)),
            LayoutProfile::Tablet
        );
        assert_eq!(
            classify_profile(Size::new(640.0, 400.0)),
            LayoutProfile::Tablet
        );
        assert_eq!(
            classify_profile(Size::new(639.0, 1_024.0)),
            LayoutProfile::Narrow
        );
    }

    #[test]
    fn portrait_aspect_promotes_wide_viewports_to_tablet() {
        assert_eq!(
            classify_profile(Size::new(1_200.0, 1_400.0)),
            LayoutProfile::Tablet
        );
        assert_eq!(
            classify_profile(Size::new(1_200.0, 1_043.0)),
            LayoutProfile::Standard
        );
    }

    #[test]
    fn context_contains_derived_geometry_and_2k_tokens() {
        let context = ResponsiveContext::new(Size::new(2_560.0, 1_440.0));

        assert_eq!(context.profile, LayoutProfile::Expanded);
        assert_approx(context.aspect_ratio, 16.0 / 9.0);
        assert_approx(context.tokens.rem(1.0), 64.0 / 3.0);
        assert_approx(context.tokens.space(16.0), 64.0 / 3.0);
        assert_approx(context.tokens.text(TextRole::Body), 56.0 / 3.0);
        assert_approx(context.tokens.reference_pixels(640.0), 480.0);
    }

    #[test]
    fn semantic_tokens_keep_the_same_ratio_at_full_and_half_width() {
        let half_1080 = ResponsiveContext::from_viewport(Size::new(960.0, 1_080.0));
        let half_2k = ResponsiveContext::from_viewport(Size::new(1_280.0, 1_440.0));

        assert_approx(
            half_2k.tokens.space(24.0) / half_1080.tokens.space(24.0),
            4.0 / 3.0,
        );
        assert_approx(
            half_2k.tokens.text(TextRole::Body) / half_1080.tokens.text(TextRole::Body),
            4.0 / 3.0,
        );
        assert_approx(
            half_2k.tokens.icon(IconRole::Medium) / half_1080.tokens.icon(IconRole::Medium),
            4.0 / 3.0,
        );
        assert_approx(
            half_2k.tokens.target(TargetRole::Control)
                / half_1080.tokens.target(TargetRole::Control),
            4.0 / 3.0,
        );
    }

    #[test]
    fn top_bar_and_sidebar_icons_keep_distinct_visual_and_target_contracts() {
        let reference = ResponsiveContext::from_viewport(Size::new(1_920.0, 1_080.0));
        let two_k = ResponsiveContext::from_viewport(Size::new(2_560.0, 1_440.0));

        assert_approx(reference.tokens.icon(IconRole::Sidebar), 24.0);
        assert_approx(reference.tokens.icon(IconRole::TopBarNavigation), 22.0);
        assert_approx(reference.tokens.icon(IconRole::TopBarAction), 18.0);
        assert_approx(reference.tokens.icon(IconRole::TopBarSearch), 18.0);
        assert_approx(reference.tokens.target(TargetRole::WindowControl), 42.0);
        assert!(
            reference.tokens.icon(IconRole::TopBarNavigation)
                < reference.tokens.target(TargetRole::WindowControl)
        );
        assert!(
            reference.tokens.icon(IconRole::TopBarAction)
                < reference.tokens.target(TargetRole::WindowControl)
        );
        assert!(
            reference.tokens.icon(IconRole::TopBarSearch)
                < reference.tokens.target(TargetRole::WindowControl)
        );
        assert_approx(
            two_k.tokens.icon(IconRole::TopBarNavigation)
                / reference.tokens.icon(IconRole::TopBarNavigation),
            4.0 / 3.0,
        );
    }

    #[test]
    fn playlist_card_density_preserves_eight_desktop_and_five_half_width_columns() {
        let full = ResponsiveContext::from_viewport(Size::new(1_920.0, 1_080.0));
        let two_k = ResponsiveContext::from_viewport(Size::new(2_560.0, 1_440.0));
        let maximized_two_k = ResponsiveContext::from_viewport(Size::new(2_558.0, 1_398.0));
        let half_1080 = ResponsiveContext::from_viewport(Size::new(960.0, 1_080.0));
        let half_2k = ResponsiveContext::from_viewport(Size::new(1_280.0, 1_440.0));
        let half_height_2k = ResponsiveContext::from_viewport(Size::new(1_280.0, 720.0));
        let small_tablet = ResponsiveContext::from_viewport(Size::new(768.0, 1_024.0));

        assert_approx(playlist_card_metrics(full).width, 174.0);
        assert_approx(playlist_card_metrics(two_k).width, 712.0 / 3.0);
        for (context, available_width) in [
            (full, 1_650.0),
            (two_k, 2_093.0),
            (maximized_two_k, 2_093.0),
        ] {
            let metrics = playlist_card_metrics(context);
            assert_eq!(
                calculate_grid_columns(available_width, metrics.width, metrics.gap),
                8
            );
        }
        let half_height = ResponsiveContext::from_viewport(Size::new(960.0, 540.0));
        assert_approx(playlist_card_metrics(half_1080).width, 151.2);
        assert_approx(playlist_card_metrics(half_height).width, 151.2);
        assert_approx(playlist_card_metrics(half_1080).gap, 18.0);
        assert_approx(playlist_card_metrics(half_2k).width, 179.2);
        assert_approx(playlist_card_metrics(half_height_2k).width, 179.2);
        let half_1080_grid = playlist_card_metrics_for_width(half_1080, 828.0);
        let half_2k_grid = playlist_card_metrics_for_width(half_2k, 1_104.0);
        let half_height_2k_grid = playlist_card_metrics_for_width(half_height_2k, 969.0);
        assert_approx(half_1080_grid.width, 151.2);
        assert_approx(half_2k_grid.width, 201.6);
        assert_approx(half_height_2k_grid.width, 179.2);
        assert_approx(half_2k_grid.width * 5.0 + half_2k_grid.gap * 4.0, 1_104.0);
        assert_approx(
            playlist_card_metrics(small_tablet).width,
            small_tablet.tokens.size(168.0),
        );
    }

    #[test]
    fn artwork_radii_and_home_feature_height_use_shared_tokens() {
        let tokens = UiTokens::default();

        assert_approx(tokens.cover_radius(CoverRadiusRole::Thumbnail), 10.0);
        assert_approx(tokens.cover_radius(CoverRadiusRole::Card), 18.0);
        assert_approx(tokens.cover_radius(CoverRadiusRole::Hero), 24.0);
        assert_approx(tokens.card(CardRole::Feature).height, 204.0);
        assert_approx(tokens.card(CardRole::Playlist).radius, 18.0);
        assert_approx(tokens.chrome(ChromeRole::PlayerBar), 88.0);
    }

    #[test]
    fn shared_theme_metrics_follow_the_same_root_rem() {
        let reference = ResponsiveContext::from_viewport(Size::new(1_920.0, 1_080.0))
            .tokens
            .theme_metrics();
        let two_k = ResponsiveContext::from_viewport(Size::new(2_560.0, 1_440.0))
            .tokens
            .theme_metrics();

        assert_approx(two_k.small_radius / reference.small_radius, 4.0 / 3.0);
        assert_approx(two_k.border_width / reference.border_width, 4.0 / 3.0);
        assert_approx(
            two_k.popup_shadow_blur / reference.popup_shadow_blur,
            4.0 / 3.0,
        );
    }

    #[test]
    fn token_floors_keep_compact_controls_and_text_readable() {
        let tokens = UiTokens::new(RootRem::from_scale(0.1));

        assert!(tokens.target(TargetRole::Icon) >= MIN_INTERACTION_TARGET);
        assert!(tokens.text(TextRole::Caption) >= 11.0);
        assert!(tokens.icon(IconRole::Small) >= 14.0);
    }

    #[test]
    fn bounded_geometry_never_exceeds_the_available_viewport() {
        assert_eq!(bounded_width(360.0, 800.0, 16.0), 360.0);
        assert_eq!(bounded_width(600.0, 400.0, 16.0), 368.0);
        assert_eq!(bounded_height(400.0, 300.0, 12.0), 276.0);

        assert_eq!(
            bounded_panel_size(
                Size::new(600.0, 400.0),
                Size::new(400.0, 300.0),
                16.0,
                12.0,
                40.0,
            ),
            Size::new(368.0, 236.0)
        );
        assert_eq!(bounded_width(f32::NAN, 400.0, 16.0), 0.0);
        assert_eq!(bounded_height(400.0, -1.0, 12.0), 0.0);
    }

    #[test]
    fn grid_math_handles_complete_columns_and_invalid_values() {
        assert_eq!(calculate_grid_columns(160.0, 160.0, 24.0), 1);
        assert_eq!(calculate_grid_columns(528.0, 160.0, 24.0), 3);
        assert_eq!(calculate_grid_columns(-1.0, 160.0, 24.0), 1);
        assert_eq!(calculate_grid_columns(500.0, 0.0, 24.0), 1);
        assert_eq!(calculate_grid_columns(f32::NAN, 160.0, 24.0), 1);
        assert_eq!(calculate_grid_columns_clamped(4_000.0, 160.0, 24.0, 5), 5);
    }

    #[test]
    fn detail_header_metrics_scale_without_changing_the_horizontal_contract() {
        let reference = detail_header_metrics(ResponsiveContext::from_viewport(Size::new(
            1_920.0, 1_080.0,
        )));
        let two_k = detail_header_metrics(ResponsiveContext::from_viewport(Size::new(
            2_560.0, 1_440.0,
        )));
        let standard = detail_header_metrics(ResponsiveContext::from_viewport(Size::new(
            1_280.0, 1_080.0,
        )));
        let half_width =
            detail_header_metrics(ResponsiveContext::from_viewport(Size::new(960.0, 1_080.0)));
        let narrow =
            detail_header_metrics(ResponsiveContext::from_viewport(Size::new(560.0, 800.0)));

        assert_approx(reference.artwork_size, 224.0);
        assert_approx(reference.title_size, 44.0);
        assert_approx(reference.prominent_title_size, 52.0);
        assert_approx(reference.top_padding, 76.0);
        assert_approx(two_k.artwork_size, 896.0 / 3.0);
        assert_approx(two_k.title_size, 176.0 / 3.0);
        assert_approx(two_k.prominent_title_size, 208.0 / 3.0);
        assert_approx(two_k.top_padding, 304.0 / 3.0);
        assert_approx(standard.title_size, 40.0);
        assert_approx(standard.prominent_title_size, 48.0);
        assert_approx(half_width.artwork_size, 184.0);
        assert_approx(half_width.title_size, 34.0);
        assert_approx(half_width.prominent_title_size, 44.0);
        assert_approx(half_width.top_padding, 58.0);
        assert_approx(narrow.artwork_size, 136.8);
        assert_approx(narrow.title_size, 27.0);
        assert_approx(narrow.prominent_title_size, 32.4);
        assert_approx(narrow.top_padding, 46.8);

        for viewport in [Size::new(1_280.0, 1_440.0), Size::new(720.0, 800.0)] {
            let metrics = detail_header_metrics(ResponsiveContext::from_viewport(viewport));
            assert!(metrics.artwork_size > 0.0);
            assert!(metrics.title_size >= 24.0);
        }
    }

    #[test]
    fn lyrics_layout_preserves_wide_split_and_focuses_only_when_width_is_insufficient() {
        for viewport in [Size::new(1_920.0, 1_080.0), Size::new(2_560.0, 1_440.0)] {
            assert_eq!(
                lyrics_page_layout(ResponsiveContext::from_viewport(viewport)),
                LyricsPageLayout::Split
            );
        }

        for viewport in [
            Size::new(960.0, 1_080.0),
            Size::new(720.0, 800.0),
            Size::new(960.0, 540.0),
            Size::new(560.0, 800.0),
        ] {
            assert_eq!(
                lyrics_page_layout(ResponsiveContext::from_viewport(viewport)),
                LyricsPageLayout::Focus
            );
        }
    }

    #[test]
    fn lyrics_media_width_is_shared_and_bounded_for_validation_viewports() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), 480.0),
            (Size::new(2_560.0, 1_440.0), 640.0),
            (Size::new(960.0, 1_080.0), 500.0),
            (Size::new(1_280.0, 1_440.0), 2_000.0 / 3.0),
            (Size::new(720.0, 800.0), 450.0),
            (Size::new(960.0, 540.0), 306.0),
            (Size::new(560.0, 800.0), 360.0),
        ];

        for (viewport, expected) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            let width = lyrics_media_width(context);
            assert_approx(width, expected);
            assert!(width <= viewport.width);
            assert!(width <= viewport.height);
        }
    }

    #[test]
    fn lyrics_media_restores_the_root_rem_target_at_two_k_half_width() {
        let half_1080 =
            lyrics_media_width(ResponsiveContext::from_viewport(Size::new(960.0, 1_080.0)));
        let half_2k = lyrics_media_width(ResponsiveContext::from_viewport(Size::new(
            1_280.0, 1_440.0,
        )));

        assert_approx(half_1080, 500.0);
        assert_approx(half_2k, 2_000.0 / 3.0);
    }

    #[test]
    fn wide_focus_lyrics_media_uses_height_cap_only_when_needed() {
        let half_height =
            lyrics_media_width(ResponsiveContext::from_viewport(Size::new(960.0, 540.0)));
        let full_height =
            lyrics_media_width(ResponsiveContext::from_viewport(Size::new(960.0, 1_080.0)));

        assert_approx(half_height, 306.0);
        assert_approx(full_height, 500.0);
        assert!(full_height > half_height);
    }

    #[test]
    fn shortcut_tables_change_pair_arrangement_without_changing_row_columns() {
        for viewport in [Size::new(1_920.0, 1_080.0), Size::new(2_560.0, 1_440.0)] {
            assert_eq!(
                shortcut_tables_layout(ResponsiveContext::from_viewport(viewport)),
                ShortcutTablesLayout::SideBySide
            );
        }

        for viewport in [
            Size::new(1_120.0, 900.0),
            Size::new(960.0, 1_080.0),
            Size::new(768.0, 1_024.0),
            Size::new(720.0, 800.0),
            Size::new(960.0, 540.0),
            Size::new(560.0, 800.0),
        ] {
            assert_eq!(
                shortcut_tables_layout(ResponsiveContext::from_viewport(viewport)),
                ShortcutTablesLayout::Stacked
            );
        }
    }

    #[test]
    fn validation_fixtures_cover_resize_profile_transitions() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), LayoutProfile::Expanded),
            (Size::new(2_560.0, 1_440.0), LayoutProfile::Expanded),
            (Size::new(960.0, 1_080.0), LayoutProfile::Tablet),
            (Size::new(768.0, 1_024.0), LayoutProfile::Tablet),
            (Size::new(720.0, 800.0), LayoutProfile::Tablet),
            (Size::new(960.0, 540.0), LayoutProfile::Compact),
            (Size::new(560.0, 800.0), LayoutProfile::Narrow),
        ];

        for (viewport, expected_profile) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            assert_eq!(context.profile, expected_profile);
            assert!(context.tokens.target(TargetRole::Control) >= MIN_INTERACTION_TARGET);
        }

        let transition = [
            Size::new(1_920.0, 1_080.0),
            Size::new(1_119.0, 700.0),
            Size::new(960.0, 540.0),
            Size::new(768.0, 1_024.0),
            Size::new(560.0, 800.0),
            Size::new(768.0, 1_024.0),
            Size::new(1_920.0, 1_080.0),
        ];
        let profiles: Vec<_> = transition
            .into_iter()
            .map(ResponsiveContext::from_viewport)
            .map(|context| context.profile)
            .collect();

        assert_eq!(
            profiles,
            vec![
                LayoutProfile::Expanded,
                LayoutProfile::Compact,
                LayoutProfile::Compact,
                LayoutProfile::Tablet,
                LayoutProfile::Narrow,
                LayoutProfile::Tablet,
                LayoutProfile::Expanded,
            ]
        );
    }

    #[test]
    fn drawer_presentation_keeps_navigation_available_in_compact_profiles() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), SidebarPresentation::Full),
            (Size::new(2_560.0, 1_440.0), SidebarPresentation::Full),
            (Size::new(960.0, 1_080.0), SidebarPresentation::Rail),
            (Size::new(768.0, 1_024.0), SidebarPresentation::Rail),
            (Size::new(720.0, 800.0), SidebarPresentation::Rail),
            (Size::new(960.0, 540.0), SidebarPresentation::Rail),
            (Size::new(560.0, 800.0), SidebarPresentation::Hidden),
        ];

        for (viewport, expected_presentation) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            assert_eq!(
                sidebar_presentation(context.profile, false),
                expected_presentation,
                "unexpected sidebar presentation for {viewport:?}"
            );
        }

        assert_eq!(
            sidebar_presentation(LayoutProfile::Compact, true),
            SidebarPresentation::Drawer
        );
        assert_eq!(
            sidebar_presentation(LayoutProfile::Tablet, true),
            SidebarPresentation::Drawer
        );
        assert_eq!(
            sidebar_presentation(LayoutProfile::Narrow, true),
            SidebarPresentation::Drawer
        );
    }

    #[test]
    fn top_bar_height_follows_the_root_rem() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), 68.0),
            (Size::new(2_560.0, 1_440.0), 272.0 / 3.0),
            (Size::new(960.0, 1_080.0), 68.0),
            (Size::new(1_280.0, 1_440.0), 272.0 / 3.0),
            (Size::new(768.0, 1_024.0), 8_704.0 / 135.0),
            (Size::new(720.0, 800.0), 61.2),
            (Size::new(960.0, 540.0), 61.2),
            (Size::new(560.0, 800.0), 61.2),
        ];

        for (viewport, expected_height) in fixtures {
            assert_approx(
                top_bar_height(&ResponsiveContext::from_viewport(viewport)),
                expected_height,
            );
        }
    }
}
