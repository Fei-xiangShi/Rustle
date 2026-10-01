//! Download management panel — matching settings page layout conventions

use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{Space, button, column, container, row, scrollable, svg, text};
use iced::{Alignment, Background, Color, Element, Length, Padding};

use crate::app::{DownloadTab, ImageState, Message};
use crate::download::{DownloadManager, DownloadStatus, DownloadTask};
use crate::i18n::{Key, Locale};
use crate::image::ImageKind;
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::components::cover_image;
use crate::ui::responsive::{
    CoverRadiusRole, LayoutProfile, RadiusRole, ResponsiveContext, TargetRole, TextRole,
    top_bar_height,
};
use crate::ui::theme;

pub fn download_panel(
    locale: Locale,
    manager: &DownloadManager,
    tab: DownloadTab,
    image_state: &ImageState,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let header = column![
        text(locale.get(Key::DownloadPanelTitle))
            .size(tokens.text(TextRole::Hero))
            .style(|theme| text::Style {
                color: Some(theme::text_primary(theme))
            }),
        text("管理你的本地离线音乐库")
            .size(tokens.text(TextRole::Label))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            }),
    ]
    .spacing(tokens.space(6.0));

    let active_count = manager.active.len() + manager.pending.len();
    let completed_count = manager.completed.len();
    let failed_count = manager.failed.len();
    let tab_active = make_tab_button(
        format!("{} ({})", locale.get(Key::DownloadActive), active_count),
        tab == DownloadTab::Active,
        Message::SwitchDownloadTab(DownloadTab::Active),
        context,
    );
    let tab_done = make_tab_button(
        format!(
            "{} ({})",
            locale.get(Key::DownloadCompleted),
            completed_count
        ),
        tab == DownloadTab::Completed,
        Message::SwitchDownloadTab(DownloadTab::Completed),
        context,
    );
    let tab_failed = make_tab_button(
        format!("{} ({})", locale.get(Key::DownloadFailed), failed_count),
        tab == DownloadTab::Failed,
        Message::SwitchDownloadTab(DownloadTab::Failed),
        context,
    );

    let body: Element<'static, Message> = match tab {
        DownloadTab::Active => {
            if active_count == 0 {
                empty_placeholder(locale.get(Key::DownloadNoActive).to_string(), context)
            } else {
                let cards = manager
                    .active
                    .iter()
                    .map(|task| build_active_card(task, image_state, locale, context))
                    .chain(
                        manager
                            .pending
                            .iter()
                            .map(|task| build_pending_card(task, image_state, locale, context)),
                    )
                    .collect::<Vec<_>>();
                column(cards).spacing(tokens.space(16.0)).into()
            }
        }
        DownloadTab::Completed => {
            if manager.completed.is_empty() {
                empty_placeholder(locale.get(Key::DownloadNoCompleted).to_string(), context)
            } else {
                column(
                    manager
                        .completed
                        .iter()
                        .map(|t| build_completed_card(t, image_state, context))
                        .collect::<Vec<_>>(),
                )
                .spacing(tokens.space(8.0))
                .into()
            }
        }
        DownloadTab::Failed => {
            if manager.failed.is_empty() {
                empty_placeholder(locale.get(Key::DownloadNoFailed).to_string(), context)
            } else {
                column(
                    manager
                        .failed
                        .iter()
                        .map(|t| build_failed_card(t, image_state, locale, context))
                        .collect::<Vec<_>>(),
                )
                .spacing(tokens.space(12.0))
                .into()
            }
        }
    };

    // Fixed header — matches settings header_container
    let header_container = container(
        column![
            header,
            Space::new().height(tokens.space(24.0)),
            crate::ui::widgets::scaled_scroll(
                scrollable(row![tab_active, tab_done, tab_failed].spacing(0))
                    .direction(crate::ui::widgets::hidden_horizontal_scrollbar())
                    .id(iced::widget::Id::new("downloads_tabs_scroll"))
                    .width(Length::Fill),
                tokens,
            ),
        ]
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(
        Padding::new(tokens.space(40.0))
            .top(top_bar_height(&context))
            .right(tokens.space(32.0))
            .bottom(tokens.space(20.0))
            .left(tokens.space(32.0)),
    )
    .style(|theme| container::Style {
        background: Some(Background::Color(theme::background(theme))),
        ..Default::default()
    });

    // Scrollable body — matches settings scrollable_content
    let scrollable_content = crate::ui::widgets::smooth_scroll(
        scrollable(
            container(body).width(Length::Fill).padding(
                Padding::new(tokens.space(20.0))
                    .right(tokens.space(32.0))
                    .bottom(tokens.space(60.0))
                    .left(tokens.space(32.0)),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .direction(crate::ui::widgets::vertical_scrollbar(tokens))
        .id(iced::widget::Id::new("downloads_scroll")),
        SmoothScrollTarget::Native("downloads_scroll"),
        tokens,
        Message::SmoothScroll,
    );

    container(
        column![header_container, scrollable_content]
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::main_content)
    .into()
}

fn empty_placeholder(msg: String, context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    container(
        text(msg)
            .size(tokens.text(TextRole::Body))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            }),
    )
    .width(Length::Fill)
    .center_x(Length::Fill)
    .padding(tokens.space(60.0))
    .into()
}

// ── Tabs (matches settings brand accent style) ────────────────────────────────

fn make_tab_button(
    label: String,
    active: bool,
    msg: Message,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let underline: Element<'static, Message> = if active {
        container(Space::new().height(tokens.size(2.0)))
            .width(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(Background::Color(theme::accent(_theme))),
                ..Default::default()
            })
            .into()
    } else {
        Space::new().height(tokens.size(2.0)).into()
    };
    column![
        button(
            text(label)
                .size(tokens.text(TextRole::Body))
                .style(move |theme| text::Style {
                    color: Some(if active {
                        theme::accent(theme)
                    } else {
                        theme::text_muted(theme)
                    }),
                }),
        )
        .on_press(msg)
        .padding([tokens.space(12.0), 0.0])
        .style(|_theme, status| {
            let bg = if status == button::Status::Hovered {
                Background::Color(theme::accent_subtle(_theme))
            } else {
                Background::Color(Color::TRANSPARENT)
            };
            button::Style {
                background: Some(bg),
                ..Default::default()
            }
        }),
        underline,
    ]
    .width(tokens.size(110.0))
    .align_x(Alignment::Center)
    .into()
}

// ── Active download card ──────────────────────────────────────────────────

fn build_active_card(
    task: &DownloadTask,
    image_state: &ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let (progress, speed) = match &task.status {
        DownloadStatus::Active { progress, speed } => (*progress, speed.clone()),
        _ => (0.0, String::new()),
    };
    let title = task.metadata.title.clone();
    let artist = task.metadata.artist.clone();
    let song_id = task.song_id;

    let (cover_kind, cover_id) = (ImageKind::SongCover, task.ncm_id);
    let cover = download_cover(image_state.get(cover_kind, cover_id), cover_kind, tokens);

    let info = column![
        text(title)
            .size(tokens.text(TextRole::BodyLarge))
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::text_primary(theme))
            }),
        text(artist)
            .size(tokens.text(TextRole::Label))
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::text_secondary(theme))
            }),
    ]
    .spacing(tokens.space(2.0));

    // Responsive progress bar with FillPortion
    let pct = (progress * 100.0) as u32;
    let bounded_progress = if progress.is_nan() {
        0.01
    } else {
        progress.clamp(0.01, 1.0)
    };
    let fill_portion = (bounded_progress * 1000.0) as u16;
    let empty_portion = 1000u16.saturating_sub(fill_portion);
    let bar = container(
        row![
            container(Space::new().height(tokens.size(4.0)))
                .width(Length::FillPortion(fill_portion))
                .style(|_theme| container::Style {
                    background: Some(Background::Color(theme::accent(_theme))),
                    ..Default::default()
                }),
            Space::new().width(Length::FillPortion(empty_portion)),
        ]
        .width(Length::Fill),
    )
    .style(move |_theme| container::Style {
        background: Some(Background::Color(rustle_ui::theme::white(0.06))),
        border: iced::Border {
            radius: tokens.radius(RadiusRole::Small).into(),
            ..Default::default()
        },
        ..Default::default()
    });

    let downloaded_mb = progress * task.file_size as f32 / 1_048_576.0;
    let total_mb = task.file_size as f32 / 1_048_576.0;
    let progress_info = column![
        row![
            text(format!("{:.1} MB / {:.1} MB", downloaded_mb, total_mb))
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            Space::new().width(Length::Fill),
            text(format!("{} {}", locale.get(Key::DownloadSpeed), speed))
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            text("·")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            text(format!("{}%", pct))
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
        ],
        Space::new().height(tokens.space(4.0)),
        bar,
    ]
    .spacing(tokens.space(2.0));

    let cancel_btn = icon_danger_btn(song_id, tokens);

    let card_content: Element<'static, Message> = match context.profile {
        LayoutProfile::Tablet | LayoutProfile::Narrow => column![
            row![cover, info.width(Length::Fill), cancel_btn]
                .align_y(Alignment::Center)
                .spacing(tokens.space(16.0)),
            progress_info,
        ]
        .spacing(tokens.space(10.0))
        .width(Length::Fill)
        .into(),
        LayoutProfile::Expanded | LayoutProfile::Standard | LayoutProfile::Compact => row![
            cover,
            column![info, progress_info]
                .spacing(tokens.space(10.0))
                .width(Length::Fill),
            cancel_btn,
        ]
        .align_y(Alignment::Center)
        .spacing(tokens.space(16.0))
        .into(),
    };

    container(card_content)
        .padding(tokens.space(16.0))
        .style(move |theme| container::Style {
            background: Some(Background::Color(theme::panel_bg(theme))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                width: tokens.size(1.0),
                color: theme::panel_border(theme),
            },
            ..Default::default()
        })
        .into()
}

fn build_pending_card(
    task: &DownloadTask,
    image_state: &ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let title = task.metadata.title.clone();
    let artist = task.metadata.artist.clone();
    let song_id = task.song_id;

    let (cover_kind, cover_id) = (ImageKind::SongCover, task.ncm_id);
    let cover = download_cover(image_state.get(cover_kind, cover_id), cover_kind, tokens);

    let info = column![
        text(title)
            .size(tokens.text(TextRole::BodyLarge))
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::text_primary(theme))
            }),
        row![
            text(artist)
                .size(tokens.text(TextRole::Label))
                .width(Length::Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(|theme| text::Style {
                    color: Some(theme::text_secondary(theme))
                }),
            text("·")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            text(locale.get(Key::DownloadPending))
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            text("·")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            quality_badge_el(
                task.quality.display_name(),
                quality_tone(task.quality),
                tokens
            ),
        ]
        .spacing(tokens.space(8.0))
        .align_y(Alignment::Center),
    ]
    .spacing(tokens.space(4.0));

    container(
        row![
            cover,
            info.width(Length::Fill),
            icon_danger_btn(song_id, tokens),
        ]
        .align_y(Alignment::Center)
        .spacing(tokens.space(16.0)),
    )
    .padding(tokens.space(16.0))
    .style(move |theme| container::Style {
        background: Some(Background::Color(theme::panel_bg(theme))),
        border: iced::Border {
            radius: tokens.radius(RadiusRole::Medium).into(),
            width: tokens.size(1.0),
            color: theme::panel_border(theme),
        },
        ..Default::default()
    })
    .into()
}

// ── Completed download card ───────────────────────────────────────────────

fn build_completed_card(
    task: &DownloadTask,
    image_state: &ImageState,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let title = task.metadata.title.clone();
    let artist = task.metadata.artist.clone();
    let quality = task.quality;
    let size_str = format_size(task.file_size);
    let song_id = task.song_id;
    let downloaded_time = task
        .downloaded_at
        .map(crate::utils::format_relative_time)
        .unwrap_or_default();

    let title_row = row![
        text(title)
            .size(tokens.text(TextRole::Body))
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::text_primary(theme))
            }),
        quality_badge_el(quality.display_name(), quality_tone(quality), tokens),
    ]
    .spacing(tokens.space(8.0))
    .align_y(Alignment::Center);

    let artist_text = text(artist)
        .size(tokens.text(TextRole::Label))
        .width(Length::Fill)
        .wrapping(Wrapping::None)
        .ellipsis(Ellipsis::End)
        .style(|theme| text::Style {
            color: Some(theme::text_muted(theme)),
        });

    let metadata = row![
        text(size_str)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            }),
        text("·")
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            }),
        text(downloaded_time)
            .size(tokens.text(TextRole::Caption))
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme)),
            }),
    ]
    .spacing(tokens.space(8.0))
    .align_y(Alignment::Center);

    // Action buttons using SVG icons
    let actions = row![
        svg_btn(crate::ui::icons::PLAY, Message::PlaySong(song_id), tokens),
        svg_btn(
            crate::ui::icons::EDIT,
            Message::EditSongTags(song_id),
            tokens
        ),
        svg_btn(
            crate::ui::icons::TRASH,
            Message::DeleteDownloadHistory(song_id),
            tokens,
        ),
    ]
    .spacing(tokens.space(4.0));

    let (cover_kind, cover_id) = (ImageKind::SongCover, task.ncm_id);
    let cover = download_cover(image_state.get(cover_kind, cover_id), cover_kind, tokens);

    let content: Element<'static, Message> = match context.profile {
        LayoutProfile::Tablet | LayoutProfile::Narrow => column![
            row![
                cover,
                column![title_row, artist_text]
                    .spacing(tokens.space(4.0))
                    .width(Length::Fill),
                actions
            ]
            .align_y(Alignment::Center)
            .spacing(tokens.space(16.0)),
            metadata,
        ]
        .width(Length::Fill)
        .spacing(tokens.space(8.0))
        .into(),
        LayoutProfile::Expanded | LayoutProfile::Standard | LayoutProfile::Compact => {
            let details = row![
                artist_text,
                text("·")
                    .size(tokens.text(TextRole::Caption))
                    .style(|theme| text::Style {
                        color: Some(theme::text_muted(theme)),
                    }),
                metadata,
            ]
            .spacing(tokens.space(8.0))
            .align_y(Alignment::Center);
            let info = column![title_row, details]
                .spacing(tokens.space(4.0))
                .width(Length::Fill);

            row![cover, info, actions]
                .align_y(Alignment::Center)
                .spacing(tokens.space(16.0))
                .into()
        }
    };

    container(content)
        .padding(tokens.space(12.0))
        .style(move |theme| container::Style {
            background: Some(Background::Color(theme::panel_bg(theme))),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
            ..Default::default()
        })
        .into()
}

fn build_failed_card(
    task: &DownloadTask,
    image_state: &ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let title = task.metadata.title.clone();
    let artist = task.metadata.artist.clone();
    let song_id = task.song_id;
    let error = match &task.status {
        DownloadStatus::Failed(error) => error.user_summary().to_owned(),
        _ => String::new(),
    };

    let (cover_kind, cover_id) = (ImageKind::SongCover, task.ncm_id);
    let cover = download_cover(image_state.get(cover_kind, cover_id), cover_kind, tokens);

    let info = column![
        row![
            text(title)
                .size(tokens.text(TextRole::BodyLarge))
                .width(Length::Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(|theme| text::Style {
                    color: Some(theme::text_primary(theme))
                }),
            quality_badge_el(
                task.quality.display_name(),
                quality_tone(task.quality),
                tokens
            ),
        ]
        .spacing(tokens.space(8.0))
        .align_y(Alignment::Center),
        row![
            text(artist)
                .size(tokens.text(TextRole::Label))
                .width(Length::Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(|theme| text::Style {
                    color: Some(theme::text_secondary(theme))
                }),
            text("·")
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme))
                }),
            text(locale.get(Key::DownloadFailed))
                .size(tokens.text(TextRole::Caption))
                .style(|_theme| text::Style {
                    color: Some(theme::danger(_theme))
                }),
        ]
        .spacing(tokens.space(8.0))
        .align_y(Alignment::Center),
        text(error)
            .size(tokens.text(TextRole::Caption))
            .width(Length::Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            }),
    ]
    .spacing(tokens.space(4.0));

    container(
        row![
            cover,
            info.width(Length::Fill),
            icon_danger_btn(song_id, tokens),
        ]
        .align_y(Alignment::Center)
        .spacing(tokens.space(16.0)),
    )
    .padding(tokens.space(16.0))
    .style(move |theme| container::Style {
        background: Some(Background::Color(theme::panel_bg(theme))),
        border: iced::Border {
            radius: tokens.radius(RadiusRole::Medium).into(),
            width: tokens.size(1.0),
            color: theme::panel_border(theme),
        },
        ..Default::default()
    })
    .into()
}

fn quality_tone(
    quality: crate::features::settings::MusicQuality,
) -> rustle_ui::theme::decoration::Tone {
    use crate::features::settings::MusicQuality;
    use rustle_ui::theme::decoration::Tone;
    match quality {
        MusicQuality::Standard => Tone::Slate,
        MusicQuality::Higher | MusicQuality::Dolby => Tone::Cyan,
        MusicQuality::High => Tone::Green,
        MusicQuality::Lossless | MusicQuality::JvEffect => Tone::Violet,
        MusicQuality::HiRes => Tone::Gold,
        MusicQuality::Sky => Tone::Blue,
        MusicQuality::JyMaster => Tone::Orange,
    }
}

fn download_cover(
    handle: Option<&iced::widget::image::Handle>,
    kind: ImageKind,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    cover_image::custom(
        handle,
        kind,
        tokens.size(56.0),
        tokens.cover_radius(CoverRadiusRole::Thumbnail),
        tokens,
    )
}

fn quality_badge_el(
    label: &'static str,
    tone: rustle_ui::theme::decoration::Tone,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    container(
        text(label)
            .size(tokens.text(TextRole::Micro))
            .style(move |theme| text::Style {
                color: Some(rustle_ui::theme::decoration::ink(tone, theme)),
            }),
    )
    .padding(
        Padding::new(tokens.space(1.0))
            .left(tokens.space(6.0))
            .right(tokens.space(6.0)),
    )
    .style(move |_theme| container::Style {
        background: Some(Background::Color(
            rustle_ui::theme::decoration::ink(tone, _theme).scale_alpha(0.15),
        )),
        border: iced::Border {
            radius: tokens.radius(RadiusRole::Small).into(),
            width: tokens.size(1.0),
            color: rustle_ui::theme::decoration::ink(tone, _theme).scale_alpha(0.25),
        },
        ..Default::default()
    })
    .into()
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f32 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f32 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

// ── Button helpers (SVG icons, no emoji) ─────────────────────────────────

fn svg_btn(
    icon_svg: &'static str,
    msg: Message,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    button(
        svg(svg::Handle::from_memory(icon_svg.as_bytes()))
            .width(tokens.icon(crate::ui::responsive::IconRole::Small))
            .height(tokens.icon(crate::ui::responsive::IconRole::Small))
            .style(|theme, _status| svg::Style {
                color: Some(theme::text_secondary(theme)),
            }),
    )
    .on_press(msg)
    .width(tokens.target(TargetRole::Control))
    .height(tokens.target(TargetRole::Control))
    .padding([tokens.space(6.0), tokens.space(10.0)])
    .style(move |theme, status| {
        let bg = if status == button::Status::Hovered {
            Background::Color(theme::surface(theme))
        } else {
            Background::Color(Color::TRANSPARENT)
        };
        button::Style {
            background: Some(bg),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Medium).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}

fn icon_danger_btn(
    song_id: i64,
    tokens: crate::ui::responsive::UiTokens,
) -> Element<'static, Message> {
    button(
        svg(svg::Handle::from_memory(crate::ui::icons::CLOSE.as_bytes()))
            .width(tokens.icon(crate::ui::responsive::IconRole::Small))
            .height(tokens.icon(crate::ui::responsive::IconRole::Small))
            .style(|theme, status| svg::Style {
                color: Some(if status == svg::Status::Hovered {
                    theme::danger(theme)
                } else {
                    theme::text_muted(theme)
                }),
            }),
    )
    .on_press(Message::DownloadCancel(song_id))
    .width(tokens.target(TargetRole::Control))
    .height(tokens.target(TargetRole::Control))
    .padding(0)
    .style(move |_theme, status| {
        let bg = if status == button::Status::Hovered {
            Background::Color(theme::danger(_theme).scale_alpha(0.1))
        } else {
            Background::Color(Color::TRANSPARENT)
        };
        button::Style {
            background: Some(bg),
            border: iced::Border {
                radius: tokens.radius(RadiusRole::Pill).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}
