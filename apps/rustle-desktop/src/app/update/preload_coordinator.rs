//! Preload coordinator - unified lifecycle management for all preload types
//!
//! Architecture:
//! - PreloadCoordinator: owns the preload window (which songs to prepare) — THE single source of truth
//! - Individual managers handle resource-specific preload implementation:
//!   - AudioPreloadManager (audio): next/prev sink/buffer preloading
//!   - LyricsPreloadManager (lyrics): text fetch + cache warmup
//!   - Background/cover state tracked inline via BackgroundSlot
//!
//! All preload consumers (audio, lyrics, background) read from the same PreloadWindow.

use std::collections::HashMap;

/// Which songs should be preloaded right now
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreloadWindow {
    pub current_song_id: Option<i64>,
    pub current_index: Option<usize>,
    pub next_index: Option<usize>,
    pub next_song_id: Option<i64>,
    pub prev_index: Option<usize>,
    pub prev_song_id: Option<i64>,
}

impl PreloadWindow {
    pub(super) fn contains_song(&self, song_id: i64) -> bool {
        self.current_song_id == Some(song_id)
            || self.next_song_id == Some(song_id)
            || self.prev_song_id == Some(song_id)
    }
}

/// What changed when the window was refreshed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChange {
    Unchanged,
    SongChanged,
    AdjacentChanged,
    Cleared,
}

/// Per-song background/cover preload readiness (with cached data for instant install)
#[derive(Debug, Clone)]
pub struct BackgroundSlot {
    pub cover_path: Option<String>,
    pub colors_ready: bool,
    pub texture_ready: bool,
    /// Last accepted colors for this song. Retained across cover changes for
    /// player chrome; shader installation additionally requires colors_ready.
    pub primary: Option<[f32; 4]>,
    pub secondary: Option<[f32; 4]>,
    pub tertiary: Option<[f32; 4]>,
    /// Cached cover image RGB data for instant textured background install
    pub image_data: Option<Vec<u8>>,
    pub image_width: u32,
    pub image_height: u32,
}

/// Background resources prepared together for one song and installed as a unit.
#[derive(Debug, Clone)]
pub struct PreparedBackground {
    pub cover_path: Option<String>,
    pub primary: [f32; 4],
    pub secondary: [f32; 4],
    pub tertiary: [f32; 4],
    pub image_data: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
}

/// Per-song lyrics preload readiness
#[derive(Debug, Clone, Default)]
pub struct LyricsSlot {
    pub text_ready: bool,
    pub engine_lines_ready: bool,
    pub shaped_lines_ready: bool,
    pub shape_generation: u64,
    pub content_width: f32,
    pub font_size: f32,
}

/// Central preload coordinator
///
/// Tracks the preload window and per-song resource readiness for up to 3 songs
/// (current / next / prev). All preload consumers read from this single window.
#[derive(Debug, Default)]
pub struct PreloadCoordinator {
    window: PreloadWindow,
    /// Background preload state keyed by song_id (at most 3 entries)
    background_slots: HashMap<i64, BackgroundSlot>,
    /// Lyrics preload state keyed by song_id (at most 3 entries)
    lyrics_slots: HashMap<i64, LyricsSlot>,
}

impl PreloadCoordinator {
    // ── Window management ──

    pub fn window(&self) -> &PreloadWindow {
        &self.window
    }

    /// The adjacent indices from the current window — single source for audio/lyrics/background.
    pub fn adjacent_indices(&self) -> (Option<usize>, Option<usize>) {
        (self.window.next_index, self.window.prev_index)
    }

    /// Refresh using pre-computed indices.
    /// Clears slots for songs that left the window.
    pub fn refresh_window_with_indices(
        &mut self,
        current_song_id: Option<i64>,
        current_index: Option<usize>,
        next_index: Option<usize>,
        next_song_id: Option<i64>,
        prev_index: Option<usize>,
        prev_song_id: Option<i64>,
    ) -> WindowChange {
        let new_window = PreloadWindow {
            current_song_id,
            current_index,
            next_index,
            next_song_id,
            prev_index,
            prev_song_id,
        };

        if new_window.current_song_id.is_none() {
            self.background_slots.clear();
            self.lyrics_slots.clear();
            self.window = new_window;
            WindowChange::Cleared
        } else if self.window.current_song_id != new_window.current_song_id {
            // Song changed: keep only slots for songs still in the new window
            self.retain_window_slots(&new_window);
            self.window = new_window;
            WindowChange::SongChanged
        } else if self.window != new_window {
            self.retain_window_slots(&new_window);
            self.window = new_window;
            WindowChange::AdjacentChanged
        } else {
            WindowChange::Unchanged
        }
    }

    pub fn clear_window(&mut self) {
        self.window = PreloadWindow::default();
        self.background_slots.clear();
        self.lyrics_slots.clear();
    }

    /// Remove slots for songs no longer in the given window (current + next + prev).
    fn retain_window_slots(&mut self, window: &PreloadWindow) {
        self.background_slots
            .retain(|id, _| window.contains_song(*id));
        self.lyrics_slots.retain(|id, _| window.contains_song(*id));
    }

    // ── Background slot ──

    /// Ensure a background slot exists for the given song.
    /// Resets ready flags if the cover_path changed.
    pub fn ensure_background_slot(&mut self, song_id: i64, cover_path: Option<String>) {
        let slot = self
            .background_slots
            .entry(song_id)
            .or_insert_with(|| BackgroundSlot {
                cover_path: cover_path.clone(),
                colors_ready: false,
                texture_ready: false,
                primary: None,
                secondary: None,
                tertiary: None,
                image_data: None,
                image_width: 0,
                image_height: 0,
            });

        // If cover path changed, reset ready flags
        if slot.cover_path != cover_path {
            slot.cover_path = cover_path;
            slot.colors_ready = false;
            slot.texture_ready = false;
        }
    }

    /// Store cached dominant colors for a song.
    pub fn store_background_colors(
        &mut self,
        song_id: i64,
        cover_path: String,
        primary: [f32; 4],
        secondary: [f32; 4],
        tertiary: [f32; 4],
    ) {
        let Some(slot) = self.background_slots.get_mut(&song_id) else {
            return;
        };

        if slot.cover_path.as_deref() != Some(cover_path.as_str()) {
            return;
        }

        slot.primary = Some(primary);
        slot.secondary = Some(secondary);
        slot.tertiary = Some(tertiary);
        slot.colors_ready = true;
    }

    /// Store cached cover image data for a song.
    pub fn store_background_texture(
        &mut self,
        song_id: i64,
        cover_path: String,
        image_data: Vec<u8>,
        width: u32,
        height: u32,
    ) {
        let Some(slot) = self.background_slots.get_mut(&song_id) else {
            return;
        };

        if slot.cover_path.as_deref() != Some(cover_path.as_str()) {
            return;
        }

        slot.image_data = Some(image_data);
        slot.image_width = width;
        slot.image_height = height;
        slot.texture_ready = true;
    }

    /// Player chrome follows the song lifetime, not background preparation.
    /// Keep the last accepted colors while a new derivative is pending or fails.
    /// Slot eviction / clear_window releases them; another song never uses them.
    pub fn progress_colors(&self, song_id: i64) -> Option<([f32; 4], [f32; 4], [f32; 4])> {
        let slot = self.background_slots.get(&song_id)?;
        Some((slot.primary?, slot.secondary?, slot.tertiary?))
    }

    /// Background is ready only if colors + texture are done AND slot exists with matching cover.
    pub fn is_background_ready(&self, song_id: i64, cover_path: Option<&str>) -> bool {
        self.background_slots.get(&song_id).is_some_and(|s| {
            s.colors_ready
                && s.texture_ready
                && s.primary.is_some()
                && s.secondary.is_some()
                && s.tertiary.is_some()
                && s.image_data.is_some()
                && s.image_width > 0
                && s.image_height > 0
                && s.cover_path.as_deref() == cover_path
        })
    }

    /// Clone cached background data for installation into shader.
    pub fn background_data(&self, song_id: i64) -> Option<PreparedBackground> {
        let slot = self.background_slots.get(&song_id)?;
        if !self.is_background_ready(song_id, slot.cover_path.as_deref()) {
            return None;
        }
        let cover_path = slot.cover_path.clone();
        let primary = slot.primary?;
        let secondary = slot.secondary?;
        let tertiary = slot.tertiary?;
        let image_data = slot.image_data.clone()?;
        let (w, h) = (slot.image_width, slot.image_height);
        Some(PreparedBackground {
            cover_path,
            primary,
            secondary,
            tertiary,
            image_data,
            image_width: w,
            image_height: h,
        })
    }

    // ── Lyrics slot ──

    pub fn ensure_lyrics_slot(&mut self, song_id: i64) {
        self.lyrics_slots.entry(song_id).or_default();
    }

    pub fn mark_lyrics_text_ready(&mut self, song_id: i64) {
        if let Some(slot) = self.lyrics_slots.get_mut(&song_id) {
            slot.text_ready = true;
        }
    }

    pub fn mark_lyrics_engine_lines_ready(&mut self, song_id: i64) {
        if let Some(slot) = self.lyrics_slots.get_mut(&song_id) {
            slot.engine_lines_ready = true;
        }
    }

    pub fn mark_lyrics_shaped_lines_ready(
        &mut self,
        song_id: i64,
        generation: u64,
        content_width: f32,
        font_size: f32,
    ) {
        if let Some(slot) = self.lyrics_slots.get_mut(&song_id) {
            slot.shaped_lines_ready = true;
            slot.shape_generation = generation;
            slot.content_width = content_width;
            slot.font_size = font_size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreloadCoordinator, WindowChange};

    #[test]
    fn replacing_adjacent_song_at_same_index_evicts_old_resources() {
        let mut coordinator = PreloadCoordinator::default();
        coordinator.refresh_window_with_indices(Some(1), Some(0), Some(1), Some(2), None, None);
        coordinator.ensure_background_slot(2, Some("old.png".into()));
        let color = [0.8, 0.2, 0.3, 1.0];
        coordinator.store_background_colors(2, "old.png".into(), color, color, color);
        assert_eq!(
            coordinator.refresh_window_with_indices(Some(1), Some(0), Some(1), Some(3), None, None),
            WindowChange::AdjacentChanged
        );
        assert_eq!(coordinator.window().next_song_id, Some(3));
        assert_eq!(coordinator.progress_colors(2), None);
        coordinator.store_background_colors(2, "old.png".into(), color, color, color);
        assert_eq!(coordinator.progress_colors(2), None);
    }

    #[test]
    fn progress_colors_retain_last_accepted_cover_while_background_is_pending() {
        let mut coordinator = PreloadCoordinator::default();
        let primary = [0.8, 0.2, 0.3, 1.0];
        let secondary = [0.3, 0.6, 0.9, 1.0];
        let tertiary = [0.5, 0.2, 0.8, 1.0];

        coordinator.ensure_background_slot(1, Some("first.png".to_string()));
        coordinator.store_background_colors(
            1,
            "other.png".to_string(),
            primary,
            secondary,
            tertiary,
        );
        assert_eq!(coordinator.progress_colors(1), None);

        coordinator.store_background_colors(
            1,
            "first.png".to_string(),
            primary,
            secondary,
            tertiary,
        );
        assert_eq!(
            coordinator.progress_colors(1),
            Some((primary, secondary, tertiary))
        );

        coordinator.ensure_background_slot(1, Some("second.png".to_string()));
        assert_eq!(
            coordinator.progress_colors(1),
            Some((primary, secondary, tertiary))
        );
        assert!(!coordinator.is_background_ready(1, Some("second.png")));
        assert!(coordinator.background_data(1).is_none());
    }

    #[test]
    fn progress_colors_survive_thumbnail_hero_thumbnail_round_trip() {
        let mut coordinator = PreloadCoordinator::default();
        let thumbnail = [0.8, 0.2, 0.3, 1.0];
        let hero = [0.3, 0.6, 0.9, 1.0];
        coordinator.ensure_background_slot(1, Some("thumbnail.png".into()));
        coordinator.store_background_colors(
            1,
            "thumbnail.png".into(),
            thumbnail,
            thumbnail,
            thumbnail,
        );
        coordinator.store_background_texture(1, "thumbnail.png".into(), vec![1, 2, 3], 1, 1);
        assert!(coordinator.background_data(1).is_some());

        coordinator.ensure_background_slot(1, Some("hero.png".into()));
        assert_eq!(
            coordinator.progress_colors(1),
            Some((thumbnail, thumbnail, thumbnail))
        );
        coordinator.store_background_colors(1, "hero.png".into(), hero, hero, hero);
        // New colors must not be installed alongside the old thumbnail texture.
        assert!(coordinator.background_data(1).is_none());
        coordinator.store_background_texture(1, "hero.png".into(), vec![4, 5, 6], 1, 1);
        assert!(coordinator.background_data(1).is_some());

        coordinator.ensure_background_slot(1, Some("thumbnail.png".into()));
        assert_eq!(coordinator.progress_colors(1), Some((hero, hero, hero)));
        assert!(coordinator.background_data(1).is_none());
        coordinator.store_background_colors(
            1,
            "thumbnail.png".into(),
            thumbnail,
            thumbnail,
            thumbnail,
        );
        coordinator.store_background_texture(1, "thumbnail.png".into(), vec![1, 2, 3], 1, 1);
        assert!(coordinator.background_data(1).is_some());

        // A late Hero result cannot replace the newly accepted Thumbnail.
        coordinator.store_background_colors(1, "hero.png".into(), hero, hero, hero);
        coordinator.store_background_texture(1, "hero.png".into(), vec![4, 5, 6], 1, 1);
        assert_eq!(
            coordinator.progress_colors(1),
            Some((thumbnail, thumbnail, thumbnail))
        );
        assert_eq!(
            coordinator.background_data(1).unwrap().image_data,
            vec![1, 2, 3]
        );
    }

    #[test]
    fn progress_colors_follow_song_window_and_reject_evicted_results() {
        let mut coordinator = PreloadCoordinator::default();
        let color = [0.8, 0.2, 0.3, 1.0];
        coordinator.refresh_window_with_indices(Some(1), Some(0), Some(1), Some(2), None, None);
        coordinator.ensure_background_slot(1, Some("cover.png".into()));
        coordinator.store_background_colors(1, "cover.png".into(), color, color, color);
        coordinator.ensure_background_slot(2, Some("other.png".into()));
        assert_eq!(coordinator.progress_colors(2), None);

        coordinator.refresh_window_with_indices(Some(2), Some(1), None, None, Some(0), Some(1));
        assert_eq!(coordinator.progress_colors(1), Some((color, color, color)));
        assert_eq!(coordinator.progress_colors(2), None);
        coordinator.refresh_window_with_indices(Some(3), Some(2), None, None, Some(1), Some(2));
        coordinator.store_background_colors(1, "cover.png".into(), color, color, color);
        assert_eq!(coordinator.progress_colors(1), None);

        coordinator.store_background_colors(2, "other.png".into(), color, color, color);
        coordinator.clear_window();
        coordinator.store_background_colors(2, "other.png".into(), color, color, color);
        assert_eq!(coordinator.progress_colors(2), None);
    }

    #[test]
    fn prepared_background_keeps_identity_colors_and_texture_together() {
        let mut coordinator = PreloadCoordinator::default();
        let cover_path = "cover.png".to_string();
        let primary = [0.8, 0.2, 0.3, 1.0];
        let secondary = [0.3, 0.6, 0.9, 1.0];
        let tertiary = [0.5, 0.2, 0.8, 1.0];
        let image_data = vec![1, 2, 3, 4, 5, 6];

        coordinator.ensure_background_slot(7, Some(cover_path.clone()));
        coordinator.store_background_colors(7, cover_path.clone(), primary, secondary, tertiary);
        coordinator.store_background_texture(7, cover_path.clone(), image_data.clone(), 2, 1);

        let prepared = coordinator
            .background_data(7)
            .expect("complete background should produce one installation payload");
        assert_eq!(prepared.cover_path.as_deref(), Some(cover_path.as_str()));
        assert_eq!(prepared.primary, primary);
        assert_eq!(prepared.secondary, secondary);
        assert_eq!(prepared.tertiary, tertiary);
        assert_eq!(prepared.image_data, image_data);
        assert_eq!((prepared.image_width, prepared.image_height), (2, 1));
    }
}
