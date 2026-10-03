//! Controlled popup below an anchor, with dismissal and keyboard navigation.

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::mouse::{self, Cursor};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// Places caller-controlled content below an anchor without changing its layout.
pub struct AnchoredPopup<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: renderer::Renderer,
{
    anchor: Element<'a, Message, Theme, Renderer>,
    popup: Element<'a, Message, Theme, Renderer>,
    gap: f32,
    open: bool,
    dismiss: Message,
    previous: Message,
    next: Message,
    submit: Message,
}

impl<'a, Message, Theme, Renderer> AnchoredPopup<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Actions are dismiss, previous, next and submit.
    pub fn new(
        anchor: impl Into<Element<'a, Message, Theme, Renderer>>,
        popup: impl Into<Element<'a, Message, Theme, Renderer>>,
        gap: f32,
        open: bool,
        actions: [Message; 4],
    ) -> Self {
        let [dismiss, previous, next, submit] = actions;
        Self {
            anchor: anchor.into(),
            popup: popup.into(),
            gap: gap.max(0.0),
            open,
            dismiss,
            previous,
            next,
            submit,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for AnchoredPopup<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
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
        // Cancel even a pending popup on blur or IME composition. Preedit must
        // reach the text input; its Enter/arrow keys belong to the IME.
        let dismiss = match event {
            Event::Mouse(mouse::Event::ButtonPressed(_)) => !cursor.is_over(layout.bounds()),
            Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => {
                !layout.bounds().contains(*position)
            }
            Event::Window(iced::window::Event::Unfocused) => true,
            Event::InputMethod(iced::advanced::input_method::Event::Preedit(value, _)) => {
                !value.is_empty()
            }
            Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) => matches!(
                key,
                iced::keyboard::Key::Named(
                    iced::keyboard::key::Named::Tab | iced::keyboard::key::Named::Escape
                )
            ),
            _ => false,
        };
        if dismiss {
            shell.publish(self.dismiss.clone());
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
        if self.open {
            Some(overlay::Element::new(Box::new(PopupOverlay {
                popup: &mut self.popup,
                tree: &mut tree.children[1],
                dismiss: self.dismiss.clone(),
                previous: self.previous.clone(),
                next: self.next.clone(),
                submit: self.submit.clone(),
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

impl<'a, Message, Theme, Renderer> From<AnchoredPopup<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(popup: AnchoredPopup<'a, Message, Theme, Renderer>) -> Self {
        Element::new(popup)
    }
}

struct PopupOverlay<'a, 'b, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    popup: &'a mut Element<'b, Message, Theme, Renderer>,
    tree: &'a mut Tree,
    dismiss: Message,
    previous: Message,
    next: Message,
    submit: Message,
    anchor_bounds: Rectangle,
    gap: f32,
    viewport: Rectangle,
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PopupOverlay<'_, '_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.popup.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(
                Size::ZERO,
                Size::new(
                    bounds.width,
                    (bounds.height - self.anchor_bounds.y - self.anchor_bounds.height - self.gap)
                        .max(0.0),
                ),
            ),
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
        use iced::keyboard::{Key, key::Named};
        if let Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event {
            let action = match key {
                Key::Named(Named::Escape) => Some(&self.dismiss),
                Key::Named(Named::ArrowUp) => Some(&self.previous),
                Key::Named(Named::ArrowDown) => Some(&self.next),
                Key::Named(Named::Enter) => Some(&self.submit),
                _ => None,
            };
            if let Some(action) = action {
                shell.publish(action.clone());
                shell.capture_event();
                return;
            }
        }
        let popup_bounds = layout.bounds();
        if matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_)))
            && !cursor.is_over(popup_bounds)
            && !cursor.is_over(self.anchor_bounds)
        {
            shell.publish(self.dismiss.clone());
        }
        if let Event::Touch(iced::touch::Event::FingerPressed { position, .. }) = event
            && !popup_bounds.contains(*position)
            && !self.anchor_bounds.contains(*position)
        {
            shell.publish(self.dismiss.clone());
        }
        self.popup.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            shell,
            &self.viewport,
        );
        let touch_over_popup = matches!(event,
            Event::Touch(iced::touch::Event::FingerPressed { position, .. }
                | iced::touch::Event::FingerMoved { position, .. }
                | iced::touch::Event::FingerLifted { position, .. }) if popup_bounds.contains(*position));
        if (cursor.is_over(popup_bounds) && matches!(event, Event::Mouse(_))) || touch_over_popup {
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
        self.popup.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &self.viewport,
            renderer,
        )
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

fn popup_position(anchor: Rectangle, popup: Size, viewport: Size, gap: f32) -> Point {
    Point::new(
        anchor.x.clamp(0.0, (viewport.width - popup.width).max(0.0)),
        (anchor.y + anchor.height + gap).min((viewport.height - popup.height).max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_is_below_anchor_and_clamped_to_viewport() {
        let anchor = Rectangle::new(Point::new(700.0, 20.0), Size::new(80.0, 40.0));
        assert_eq!(
            popup_position(
                anchor,
                Size::new(200.0, 300.0),
                Size::new(800.0, 600.0),
                8.0
            ),
            Point::new(600.0, 68.0)
        );
    }
}
