//! Circular progress ring primitive
//!
//! A customizable circular progress indicator using iced's Canvas.
//!
//! # Design
//!
//! This is a primitive component that implements `canvas::Program` trait.
//! It uses generic Message types and does not depend on application-specific types.

use iced::widget::Canvas;
use iced::widget::canvas::{Frame, Geometry, Path, Program, Stroke};
use iced::{Color, Element, Point, Radians, Renderer, Theme, mouse};

/// Progress ring configuration
#[derive(Debug, Clone, Copy)]
pub struct ProgressRing {
    /// Progress value (0.0 - 1.0)
    pub progress: f32,
    /// Rotation in radians for indeterminate progress.
    pub rotation: f32,
    /// Animated check stroke (0..1) drawn inside the ring.
    pub check_progress: f32,
    pub opacity: f32,
    /// Ring stroke width
    pub stroke_width: f32,
    /// Inset between the ring stroke and the canvas edge.
    pub edge_inset: f32,
    /// Background ring color
    pub background_color: Option<Color>,
    /// Progress ring color
    pub progress_color: Option<Color>,
}

impl ProgressRing {
    pub fn new(progress: f32, stroke_width: f32, edge_inset: f32) -> Self {
        Self {
            progress: progress.clamp(0.0, 1.0),
            rotation: 0.0,
            check_progress: 0.0,
            opacity: 1.0,
            stroke_width,
            edge_inset,
            background_color: None,
            progress_color: None,
        }
    }

    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn progress_color(mut self, color: Color) -> Self {
        self.progress_color = Some(color);
        self
    }
}

impl<Message> Program<Message> for ProgressRing {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius =
            (bounds.width.min(bounds.height) / 2.0) - (self.stroke_width / 2.0) - self.edge_inset;

        let fade = |mut color: Color| {
            color.a *= self.opacity;
            color
        };
        // Background circle
        let background_circle = Path::circle(center, radius);
        frame.stroke(
            &background_circle,
            Stroke::default()
                .with_width(self.stroke_width)
                .with_color(fade(
                    self.background_color
                        .unwrap_or_else(|| crate::theme::divider(_theme)),
                )),
        );

        // Progress arc
        if self.progress > 0.0 {
            let start_angle = -std::f32::consts::FRAC_PI_2 + self.rotation; // Start from top
            let sweep_angle = self.progress * std::f32::consts::TAU;

            let progress_arc = Path::new(|builder| {
                builder.arc(iced::widget::canvas::path::Arc {
                    center,
                    radius,
                    start_angle: Radians(start_angle),
                    end_angle: Radians(start_angle + sweep_angle),
                });
            });

            frame.stroke(
                &progress_arc,
                Stroke::default()
                    .with_width(self.stroke_width)
                    .with_color(fade(
                        self.progress_color
                            .unwrap_or_else(|| crate::theme::accent(_theme)),
                    )),
            );
        }

        let check = self.check_progress.clamp(0.0, 1.0);
        if check > 0.0 {
            let a = Point::new(center.x - radius * 0.45, center.y);
            let b = Point::new(center.x - radius * 0.10, center.y + radius * 0.30);
            let c = Point::new(center.x + radius * 0.48, center.y - radius * 0.35);
            let lerp = |a: Point, b: Point, t: f32| {
                Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
            };
            let path = Path::new(|builder| {
                builder.move_to(a);
                builder.line_to(lerp(a, b, (check / 0.35).min(1.0)));
                if check > 0.35 {
                    builder.line_to(lerp(b, c, (check - 0.35) / 0.65));
                }
            });
            frame.stroke(
                &path,
                Stroke::default()
                    .with_width(self.stroke_width)
                    .with_color(fade(
                        self.progress_color
                            .unwrap_or_else(|| crate::theme::accent(_theme)),
                    )),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Create a customized progress ring element
pub fn view_progress_ring_styled<'a, Message: 'a>(
    ring: ProgressRing,
    size: f32,
) -> Element<'a, Message> {
    Canvas::new(ring).width(size).height(size).into()
}
