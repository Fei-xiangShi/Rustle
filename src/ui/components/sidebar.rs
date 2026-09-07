//! Left sidebar navigation component
//! Dark gray panel with logo, menu, library section, and user profile

use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, svg, text, tooltip,
};
use iced::{Alignment, Color, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message, Route, SidebarId};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::animation::{HoverAnimations, SmoothScrollTarget};
use crate::ui::components::importing_card::{self, ImportingPlaylist};
use crate::ui::components::window_drag_region;
use crate::ui::responsive::{
    ChromeRole, CoverRadiusRole, IconRole, RadiusRole, ResponsiveContext, SidebarPresentation,
    TargetRole, TextRole, bounded_width, sidebar_presentation,
};
use crate::ui::theme::{self, BOLD_WEIGHT};

// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
const SIDEBAR_ITEM_SPACING: f32 = 4.0;
const RAIL_PLAYLIST_SCROLL_ID: &str = "sidebar_rail_playlist_scroll";

#[derive(Debug, Clone, Copy)]
struct SidebarMetrics {
    text_size: f32,
    header_text_size: f32,
    title_size: f32,
    icon_size: f32,
    icon_gap: f32,
    item_spacing: f32,
    content_padding: f32,
    logo_icon_size: f32,
    logo_gap: f32,
    logo_bottom: f32,
    section_gap: f32,
    button_padding: f32,
    button_horizontal_padding: f32,
    header_padding: f32,
    content_bottom: f32,
    cover_size: f32,
    cover_radius: f32,
    target_size: f32,
    button_radius: f32,
    tokens: crate::ui::responsive::UiTokens,
}

impl SidebarMetrics {
    fn from_context(context: &ResponsiveContext) -> Self {
        let tokens = &context.tokens;
        Self {
            text_size: tokens.text(TextRole::BodyLarge),
            header_text_size: tokens.text(TextRole::Body),
            title_size: tokens.text(TextRole::Title),
            icon_size: tokens.icon(IconRole::Sidebar),
            icon_gap: tokens.space(16.0),
            item_spacing: tokens.space(SIDEBAR_ITEM_SPACING),
            content_padding: tokens.space(16.0),
            logo_icon_size: tokens.size(30.0),
            logo_gap: tokens.space(12.0),
            logo_bottom: tokens.space(32.0),
            section_gap: tokens.space(22.0),
            button_padding: tokens.space(10.0),
            button_horizontal_padding: tokens.space(14.0),
            header_padding: tokens.space(10.0),
            content_bottom: tokens.space(14.0),
            cover_size: tokens.size(42.0),
            cover_radius: tokens.cover_radius(CoverRadiusRole::Thumbnail),
            target_size: tokens.target(TargetRole::Icon),
            button_radius: tokens.radius(RadiusRole::Large),
            tokens: *tokens,
        }
    }
}

/// Navigation menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Home,
    Radio,
    Downloads,
    Settings,
    AudioEngine,
}

impl NavItem {
    pub fn i18n_key(&self) -> Key {
        match self {
            NavItem::Home => Key::NavHome,
            NavItem::Radio => Key::NavRadio,
            NavItem::Downloads => Key::NavDownloads,
            NavItem::Settings => Key::NavSettings,
            NavItem::AudioEngine => Key::NavAudioEngine,
        }
    }

    pub fn icon_svg(&self) -> &'static str {
        match self {
            NavItem::Home => crate::ui::icons::HOME,
            NavItem::Radio => crate::ui::icons::RADIO,
            NavItem::Downloads => crate::ui::icons::DOWNLOAD,
            NavItem::Settings => crate::ui::icons::SETTINGS,
            NavItem::AudioEngine => crate::ui::icons::EQUALIZER,
        }
    }
}

/// Library section items (local only now)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryItem {
    RecentlyPlayed,
}

impl LibraryItem {
    pub fn i18n_key(&self) -> Key {
        match self {
            LibraryItem::RecentlyPlayed => Key::LibraryRecentlyPlayed,
        }
    }

    pub fn icon_svg(&self) -> &'static str {
        match self {
            LibraryItem::RecentlyPlayed => crate::ui::icons::CLOCK,
        }
    }
}

pub struct SidebarView<'a> {
    pub current_route: &'a Route,
    pub locale: Locale,
    pub is_logged_in: bool,
    pub importing_playlist: Option<&'a ImportingPlaylist>,
    pub playlists: &'a [crate::database::DbPlaylist],
    pub user_playlists: &'a [crate::api::PlaylistSummary],
    pub image_state: &'a ImageState,
    pub sidebar_animations: &'a HoverAnimations<SidebarId>,
    pub sidebar_width: f32,
    pub my_playlists_expanded: bool,
    pub collected_playlists_expanded: bool,
    pub context: ResponsiveContext,
}

/// Build the profile-specific sidebar presentation.
pub fn view(props: SidebarView<'_>, drawer_open: bool) -> Element<'static, Message> {
    match sidebar_presentation(props.context.profile, drawer_open) {
        SidebarPresentation::Full => {
            let rendered_width = props
                .context
                .tokens
                .size(props.sidebar_width.clamp(240.0, 440.0));
            full_view(&props, rendered_width)
        }
        SidebarPresentation::Rail | SidebarPresentation::Drawer => rail_view(&props),
        SidebarPresentation::Hidden => Space::new().width(0).height(Fill).into(),
    }
}

/// Render the complete playlist/navigation drawer as an overlay surface.
pub fn drawer_view(props: SidebarView<'_>, transition_progress: f32) -> Element<'static, Message> {
    if !props.context.profile.uses_navigation_drawer() {
        return Space::new().width(0).height(0).into();
    }

    let drawer_width = bounded_width(
        props.context.tokens.size(props.sidebar_width.max(320.0)),
        props.context.width(),
        props.context.tokens.space(16.0),
    )
    .max(props.context.tokens.size(240.0).min(props.context.width()));
    let transition_progress = transition_progress.clamp(0.0, 1.0);
    let drawer = full_view(
        &props,
        // `full_view` receives rendered logical pixels (the same contract as
        // the desktop call above), so do not convert the token-scaled drawer
        // width back to reference units here.
        drawer_width,
    );
    let backdrop = mouse_area(
        container(Space::new().width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .style(move |theme| iced::widget::container::Style {
                background: Some(theme::overlay_backdrop(theme, 0.56 * transition_progress).into()),
                ..Default::default()
            }),
    )
    .on_press(Message::CloseSidebarDrawer);

    crate::ui::overlay::block_mouse_events(
        iced::widget::stack![
            backdrop,
            container(drawer)
                .width(Length::Fixed(drawer_width * transition_progress))
                .height(Fill)
                .clip(true)
                .style(theme::sidebar),
        ]
        .width(Fill)
        .height(Fill)
        .into(),
    )
}

fn full_view(props: &SidebarView<'_>, rendered_width: f32) -> Element<'static, Message> {
    let current_route = props.current_route;
    let locale = props.locale;
    let is_logged_in = props.is_logged_in;
    let importing_playlist = props.importing_playlist;
    let playlists = props.playlists;
    let user_playlists = props.user_playlists;
    let image_state = props.image_state;
    let sidebar_animations = props.sidebar_animations;
    let my_playlists_expanded = props.my_playlists_expanded;
    let collected_playlists_expanded = props.collected_playlists_expanded;
    let context = props.context;
    let sidebar_width = rendered_width.max(context.tokens.chrome(ChromeRole::Sidebar));
    let metrics = SidebarMetrics::from_context(&context);
    // Logo section
    let logo_content = row![
        // The music-note glyph carries slightly more visual weight on its
        // left. A small leading-only inset keeps the complete intrinsic group
        // optically centered without changing sidebar geometry.
        Space::new().width(context.tokens.space(4.0)),
        // Pink music icon
        container(
            svg(svg::Handle::from_memory(
                crate::ui::icons::MUSIC_LOGO.as_bytes()
            ))
            .width(metrics.logo_icon_size)
            .height(metrics.logo_icon_size)
            .style(|_theme, _status| svg::Style {
                color: Some(theme::ACCENT_PINK),
            })
        ),
        Space::new().width(metrics.logo_gap),
        container(
            text(locale.get(Key::AppName))
                .size(metrics.title_size)
                .style(|theme| text::Style {
                    color: Some(theme::text_primary(theme))
                })
                .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        )
        .padding(Padding::new(0.0).top(context.tokens.space(1.0)))
    ]
    .align_y(Alignment::Center);

    let logo = iced::widget::stack![
        container(logo_content)
            .width(Fill)
            .align_x(Alignment::Center)
            .padding(
                Padding::new(metrics.content_padding)
                    .left(0.0)
                    .right(0.0)
                    .bottom(metrics.logo_bottom),
            ),
        window_drag_region::view(context),
    ]
    .width(Fill);

    // Main navigation menu with hover animations
    let nav_items = [NavItem::Home, NavItem::Radio, NavItem::Downloads];
    let nav_menu = column(nav_items.into_iter().enumerate().map(|(idx, item)| {
        let is_active = matches!(current_route.nav_item(), Some(active) if active == item);
        let hover_progress = sidebar_animations.get_progress(&SidebarId::Nav(idx));
        sidebar_button_animated(
            item.icon_svg(),
            locale.get(item.i18n_key()).to_string(),
            is_active,
            hover_progress,
            SidebarId::Nav(idx),
            Message::Navigate(item),
            metrics,
        )
    }))
    .spacing(metrics.item_spacing);

    // Library section header
    let library_header = text(locale.get(Key::LibraryTitle))
        .size(metrics.header_text_size)
        .style(|theme| text::Style {
            color: Some(theme::text_muted(theme)),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
        .width(Fill);

    // Recently played button - use same animated style as nav buttons
    let recently_played_item = LibraryItem::RecentlyPlayed;
    let recently_played_progress = sidebar_animations.get_progress(&SidebarId::Library(0));
    let recently_played = sidebar_button_animated(
        recently_played_item.icon_svg(),
        locale.get(recently_played_item.i18n_key()).to_string(),
        matches!(current_route, Route::RecentlyPlayed),
        recently_played_progress,
        SidebarId::Library(0),
        Message::LibrarySelect(recently_played_item),
        metrics,
    );

    // Import local playlist button - use same animated style as nav buttons
    let import_progress = sidebar_animations.get_progress(&SidebarId::Library(1));
    let import_playlist_btn = sidebar_button_animated(
        crate::ui::icons::PLUS,
        locale.get(Key::ImportLocalPlaylist).to_string(),
        false, // not active
        import_progress,
        SidebarId::Library(1),
        Message::ImportLocalPlaylist,
        metrics,
    );

    // Build library section with proper spacing (same as nav_menu)
    let mut library_items: Vec<Element<'static, Message>> = vec![recently_played];

    // Show importing playlist if any
    if let Some(playlist) = importing_playlist {
        library_items.push(importing_card::view(playlist, context.tokens));
    }

    // Show local playlists with hover animations
    for playlist in playlists {
        let name = playlist.name.clone();
        let id = playlist.id;
        let is_active = matches!(current_route, Route::Playlist(current_id) if *current_id == id);
        let hover_progress = sidebar_animations.get_progress(&SidebarId::Playlist(id));
        let cover_handle = u64::try_from(id)
            .ok()
            .and_then(|id| image_state.get(ImageKind::LocalPlaylistCover, id));
        let cover_el = Some(crate::ui::components::cover_image::custom(
            cover_handle,
            ImageKind::LocalPlaylistCover,
            metrics.cover_size,
            metrics.cover_radius,
            context.tokens,
        ));
        library_items.push(sidebar_button_animated_opt_cover(SidebarButtonView {
            fallback_svg: crate::ui::icons::MUSIC,
            cover_icon: cover_el,
            label: name,
            is_active,
            hover_progress,
            sidebar_id: SidebarId::Playlist(id),
            on_press: Message::OpenPlaylist(id),
            metrics,
        }));
    }

    library_items.push(import_playlist_btn);

    // Library section with spacing matching nav_menu
    let library_section = column(library_items).spacing(metrics.item_spacing);

    // Build scrollable content (only library and cloud playlists, not logo/nav)
    let mut scrollable_items: Vec<Element<'static, Message>> = vec![
        sidebar_divider(metrics),
        Space::new().height(metrics.header_padding).into(),
        container(library_header)
            .padding(
                Padding::new(0.0)
                    .left(metrics.content_padding)
                    .bottom(metrics.header_padding),
            )
            .into(),
        library_section.into(),
    ];

    // Only show cloud playlists section if logged in
    if is_logged_in {
        // Split user playlists into owned and collected
        let (owned_playlists, collected_playlists): (Vec<_>, Vec<_>) =
            user_playlists.iter().partition(|pl| !pl.subscribed);

        // Helper to render a playlist button
        let render_playlist_btn =
            |playlist: &crate::api::PlaylistSummary| -> Element<'static, Message> {
                let name = playlist.name.clone();
                let id = playlist.id;
                let is_active =
                    matches!(current_route, Route::NcmPlaylist(current_id) if *current_id == id);
                let hover_progress = sidebar_animations.get_progress(&SidebarId::UserPlaylist(id));

                let cover_el = crate::ui::components::cover_image::custom(
                    image_state.get(ImageKind::PlaylistCover, id),
                    ImageKind::PlaylistCover,
                    metrics.cover_size,
                    metrics.cover_radius,
                    context.tokens,
                );
                sidebar_button_animated_opt_cover(SidebarButtonView {
                    fallback_svg: crate::ui::icons::MUSIC,
                    cover_icon: Some(cover_el),
                    label: name,
                    is_active,
                    hover_progress,
                    sidebar_id: SidebarId::UserPlaylist(id),
                    on_press: Message::OpenNcmPlaylist(id),
                    metrics,
                })
            };

        scrollable_items.push(Space::new().height(context.tokens.space(10.0)).into());
        scrollable_items.push(sidebar_divider(metrics));
        scrollable_items.push(Space::new().height(context.tokens.space(10.0)).into());

        // "My Playlists" section (owned)
        scrollable_items.push(collapsible_section_header(
            locale.get(Key::CloudPlaylistsTitle).to_string(),
            my_playlists_expanded,
            Message::ToggleMyPlaylistsSection,
            metrics,
        ));
        if my_playlists_expanded {
            let my_items: Vec<Element<'static, Message>> = owned_playlists
                .into_iter()
                .map(render_playlist_btn)
                .collect();
            scrollable_items.push(column(my_items).spacing(metrics.item_spacing).into());
        }

        // "Collected Playlists" section
        if !collected_playlists.is_empty() {
            scrollable_items.push(Space::new().height(context.tokens.space(18.0)).into());
            scrollable_items.push(collapsible_section_header(
                locale.get(Key::CollectedPlaylistsTitle).to_string(),
                collected_playlists_expanded,
                Message::ToggleCollectedPlaylistsSection,
                metrics,
            ));
            if collected_playlists_expanded {
                let collected_items: Vec<Element<'static, Message>> = collected_playlists
                    .into_iter()
                    .map(render_playlist_btn)
                    .collect();
                scrollable_items.push(column(collected_items).spacing(metrics.item_spacing).into());
            }
        }
    }

    // Scrollable area for library and cloud playlists only (hidden scrollbar)
    let scrollable_content = crate::ui::widgets::smooth_scroll(
        scrollable(
            column(scrollable_items).padding(Padding::new(0.0).bottom(metrics.content_bottom)),
        )
        .height(Fill)
        .direction(crate::ui::widgets::hidden_vertical_scrollbar())
        .id(iced::widget::Id::new("sidebar_scroll"))
        .style(hidden_scrollable_style),
        SmoothScrollTarget::Native("sidebar_scroll"),
        context.tokens,
        Message::SmoothScroll,
    );

    let top_content = column![
        logo,
        nav_menu,
        Space::new().height(metrics.section_gap),
        scrollable_content,
    ]
    .padding(Padding::new(metrics.content_padding).bottom(0.0))
    .width(sidebar_width)
    .height(Fill);

    let content = container(top_content).width(sidebar_width).height(Fill);

    // Wrap entire sidebar in mouse_area to clear hover when leaving sidebar
    let sidebar_container = container(content)
        .width(sidebar_width)
        .height(Fill)
        .style(theme::sidebar);

    mouse_area(sidebar_container)
        .on_exit(Message::HoverSidebar(None))
        .into()
}

fn sidebar_divider(metrics: SidebarMetrics) -> Element<'static, Message> {
    container(
        container(Space::new().height(metrics.tokens.size(1.0)))
            .width(Fill)
            .style(|theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::divider(theme))),
                ..Default::default()
            }),
    )
    .width(Fill)
    .padding(
        Padding::new(0.0)
            .left(metrics.content_padding)
            .right(metrics.content_padding),
    )
    .into()
}

/// Render the compact icon rail with the same playlist destinations as the
/// full sidebar. The rail only maps existing data and messages; responsive
/// policy remains unaware of playlist or image state.
fn rail_view(props: &SidebarView<'_>) -> Element<'static, Message> {
    let current_route = props.current_route;
    let locale = props.locale;
    let is_logged_in = props.is_logged_in;
    let playlists = props.playlists;
    let user_playlists = props.user_playlists;
    let image_state = props.image_state;
    let sidebar_animations = props.sidebar_animations;
    let context = props.context;
    let metrics = SidebarMetrics::from_context(&context);
    let rail_width = context.tokens.chrome(ChromeRole::SidebarRail);

    let menu_button = rail_button(
        crate::ui::icons::LIST,
        locale.get(Key::NavigationMenu).to_string(),
        false,
        0.0,
        metrics,
        SidebarId::Library(usize::MAX),
        Message::ToggleSidebarDrawer,
    );

    let nav_items = [NavItem::Home, NavItem::Radio, NavItem::Downloads];
    let nav_buttons: Vec<Element<'static, Message>> = nav_items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let active = current_route.nav_item() == Some(item);
            rail_button(
                item.icon_svg(),
                locale.get(item.i18n_key()).to_string(),
                active,
                sidebar_animations.get_progress(&SidebarId::Nav(idx)),
                metrics,
                SidebarId::Nav(idx),
                Message::Navigate(item),
            )
        })
        .collect();

    let recently_played = rail_button(
        LibraryItem::RecentlyPlayed.icon_svg(),
        locale.get(Key::LibraryRecentlyPlayed).to_string(),
        matches!(current_route, Route::RecentlyPlayed),
        sidebar_animations.get_progress(&SidebarId::Library(0)),
        metrics,
        SidebarId::Library(0),
        Message::LibrarySelect(LibraryItem::RecentlyPlayed),
    );

    let mut playlist_buttons = playlists
        .iter()
        .map(|playlist| {
            let id = playlist.id;
            let cover_handle = u64::try_from(id)
                .ok()
                .and_then(|id| image_state.get(ImageKind::LocalPlaylistCover, id));
            rail_cover_button(RailCoverButtonView {
                cover_handle,
                image_kind: ImageKind::LocalPlaylistCover,
                label: playlist.name.clone(),
                is_active: matches!(current_route, Route::Playlist(current_id) if *current_id == id),
                hover_progress: sidebar_animations.get_progress(&SidebarId::Playlist(id)),
                metrics,
                sidebar_id: SidebarId::Playlist(id),
                on_press: Message::OpenPlaylist(id),
            })
        })
        .collect::<Vec<_>>();

    if is_logged_in {
        playlist_buttons.extend(user_playlists.iter().map(|playlist| {
            let id = playlist.id;
            rail_cover_button(RailCoverButtonView {
                cover_handle: image_state.get(ImageKind::PlaylistCover, id),
                image_kind: ImageKind::PlaylistCover,
                label: playlist.name.clone(),
                is_active: matches!(current_route, Route::NcmPlaylist(current_id) if *current_id == id),
                hover_progress: sidebar_animations.get_progress(&SidebarId::UserPlaylist(id)),
                metrics,
                sidebar_id: SidebarId::UserPlaylist(id),
                on_press: Message::OpenNcmPlaylist(id),
            })
        }));
    }

    let playlist_surface: Element<'static, Message> = crate::ui::widgets::smooth_scroll(
        scrollable(
            column(playlist_buttons)
                .spacing(metrics.item_spacing)
                .align_x(Alignment::Center)
                .padding(Padding::new(0.0).bottom(metrics.content_bottom)),
        )
        .width(rail_width)
        .height(Fill)
        .direction(crate::ui::widgets::hidden_vertical_scrollbar())
        .id(iced::widget::Id::new(RAIL_PLAYLIST_SCROLL_ID))
        .style(hidden_scrollable_style),
        SmoothScrollTarget::Native(RAIL_PLAYLIST_SCROLL_ID),
        context.tokens,
        Message::SmoothScroll,
    )
    .into();

    let content = column![
        menu_button,
        Space::new().height(metrics.section_gap),
        column(nav_buttons).spacing(metrics.item_spacing),
        Space::new().height(metrics.section_gap),
        recently_played,
        Space::new().height(metrics.item_spacing),
        playlist_surface,
    ]
    .align_x(Alignment::Center)
    .padding(Padding::new(metrics.content_padding / 2.0))
    .width(rail_width)
    .height(Fill);

    mouse_area(
        container(content)
            .width(rail_width)
            .height(Fill)
            .style(theme::sidebar),
    )
    .on_exit(Message::HoverSidebar(None))
    .into()
}

struct RailCoverButtonView<'a> {
    cover_handle: Option<&'a iced::widget::image::Handle>,
    image_kind: ImageKind,
    label: String,
    is_active: bool,
    hover_progress: f32,
    metrics: SidebarMetrics,
    sidebar_id: SidebarId,
    on_press: Message,
}

fn rail_cover_button(view: RailCoverButtonView<'_>) -> Element<'static, Message> {
    let RailCoverButtonView {
        cover_handle,
        image_kind,
        label,
        is_active,
        hover_progress,
        metrics,
        sidebar_id,
        on_press,
    } = view;
    let button_size = (metrics.cover_size + metrics.item_spacing).max(metrics.target_size);
    let cover = crate::ui::components::cover_image::custom(
        cover_handle,
        image_kind,
        metrics.cover_size,
        metrics.cover_radius,
        metrics.tokens,
    );
    let button = button(
        container(cover)
            .width(button_size)
            .height(button_size)
            .center_x(button_size)
            .center_y(button_size),
    )
    .width(button_size)
    .height(button_size)
    .padding(0)
    .style(move |theme, _status| iced::widget::button::Style {
        background: Some(iced::Background::Color(theme::hover_bg_alpha(
            theme,
            if is_active {
                0.18
            } else {
                0.12 * hover_progress
            },
        ))),
        border: iced::Border {
            radius: (metrics.cover_radius + metrics.item_spacing / 2.0).into(),
            width: if is_active {
                metrics.tokens.size(1.0)
            } else {
                0.0
            },
            color: theme::ACCENT_PINK,
        },
        ..Default::default()
    })
    .on_press(on_press);

    let button: Element<'static, Message> = if is_active {
        button.into()
    } else {
        mouse_area(button)
            .on_enter(Message::HoverSidebar(Some(sidebar_id)))
            .on_exit(Message::HoverSidebar(None))
            .into()
    };

    tooltip(
        button,
        text(label).size(metrics.header_text_size),
        tooltip::Position::Right,
    )
    .padding(metrics.tokens.space(5.0))
    .into()
}

fn hidden_scrollable_style(_theme: &iced::Theme, _status: scrollable::Status) -> scrollable::Style {
    scrollable::Style {
        container: iced::widget::container::Style::default(),
        vertical_rail: scrollable::Rail {
            background: None,
            border: iced::Border::default(),
            scroller: scrollable::Scroller {
                background: iced::Background::Color(Color::TRANSPARENT),
                border: iced::Border::default(),
            },
        },
        horizontal_rail: scrollable::Rail {
            background: None,
            border: iced::Border::default(),
            scroller: scrollable::Scroller {
                background: iced::Background::Color(Color::TRANSPARENT),
                border: iced::Border::default(),
            },
        },
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: iced::Background::Color(Color::TRANSPARENT),
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            icon: Color::TRANSPARENT,
        },
    }
}

fn rail_button(
    icon_svg: &'static str,
    label: String,
    is_active: bool,
    hover_progress: f32,
    metrics: SidebarMetrics,
    sidebar_id: SidebarId,
    on_press: Message,
) -> Element<'static, Message> {
    let icon = svg(svg::Handle::from_memory(icon_svg.as_bytes()))
        .width(metrics.icon_size)
        .height(metrics.icon_size)
        .style(move |theme, _status| svg::Style {
            color: Some(if is_active {
                theme::text_primary(theme)
            } else {
                theme::animated_brightness(theme, hover_progress)
            }),
        });
    let icon: Element<'static, Message> = container(icon)
        .center_x(metrics.target_size)
        .center_y(metrics.target_size)
        .into();
    let button = button(icon)
        .width(metrics.target_size)
        .height(metrics.target_size)
        .padding(0)
        .style(move |theme, _status| iced::widget::button::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(
                theme,
                if is_active {
                    0.12
                } else {
                    0.12 * hover_progress
                },
            ))),
            border: iced::Border {
                radius: (metrics.target_size / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .on_press(on_press);

    let button: Element<'static, Message> = if is_active {
        button.into()
    } else {
        mouse_area(button)
            .on_enter(Message::HoverSidebar(Some(sidebar_id)))
            .on_exit(Message::HoverSidebar(None))
            .into()
    };

    tooltip(
        button,
        text(label).size(metrics.header_text_size),
        tooltip::Position::Right,
    )
    .padding(metrics.tokens.space(5.0))
    .into()
}

/// Create a compact clickable header for a collapsible cloud playlist section.
fn collapsible_section_header(
    label: String,
    is_expanded: bool,
    on_press: Message,
    metrics: SidebarMetrics,
) -> Element<'static, Message> {
    let chevron = if is_expanded {
        crate::ui::icons::CHEVRON_DOWN
    } else {
        crate::ui::icons::CHEVRON_RIGHT
    };

    let content = row![
        text(label)
            .size(metrics.header_text_size)
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            })
            .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT))
            .width(Fill),
        svg(svg::Handle::from_memory(chevron.as_bytes()))
            .width(metrics.header_text_size)
            .height(metrics.header_text_size)
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_muted(theme)),
            }),
    ]
    .align_y(Alignment::Center);

    let header = button(content)
        .width(Fill)
        .padding(
            Padding::new(metrics.content_padding / 4.0)
                .left(metrics.content_padding)
                .right(metrics.content_padding * 0.75),
        )
        .style(move |theme, status| iced::widget::button::Style {
            background: match status {
                iced::widget::button::Status::Hovered => {
                    Some(iced::Background::Color(theme::hover_bg_alpha(theme, 0.06)))
                }
                _ => None,
            },
            border: iced::Border {
                radius: metrics.button_radius.into(),
                ..Default::default()
            },
            text_color: theme::text_muted(theme),
            ..Default::default()
        })
        .on_press(on_press);

    // Keep the section rhythm while moving 2 px per side outside the hover
    // surface, so the highlighted area itself is less tall.
    container(header)
        .width(Fill)
        .padding(
            Padding::new(metrics.content_padding / 8.0)
                .left(0.0)
                .right(0.0),
        )
        .into()
}

/// Create an animated sidebar button with hover transition
/// Used for both navigation items and playlist items
fn sidebar_button_animated(
    icon_svg: &'static str,
    label: String,
    is_active: bool,
    hover_progress: f32,
    sidebar_id: SidebarId,
    on_press: Message,
    metrics: SidebarMetrics,
) -> Element<'static, Message> {
    sidebar_button_animated_opt_cover(SidebarButtonView {
        fallback_svg: icon_svg,
        cover_icon: None,
        label,
        is_active,
        hover_progress,
        sidebar_id,
        on_press,
        metrics,
    })
}

struct SidebarButtonView {
    fallback_svg: &'static str,
    cover_icon: Option<Element<'static, Message>>,
    label: String,
    is_active: bool,
    hover_progress: f32,
    sidebar_id: SidebarId,
    on_press: Message,
    metrics: SidebarMetrics,
}

fn sidebar_button_animated_opt_cover(view: SidebarButtonView) -> Element<'static, Message> {
    let SidebarButtonView {
        fallback_svg,
        cover_icon,
        label,
        is_active,
        hover_progress,
        sidebar_id,
        on_press,
        metrics,
    } = view;
    let icon: Element<'static, Message> = match cover_icon {
        Some(el) => el,
        None => svg(svg::Handle::from_memory(fallback_svg.as_bytes()))
            .width(metrics.icon_size)
            .height(metrics.icon_size)
            .style(move |theme, _status| svg::Style {
                color: Some(if is_active {
                    theme::text_primary(theme)
                } else {
                    theme::animated_brightness(theme, hover_progress)
                }),
            })
            .into(),
    };

    let label_text = text(label)
        .size(metrics.text_size)
        .style(move |theme| text::Style {
            color: Some(if is_active {
                theme::text_primary(theme)
            } else {
                theme::animated_brightness(theme, hover_progress)
            }),
        })
        .font(iced::Font::DEFAULT.weight(BOLD_WEIGHT));

    let content = row![icon, Space::new().width(metrics.icon_gap), label_text]
        .align_y(Alignment::Center)
        .padding(
            Padding::new(metrics.button_padding)
                .left(metrics.button_horizontal_padding)
                .right(metrics.button_horizontal_padding),
        );

    // Use button for proper click feedback and cursor
    let btn = button(content)
        .width(Fill)
        .padding(0)
        .style(move |theme, _status| {
            let bg_alpha = if is_active {
                0.12
            } else {
                0.12 * hover_progress
            };
            iced::widget::button::Style {
                background: Some(iced::Background::Color(theme::hover_bg_alpha(
                    theme, bg_alpha,
                ))),
                border: iced::Border {
                    radius: metrics.button_radius.into(),
                    ..Default::default()
                },
                text_color: theme::text_primary(theme),
                ..Default::default()
            }
        })
        .on_press(on_press.clone());

    // Add hover events if not active
    // Each button needs on_exit to clear hover when mouse leaves
    if is_active {
        btn.into()
    } else {
        mouse_area(btn)
            .on_enter(Message::HoverSidebar(Some(sidebar_id)))
            .on_exit(Message::HoverSidebar(None))
            .into()
    }
}
