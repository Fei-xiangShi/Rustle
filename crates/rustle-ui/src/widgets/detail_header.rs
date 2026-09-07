//! Generic horizontal composition for detail-page identity headers.

use iced::widget::{Space, container, row};
use iced::{Alignment, Element, Fill, Padding};

use crate::responsive::{ResponsiveContext, detail_header_metrics};

/// Vertical relationship between the fixed artwork and flexible information
/// lane. The horizontal composition itself is invariant across page families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlignment {
    Center,
    End,
}

/// Place fixed-size artwork on the left and caller-owned identity information
/// in the remaining right lane.
///
/// The message type is generic because this widget owns no application action
/// or business state. Pages build their own interactive content before passing
/// it across this layout boundary.
pub fn view<'a, Message: 'a>(
    artwork: impl Into<Element<'a, Message>>,
    information: impl Into<Element<'a, Message>>,
    context: ResponsiveContext,
    vertical_alignment: VerticalAlignment,
) -> Element<'a, Message> {
    let metrics = detail_header_metrics(context);
    let alignment = match vertical_alignment {
        VerticalAlignment::Center => Alignment::Center,
        VerticalAlignment::End => Alignment::End,
    };

    row![
        artwork.into(),
        Space::new().width(metrics.gap),
        container(information).width(Fill),
    ]
    .align_y(alignment)
    .width(Fill)
    .padding(
        Padding::new(metrics.horizontal_padding)
            .top(metrics.top_padding)
            .bottom(metrics.bottom_padding),
    )
    .into()
}
