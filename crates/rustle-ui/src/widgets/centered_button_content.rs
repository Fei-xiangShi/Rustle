//! Layout adapter for a small visual inside a larger button hit target.
//!
//! Iced propagates a fixed [`button`](iced::widget::button) width and height as
//! minimum layout limits to its direct child. A zero-padded button containing
//! a small SVG directly will therefore stretch the SVG to the full hit-target
//! size. The intermediary container below loosens those minimum constraints
//! and centers the visual while the button keeps its accessible target size.

use iced::Element;
use iced::advanced::renderer;
use iced::widget::{Container, container};

/// Center content inside a square button target without stretching it.
pub fn centered_button_content<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    target_size: f32,
) -> Container<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: container::Catalog + 'a,
    Renderer: renderer::Renderer + 'a,
{
    container(content).center(target_size)
}

#[cfg(test)]
mod tests {
    use iced::advanced::layout::Limits;
    use iced::advanced::widget::{Tree, Widget};
    use iced::widget::{Button, Space, button};
    use iced::{Element, Size, Theme};

    use super::centered_button_content;

    #[test]
    fn fixed_button_target_does_not_stretch_centered_visual() {
        let visual_size = 5.0;
        let target_size = 36.0;
        let content: Element<'_, (), Theme, ()> = centered_button_content(
            Space::new().width(visual_size).height(visual_size),
            target_size,
        )
        .into();
        let mut control: Button<'_, (), Theme, ()> = button(content)
            .width(target_size)
            .height(target_size)
            .padding(0);
        let mut tree = Tree::new(&control as &dyn Widget<(), Theme, ()>);
        control.diff(&mut tree);

        let node = control.layout(
            &mut tree,
            &(),
            &Limits::new(Size::ZERO, Size::new(100.0, 100.0)),
        );
        let centered_content = &node.children()[0];
        let visual = &centered_content.children()[0];

        assert_eq!(node.size(), Size::new(target_size, target_size));
        assert_eq!(centered_content.size(), Size::new(target_size, target_size));
        assert_eq!(visual.size(), Size::new(visual_size, visual_size));
    }
}
