//! Responsive layout helpers shared by grid-like pages.

use iced::widget::{container, responsive, scrollable};
use iced::{Element, Fill, Length};

use crate::animation::{SmoothScrollEvent, SmoothScrollTarget};
use crate::responsive::{CardMetrics, UiTokens, calculate_grid_columns_clamped};
use crate::widgets::smooth_scroll;

const SCROLLBAR_REFERENCE_WIDTH: f32 = 10.0;

fn scrollbar(tokens: UiTokens) -> scrollable::Scrollbar {
    scrollable::Scrollbar::new()
        .width(tokens.size(SCROLLBAR_REFERENCE_WIDTH))
        .scroller_width(tokens.size(SCROLLBAR_REFERENCE_WIDTH))
}

/// Root-rem-scaled vertical scrollbar geometry.
pub fn vertical_scrollbar(tokens: UiTokens) -> scrollable::Direction {
    scrollable::Direction::Vertical(scrollbar(tokens))
}

/// Fully hidden vertical scrollbar without iced's default hit-width fallback.
pub fn hidden_vertical_scrollbar() -> scrollable::Direction {
    scrollable::Direction::Vertical(scrollable::Scrollbar::hidden())
}

/// Fully hidden horizontal scrollbar without iced's default hit-width fallback.
pub fn hidden_horizontal_scrollbar() -> scrollable::Direction {
    scrollable::Direction::Horizontal(scrollable::Scrollbar::hidden())
}

/// Build a stable vertical page scroll surface without persisting layout-only
/// measurements in application state.
pub fn page_scrollable<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    scroll_id: &'static str,
    tokens: UiTokens,
    on_scroll_event: impl Fn(SmoothScrollEvent) -> Message + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    smooth_scroll(
        scrollable(container(content).width(Fill))
            .width(Fill)
            .height(Fill)
            .id(iced::widget::Id::new(scroll_id))
            .direction(vertical_scrollbar(tokens))
            .style(move |theme, status| {
                crate::theme::dark_scrollable(theme, status, tokens.theme_metrics())
            }),
        SmoothScrollTarget::Native(scroll_id),
        tokens,
        on_scroll_event,
    )
    .into()
}

/// Build card rows from the exact width supplied by the current Iced layout
/// pass. The callback is rebuilt when its parent width changes, so view-mode
/// switches and window restoration cannot reuse a stale asynchronous value.
pub fn responsive_card_columns<'a, Message>(
    metrics: CardMetrics,
    max_columns: usize,
    view: impl Fn(usize) -> Element<'a, Message> + 'a,
) -> Element<'a, Message>
where
    Message: 'a,
{
    responsive(move |size| {
        let columns =
            calculate_grid_columns_clamped(size.width, metrics.width, metrics.gap, max_columns);

        view(columns)
    })
    .width(Fill)
    .height(Length::Shrink)
    .into()
}
