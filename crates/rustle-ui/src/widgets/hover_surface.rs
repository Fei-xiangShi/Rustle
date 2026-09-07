//! Self-contained hover background animation for arbitrary content.
//!
//! The widget keeps its animation state inside Iced's widget tree, so callers
//! do not need to add hover messages or application state for simple visual
//! transitions.

use std::time::Duration;

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::svg;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::mouse::{self, Cursor};
use iced::time::Instant;
use iced::widget::container;
use iced::{Color, Element, Event, Length, Rectangle, Size, Transformation, Vector};

const DEFAULT_DURATION: Duration = Duration::from_millis(160);
type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, f32) -> container::Style + 'a>;

/// Wraps content with a smoothly animated hover surface.
pub struct HoverSurface<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: renderer::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    style: StyleFn<'a, Theme>,
    duration: Duration,
    enabled: bool,
    hover_scale: f32,
    svg_overlay: Option<SvgOverlay>,
}

#[derive(Debug, Clone)]
struct SvgOverlay {
    handle: svg::Handle,
    size: Size<f32>,
    color: Color,
}

impl<'a, Message, Theme, Renderer> HoverSurface<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Creates a hover surface around the given content.
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            style: Box::new(|_, _| container::Style::default()),
            duration: DEFAULT_DURATION,
            enabled: true,
            hover_scale: 1.0,
            svg_overlay: None,
        }
    }

    /// Sets the style builder. `progress` moves from `0.0` to `1.0` on hover.
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, f32) -> container::Style + 'a) -> Self {
        self.style = Box::new(style);
        self
    }

    /// Enables or disables hover detection while preserving the base style.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Overrides the default transition duration.
    #[must_use]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Scales the rendered content around its center as hover progresses.
    ///
    /// The widget's layout and hit-test bounds remain unchanged, which makes
    /// this suitable for icon buttons embedded in fixed card/row slots.
    #[must_use]
    pub fn scale_on_hover(mut self, scale: f32) -> Self {
        self.hover_scale = if scale.is_finite() {
            scale.max(1.0)
        } else {
            1.0
        };
        self
    }

    /// Draws a fixed-size SVG centered above the content, using the same
    /// progress value as the hover background for its native opacity.
    #[must_use]
    pub fn svg_overlay(mut self, handle: svg::Handle, size: Size<f32>, color: Color) -> Self {
        self.svg_overlay = Some(SvgOverlay {
            handle,
            size,
            color,
        });
        self
    }
}

#[derive(Debug)]
struct State {
    progress: f32,
    from: f32,
    target: f32,
    started_at: Option<Instant>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            progress: 0.0,
            from: 0.0,
            target: 0.0,
            started_at: None,
        }
    }
}

impl State {
    fn set_target(&mut self, target: f32, now: Instant) -> bool {
        if (self.target - target).abs() < f32::EPSILON {
            return false;
        }

        self.from = self.progress;
        self.target = target;
        self.started_at = Some(now);
        true
    }

    fn tick(&mut self, now: Instant, duration: Duration) -> bool {
        let Some(started_at) = self.started_at else {
            return false;
        };

        let duration_secs = duration.as_secs_f32().max(f32::EPSILON);
        let elapsed = now.saturating_duration_since(started_at).as_secs_f32();
        let linear = (elapsed / duration_secs).clamp(0.0, 1.0);
        let eased = ease_out_cubic(linear);
        self.progress = self.from + (self.target - self.from) * eased;

        if linear >= 1.0 {
            self.progress = self.target;
            self.started_at = None;
            false
        } else {
            true
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for HoverSurface<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer + svg::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

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
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let target = if self.enabled && cursor.is_over(layout.bounds()) {
            1.0
        } else {
            0.0
        };

        if state.set_target(target, Instant::now()) {
            shell.request_redraw();
        }

        if let Event::Window(iced::window::Event::RedrawRequested(now)) = event
            && state.tick(*now, self.duration)
        {
            shell.request_redraw();
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clipped_viewport) = bounds.intersection(viewport) else {
            return;
        };
        let state = tree.state.downcast_ref::<State>();
        let style = (self.style)(theme, state.progress);
        let scale = hover_scale_at_progress(self.hover_scale, state.progress);
        let draw_contents =
            |renderer: &mut Renderer, cursor: Cursor, child_viewport: &Rectangle| {
                container::draw_background(renderer, &style, bounds);
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    &renderer::Style {
                        text_color: style.text_color.unwrap_or(renderer_style.text_color),
                    },
                    layout,
                    cursor,
                    child_viewport,
                );

                if let Some(overlay) = &self.svg_overlay {
                    let overlay_bounds = Rectangle {
                        x: bounds.center_x() - overlay.size.width / 2.0,
                        y: bounds.center_y() - overlay.size.height / 2.0,
                        width: overlay.size.width,
                        height: overlay.size.height,
                    };
                    renderer.draw_svg(
                        svg::Svg::new(overlay.handle.clone())
                            .color(overlay.color)
                            .opacity(state.progress),
                        overlay_bounds,
                        *child_viewport,
                    );
                }
            };

        if scale > 1.0 + f32::EPSILON {
            let transformation = Transformation::translate(bounds.center_x(), bounds.center_y())
                * Transformation::scale(scale)
                * Transformation::translate(-bounds.center_x(), -bounds.center_y());
            let inverse = transformation.inverse();
            let transformed_viewport = *viewport * inverse;

            renderer.with_transformation(transformation, |renderer| {
                draw_contents(renderer, cursor * inverse, &transformed_viewport);
            });
        } else {
            draw_contents(renderer, cursor, &clipped_viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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

impl<'a, Message, Theme, Renderer> From<HoverSurface<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + svg::Renderer + 'a,
{
    fn from(surface: HoverSurface<'a, Message, Theme, Renderer>) -> Self {
        Element::new(surface)
    }
}

/// Creates a self-contained animated hover surface.
pub fn hover_surface<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> HoverSurface<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer + svg::Renderer,
{
    HoverSurface::new(content)
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}

fn hover_scale_at_progress(max_scale: f32, progress: f32) -> f32 {
    1.0 + (max_scale.max(1.0) - 1.0) * progress.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{ease_out_cubic, hover_scale_at_progress};

    #[test]
    fn easing_keeps_hover_progress_bounded() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn hover_scale_interpolates_without_changing_layout_bounds() {
        assert_eq!(hover_scale_at_progress(1.06, 0.0), 1.0);
        assert_eq!(hover_scale_at_progress(1.06, 1.0), 1.06);
        assert_eq!(hover_scale_at_progress(0.8, 1.0), 1.0);
    }
}
