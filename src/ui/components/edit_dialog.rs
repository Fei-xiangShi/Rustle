//! Edit playlist dialog component

use iced::widget::{Space, button, column, container, row, svg, text, text_input, toggler};
use iced::{Alignment, Color, Element, Fill};

use crate::app::Message;
use crate::i18n::{Key, Locale};
use crate::ui::responsive::{
    CoverRadiusRole, LayoutProfile, RadiusRole, ResponsiveContext, TextRole,
};
use crate::ui::{icons, theme, widgets};

pub struct PlaylistEditorView<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub cover_path: Option<&'a str>,
    pub watch_available: bool,
    pub watch_enabled: bool,
    pub watch_path: Option<&'a str>,
    pub locale: Locale,
    pub context: ResponsiveContext,
}

/// Render the playlist editor with token-scaled controls and a stacked tablet fallback.
pub fn view_body<'a>(props: PlaylistEditorView<'a>) -> Element<'a, Message> {
    let PlaylistEditorView {
        name,
        description,
        cover_path,
        watch_available,
        watch_enabled,
        watch_path,
        locale,
        context,
    } = props;
    let tokens = context.tokens;
    let cover_size = tokens.size(120.0);
    let cover_radius = tokens.cover_radius(CoverRadiusRole::Card);

    // Cover
    let cover_content: Element<'a, Message> = if let Some(path) = cover_path {
        widgets::crossfade_image(Some(iced::widget::image::Handle::from_path(path)))
            .width(cover_size)
            .height(cover_size)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(cover_radius)
            .into()
    } else {
        container(
            svg(svg::Handle::from_memory(icons::MUSIC.as_bytes()))
                .width(tokens.size(40.0))
                .height(tokens.size(40.0))
                .style(|_theme, _status| svg::Style {
                    color: Some(theme::opaque_color(theme::icon_muted(&iced::Theme::Dark))),
                })
                .opacity(0.4_f32),
        )
        .width(cover_size)
        .height(cover_size)
        .center_x(cover_size)
        .center_y(cover_size)
        .style(move |theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::surface_container(theme))),
            border: iced::Border {
                radius: cover_radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };

    let change_cover_btn = button(
        text(locale.get(Key::EditPlaylistChangeCover).to_string())
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            }),
    )
    .padding([tokens.space(4.0), tokens.space(8.0)])
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        ..Default::default()
    })
    .on_press(Message::PickCoverImage);
    let change_cover_btn =
        widgets::hover_surface(change_cover_btn).style(move |theme, progress| {
            iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::hover_bg_alpha(
                    theme,
                    0.12 * progress,
                ))),
                border: iced::Border {
                    radius: tokens.radius(RadiusRole::Small).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let cover_section = column![
        container(cover_content)
            .width(cover_size)
            .height(cover_size)
            .style(move |_theme| iced::widget::container::Style {
                border: iced::Border {
                    radius: cover_radius.into(),
                    width: tokens.size(1.0),
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.06)
                },
                ..Default::default()
            }),
        Space::new().height(tokens.space(8.0)),
        change_cover_btn,
    ]
    .align_x(Alignment::Center);

    // Form fields
    let name_label = text(locale.get(Key::EditPlaylistName).to_string())
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_secondary(theme)),
        });
    let name_input = text_input(
        locale.get(Key::EditPlaylistNamePlaceholder),
        name.to_string(),
    )
    .on_input(Message::EditPlaylistNameChanged)
    .id(iced::widget::Id::new("edit_playlist_name"))
    .padding(tokens.space(12.0))
    .size(tokens.text(TextRole::BodyLarge))
    .style(move |theme, _status| text_input::Style {
        background: iced::Background::Color(theme::surface_container(theme)),
        border: iced::Border {
            color: theme::divider(theme),
            width: tokens.size(1.0),
            radius: tokens.radius(RadiusRole::Small).into(),
        },
        placeholder: theme::text_muted(theme),
        value: theme::text_primary(theme),
        selection: theme::ACCENT_PINK,
    });
    let desc_label = text(locale.get(Key::EditPlaylistDesc).to_string())
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::text_secondary(theme)),
        });
    let desc_input = text_input(
        locale.get(Key::EditPlaylistDescPlaceholder),
        description.to_string(),
    )
    .on_input(Message::EditPlaylistDescriptionChanged)
    .id(iced::widget::Id::new("edit_playlist_description"))
    .padding(tokens.space(12.0))
    .size(tokens.text(TextRole::BodyLarge))
    .style(move |theme, _status| text_input::Style {
        background: iced::Background::Color(theme::surface_container(theme)),
        border: iced::Border {
            color: theme::divider(theme),
            width: tokens.size(1.0),
            radius: tokens.radius(RadiusRole::Small).into(),
        },
        placeholder: theme::text_muted(theme),
        value: theme::text_primary(theme),
        selection: theme::ACCENT_PINK,
    });

    let watch_section: Element<'a, Message> = if watch_available {
        container(
            column![
                row![
                    column![
                        text(locale.get(Key::EditPlaylistWatchLibrary).to_string())
                            .size(tokens.text(TextRole::Body))
                            .style(|theme| text::Style {
                                color: Some(theme::text_secondary(theme))
                            }),
                        Space::new().height(tokens.space(4.0)),
                        text(locale.get(Key::EditPlaylistWatchLibraryDesc).to_string())
                            .size(tokens.text(TextRole::Caption))
                            .style(|theme| text::Style {
                                color: Some(theme::text_muted(theme))
                            }),
                    ]
                    .spacing(0)
                    .width(Fill),
                    toggler(watch_enabled)
                        .on_toggle(Message::EditPlaylistWatchEnabledChanged)
                        .size(tokens.text(TextRole::Title)),
                ]
                .align_y(Alignment::Center),
                Space::new().height(tokens.space(8.0)),
                text(watch_path.unwrap_or_default())
                    .size(tokens.text(TextRole::Caption))
                    .style(|theme| text::Style {
                        color: Some(theme::text_muted(theme))
                    }),
            ]
            .spacing(0),
        )
        .padding(tokens.space(12.0))
        .style(move |theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::surface_container(theme))),
            border: iced::Border {
                color: theme::divider(theme),
                width: tokens.size(1.0),
                radius: tokens.radius(RadiusRole::Small).into(),
            },
            ..Default::default()
        })
        .into()
    } else {
        Space::new().height(0).into()
    };
    let watch_spacing: Element<'a, Message> = if watch_available {
        Space::new().height(tokens.space(12.0)).into()
    } else {
        Space::new().height(0).into()
    };

    let form_fields = column![
        name_label,
        Space::new().height(tokens.space(8.0)),
        name_input,
        Space::new().height(tokens.space(12.0)),
        desc_label,
        Space::new().height(tokens.space(8.0)),
        desc_input,
        watch_spacing,
        watch_section,
    ]
    .width(Fill);

    if matches!(
        context.profile,
        LayoutProfile::Tablet | LayoutProfile::Narrow
    ) {
        column![
            cover_section,
            Space::new().height(tokens.space(24.0)),
            form_fields
        ]
        .width(Fill)
        .into()
    } else {
        row![
            cover_section,
            Space::new().width(tokens.space(24.0)),
            form_fields
        ]
        .align_y(Alignment::Start)
        .into()
    }
}
