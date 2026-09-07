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
    /// Ring stroke width
    pub stroke_width: f32,
    /// Inset between the ring stroke and the canvas edge.
    pub edge_inset: f32,
    /// Background ring color
    pub background_color: Color,
    /// Progress ring color
    pub progress_color: Color,
}

impl ProgressRing {
    pub fn new(progress: f32, stroke_width: f32, edge_inset: f32) -> Self {
        Self {
            progress: progress.clamp(0.0, 1.0),
            stroke_width,
            edge_inset,
            background_color: crate::theme::divider(&iced::Theme::Dark),
            progress_color: crate::theme::ACCENT_PINK,
        }
    }

    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    pub fn progress_color(mut self, color: Color) -> Self {
        self.progress_color = color;
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

        // Background circle
        let background_circle = Path::circle(center, radius);
        frame.stroke(
            &background_circle,
            Stroke::default()
                .with_width(self.stroke_width)
                .with_color(self.background_color),
        );

        // Progress arc
        if self.progress > 0.0 {
            let start_angle = -std::f32::consts::FRAC_PI_2; // Start from top
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
                    .with_color(self.progress_color),
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
