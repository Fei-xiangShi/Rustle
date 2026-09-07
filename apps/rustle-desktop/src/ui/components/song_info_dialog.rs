//! Song info / editor dialog — glass-morphism modal, theme-aware

use iced::border::Radius;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Border, Color, Element, Length};

use crate::app::{Message, SongEditDialogState};
use crate::i18n::{Key, Locale};
use crate::ui::responsive::{
    CoverRadiusRole, LayoutProfile, RadiusRole, ResponsiveContext, TextRole, UiTokens,
};
use crate::ui::{theme, widgets};

// Visual dimensions are 1080P reference pixels resolved through `UiTokens`.
const COVER_SIZE: f32 = 192.0;

// ── Public ─────────────────────────────────────────

/// Content-only song editor body with token-scaled fields and a stacked compact layout.
pub fn view_edit_body(
    e: &SongEditDialogState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'_, Message> {
    let tokens = context.tokens;
    let id = e.song_id;
    let fields = column![
        inp(
            locale.get(Key::SongEditLabelTitle),
            &e.title,
            id,
            "title",
            tokens,
        ),
        inp(
            locale.get(Key::SongEditLabelArtist),
            &e.artist,
            id,
            "artist",
            tokens,
        ),
        inp(
            locale.get(Key::SongEditLabelAlbum),
            &e.album,
            id,
            "album",
            tokens,
        ),
        spacer(tokens, 2.0),
        row![
            inp2(
                "年份",
                &e.year.map_or_else(String::new, |y| y.to_string()),
                id,
                "year",
                tokens,
            ),
            spacer(tokens, 16.0),
            inp2(
                "曲目号",
                &e.track_number.map_or_else(String::new, |n| n.to_string()),
                id,
                "track_number",
                tokens,
            ),
        ],
        spacer(tokens, 2.0),
        inp("流派", &e.genre, id, "genre", tokens),
    ]
    .width(Length::Fill);

    let cover = edit_cover_block(
        &e.cover_path,
        locale.get(Key::SongEditCoverReplace),
        id,
        tokens,
    );

    let body: Element<'_, Message> = if matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        column![
            cover,
            iced::widget::Space::new().height(tokens.space(24.0)),
            fields
        ]
        .width(Length::Fill)
        .into()
    } else {
        row![cover, spacer(tokens, 32.0), fields]
            .align_y(iced::Alignment::Start)
            .into()
    };

    container(body).padding(tokens.space(24.0)).into()
}

// ── Cover ──────────────────────────────────────────

fn edit_cover_block<'a>(
    path: &Option<std::path::PathBuf>,
    replace_label: &str,
    song_id: i64,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'a, Message> {
    let cover_size = tokens.size(COVER_SIZE);
    let cover_radius = tokens.cover_radius(CoverRadiusRole::Card);
    let handle = path
        .as_ref()
        .filter(|p| p.exists())
        .map(|p| iced::widget::image::Handle::from_path(p.clone()));
    let img = crate::ui::components::cover_image::custom(
        handle.as_ref(),
        crate::image::ImageKind::SongCover,
        cover_size,
        cover_radius,
        tokens,
    );
    let label = replace_label.to_string();
    let replace_button = button(text(label).size(tokens.text(TextRole::Caption)))
        .style(dim_btn)
        .padding([tokens.space(4.0), tokens.space(12.0)])
        .on_press(Message::PickSongEditCover(song_id));
    let replace_button =
        widgets::hover_surface(replace_button).style(move |theme, progress| container::Style {
            background: Some(iced::Background::Color(theme::lerp_color(
                Color::TRANSPARENT,
                theme::surface_hover(theme),
                progress,
            ))),
            border: r(tokens.radius(RadiusRole::Medium)),
            ..Default::default()
        });

    column![
        container(img)
            .width(cover_size)
            .height(cover_size)
            .style(move |theme| cover_border(theme, cover_radius, tokens)),
        iced::widget::Space::new().height(tokens.space(6.0)),
        replace_button,
    ]
    .into()
}

// ── Fields ─────────────────────────────────────────

fn inp<'a>(
    label: &str,
    value: &str,
    sid: i64,
    field: &'static str,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'a, Message> {
    let l = label.to_string();
    let v = value.to_string();
    column![
        text(l)
            .size(tokens.text(TextRole::Caption))
            .style(st(theme::text_secondary)),
        iced::widget::Space::new().height(tokens.space(4.0)),
        text_input("", v)
            .on_input(move |x| msg_f(sid, field, x))
            .id(format!("song_edit_{field}"))
            .padding([tokens.space(8.0), tokens.space(10.0)])
            .size(tokens.text(TextRole::Body))
            .style(move |theme, status| {
                inp_s(theme, status, tokens.radius(RadiusRole::Medium), tokens)
            }),
        iced::widget::Space::new().height(tokens.space(10.0)),
    ]
    .width(Length::Fill)
    .into()
}

fn inp2<'a>(
    label: &str,
    value: &str,
    sid: i64,
    field: &'static str,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'a, Message> {
    let l = label.to_string();
    let v = value.to_string();
    column![
        text(l)
            .size(tokens.text(TextRole::Caption))
            .style(st(theme::text_secondary)),
        iced::widget::Space::new().height(tokens.space(4.0)),
        text_input("", v)
            .on_input(move |x| msg_f(sid, field, x))
            .id(format!("song_edit_{field}"))
            .width(Length::Fixed(tokens.size(90.0)))
            .padding([tokens.space(8.0), tokens.space(10.0)])
            .size(tokens.text(TextRole::Body))
            .style(move |theme, status| {
                inp_s(theme, status, tokens.radius(RadiusRole::Medium), tokens)
            }),
    ]
    .into()
}

fn msg_f(sid: i64, field: &str, v: String) -> Message {
    Message::SongEditFieldChanged {
        song_id: sid,
        field: field.into(),
        value: v,
    }
}

// ── Buttons ────────────────────────────────────────

fn dim_btn(t: &iced::Theme, s: button::Status) -> button::Style {
    let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
    let tc = if hover {
        theme::text_primary(t)
    } else {
        theme::text_secondary(t)
    };
    button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        text_color: tc,
        ..Default::default()
    }
}

fn inp_s(
    t: &iced::Theme,
    s: text_input::Status,
    radius: f32,
    tokens: UiTokens,
) -> text_input::Style {
    let purple = Color::from_rgba(0.659, 0.333, 0.969, 1.0);
    let c = match s {
        text_input::Status::Focused { .. } => Color::from_rgba(0.659, 0.333, 0.969, 0.50),
        text_input::Status::Hovered => Color::from_rgba(1.0, 1.0, 1.0, 0.20),
        _ => Color::from_rgba(1.0, 1.0, 1.0, 0.10),
    };
    text_input::Style {
        background: iced::Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.05)),
        border: Border {
            color: c,
            width: tokens.size(1.0),
            radius: Radius::new(radius),
        },
        placeholder: theme::TEXT_MUTED,
        value: theme::text_primary(t),
        selection: purple,
    }
}

// ── Theme-aware color helpers ──────────────────────

fn glass_border(t: &iced::Theme) -> Color {
    if theme::is_dark_theme(t) {
        Color::from_rgba(1.0, 1.0, 1.0, 0.10)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.10)
    }
}

fn r(v: f32) -> Border {
    Border {
        radius: Radius::new(v),
        ..Default::default()
    }
}
fn spacer<'a>(tokens: crate::ui::responsive::UiTokens, w: f32) -> Element<'a, Message> {
    let size = tokens.space(w);
    iced::widget::Space::new().width(size).height(size).into()
}

fn st(f: fn(&iced::Theme) -> Color) -> impl Fn(&iced::Theme) -> text::Style {
    move |t| text::Style { color: Some(f(t)) }
}

fn cover_border(t: &iced::Theme, radius: f32, tokens: UiTokens) -> container::Style {
    container::Style {
        border: Border {
            radius: Radius::new(radius),
            width: tokens.size(1.0),
            color: glass_border(t),
        },
        ..Default::default()
    }
}
