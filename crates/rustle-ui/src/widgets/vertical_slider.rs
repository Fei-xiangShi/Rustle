//! Vertical slider widget for equalizer bands
//!
//! A custom vertical slider implementation since iced doesn't provide one natively.

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Widget};
use iced::event::Event;
use iced::mouse;
use iced::{Background, Border, Color, Element, Length, Rectangle, Size, Theme};

use crate::responsive::UiTokens;

/// A vertical slider widget
pub struct VerticalSlider<'a, Message> {
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
    width: Length,
    height: Length,
    rail_width: f32,
    handle_radius: f32,
    rail_color: Color,
    handle_color: Color,
    handle_color_hovered: Color,
    handle_color_dragging: Color,
}

impl<'a, Message> VerticalSlider<'a, Message> {
    /// Creates a new vertical slider
    pub fn new<F>(
        range: std::ops::RangeInclusive<f32>,
        value: f32,
        tokens: UiTokens,
        on_change: F,
    ) -> Self
    where
        F: Fn(f32) -> Message + 'a,
    {
        Self {
            value: value.clamp(*range.start(), *range.end()),
            range,
            step: 0.1,
            on_change: Box::new(on_change),
            width: Length::Fixed(tokens.size(40.0)),
            height: Length::Fixed(tokens.size(180.0)),
            rail_width: tokens.size(4.0),
            handle_radius: tokens.size(8.0),
            rail_color: crate::theme::divider(&iced::Theme::Dark),
            handle_color: crate::theme::TEXT_SECONDARY,
            handle_color_hovered: Color::WHITE,
            handle_color_dragging: crate::theme::ACCENT_PINK,
        }
    }

    /// Sets the step size
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Sets the width
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

/// State for the vertical slider
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    is_dragging: bool,
}

impl<'a, Message, Renderer> Widget<Message, Theme, Renderer> for VerticalSlider<'a, Message>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let size = limits.resolve(self.width, self.height, Size::ZERO);
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                state.is_dragging = true;
                if let Some(position) = cursor.position() {
                    let new_value = self.value_from_position(position.y, bounds);
                    shell.publish((self.on_change)(new_value));
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.is_dragging =>
            {
                state.is_dragging = false;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.is_dragging => {
                if let Some(position) = cursor.position() {
                    let new_value = self.value_from_position(position.y, bounds);
                    shell.publish((self.on_change)(new_value));
                }
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let is_hovered = cursor.is_over(bounds);

        // Calculate rail position (centered horizontally)
        let rail_x = bounds.x + (bounds.width - self.rail_width) / 2.0;
        let rail_bounds = Rectangle {
            x: rail_x,
            y: bounds.y + self.handle_radius,
            width: self.rail_width,
            height: bounds.height - self.handle_radius * 2.0,
        };

        // Draw rail
        renderer.fill_quad(
            renderer::Quad {
                bounds: rail_bounds,
                border: Border::default().rounded(self.rail_width / 2.0),
                ..Default::default()
            },
            Background::Color(self.rail_color),
        );

        // Calculate handle position
        let range = *self.range.end() - *self.range.start();
        let normalized = (self.value - *self.range.start()) / range;
        // Invert: top = max, bottom = min
        let handle_y = bounds.y + self.handle_radius + (1.0 - normalized) * rail_bounds.height;

        // Determine handle color
        let handle_color = if state.is_dragging {
            self.handle_color_dragging
        } else if is_hovered {
            self.handle_color_hovered
        } else {
            self.handle_color
        };

        // Draw handle
        let handle_bounds = Rectangle {
            x: bounds.x + (bounds.width - self.handle_radius * 2.0) / 2.0,
            y: handle_y - self.handle_radius,
            width: self.handle_radius * 2.0,
            height: self.handle_radius * 2.0,
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds: handle_bounds,
                border: Border::default().rounded(self.handle_radius),
                ..Default::default()
            },
            Background::Color(handle_color),
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        if state.is_dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message> VerticalSlider<'a, Message> {
    /// Convert Y position to value
    fn value_from_position(&self, y: f32, bounds: Rectangle) -> f32 {
        let usable_height = bounds.height - self.handle_radius * 2.0;
        let relative_y = (y - bounds.y - self.handle_radius).clamp(0.0, usable_height);
        // Invert: top = max, bottom = min
        let normalized = 1.0 - (relative_y / usable_height);
        let range = *self.range.end() - *self.range.start();
        let raw_value = *self.range.start() + normalized * range;

        // Apply step
        let stepped = (raw_value / self.step).round() * self.step;
        stepped.clamp(*self.range.start(), *self.range.end())
    }
}

impl<'a, Message> From<VerticalSlider<'a, Message>> for Element<'a, Message, Theme>
where
    Message: Clone + 'a,
{
    fn from(slider: VerticalSlider<'a, Message>) -> Self {
        Element::new(slider)
    }
}

/// Creates a new vertical slider
pub fn vertical_slider<'a, Message>(
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    tokens: UiTokens,
    on_change: impl Fn(f32) -> Message + 'a,
) -> VerticalSlider<'a, Message> {
    VerticalSlider::new(range, value, tokens, on_change)
}
