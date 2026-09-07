//! Multi-track slider widget
//!
//! A slider that supports displaying multiple tracks:
//! - Playback progress (primary track)
//! - Download progress (secondary track behind the unplayed portion)
//!
//! Based on iced's slider widget with modifications for multi-track rendering.

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Layout, Shell, Widget};
use iced::border::Border;
use iced::keyboard;
use iced::keyboard::key::{self, Key};
use iced::mouse;
use iced::touch;
use iced::window;
use iced::{Background, Color, Element, Event, Length, Pixels, Point, Rectangle, Size, Theme};

use std::ops::RangeInclusive;

use crate::responsive::UiTokens;

type SliderStyleFn<'a> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

/// Multi-track slider widget
pub struct MultiTrackSlider<'a, Message> {
    range: RangeInclusive<f32>,
    step: f32,
    value: f32,
    /// Secondary track value (e.g., download progress)
    secondary_value: Option<f32>,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
    on_release: Option<Message>,
    width: Length,
    height: f32,
    style: SliderStyleFn<'a>,
    status: Option<Status>,
}

impl<'a, Message> MultiTrackSlider<'a, Message>
where
    Message: Clone,
{
    pub fn new<F>(
        range: RangeInclusive<f32>,
        value: f32,
        height: f32,
        tokens: UiTokens,
        on_change: F,
    ) -> Self
    where
        F: 'a + Fn(f32) -> Message,
    {
        let value = value.clamp(*range.start(), *range.end());

        Self {
            value,
            range,
            step: 0.001,
            secondary_value: None,
            on_change: Box::new(on_change),
            on_release: None,
            width: Length::Fill,
            height,
            style: Box::new(move |theme, status| default_style(theme, status, tokens)),
            status: None,
        }
    }

    /// Set secondary track value (e.g., download progress)
    pub fn secondary(mut self, value: Option<f32>) -> Self {
        self.secondary_value = value.map(|v| v.clamp(0.0, 1.0));
        self
    }

    pub fn on_release(mut self, on_release: Message) -> Self {
        self.on_release = Some(on_release);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Pixels>) -> Self {
        self.height = height.into().0;
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.style = Box::new(style);
        self
    }
}

impl<Message, Renderer> Widget<Message, Theme, Renderer> for MultiTrackSlider<'_, Message>
where
    Message: Clone,
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();

        let locate = |cursor_position: Point| -> Option<f32> {
            if cursor_position.x <= bounds.x {
                Some(*self.range.start())
            } else if cursor_position.x >= bounds.x + bounds.width {
                Some(*self.range.end())
            } else {
                let start = *self.range.start() as f64;
                let end = *self.range.end() as f64;
                let step = self.step as f64;

                let percent = f64::from(cursor_position.x - bounds.x) / f64::from(bounds.width);

                let steps = (percent * (end - start) / step).round();
                let value = steps * step + start;

                Some((value.min(end) as f32).clamp(*self.range.start(), *self.range.end()))
            }
        };

        match &event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(cursor_position) = cursor.position_over(bounds) {
                    if let Some(new_value) = locate(cursor_position)
                        && (self.value - new_value).abs() > f32::EPSILON
                    {
                        shell.publish((self.on_change)(new_value));
                        self.value = new_value;
                    }
                    state.is_dragging = true;
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. })
            | Event::Touch(touch::Event::FingerLost { .. })
                if state.is_dragging =>
            {
                if let Some(on_release) = self.on_release.clone() {
                    shell.publish(on_release);
                }
                state.is_dragging = false;
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. })
                if state.is_dragging =>
            {
                if let Some(pos) = cursor.land().position()
                    && let Some(new_value) = locate(pos)
                    && (self.value - new_value).abs() > f32::EPSILON
                {
                    shell.publish((self.on_change)(new_value));
                    self.value = new_value;
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) if cursor.is_over(bounds) => {
                let step = self.step;
                let current = self.value;
                match key {
                    Key::Named(key::Named::ArrowUp) | Key::Named(key::Named::ArrowRight) => {
                        let new_value = (current + step).min(*self.range.end());
                        if (self.value - new_value).abs() > f32::EPSILON {
                            shell.publish((self.on_change)(new_value));
                            self.value = new_value;
                        }
                        shell.capture_event();
                    }
                    Key::Named(key::Named::ArrowDown) | Key::Named(key::Named::ArrowLeft) => {
                        let new_value = (current - step).max(*self.range.start());
                        if (self.value - new_value).abs() > f32::EPSILON {
                            shell.publish((self.on_change)(new_value));
                            self.value = new_value;
                        }
                        shell.capture_event();
                    }
                    _ => (),
                }
            }
            _ => {}
        }

        let current_status = if state.is_dragging {
            Status::Dragged
        } else if cursor.is_over(bounds) {
            Status::Hovered
        } else {
            Status::Active
        };

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            self.status = Some(current_status);
        } else if self.status.is_some_and(|status| status != current_status) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let style = (self.style)(theme, self.status.unwrap_or(Status::Active));

        let (handle_width, handle_height, handle_border_radius) = match style.handle.shape {
            HandleShape::Circle { radius } => (radius * 2.0, radius * 2.0, radius.into()),
            HandleShape::Rectangle {
                width,
                border_radius,
            } => (f32::from(width), bounds.height, border_radius),
        };

        let value = self.value;
        let (range_start, range_end) = self.range.clone().into_inner();

        let offset = if range_start >= range_end {
            0.0
        } else {
            (bounds.width - handle_width) * (value - range_start) / (range_end - range_start)
        };

        let rail_y = bounds.y + bounds.height / 2.0;

        // Calculate secondary track offset (download progress)
        // Secondary track uses full width - not affected by handle size
        let secondary_offset = self.secondary_value.map(|sv| {
            if range_start >= range_end {
                0.0
            } else {
                bounds.width * (sv - range_start) / (range_end - range_start)
            }
        });

        // Draw the rail in three parts:
        // 1. Played portion (primary color) - from start to playback position
        // 2. Downloaded but not played (secondary color) - from playback to download position
        // 3. Not downloaded (background color) - from download position to end

        // Part 1: Played portion (primary track). Draw the background at the
        // full rail width and reveal it through a progress-sized clip. This
        // keeps gradients anchored to the full timeline instead of squeezing
        // the complete gradient into the played portion.
        let full_rail_bounds = Rectangle {
            x: bounds.x,
            y: rail_y - style.rail.width / 2.0,
            width: bounds.width,
            height: style.rail.width,
        };
        let played_clip_bounds = Rectangle {
            width: offset + handle_width / 2.0,
            ..full_rail_bounds
        };
        if played_clip_bounds.width > 0.0 {
            renderer.with_layer(played_clip_bounds, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: full_rail_bounds,
                        border: style.rail.border,
                        ..renderer::Quad::default()
                    },
                    style.rail.backgrounds.0,
                );
            });
        }

        // Part 2 & 3: Handle secondary track if present
        if let Some(sec_offset) = secondary_offset {
            let playback_end = bounds.x + offset + handle_width / 2.0;
            // Secondary track end position - uses full width, not affected by handle
            let download_end = bounds.x + sec_offset;

            if download_end > playback_end {
                // Downloaded but not played portion (secondary color)
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: playback_end,
                            y: rail_y - style.rail.width / 2.0,
                            width: download_end - playback_end,
                            height: style.rail.width,
                        },
                        border: style.rail.border,
                        ..renderer::Quad::default()
                    },
                    style
                        .rail
                        .secondary_background
                        .unwrap_or(style.rail.backgrounds.1),
                );
            }

            // Not downloaded portion (background)
            let remaining_start = download_end.max(playback_end);
            let remaining_width = bounds.x + bounds.width - remaining_start;
            if remaining_width > 0.0 {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: remaining_start,
                            y: rail_y - style.rail.width / 2.0,
                            width: remaining_width,
                            height: style.rail.width,
                        },
                        border: style.rail.border,
                        ..renderer::Quad::default()
                    },
                    style.rail.backgrounds.1,
                );
            }
        } else {
            // No secondary track - just draw the remaining portion
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: bounds.x + offset + handle_width / 2.0,
                        y: rail_y - style.rail.width / 2.0,
                        width: bounds.width - offset - handle_width / 2.0,
                        height: style.rail.width,
                    },
                    border: style.rail.border,
                    ..renderer::Quad::default()
                },
                style.rail.backgrounds.1,
            );
        }

        // The played gradient is recorded in its own clipped layer above. iced
        // renders that layer after the base layer, regardless of the order of
        // these draw calls, so a handle left in the base layer would be covered
        // by the rail. Record the visible handle in a final layer to keep it on
        // top of every rail segment.
        if handle_width > 0.0 && handle_height > 0.0 {
            let handle_bounds = Rectangle {
                x: bounds.x + offset,
                y: rail_y - handle_height / 2.0,
                width: handle_width,
                height: handle_height,
            };

            renderer.with_layer(handle_bounds, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: handle_bounds,
                        border: Border {
                            radius: handle_border_radius,
                            width: style.handle.border_width,
                            color: style.handle.border_color,
                        },
                        ..renderer::Quad::default()
                    },
                    style.handle.background,
                );
            });
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();

        if state.is_dragging {
            if cfg!(target_os = "windows") {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::Grabbing
            }
        } else if cursor.is_over(layout.bounds()) {
            if cfg!(target_os = "windows") {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::Grab
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message, Renderer> From<MultiTrackSlider<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(slider: MultiTrackSlider<'a, Message>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(slider)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct State {
    is_dragging: bool,
}

/// Status of the slider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Hovered,
    Dragged,
}

/// Style for the multi-track slider
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub rail: Rail,
    pub handle: Handle,
}

/// Rail appearance
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rail {
    /// (played, unplayed) backgrounds
    pub backgrounds: (Background, Background),
    /// Secondary track background (downloaded but not played)
    pub secondary_background: Option<Background>,
    pub width: f32,
    pub border: Border,
}

/// Handle appearance
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    pub shape: HandleShape,
    pub background: Background,
    pub border_width: f32,
    pub border_color: Color,
}

/// Handle shape
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandleShape {
    Circle {
        radius: f32,
    },
    Rectangle {
        width: u16,
        border_radius: iced::border::Radius,
    },
}

fn default_style(_theme: &Theme, _status: Status, tokens: UiTokens) -> Style {
    Style {
        rail: Rail {
            backgrounds: (
                Background::Color(Color::from_rgb(0.8, 0.2, 0.5)),
                Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)),
            ),
            secondary_background: Some(Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.4))),
            width: tokens.size(4.0),
            border: Border {
                radius: tokens.size(2.0).into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
        },
        handle: Handle {
            shape: HandleShape::Circle {
                radius: tokens.size(6.0),
            },
            background: Background::Color(Color::from_rgb(0.8, 0.2, 0.5)),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}
