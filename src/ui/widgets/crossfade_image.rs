//! Stateful image widget that cross-fades whenever its handle changes.

use std::time::Duration;

use iced::advanced::Shell;
use iced::advanced::image;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::mouse::{self, Cursor};
use iced::time::Instant;
use iced::widget::image::{self as image_widget, FilterMethod};
use iced::{ContentFit, Element, Event, Length, Point, Rectangle, Rotation, Size};

const DEFAULT_DURATION: Duration = Duration::from_millis(280);

/// Positions fitted image content inside its layout bounds.
///
/// The normalized coordinates mirror CSS `object-position`: `0.0` aligns the
/// leading edge, `0.5` centers it, and `1.0` aligns the trailing edge. Values
/// outside this range are clamped when the image is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentPosition {
    horizontal: f32,
    vertical: f32,
}

impl ContentPosition {
    pub const TOP_LEFT: Self = Self::new(0.0, 0.0);
    pub const TOP: Self = Self::new(0.5, 0.0);
    pub const TOP_RIGHT: Self = Self::new(1.0, 0.0);
    pub const LEFT: Self = Self::new(0.0, 0.5);
    pub const CENTER: Self = Self::new(0.5, 0.5);
    pub const RIGHT: Self = Self::new(1.0, 0.5);
    pub const BOTTOM_LEFT: Self = Self::new(0.0, 1.0);
    pub const BOTTOM: Self = Self::new(0.5, 1.0);
    pub const BOTTOM_RIGHT: Self = Self::new(1.0, 1.0);

    pub const fn new(horizontal: f32, vertical: f32) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

impl Default for ContentPosition {
    fn default() -> Self {
        Self::CENTER
    }
}

/// An image that keeps the previous handle mounted while the next one fades in.
pub struct CrossfadeImage {
    handle: Option<image::Handle>,
    width: Length,
    height: Length,
    border_radius: iced::border::Radius,
    content_fit: ContentFit,
    content_position: Option<ContentPosition>,
    filter_method: FilterMethod,
    scale: f32,
    duration: Duration,
}

impl CrossfadeImage {
    pub fn new(handle: Option<image::Handle>) -> Self {
        Self {
            handle,
            width: Length::Shrink,
            height: Length::Shrink,
            border_radius: iced::border::Radius::default(),
            content_fit: ContentFit::default(),
            content_position: None,
            filter_method: FilterMethod::default(),
            scale: 1.0,
            duration: DEFAULT_DURATION,
        }
    }

    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    #[must_use]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    #[must_use]
    pub fn content_fit(mut self, content_fit: ContentFit) -> Self {
        self.content_fit = content_fit;
        self
    }

    /// Sets the position of the fitted content within the image bounds.
    ///
    /// For [`ContentFit::Cover`], this controls which part remains visible
    /// after the overflowing axis is clipped.
    #[must_use]
    pub fn content_position(mut self, content_position: ContentPosition) -> Self {
        self.content_position = Some(content_position);
        self
    }

    #[must_use]
    pub fn border_radius(mut self, border_radius: impl Into<iced::border::Radius>) -> Self {
        self.border_radius = border_radius.into();
        self
    }

    #[must_use]
    pub fn scale(mut self, scale: impl Into<f32>) -> Self {
        self.scale = scale.into();
        self
    }

    #[must_use]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }
}

#[derive(Debug, Default)]
struct State {
    previous: Option<image::Handle>,
    current: Option<image::Handle>,
    progress: f32,
    started_at: Option<Instant>,
}

impl State {
    fn sync_handle(&mut self, handle: &Option<image::Handle>, now: Instant) -> bool {
        if self.current == *handle {
            return false;
        }

        self.previous = self.current.take();
        self.current = handle.clone();
        self.progress = 0.0;
        self.started_at = Some(now);
        true
    }

    fn tick(&mut self, now: Instant, duration: Duration) -> bool {
        let Some(started_at) = self.started_at else {
            return false;
        };

        let elapsed = now.saturating_duration_since(started_at).as_secs_f32();
        let linear = (elapsed / duration.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0);
        self.progress = ease_out_cubic(linear);

        if linear >= 1.0 {
            self.progress = 1.0;
            self.previous = None;
            self.started_at = None;
            false
        } else {
            true
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for CrossfadeImage
where
    Renderer: renderer::Renderer + image::Renderer<Handle = image::Handle>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        if let Some(handle) = self.handle.as_ref() {
            image_widget::layout(
                renderer,
                limits,
                handle,
                self.width,
                self.height,
                None,
                self.content_fit,
                Rotation::default(),
                false,
            )
        } else {
            layout::Node::new(limits.resolve(self.width, self.height, Size::ZERO))
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let now = match event {
            Event::Window(iced::window::Event::RedrawRequested(now)) => *now,
            _ => Instant::now(),
        };

        if state.sync_handle(&self.handle, now) {
            shell.request_redraw();
        }

        if matches!(
            event,
            Event::Window(iced::window::Event::RedrawRequested(_))
        ) && state.tick(now, self.duration)
        {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let progress = state.progress.clamp(0.0, 1.0);

        if let Some(previous) = state.previous.as_ref() {
            draw_image(
                renderer,
                layout,
                previous,
                self.border_radius,
                self.content_fit,
                self.content_position,
                self.filter_method,
                1.0 - progress,
                self.scale,
            );
        }

        if let Some(current) = state.current.as_ref() {
            draw_image(
                renderer,
                layout,
                current,
                self.border_radius,
                self.content_fit,
                self.content_position,
                self.filter_method,
                progress,
                self.scale,
            );
        }
    }
}

impl<'a, Message, Theme, Renderer> From<CrossfadeImage> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + image::Renderer<Handle = image::Handle> + 'a,
{
    fn from(image: CrossfadeImage) -> Self {
        Element::new(image)
    }
}

pub fn crossfade_image(handle: Option<image::Handle>) -> CrossfadeImage {
    CrossfadeImage::new(handle)
}

#[expect(
    clippy::too_many_arguments,
    reason = "renderer leaf mirrors iced draw state and two image layers; remove when CrossfadeDrawContext owns the renderer inputs"
)]
fn draw_image<Renderer>(
    renderer: &mut Renderer,
    layout: Layout<'_>,
    handle: &image::Handle,
    border_radius: iced::border::Radius,
    content_fit: ContentFit,
    content_position: Option<ContentPosition>,
    filter_method: FilterMethod,
    opacity: f32,
    scale: f32,
) where
    Renderer: renderer::Renderer + image::Renderer<Handle = image::Handle>,
{
    let bounds = layout.bounds();
    let positioned_bounds = content_position.and_then(|position| {
        renderer.measure_image(handle).and_then(|image_size| {
            positioned_image_bounds(image_size, bounds, content_fit, position, scale)
        })
    });

    if let Some(drawing_bounds) = positioned_bounds {
        renderer.draw_image(
            image::Image {
                handle: handle.clone(),
                border_radius,
                filter_method,
                rotation: Rotation::default().radians(),
                opacity,
            },
            drawing_bounds,
            bounds,
        );
    } else {
        image_widget::draw(
            renderer,
            layout,
            handle,
            None,
            border_radius,
            content_fit,
            filter_method,
            Rotation::default(),
            opacity,
            scale,
        );
    }
}

fn positioned_image_bounds(
    image_size: Size<u32>,
    bounds: Rectangle,
    content_fit: ContentFit,
    content_position: ContentPosition,
    scale: f32,
) -> Option<Rectangle> {
    if image_size.width == 0
        || image_size.height == 0
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || !content_position.horizontal.is_finite()
        || !content_position.vertical.is_finite()
        || !scale.is_finite()
        || scale < 0.0
    {
        return None;
    }

    let fitted_size = content_fit.fit(
        Size::new(image_size.width as f32, image_size.height as f32),
        bounds.size(),
    );
    let drawing_size = fitted_size * scale;

    if !drawing_size.width.is_finite() || !drawing_size.height.is_finite() {
        return None;
    }

    let horizontal = content_position.horizontal.clamp(0.0, 1.0);
    let vertical = content_position.vertical.clamp(0.0, 1.0);

    Some(Rectangle::new(
        Point::new(
            bounds.x + (bounds.width - drawing_size.width) * horizontal,
            bounds.y + (bounds.height - drawing_size.height) * vertical,
        ),
        drawing_size,
    ))
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}

#[cfg(test)]
mod tests {
    use super::{
        ContentPosition, DEFAULT_DURATION, State, ease_out_cubic, positioned_image_bounds,
    };
    use iced::time::Instant;
    use iced::widget::image;
    use iced::{ContentFit, Point, Rectangle, Size};

    #[test]
    fn handle_changes_retain_the_previous_image_until_the_fade_finishes() {
        let now = Instant::now();
        let first = image::Handle::from_path("first.jpg");
        let second = image::Handle::from_path("second.jpg");
        let mut state = State::default();

        assert!(state.sync_handle(&Some(first.clone()), now));
        state.tick(now + DEFAULT_DURATION, DEFAULT_DURATION);
        assert_eq!(state.current, Some(first.clone()));
        assert_eq!(state.previous, None);

        assert!(state.sync_handle(&Some(second.clone()), now));
        assert_eq!(state.previous, Some(first));
        assert_eq!(state.current, Some(second));
    }

    #[test]
    fn easing_is_bounded_and_finishes_at_full_opacity() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn cover_position_selects_the_visible_vertical_region() {
        let image_size = Size::new(300, 300);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(300.0, 180.0));

        let top = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::TOP,
            1.0,
        )
        .unwrap();
        let center = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::CENTER,
            1.0,
        )
        .unwrap();
        let bottom = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::BOTTOM,
            1.0,
        )
        .unwrap();

        assert_eq!(top.y, 0.0);
        assert_eq!(center.y, -60.0);
        assert_eq!(bottom.y, -120.0);
        assert_eq!(top.size(), Size::new(300.0, 300.0));
    }

    #[test]
    fn cover_position_selects_the_visible_horizontal_region() {
        let image_size = Size::new(300, 300);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(180.0, 300.0));

        let left = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::LEFT,
            1.0,
        )
        .unwrap();
        let center = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::CENTER,
            1.0,
        )
        .unwrap();
        let right = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::RIGHT,
            1.0,
        )
        .unwrap();

        assert_eq!(left.x, 0.0);
        assert_eq!(center.x, -60.0);
        assert_eq!(right.x, -120.0);
    }

    #[test]
    fn custom_position_is_clamped_and_fill_always_matches_bounds() {
        let image_size = Size::new(300, 300);
        let bounds = Rectangle::new(Point::new(10.0, 20.0), Size::new(300.0, 180.0));
        let clamped = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Cover,
            ContentPosition::new(-1.0, 2.0),
            1.0,
        )
        .unwrap();
        let filled = positioned_image_bounds(
            image_size,
            bounds,
            ContentFit::Fill,
            ContentPosition::BOTTOM_RIGHT,
            1.0,
        )
        .unwrap();

        assert_eq!(clamped.x, bounds.x);
        assert_eq!(clamped.y, bounds.y - 120.0);
        assert_eq!(filled, bounds);
    }

    #[test]
    fn invalid_measurements_fall_back_to_the_default_draw_path() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(300.0, 180.0));

        assert!(
            positioned_image_bounds(
                Size::new(0, 300),
                bounds,
                ContentFit::Cover,
                ContentPosition::TOP,
                1.0,
            )
            .is_none()
        );
        assert!(
            positioned_image_bounds(
                Size::new(300, 300),
                bounds,
                ContentFit::Cover,
                ContentPosition::new(f32::NAN, 0.0),
                1.0,
            )
            .is_none()
        );
    }
}
