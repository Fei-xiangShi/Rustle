//! Invisible resize handles for borderless windows.

use iced::mouse::Interaction;
use iced::widget::{Space, container, mouse_area, stack};
use iced::{Alignment, Element, Fill, Length};

use crate::app::Message;
use crate::ui::responsive::ResponsiveContext;

pub fn view<'a>(context: ResponsiveContext) -> Element<'a, Message> {
    if !crate::platform::window::needs_manual_resize_handles() {
        return Space::new().width(0).height(0).into();
    }

    use iced::window::Direction;

    let edge_size = context.tokens.size(3.0);
    let corner_size = context.tokens.size(10.0);

    fn handle<'a>(
        direction: Direction,
        interaction: Interaction,
        width: Length,
        height: Length,
        align_x: Alignment,
        align_y: Alignment,
    ) -> Element<'a, Message> {
        let area = mouse_area(container(Space::new()).width(width).height(height))
            .interaction(interaction)
            .on_press(Message::WindowResize(direction));

        container(area)
            .width(Fill)
            .height(Fill)
            .align_x(align_x)
            .align_y(align_y)
            .into()
    }

    stack![
        handle(
            Direction::North,
            Interaction::ResizingVertically,
            Length::Fill,
            Length::Fixed(edge_size),
            Alignment::Center,
            Alignment::Start,
        ),
        handle(
            Direction::South,
            Interaction::ResizingVertically,
            Length::Fill,
            Length::Fixed(edge_size),
            Alignment::Center,
            Alignment::End,
        ),
        handle(
            Direction::West,
            Interaction::ResizingHorizontally,
            Length::Fixed(edge_size),
            Length::Fill,
            Alignment::Start,
            Alignment::Center,
        ),
        handle(
            Direction::East,
            Interaction::ResizingHorizontally,
            Length::Fixed(edge_size),
            Length::Fill,
            Alignment::End,
            Alignment::Center,
        ),
        handle(
            Direction::NorthWest,
            Interaction::ResizingDiagonallyDown,
            Length::Fixed(corner_size),
            Length::Fixed(corner_size),
            Alignment::Start,
            Alignment::Start,
        ),
        handle(
            Direction::NorthEast,
            Interaction::ResizingDiagonallyUp,
            Length::Fixed(corner_size),
            Length::Fixed(corner_size),
            Alignment::End,
            Alignment::Start,
        ),
        handle(
            Direction::SouthWest,
            Interaction::ResizingDiagonallyUp,
            Length::Fixed(corner_size),
            Length::Fixed(corner_size),
            Alignment::Start,
            Alignment::End,
        ),
        handle(
            Direction::SouthEast,
            Interaction::ResizingDiagonallyDown,
            Length::Fixed(corner_size),
            Length::Fixed(corner_size),
            Alignment::End,
            Alignment::End,
        ),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
