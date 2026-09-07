//! Interactive popup that stays open across the gap above its hover anchor.
//!
//! The open state lives in Iced's widget tree. This keeps transient hover
//! state out of the application while still allowing the popup content to
//! receive and capture pointer events.

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// Shows interactive `popup` content above `anchor` while either is hovered.
pub struct HoverPopup<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: renderer::Renderer,
{
    anchor: Element<'a, Message, Theme, Renderer>,
    popup: Element<'a, Message, Theme, Renderer>,
    gap: f32,
}

impl<'a, Message, Theme, Renderer> HoverPopup<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Creates a hover popup with the popup centered above its anchor.
    pub fn new(
        anchor: impl Into<Element<'a, Message, Theme, Renderer>>,
        popup: impl Into<Element<'a, Message, Theme, Renderer>>,
        gap: f32,
    ) -> Self {
        Self {
            anchor: anchor.into(),
            popup: popup.into(),
            gap: gap.max(0.0),
        }
    }
}

#[derive(Debug, Default)]
struct State {
    is_open: bool,
    is_pointer_down: bool,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for HoverPopup<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [self.anchor.as_widget_mut(), self.popup.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        self.anchor.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.anchor
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
        self.anchor
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
        if let Event::Mouse(mouse_event) = event {
            let state = tree.state.downcast_mut::<State>();
            let should_open =
                !matches!(mouse_event, mouse::Event::CursorLeft) && cursor.is_over(layout.bounds());

            if should_open {
                set_open(state, true, shell);
            } else if !state.is_pointer_down {
                set_open(state, false, shell);
            }
        }

        self.anchor.as_widget_mut().update(
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
        self.anchor.as_widget().draw(
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
        self.anchor.as_widget().mouse_interaction(
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
        if tree.state.downcast_ref::<State>().is_open {
            Some(overlay::Element::new(Box::new(PopupOverlay {
                popup: &mut self.popup,
                tree: &mut tree.children[1],
                state: tree.state.downcast_mut::<State>(),
                anchor_bounds: layout.bounds() + translation,
                gap: self.gap,
                viewport: *viewport,
            })))
        } else {
            self.anchor.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
        }
    }
}

impl<'a, Message, Theme, Renderer> From<HoverPopup<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(popup: HoverPopup<'a, Message, Theme, Renderer>) -> Self {
        Element::new(popup)
    }
}

/// Creates an interactive hover popup centered above its anchor.
pub fn hover_popup<'a, Message, Theme, Renderer>(
    anchor: impl Into<Element<'a, Message, Theme, Renderer>>,
    popup: impl Into<Element<'a, Message, Theme, Renderer>>,
    gap: f32,
) -> HoverPopup<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    HoverPopup::new(anchor, popup, gap)
}

struct PopupOverlay<'a, 'b, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    popup: &'a mut Element<'b, Message, Theme, Renderer>,
    tree: &'a mut Tree,
    state: &'a mut State,
    anchor_bounds: Rectangle,
    gap: f32,
    viewport: Rectangle,
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PopupOverlay<'_, '_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.popup.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let popup_size = node.size();

        node.move_to(popup_position(
            self.anchor_bounds,
            popup_size,
            bounds,
            self.gap,
        ))
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.popup
            .as_widget_mut()
            .operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let popup_bounds = layout.bounds();
        let hover_bounds = hover_region(self.anchor_bounds, popup_bounds);
        let was_pointer_down = self.state.is_pointer_down;

        self.popup.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            shell,
            &self.viewport,
        );

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(popup_bounds) =>
            {
                self.state.is_pointer_down = true;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.state.is_pointer_down = false;
                if !cursor.is_over(hover_bounds) {
                    set_open(self.state, false, shell);
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                self.state.is_pointer_down = false;
                set_open(self.state, false, shell);

                if was_pointer_down {
                    reset_tree(self.popup, self.tree);
                }
            }
            Event::Mouse(_) if !self.state.is_pointer_down => {
                set_open(self.state, cursor.is_over(hover_bounds), shell);
            }
            _ => {}
        }

        let is_over_popup_or_bridge =
            cursor.is_over(hover_bounds) && !cursor.is_over(self.anchor_bounds);
        let is_pointer_event = matches!(event, Event::Mouse(_) | Event::Touch(_));

        if is_pointer_event && (is_over_popup_or_bridge || was_pointer_down) {
            shell.capture_event();
        }
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
    ) {
        self.popup.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &self.viewport,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let popup_bounds = layout.bounds();
        let hover_bounds = hover_region(self.anchor_bounds, popup_bounds);

        if self.state.is_pointer_down || cursor.is_over(popup_bounds) {
            let interaction = self.popup.as_widget().mouse_interaction(
                self.tree,
                layout,
                cursor,
                &self.viewport,
                renderer,
            );

            if interaction != mouse::Interaction::None {
                return interaction;
            }
        }

        if cursor.is_over(hover_bounds) && !cursor.is_over(self.anchor_bounds) {
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::None
        }
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.popup.as_widget_mut().overlay(
            self.tree,
            layout,
            renderer,
            &self.viewport,
            Vector::ZERO,
        )
    }
}

fn set_open<Message>(state: &mut State, is_open: bool, shell: &mut Shell<'_, Message>) {
    if state.is_open != is_open {
        state.is_open = is_open;
        if !is_open {
            state.is_pointer_down = false;
        }
        shell.invalidate_layout();
        shell.request_redraw();
    }
}

fn reset_tree<Message, Theme, Renderer>(
    popup: &mut Element<'_, Message, Theme, Renderer>,
    tree: &mut Tree,
) where
    Renderer: renderer::Renderer,
{
    *tree = Tree::new(popup.as_widget());
    popup.as_widget_mut().diff(tree);
}

fn popup_position(anchor: Rectangle, popup: Size, viewport: Size, gap: f32) -> Point {
    let max_x = (viewport.width - popup.width).max(0.0);
    let x = (anchor.center_x() - popup.width / 2.0).clamp(0.0, max_x);
    let above = anchor.y - gap - popup.height;
    let y = if above >= 0.0 {
        above
    } else {
        (anchor.y + anchor.height + gap).min((viewport.height - popup.height).max(0.0))
    };

    Point::new(x, y)
}

fn hover_region(anchor: Rectangle, popup: Rectangle) -> Rectangle {
    anchor.union(&popup)
}

#[cfg(test)]
mod tests {
    use super::{hover_region, popup_position};
    use iced::{Point, Rectangle, Size};

    #[test]
    fn hover_region_bridges_anchor_popup_gap() {
        let popup = Rectangle::new(Point::new(90.0, 20.0), Size::new(40.0, 100.0));
        let anchor = Rectangle::new(Point::new(92.0, 128.0), Size::new(36.0, 36.0));
        let region = hover_region(anchor, popup);

        assert!(region.contains(Point::new(110.0, 124.0)));
        assert!(!region.contains(Point::new(60.0, 124.0)));
    }

    #[test]
    fn popup_is_centered_above_anchor() {
        let anchor = Rectangle::new(Point::new(100.0, 200.0), Size::new(36.0, 36.0));
        let position = popup_position(anchor, Size::new(40.0, 116.0), Size::new(800.0, 600.0), 8.0);

        assert_eq!(position, Point::new(98.0, 76.0));
    }
}
