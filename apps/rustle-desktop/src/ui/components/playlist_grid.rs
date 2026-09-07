//! Responsive playlist grid and single-row layouts.

use iced::widget::{Space, column, responsive, row};
use iced::{Element, Fill};

use crate::api::PlaylistSummary;
use crate::app::{ImageState, Message};
use crate::image::ImageKind;
use crate::ui::animation::HoverAnimations;
use crate::ui::responsive::{
    CardMetrics, ResponsiveContext, playlist_card_metrics, playlist_grid_layout_for_width,
};
use crate::ui::widgets::playlist_card;

fn card<'a>(
    playlist: &'a PlaylistSummary,
    image_state: &'a ImageState,
    animations: &'a HoverAnimations<u64>,
    metrics: CardMetrics,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let hover_progress = animations.get_progress(&playlist.id);
    playlist_card::view_with_metrics(
        &playlist.name,
        image_state.get(ImageKind::PlaylistCover, playlist.id),
        image_state.get_playlist_footer(playlist.id),
        hover_progress,
        Message::OpenNcmPlaylist(playlist.id),
        Message::PlayDiscoverPlaylist(playlist.id),
        Message::HoverDiscoverPlaylist(Some(playlist.id)),
        Message::HoverDiscoverPlaylist(None),
        metrics,
        context.tokens,
    )
}

/// Render one complete responsive row using the shared card token policy.
pub fn view_single_row<'a>(
    playlists: &'a [PlaylistSummary],
    image_state: &'a ImageState,
    animations: &'a HoverAnimations<u64>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    responsive(move |size| -> Element<'a, Message> {
        let layout = playlist_grid_layout_for_width(context, size.width, 8);
        let metrics = layout.metrics;
        let columns = layout.columns;
        let cards = playlists
            .iter()
            .take(columns)
            .map(|playlist| card(playlist, image_state, animations, metrics, context))
            .collect::<Vec<_>>();

        if cards.is_empty() {
            Space::new().width(Fill).height(metrics.height).into()
        } else {
            row(cards).spacing(metrics.gap).into()
        }
    })
    .width(Fill)
    .height(iced::Length::Shrink)
    .into()
}

/// Render a complete responsive wrapping grid for full-list views.
pub fn view<'a>(
    playlists: &'a [PlaylistSummary],
    image_state: &'a ImageState,
    animations: &'a HoverAnimations<u64>,
    max_items: Option<usize>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let base_metrics = playlist_card_metrics(context);
    let items = playlists
        .iter()
        .take(max_items.unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Space::new().width(Fill).height(base_metrics.height).into();
    }

    responsive(move |size| -> Element<'a, Message> {
        let layout = playlist_grid_layout_for_width(context, size.width, 8);
        let metrics = layout.metrics;
        let columns = layout.columns;
        let rows = items
            .chunks(columns)
            .map(|chunk| {
                let cards = chunk
                    .iter()
                    .map(|playlist| card(playlist, image_state, animations, metrics, context))
                    .collect::<Vec<_>>();
                row(cards).spacing(metrics.gap).into()
            })
            .collect::<Vec<Element<'a, Message>>>();

        column(rows).spacing(metrics.gap).into()
    })
    .width(Fill)
    .height(iced::Length::Shrink)
    .into()
}

#[cfg(test)]
mod tests {
    use crate::ui::responsive::{ResponsiveContext, playlist_grid_layout_for_width};
    use iced::Size;

    fn visible_column_count(available_width: f32, context: ResponsiveContext) -> usize {
        playlist_grid_layout_for_width(context, available_width, 8).columns
    }

    #[test]
    fn responsive_row_never_reports_zero_columns() {
        let context = ResponsiveContext::from_viewport(Size::new(1_920.0, 1_080.0));
        assert!(visible_column_count(0.0, context) >= 1);
        assert!(visible_column_count(160.0, context) >= 1);
    }

    #[test]
    fn contextual_grid_uses_complete_cards_for_validation_viewports() {
        let fixtures = [
            (Size::new(1_440.0, 900.0), 1_116.0, 6),
            (Size::new(1_920.0, 1_080.0), 1_560.0, 8),
            (Size::new(2_560.0, 1_440.0), 2_080.0, 8),
            (Size::new(2_558.0, 1_398.0), 2_078.5, 8),
            (Size::new(960.0, 1_080.0), 828.0, 5),
            (Size::new(1_280.0, 1_440.0), 1_104.0, 5),
            (Size::new(1_280.0, 720.0), 969.0, 5),
            (Size::new(768.0, 1_024.0), 642.0, 3),
            (Size::new(720.0, 800.0), 601.0, 3),
            (Size::new(960.0, 540.0), 841.0, 5),
            (Size::new(560.0, 800.0), 502.0, 3),
        ];

        for (viewport, available_width, expected_columns) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            assert_eq!(
                visible_column_count(available_width, context),
                expected_columns,
                "unexpected complete playlist-card columns for {viewport:?}"
            );
        }
    }

    #[test]
    fn expanded_rows_fill_six_or_eight_complete_cards_without_touching_half_width() {
        let fixtures = [
            (Size::new(1_440.0, 900.0), 1_116.0, 6),
            (Size::new(1_920.0, 1_080.0), 1_560.0, 8),
            (Size::new(2_560.0, 1_440.0), 2_080.0, 8),
            (Size::new(960.0, 1_080.0), 828.0, 5),
        ];

        for (viewport, available_width, expected_columns) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            let layout = playlist_grid_layout_for_width(context, available_width, 8);
            assert_eq!(layout.columns, expected_columns);
            assert!(layout.metrics.width > 0.0);
            assert!(
                layout.metrics.width * expected_columns as f32
                    + layout.metrics.gap * expected_columns.saturating_sub(1) as f32
                    <= available_width + 0.001
            );
        }
    }
}
