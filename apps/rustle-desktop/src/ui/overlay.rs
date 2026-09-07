//! Overlay system — unified abstraction for modals, popups, and floating panels.
//!
//! # Architecture
//!
//! The overlay module provides:
//! - **backdrop**: Semi-transparent full-screen click-to-dismiss layer with event isolation
//! - **modal**: Unified centered modal dialog component (backdrop + panel + opaque)
//!
//! # Event Isolation
//!
//! Blocking overlays use [`block_mouse_events`] to prevent cursor, click, and scroll events
//! from penetrating through to widgets beneath while preserving interactions inside the
//! overlay itself.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::border::Radius;
use iced::mouse::{self, Cursor, Interaction};
use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Rectangle, Shadow, Size, Vector,
};

use crate::app::Message;
use crate::ui::responsive::{
    RadiusRole, ResponsiveContext, TargetRole, TextRole, bounded_height, bounded_width,
};
use crate::ui::{theme, widgets};

// ============================================================================
// ModalKind — Enum covering all modal dialog types with typed payloads
// ============================================================================

/// Each variant holds the specific data needed to render and handle that modal.
#[derive(Debug, Clone)]
pub enum ModalKind {
    /// Editable song metadata form (title, artist, album, year, etc.).
    SongEdit(crate::app::SongEditDialogState),
    /// Edit playlist form (name, description, cover, watch folder).
    PlaylistEdit {
        playlist_id: i64,
        name: String,
        description: String,
        cover_path: Option<String>,
        watch_enabled: bool,
        watch_available: bool,
        watch_path: Option<String>,
    },
    /// Delete playlist confirmation prompt.
    DeleteConfirm {
        playlist_id: i64,
        playlist_name: String,
    },
    /// Batch download confirmation prompt.
    DownloadConfirm {
        playlist_id: i64,
        playlist_name: String,
        song_count: u32,
    },
    /// Exit application confirmation prompt.
    ExitConfirm { remember_choice: bool },
    /// Pick a target playlist for adding a song.
    PlaylistPicker {
        song_id: i64,
        /// NCM online playlists (user's created + subscribed); None for local songs
        ncm_playlists: Option<Vec<crate::api::PlaylistSummary>>,
    },
}

// ============================================================================
// OverlayKind + OverlayEntry — Stack-based overlay management
// ============================================================================

/// The kind of overlay currently active.
#[derive(Debug, Clone)]
pub enum OverlayKind {
    Modal(ModalKind, ModalConfig),
}

/// A single entry in the overlay stack (LIFO: last = topmost).
#[derive(Debug, Clone)]
pub struct OverlayEntry {
    pub kind: OverlayKind,
}

impl OverlayEntry {
    pub fn new(kind: OverlayKind) -> Self {
        Self { kind }
    }
}

// ============================================================================
// Backdrop — Semi-transparent event-blocking layer
// ============================================================================

fn backdrop_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 0.6)
}

/// Prevent pointer interaction from reaching layers below an overlay.
///
/// [`iced::widget::opaque`] only captures left-button presses. Overlay surfaces also need to
/// own right/middle-button and scroll events so a context menu or modal cannot accidentally
/// trigger controls underneath it. The wrapped content is updated first, which means buttons
/// and other interactive controls inside the overlay retain their normal behavior.
pub fn block_mouse_events<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    Element::new(BlockMouseEvents { content })
}

/// A pointer barrier for a stack layer.
///
/// Iced's built-in `opaque` follows the documented semantics of capturing button presses only.
/// That is enough for ordinary overlays, but it still lets a lower custom widget inspect raw
/// `CursorMoved` events. This wrapper follows the same update order as `opaque`—the content gets
/// the event first—then captures every pointer event that was not handled by the content itself.
/// Consequently, controls inside the overlay continue to work while widgets below it never see
/// the event.
struct BlockMouseEvents<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for BlockMouseEvents<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );

        if shell.is_event_captured() || !pointer_is_over(event, cursor, layout.bounds()) {
            return;
        }

        if is_pointer_event(event) {
            shell.capture_event();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> Interaction {
        let interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );

        if interaction == Interaction::None && cursor.is_over(layout.bounds()) {
            Interaction::Idle
        } else {
            interaction
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<BlockMouseEvents<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(blocker: BlockMouseEvents<'a, Message, Theme, Renderer>) -> Self {
        Element::new(blocker)
    }
}

fn is_pointer_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(
            mouse::Event::CursorEntered
                | mouse::Event::CursorMoved { .. }
                | mouse::Event::ButtonPressed(_)
                | mouse::Event::ButtonReleased(_)
                | mouse::Event::WheelScrolled { .. }
        ) | Event::Touch(_)
    )
}

fn pointer_is_over(event: &Event, cursor: Cursor, bounds: Rectangle) -> bool {
    match event {
        // Use Iced's cursor state instead of the raw event position here. A levitating cursor
        // means another layer above this widget already owns the pointer, so this layer must not
        // capture the event on behalf of itself.
        Event::Mouse(mouse::Event::CursorMoved { .. }) => cursor.is_over(bounds),
        Event::Touch(touch_event) => match touch_event {
            iced::touch::Event::FingerPressed { position, .. }
            | iced::touch::Event::FingerMoved { position, .. }
            | iced::touch::Event::FingerLifted { position, .. }
            | iced::touch::Event::FingerLost { position, .. } => bounds.contains(*position),
        },
        _ => cursor.is_over(bounds),
    }
}

// ============================================================================
// Modal — Unified centered dialog component
// ============================================================================

/// Configuration for a modal dialog.
#[derive(Debug, Clone)]
pub struct ModalConfig {
    /// Width in 1080P reference pixels, resolved through the current root rem.
    pub width: f32,
    /// Whether clicking the backdrop dismisses the modal.
    pub backdrop_dismiss: bool,
    /// Whether pressing Escape dismisses the modal.
    pub escape_close: bool,
    /// Border radius in 1080P reference pixels, resolved through root rem.
    pub border_radius: f32,
}

impl Default for ModalConfig {
    fn default() -> Self {
        Self {
            width: 480.0,
            backdrop_dismiss: true,
            escape_close: true,
            border_radius: 16.0,
        }
    }
}

impl ModalConfig {
    pub fn width(mut self, reference_width: f32) -> Self {
        self.width = reference_width;
        self
    }

    pub fn no_backdrop_dismiss(mut self) -> Self {
        self.backdrop_dismiss = false;
        self
    }
}

/// Render a modal bounded by the current logical viewport.
pub fn modal_view<'a>(
    context: ResponsiveContext,
    config: &ModalConfig,
    content: Element<'a, Message>,
    on_dismiss: Message,
) -> Element<'a, Message> {
    let bg = backdrop_color();
    let tokens = context.tokens;
    let panel_width = bounded_width(
        tokens.size(config.width),
        context.width(),
        tokens.space(16.0),
    );
    let compact_height = bounded_height(tokens.size(720.0), context.height(), tokens.space(56.0));
    let border_radius = tokens.size(config.border_radius);

    let content = if context.profile.is_compact() {
        crate::ui::widgets::scaled_scroll(
            scrollable(container(content).width(Length::Fill))
                .direction(crate::ui::widgets::vertical_scrollbar(tokens))
                .id(iced::widget::Id::new("modal_body_scroll"))
                .height(Length::Fixed(compact_height)),
            tokens,
        )
        .into()
    } else {
        content
    };

    let panel = opaque(
        container(content)
            .width(Length::Fixed(panel_width))
            .style(move |t| container::Style {
                background: Some(Background::Color(theme::surface(t))),
                border: Border {
                    color: theme::divider(t),
                    width: tokens.size(1.0),
                    radius: Radius::new(border_radius),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    offset: iced::Vector::new(0.0, tokens.size(8.0)),
                    blur_radius: tokens.size(32.0),
                },
                ..Default::default()
            }),
    );

    let backdrop_container = container(panel)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            ..Default::default()
        });

    let inner: Element<'a, Message> = if config.backdrop_dismiss {
        mouse_area(backdrop_container).on_press(on_dismiss).into()
    } else {
        backdrop_container.into()
    };

    block_mouse_events(inner)
}

// ============================================================================
// Unified Modal Layout — Header / Body / Footer with dividers
// ============================================================================

/// Header bar with rem-aware typography and a stable close hit target.
pub fn modal_header(
    context: ResponsiveContext,
    title: String,
    on_close: Message,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let close_size = tokens.target(TargetRole::WindowControl);
    let close_button = button(
        container(
            text("✕")
                .size(tokens.text(TextRole::Subtitle))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme)),
                }),
        )
        .width(close_size)
        .height(close_size)
        .center_x(close_size)
        .center_y(close_size),
    )
    .style(close_btn_style)
    .padding(0)
    .on_press(on_close);
    let close_button =
        widgets::hover_surface(close_button).style(move |theme, progress| container::Style {
            background: Some(Background::Color(theme::hover_bg_alpha(
                theme,
                0.08 * progress,
            ))),
            border: Border {
                radius: tokens.radius(RadiusRole::Small).into(),
                ..Default::default()
            },
            ..Default::default()
        });

    container(
        row![
            text(title)
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| text::Style {
                    color: Some(theme::text_primary(theme))
                }),
            iced::widget::Space::new().width(Length::Fill),
            close_button,
        ]
        .align_y(Alignment::Center)
        .padding([tokens.space(14.0), tokens.space(20.0)]),
    )
    .into()
}

/// Footer bar with token-scaled spacing.
pub fn modal_footer<'a>(
    context: ResponsiveContext,
    buttons: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let mut row_children: Vec<Element<'a, Message>> =
        vec![iced::widget::Space::new().width(Length::Fill).into()];
    row_children.extend(buttons);
    let btn_row = row(row_children)
        .align_y(Alignment::Center)
        .spacing(tokens.space(12.0))
        .padding([tokens.space(14.0), tokens.space(20.0)]);
    column![divider(context), btn_row].into()
}

/// Content area with rem-aware standard padding.
pub fn modal_body<'a>(
    context: ResponsiveContext,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    container(content)
        .padding(context.tokens.space(24.0))
        .into()
}

/// Full modal layout with rem-aware header, body, and footer spacing.
pub fn modal_section<'a>(
    context: ResponsiveContext,
    title: String,
    on_close: Message,
    body: Element<'a, Message>,
    footer: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        modal_header(context, title, on_close),
        divider(context),
        body,
        footer
    ]
    .into()
}

fn divider(context: ResponsiveContext) -> Element<'static, Message> {
    let line_size = context.tokens.space(1.0);
    container(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(line_size),
    )
    .height(Length::Fixed(line_size))
    .style(|theme| container::Style {
        background: Some(Background::Color(theme::divider(theme))),
        ..Default::default()
    })
    .into()
}

fn close_btn_style(_: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    }
}
