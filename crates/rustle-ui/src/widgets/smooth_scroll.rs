//! Event adapter that adds smooth line-wheel behavior to native scrollables.

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::keyboard;
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

use crate::animation::{SmoothScrollEvent, SmoothScrollTarget};
use crate::responsive::UiTokens;

/// Wraps a native Iced scrollable and converts vertical line-wheel input into
/// application-driven smooth-scroll requests.
pub struct SmoothScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    target: SmoothScrollTarget,
    line_scroll_pixels: f32,
    on_event: Box<dyn Fn(SmoothScrollEvent) -> Message + 'a>,
}

impl<'a, Message, Theme, Renderer> SmoothScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        target: SmoothScrollTarget,
        tokens: UiTokens,
        on_event: impl Fn(SmoothScrollEvent) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            target,
            line_scroll_pixels: tokens.size(60.0),
            on_event: Box::new(on_event),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SmoothScroll<'_, Message, Theme, Renderer>
where
    Message: Clone,
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
        let cursor_over = cursor.is_over(layout.bounds());

        if cursor_over
            && let Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { y, .. },
            }) = event
            && let Some(delta) = line_scroll_delta(*y, self.line_scroll_pixels)
        {
            shell.publish((self.on_event)(SmoothScrollEvent::Requested {
                target: self.target,
                delta,
            }));
            shell.capture_event();
            return;
        }

        if cancels_smooth_scroll(event, layout.bounds(), cursor) {
            shell.publish((self.on_event)(SmoothScrollEvent::Cancelled {
                target: self.target,
            }));
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

impl<'a, Message, Theme, Renderer> From<SmoothScroll<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(scroll: SmoothScroll<'a, Message, Theme, Renderer>) -> Self {
        Element::new(scroll)
    }
}

pub fn smooth_scroll<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    target: SmoothScrollTarget,
    tokens: UiTokens,
    on_event: impl Fn(SmoothScrollEvent) -> Message + 'a,
) -> SmoothScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    SmoothScroll::new(content, target, tokens, on_event)
}

/// Event adapter for native scrollables that do not participate in the
/// application smooth-scroll state. It preserves pixel-wheel input and
/// rewrites line-wheel input to rem-scaled pixels before iced applies it.
pub struct ScaledScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    line_scroll_pixels: f32,
}

impl<'a, Message, Theme, Renderer> ScaledScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        tokens: UiTokens,
    ) -> Self {
        Self {
            content: content.into(),
            line_scroll_pixels: tokens.size(60.0),
        }
    }
}

#[derive(Debug, Default)]
struct ScaledScrollState {
    keyboard_modifiers: keyboard::Modifiers,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ScaledScroll<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ScaledScrollState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ScaledScrollState::default())
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
        let state = tree.state.downcast_mut::<ScaledScrollState>();
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.keyboard_modifiers = *modifiers;
        }

        let rem_scaled_event = if cursor.is_over(layout.bounds()) {
            rem_scaled_wheel_event(event, state.keyboard_modifiers, self.line_scroll_pixels)
        } else {
            None
        };
        let event = rem_scaled_event.as_ref().unwrap_or(event);

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

impl<'a, Message, Theme, Renderer> From<ScaledScroll<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(scroll: ScaledScroll<'a, Message, Theme, Renderer>) -> Self {
        Element::new(scroll)
    }
}

pub fn scaled_scroll<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    tokens: UiTokens,
) -> ScaledScroll<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    ScaledScroll::new(content, tokens)
}

fn rem_scaled_wheel_event(
    event: &Event,
    modifiers: keyboard::Modifiers,
    line_scroll_pixels: f32,
) -> Option<Event> {
    let Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines { x, y },
    }) = event
    else {
        return None;
    };

    let is_shift_pressed = modifiers.shift();
    let (x, y) = if cfg!(target_os = "macos") && is_shift_pressed {
        (*y, *x)
    } else {
        (*x, *y)
    };
    let (x, y) = if is_shift_pressed { (y, x) } else { (x, y) };

    Some(Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels {
            x: x * line_scroll_pixels,
            y: y * line_scroll_pixels,
        },
    }))
}

fn line_scroll_delta(lines_y: f32, line_scroll_pixels: f32) -> Option<f32> {
    let delta = -lines_y * line_scroll_pixels;
    (delta.abs() > f32::EPSILON).then_some(delta)
}

fn cancels_smooth_scroll(event: &Event, bounds: Rectangle, cursor: Cursor) -> bool {
    match event {
        Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels { .. },
        })
        | Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left | mouse::Button::Middle)) => {
            cursor.is_over(bounds)
        }
        Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => {
            bounds.contains(*position)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{line_scroll_delta, rem_scaled_wheel_event};
    use iced::{Event, keyboard, mouse};

    #[test]
    fn line_wheel_is_converted_to_content_offset_direction() {
        assert_eq!(line_scroll_delta(1.0, 60.0), Some(-60.0));
        assert_eq!(line_scroll_delta(-2.0, 60.0), Some(120.0));
        assert_eq!(line_scroll_delta(0.0, 60.0), None);
    }

    #[test]
    fn native_line_wheel_is_rewritten_to_scaled_pixels() {
        let event = Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -2.0 },
        });
        let scaled = rem_scaled_wheel_event(&event, keyboard::Modifiers::default(), 80.0);

        assert!(matches!(
            scaled,
            Some(Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -160.0 }
            }))
        ));
    }
}
