//! Discovery-first home page.

use iced::widget::{Space, column, container, responsive, row, scrollable, text};
use iced::{Element, Fill, Length, Padding};

use crate::api::PRIVATE_RADAR_PLAYLIST_ID;
use crate::app::{DiscoverPageState, DiscoverViewMode, ImageState, Message};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::components::{feature_card, playlist_grid};
use crate::ui::responsive::{
    CardRole, ResponsiveContext, TextRole, UiTokens, calculate_grid_columns_clamped, top_bar_height,
};
use crate::ui::theme;
use crate::ui::widgets::{self, section_header};

const DAILY_FEATURE_ID: u64 = 0;
const PERSONAL_FM_FEATURE_ID: u64 = u64::MAX;
const FEATURE_SCROLL_ID: &str = "discover_feature_scroll";
fn personal_fm_action() -> Message {
    Message::Navigate(crate::ui::components::NavItem::Radio)
}

fn page_padding(tokens: UiTokens) -> Padding {
    Padding::new(tokens.space(32.0))
}

pub fn view<'a>(
    state: &'a DiscoverPageState,
    image_state: &'a ImageState,
    locale: Locale,
    active_personal_fm_cover: Option<&'a iced::widget::image::Handle>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    match state.view_mode {
        DiscoverViewMode::Overview => view_overview(
            state,
            image_state,
            locale,
            active_personal_fm_cover,
            context,
        ),
        DiscoverViewMode::AllRecommended => view_all_playlists(
            state,
            image_state,
            locale,
            Key::DiscoverRecommended,
            &state.recommended_playlists,
            context,
        ),
        DiscoverViewMode::AllHot => view_all_playlists(
            state,
            image_state,
            locale,
            Key::DiscoverHot,
            &state.hot_playlists,
            context,
        ),
        DiscoverViewMode::AllOfficial => view_all_playlists(
            state,
            image_state,
            locale,
            Key::DiscoverOfficialPicks,
            &state.official_playlists,
            context,
        ),
    }
}

fn view_overview<'a>(
    state: &'a DiscoverPageState,
    image_state: &'a ImageState,
    locale: Locale,
    active_personal_fm_cover: Option<&'a iced::widget::image::Handle>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let feature_row = responsive(move |size| {
        personal_feature_row(
            state,
            image_state,
            locale,
            active_personal_fm_cover,
            feature_cards_fit(size.width, context),
            context,
        )
    })
    .width(Fill)
    .height(Length::Shrink);

    let content = column![
        Space::new().height(top_bar_height(&context) + tokens.space(4.0)),
        feature_row,
        Space::new().height(tokens.space(40.0)),
        section_header::view(
            locale.get(Key::DiscoverRecommended),
            locale.get(Key::DiscoverSeeAll),
            Some(Message::SeeAllRecommended),
            tokens,
        ),
        Space::new().height(tokens.space(16.0)),
        playlist_grid::view_single_row(
            &state.recommended_playlists,
            image_state,
            &state.card_animations,
            context,
        ),
        Space::new().height(tokens.space(40.0)),
        section_header::view(
            locale.get(Key::DiscoverHot),
            locale.get(Key::DiscoverSeeAll),
            Some(Message::SeeAllHot),
            tokens,
        ),
        Space::new().height(tokens.space(16.0)),
        playlist_grid::view_single_row(
            &state.hot_playlists,
            image_state,
            &state.card_animations,
            context,
        ),
        Space::new().height(tokens.space(40.0)),
        section_header::view(
            locale.get(Key::DiscoverOfficialPicks),
            locale.get(Key::DiscoverSeeAll),
            Some(Message::SeeAllOfficial),
            tokens,
        ),
        Space::new().height(tokens.space(16.0)),
        playlist_grid::view_single_row(
            &state.official_playlists,
            image_state,
            &state.card_animations,
            context,
        ),
        Space::new().height(tokens.space(40.0)),
    ]
    .padding(page_padding(tokens));

    container(widgets::page_scrollable(
        content,
        "discover_scroll",
        tokens,
        Message::SmoothScroll,
    ))
    .width(Fill)
    .height(Fill)
    .style(theme::main_content)
    .into()
}

fn personal_feature_row<'a>(
    state: &'a DiscoverPageState,
    image_state: &'a ImageState,
    locale: Locale,
    active_personal_fm_cover: Option<&'a iced::widget::image::Handle>,
    fits_inline: bool,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let feature_metrics = tokens.card(CardRole::Feature);
    let feature_width = if fits_inline {
        Length::FillPortion(1)
    } else {
        Length::Fixed(feature_metrics.width)
    };
    let day = chrono::Local::now().format("%d").to_string();
    let daily = feature_card::view(
        locale.get(Key::DiscoverDailyRecommend).to_string(),
        locale.get(Key::DiscoverDailyRecommendDesc).to_string(),
        Some(day),
        crate::ui::icons::CALENDAR,
        state
            .daily_recommend_preview
            .as_ref()
            .and_then(|track| image_state.get(ImageKind::SongCover, track.id)),
        (
            rustle_ui::theme::decoration::feature(0)[0],
            rustle_ui::theme::decoration::feature(0)[1],
        ),
        feature_width,
        state.card_animations.get_progress(&DAILY_FEATURE_ID),
        Message::OpenNcmPlaylist(DAILY_FEATURE_ID),
        Message::PlayDiscoverPlaylist(DAILY_FEATURE_ID),
        Message::HoverDiscoverPlaylist(Some(DAILY_FEATURE_ID)),
        Message::HoverDiscoverPlaylist(None),
        context,
    );

    let radar = state.private_radar.as_ref();
    let radar_title = radar
        .map(|playlist| playlist.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| locale.get(Key::DiscoverPrivateRadar).to_string());
    let radar_subtitle = locale.get(Key::DiscoverPrivateRadarDesc).to_string();
    let radar = feature_card::view(
        radar_title,
        radar_subtitle,
        None,
        crate::ui::icons::BROWSE,
        image_state.get(ImageKind::PlaylistCover, PRIVATE_RADAR_PLAYLIST_ID),
        (
            rustle_ui::theme::decoration::feature(1)[0],
            rustle_ui::theme::decoration::feature(1)[1],
        ),
        feature_width,
        state
            .card_animations
            .get_progress(&PRIVATE_RADAR_PLAYLIST_ID),
        Message::OpenNcmPlaylist(PRIVATE_RADAR_PLAYLIST_ID),
        Message::PlayDiscoverPlaylist(PRIVATE_RADAR_PLAYLIST_ID),
        Message::HoverDiscoverPlaylist(Some(PRIVATE_RADAR_PLAYLIST_ID)),
        Message::HoverDiscoverPlaylist(None),
        context,
    );

    let personal_fm_action = personal_fm_action();
    let personal_fm = feature_card::view(
        locale.get(Key::DiscoverPersonalFm).to_string(),
        locale.get(Key::DiscoverPersonalFmDesc).to_string(),
        None,
        crate::ui::icons::RADIO,
        active_personal_fm_cover.or_else(|| {
            state
                .personal_fm_preview
                .as_ref()
                .and_then(|track| image_state.get(ImageKind::SongCover, track.id))
        }),
        (
            rustle_ui::theme::decoration::feature(2)[0],
            rustle_ui::theme::decoration::feature(2)[1],
        ),
        feature_width,
        state.card_animations.get_progress(&PERSONAL_FM_FEATURE_ID),
        personal_fm_action.clone(),
        personal_fm_action,
        Message::HoverDiscoverPlaylist(Some(PERSONAL_FM_FEATURE_ID)),
        Message::HoverDiscoverPlaylist(None),
        context,
    );

    let cards = row![daily, radar, personal_fm].spacing(feature_metrics.gap);
    if fits_inline {
        cards.width(Fill).into()
    } else {
        widgets::scaled_scroll(
            scrollable(cards)
                .direction(widgets::hidden_horizontal_scrollbar())
                .id(iced::widget::Id::new(FEATURE_SCROLL_ID))
                .width(Fill)
                .height(feature_metrics.height),
            tokens,
        )
        .into()
    }
}

fn feature_cards_fit(available_width: f32, context: ResponsiveContext) -> bool {
    let metrics = context.tokens.card(CardRole::Feature);
    calculate_grid_columns_clamped(available_width, metrics.width, metrics.gap, 3) == 3
}

fn view_all_playlists<'a>(
    state: &'a DiscoverPageState,
    image_state: &'a ImageState,
    locale: Locale,
    title: Key,
    playlists: &'a [crate::api::PlaylistSummary],
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let content = column![
        Space::new().height(top_bar_height(&context) + tokens.space(4.0)),
        text(locale.get(title)).size(tokens.text(TextRole::Title)),
        Space::new().height(tokens.space(24.0)),
        playlist_grid::view(
            playlists,
            image_state,
            &state.card_animations,
            None,
            context,
        ),
        Space::new().height(tokens.space(40.0)),
    ]
    .padding(page_padding(tokens));

    container(widgets::page_scrollable(
        content,
        "discover_scroll",
        tokens,
        Message::SmoothScroll,
    ))
    .width(Fill)
    .height(Fill)
    .style(theme::main_content)
    .into()
}

#[cfg(test)]
mod tests {
    use super::{feature_cards_fit, personal_fm_action};
    use crate::app::Message;
    use crate::ui::components::NavItem;
    use crate::ui::responsive::ResponsiveContext;
    use iced::Size;

    #[test]
    fn personal_fm_card_reuses_sidebar_navigation_action() {
        assert!(matches!(
            personal_fm_action(),
            Message::Navigate(NavItem::Radio)
        ));
    }

    #[test]
    fn feature_cards_scroll_as_one_horizontal_sequence_when_three_do_not_fit() {
        let fixtures = [
            (Size::new(1_920.0, 1_080.0), 1_575.0, true),
            (Size::new(2_560.0, 1_440.0), 2_100.0, true),
            (Size::new(960.0, 1_080.0), 833.0, false),
            (Size::new(768.0, 1_024.0), 647.0, false),
            (Size::new(720.0, 800.0), 605.0, false),
            (Size::new(960.0, 540.0), 845.0, false),
            (Size::new(560.0, 800.0), 506.0, false),
        ];

        for (viewport, available_width, expected_inline) in fixtures {
            let context = ResponsiveContext::from_viewport(viewport);
            assert_eq!(
                feature_cards_fit(available_width, context),
                expected_inline,
                "unexpected feature-card composition for {viewport:?}"
            );
        }
    }
}
