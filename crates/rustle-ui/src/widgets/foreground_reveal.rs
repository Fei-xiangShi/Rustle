//! Draw-only centered reveal for arbitrary foreground content.
//!
//! Layout and interaction geometry stay unchanged. Only the child's recorded
//! draw output is clipped, which makes this suitable for component transitions
//! above a persistent background without forcing expensive child relayouts.

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// Clips foreground content vertically around its center while preserving the
/// child's normal layout and event surface.
pub struct ForegroundReveal<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    reveal: f32,
}

impl<'a, Message, Theme, Renderer> ForegroundReveal<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Creates a foreground reveal where `0.0` is fully hidden and `1.0` is
    /// fully visible.
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, reveal: f32) -> Self {
        Self {
            content: content.into(),
            reveal: normalized_reveal(reveal),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ForegroundReveal<'_, Message, Theme, Renderer>
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
        shell: &mut Shell<'_, Message>,
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
        if self.reveal >= 1.0 - f32::EPSILON {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
            return;
        }

        let Some(reveal_bounds) = centered_vertical_reveal(layout.bounds(), self.reveal) else {
            return;
        };
        let Some(clipped_viewport) = reveal_bounds.intersection(viewport) else {
            return;
        };

        renderer.with_layer(clipped_viewport, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                &clipped_viewport,
            );
        });
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

impl<'a, Message, Theme, Renderer> From<ForegroundReveal<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(reveal: ForegroundReveal<'a, Message, Theme, Renderer>) -> Self {
        Element::new(reveal)
    }
}

/// Creates a draw-only centered foreground reveal.
pub fn foreground_reveal<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    reveal: f32,
) -> ForegroundReveal<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    ForegroundReveal::new(content, reveal)
}

#[inline]
fn normalized_reveal(reveal: f32) -> f32 {
    if reveal.is_finite() {
        reveal.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[inline]
fn centered_vertical_reveal(bounds: Rectangle, reveal: f32) -> Option<Rectangle> {
    let reveal = normalized_reveal(reveal);
    if reveal <= f32::EPSILON || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return None;
    }

    let height = bounds.height * reveal;
    Some(Rectangle {
        x: bounds.x,
        y: bounds.center_y() - height / 2.0,
        width: bounds.width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::{centered_vertical_reveal, normalized_reveal};
    use iced::Rectangle;

    #[test]
    fn reveal_is_clamped_and_non_finite_values_fail_open() {
        assert_eq!(normalized_reveal(-1.0), 0.0);
        assert_eq!(normalized_reveal(0.4), 0.4);
        assert_eq!(normalized_reveal(2.0), 1.0);
        assert_eq!(normalized_reveal(f32::NAN), 1.0);
    }

    #[test]
    fn centered_reveal_preserves_layout_bounds_and_hides_at_zero() {
        let bounds = Rectangle {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        };

        assert_eq!(centered_vertical_reveal(bounds, 0.0), None);
        assert_eq!(centered_vertical_reveal(bounds, 1.0), Some(bounds));
        assert_eq!(
            centered_vertical_reveal(bounds, 0.5),
            Some(Rectangle {
                x: 10.0,
                y: 70.0,
                width: 300.0,
                height: 100.0,
            })
        );
    }
}
