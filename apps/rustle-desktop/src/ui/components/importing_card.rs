//! Importing playlist card component
//!
//! Shows a playlist card during import with circular progress indicator.
//! This is a business-specific component that uses the generic ProgressRing widget.

use std::path::PathBuf;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::Message;
use crate::ui::responsive::{TextRole, UiTokens};
use crate::ui::theme;
use crate::ui::widgets::{ProgressRing, view_progress_ring_styled};

/// State for an importing playlist
#[derive(Debug, Clone)]
pub struct ImportingPlaylist {
    /// Playlist name (folder name)
    pub name: String,
    /// Cover image path (first song's cover, if available)
    pub cover_path: Option<String>,
    /// Current progress (0.0 - 1.0)
    pub progress: f32,
    /// Current file count
    pub current: u64,
    /// Total file count
    pub total: u64,
    /// Is import complete
    pub completed: bool,
    /// Whether cancellation has been requested
    pub cancelling: bool,
    /// Database ID of created playlist (set after completion)
    pub playlist_id: Option<i64>,
    /// Root folder being imported into the local library playlist
    pub root_path: PathBuf,
    /// Optional in-progress status message
    pub status_text: Option<String>,
    /// Number of skipped files during this import
    pub skipped: u64,
    /// Number of database or unexpected errors during this import
    pub errors: u64,
    /// Recent skipped files with human-readable reasons
    pub recent_skips: Vec<String>,
}

impl ImportingPlaylist {
    pub fn new(name: String, root_path: PathBuf) -> Self {
        Self {
            name,
            cover_path: None,
            progress: 0.0,
            current: 0,
            total: 0,
            completed: false,
            cancelling: false,
            playlist_id: None,
            root_path,
            status_text: None,
            skipped: 0,
            errors: 0,
            recent_skips: Vec::new(),
        }
    }

    pub fn update_progress(&mut self, current: u64, total: u64) {
        self.current = current;
        self.total = total;
        self.progress = if total > 0 {
            current as f32 / total as f32
        } else {
            0.0
        };
    }

    pub fn set_cover(&mut self, path: String) {
        if self.cover_path.is_none() {
            self.cover_path = Some(path);
        }
    }

    pub fn complete(&mut self, imported: u64, skipped: u64, errors: u64) {
        self.completed = true;
        self.progress = 1.0;
        self.cancelling = false;
        self.skipped = skipped;
        self.errors = errors;
        self.status_text = Some(if errors > 0 {
            format!("完成：{} 首成功，{} 个错误", imported, errors)
        } else if skipped > 0 {
            format!("完成：{} 首成功，{} 个跳过", imported, skipped)
        } else {
            format!("完成：{} 首歌曲", imported)
        });
    }

    pub fn begin_cancelling(&mut self) {
        self.cancelling = true;
        self.status_text = Some("正在取消...".to_string());
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status_text = Some(status.into());
    }

    pub fn record_skip(&mut self, file_name: &str, reason: &str) {
        self.skipped += 1;
        let mut file_name = file_name.to_string();
        if file_name.chars().count() > 24 {
            file_name = format!("{}...", file_name.chars().take(24).collect::<String>());
        }
        self.recent_skips
            .insert(0, format!("跳过 {}：{}", file_name, reason));
        self.recent_skips.truncate(3);
    }

    pub fn record_error(&mut self) {
        self.errors += 1;
    }
}

/// Build an importing playlist card for the sidebar
pub fn view(playlist: &ImportingPlaylist, tokens: UiTokens) -> Element<'static, Message> {
    let name = playlist.name.clone();
    let progress = playlist.progress;
    let percentage = (progress * 100.0) as u32;

    // Progress indicator - show checkmark when completed, progress ring otherwise
    let progress_indicator: Element<'static, Message> = if playlist.completed {
        // Show checkmark icon when completed
        container(
            iced::widget::svg(iced::widget::svg::Handle::from_memory(
                crate::ui::icons::CHECK.as_bytes(),
            ))
            .width(tokens.size(22.0))
            .height(tokens.size(22.0))
            .style(|_theme, _status| iced::widget::svg::Style {
                color: Some(theme::ACCENT_PINK),
            }),
        )
        .width(tokens.size(22.0))
        .height(tokens.size(22.0))
        .center_x(tokens.size(22.0))
        .center_y(tokens.size(22.0))
        .into()
    } else {
        // Show progress ring with percentage during import
        let progress_ring = ProgressRing::new(progress, tokens.size(2.5), tokens.size(1.0))
            .background_color(theme::SURFACE_LIGHT)
            .progress_color(theme::ACCENT_PINK);

        container(
            column![
                view_progress_ring_styled(progress_ring, tokens.size(32.0)),
                text(format!("{}%", percentage))
                    .size(tokens.text(TextRole::Caption))
                    .style(|theme| text::Style {
                        color: Some(theme::text_muted(theme))
                    })
                    .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT))
            ]
            .align_x(Alignment::Center)
            .spacing(tokens.space(2.0)),
        )
        .width(tokens.size(38.0))
        .center_x(tokens.size(38.0))
        .center_y(tokens.size(38.0))
        .into()
    };

    // Playlist info
    let status_text = if playlist.cancelling {
        "正在取消...".to_string()
    } else if let Some(status) = &playlist.status_text {
        status.clone()
    } else if playlist.completed {
        "导入完成".to_string()
    } else if playlist.total > 0 {
        format!("{}/{}", playlist.current, playlist.total)
    } else {
        "扫描中...".to_string()
    };

    let skip_detail = playlist.recent_skips.first().cloned();
    let completed = playlist.completed;
    let mut info = column![
        text(name)
            .size(tokens.text(TextRole::BodyLarge))
            .style(move |theme| text::Style {
                color: Some(if completed {
                    theme::text_primary(theme)
                } else {
                    theme::text_secondary(theme)
                })
            })
            .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT)),
        text(status_text)
            .size(tokens.text(TextRole::Body))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            })
            .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT))
    ];
    if let Some(detail) = skip_detail {
        info = info.push(
            text(detail)
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT)),
        );
    }
    let info = info.spacing(tokens.space(3.0));

    let trailing: Element<'static, Message> = if !playlist.completed {
        if playlist.cancelling {
            text("取消中")
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::text_muted(theme)),
                })
                .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT))
                .into()
        } else {
            button(
                text("取消")
                    .size(tokens.text(TextRole::Body))
                    .style(|theme| text::Style {
                        color: Some(theme::text_muted(theme)),
                    })
                    .font(iced::Font::DEFAULT.weight(theme::BOLD_WEIGHT)),
            )
            .style(theme::text_button)
            .padding([tokens.space(5.0), tokens.space(10.0)])
            .on_press(Message::CancelScan)
            .into()
        }
    } else {
        Space::new().width(tokens.size(1.0)).into()
    };

    let content = row![
        progress_indicator,
        Space::new().width(tokens.space(14.0)),
        info,
        Space::new().width(Fill),
        trailing,
    ]
    .align_y(Alignment::Center)
    .padding(
        Padding::new(tokens.space(12.0))
            .left(tokens.space(16.0))
            .right(tokens.space(16.0)),
    );

    // Make it a button only if completed
    if playlist.completed {
        let on_press = playlist
            .playlist_id
            .map(Message::OpenPlaylist)
            .unwrap_or(Message::PlayHero);
        button(content)
            .width(Fill)
            .padding(0)
            .style(move |theme, status| theme::nav_item(theme, status, tokens.theme_metrics()))
            .on_press(on_press)
            .into()
    } else {
        // Non-clickable during import
        container(content)
            .width(Fill)
            .style(|_theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                ..Default::default()
            })
            .into()
    }
}
