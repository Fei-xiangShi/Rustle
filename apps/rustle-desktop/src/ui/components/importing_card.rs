//! Importing playlist card component
//!
//! Shows a playlist card during import with circular progress indicator.
//! This is a business-specific component that uses the generic ProgressRing widget.

use std::path::PathBuf;

use crate::app::Message;
use crate::ui::responsive::UiTokens;
use crate::ui::theme;
use crate::ui::widgets::{ProgressRing, view_progress_ring_styled};
use iced::Element;
use iced::widget::{Space, container};
use std::time::Instant;

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
    pub finalized: bool,
    pub display_progress: f32,
    pub rotation: f32,
    pub completion_started: Option<Instant>,
    pub animation_time: f32,
    last_frame: Instant,
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
            finalized: false,
            display_progress: 0.0,
            rotation: 0.0,
            completion_started: None,
            animation_time: 0.0,
            last_frame: Instant::now(),
            cancelling: false,
            playlist_id: None,
            root_path,
            status_text: None,
            skipped: 0,
            errors: 0,
            recent_skips: Vec::new(),
        }
    }

    /// Keep the ID for cleanup even when cancellation beat playlist creation.
    pub fn attach_prepared(&mut self, playlist_id: i64) -> bool {
        self.playlist_id = Some(playlist_id);
        !self.cancelling
    }

    pub fn update_progress(&mut self, current: u64, total: u64) {
        self.current = current;
        self.total = total;
        self.progress = if total > 0 {
            (current as f32 / total as f32 * 0.95).clamp(self.progress, 0.95)
        } else {
            0.0
        };
    }

    /// Start the check animation only after the persisted playlist is loaded.
    pub fn ready(&mut self) {
        if self.completed && self.finalized && self.completion_started.is_none() {
            self.completion_started = Some(Instant::now());
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.1);
        self.last_frame = now;
        self.rotation = (self.rotation + dt * 4.0) % std::f32::consts::TAU;
        self.display_progress +=
            (self.progress - self.display_progress) * (1.0 - (-12.0 * dt).exp());
        if let Some(start) = self.completion_started {
            self.animation_time = now.saturating_duration_since(start).as_secs_f32();
            if self.animation_time >= 0.25 {
                self.display_progress = 1.0;
            }
        }
        self.completion_started.is_some() && self.animation_time >= 1.4
    }

    pub fn opacity(&self) -> f32 {
        1.0 - ((self.animation_time - 1.0) / 0.4).clamp(0.0, 1.0)
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

/// Shared cover-sized animation for the sidebar and compact rail.
pub fn indicator(
    playlist: &ImportingPlaylist,
    size: f32,
    tokens: UiTokens,
) -> Element<'static, Message> {
    let opacity = playlist.opacity();
    let mut ring = ProgressRing::new(
        playlist.display_progress,
        tokens.size(2.5),
        tokens.size(2.0),
    );
    ring.opacity = opacity;
    ring.check_progress = ((playlist.animation_time - 0.25) / 0.3).clamp(0.0, 1.0);
    if playlist.total == 0 && !playlist.completed {
        ring.progress = 0.22;
        ring.rotation = playlist.rotation;
    }
    let backdrop = container(Space::new())
        .width(size)
        .height(size)
        .style(move |theme| {
            let mut color = theme::surface_container(theme);
            color.a *= opacity;
            iced::widget::container::Style {
                background: Some(color.into()),
                border: iced::Border {
                    radius: tokens.size(6.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
    iced::widget::stack![backdrop, view_progress_ring_styled(ring, size)].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cancellation_before_creation_prevents_scan_and_retains_cleanup_id() {
        let mut playlist = ImportingPlaylist::new("Music".into(), PathBuf::new());
        playlist.begin_cancelling();
        assert!(!playlist.attach_prepared(42));
        assert_eq!(playlist.playlist_id, Some(42));
        assert!(playlist.cancelling);
    }

    #[test]
    fn import_animation_waits_for_persisted_playlist_before_check_and_fade() {
        let mut playlist = ImportingPlaylist::new("Music".into(), PathBuf::new());
        playlist.complete(1, 0, 0);
        playlist.ready();
        assert!(playlist.completion_started.is_none());
        assert!(!playlist.tick(Instant::now() + Duration::from_secs(2)));
        playlist.finalized = true;
        playlist.ready();
        let start = playlist.completion_started.unwrap();
        assert!(!playlist.tick(start + Duration::from_millis(800)));
        assert_eq!(playlist.opacity(), 1.0);
        assert_eq!(playlist.display_progress, 1.0);
        assert!(!playlist.tick(start + Duration::from_millis(1200)));
        assert!((playlist.opacity() - 0.5).abs() < 0.001);
        assert!(playlist.tick(start + Duration::from_millis(1400)));
        assert!(playlist.opacity() < 0.001);
    }

    #[test]
    fn import_animation_smooths_progress_and_reserves_finalization() {
        let mut playlist = ImportingPlaylist::new("Music".into(), PathBuf::new());
        let start = playlist.last_frame;
        playlist.update_progress(5, 10);
        assert_eq!(playlist.display_progress, 0.0);
        playlist.tick(start + Duration::from_millis(16));
        assert!(playlist.display_progress > 0.0);
        assert!(playlist.display_progress < playlist.progress);
        playlist.update_progress(3, 10);
        assert_eq!(playlist.progress, 0.475);
        playlist.update_progress(10, 10);
        assert_eq!(playlist.progress, 0.95);
        playlist.complete(10, 0, 0);
        assert_eq!(playlist.progress, 1.0);
    }
}
