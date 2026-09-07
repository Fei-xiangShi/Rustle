//! Shared empty-chrome surface used to start native window dragging.

use iced::advanced::renderer;
use iced::widget::{Space, container, mouse_area};
use iced::{Alignment, Element, Fill, Length, mouse};

use crate::responsive::{ResponsiveContext, top_bar_height};

const TOP_BAR_FRACTION: f32 = 1.0 / 2.0;

/// Resolve the drag layer height from the shared top-bar contract.
pub fn height(context: ResponsiveContext) -> f32 {
    top_bar_height(&context) * TOP_BAR_FRACTION
}

/// Build a full-width drag layer for use below real top-chrome controls.
pub fn view<Message: Clone + 'static>(
    context: ResponsiveContext,
    on_press: Message,
) -> Element<'static, Message> {
    drag_layer(context, on_press)
}

fn drag_layer<'a, Message, Theme, Renderer>(
    context: ResponsiveContext,
    on_press: Message,
) -> Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: container::Catalog + 'a,
    Renderer: renderer::Renderer + 'a,
{
    let top_bar_height = top_bar_height(&context);
    let drag_target = mouse_area(
        container(Space::new())
            .width(Fill)
            .height(Length::Fixed(height(context))),
    )
    .on_press(on_press)
    // `Idle` keeps the ordinary arrow cursor while remaining a non-None stack
    // interaction, so empty chrome still shields hover from lower layers.
    .interaction(mouse::Interaction::Idle);

    // A fixed-height Stack passes its exact minimum height to the base layer.
    // Let this outer container satisfy that contract, then loosen and top-align
    // the actual MouseArea so its hit bounds remain the intended half height.
    container(drag_target)
        .width(Fill)
        .height(Length::Fixed(top_bar_height))
        .align_y(Alignment::Start)
        .into()
}

#[cfg(test)]
mod tests {
    use super::{drag_layer, height};
    use crate::responsive::{ResponsiveContext, top_bar_height};
    use iced::advanced::layout::Limits;
    use iced::advanced::widget::Tree;
    use iced::{Size, Theme};

    #[test]
    fn drag_region_is_exactly_half_of_the_top_bar() {
        for viewport in [
            Size::new(1_920.0, 1_080.0),
            Size::new(960.0, 1_080.0),
            Size::new(960.0, 540.0),
        ] {
            let context = ResponsiveContext::from_viewport(viewport);
            let expected = top_bar_height(&context) / 2.0;
            assert!((height(context) - expected).abs() <= f32::EPSILON * expected.abs());
        }
    }

    #[test]
    fn stack_base_slot_does_not_stretch_the_mouse_area() {
        for viewport in [
            Size::new(1_920.0, 1_080.0),
            Size::new(1_280.0, 1_440.0),
            Size::new(960.0, 1_080.0),
            Size::new(960.0, 540.0),
        ] {
            let context = ResponsiveContext::from_viewport(viewport);
            let top_bar_height = top_bar_height(&context);
            let mut layer = drag_layer::<(), Theme, ()>(context, ());
            let mut tree = Tree::new(layer.as_widget());
            layer.as_widget_mut().diff(&mut tree);

            let node = layer.as_widget_mut().layout(
                &mut tree,
                &(),
                &Limits::new(
                    Size::new(viewport.width, top_bar_height),
                    Size::new(viewport.width, top_bar_height),
                ),
            );
            let mouse_area = &node.children()[0];

            assert_eq!(node.size(), Size::new(viewport.width, top_bar_height));
            assert_eq!(
                mouse_area.size(),
                Size::new(viewport.width, top_bar_height / 2.0)
            );
            assert_eq!(mouse_area.bounds().y, 0.0);
        }
    }
}
