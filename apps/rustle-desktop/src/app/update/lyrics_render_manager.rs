//! Lyrics render manager — per-song render cache for engine lines + shaped lines
//!
//! Stores pre-computed render entries keyed by song_id. Enables:
//! - Keeping shaped lines for the 3-song preload window (current / next / prev)
//! - Avoiding re-shaping when lyrics page opens for a song that's already been prepared
//! - Background render preparation for adjacent songs after text warmup completes

use std::collections::HashMap;
use std::sync::Arc;

use crate::features::lyrics::engine::{CachedShapedLine, LyricLineData};

/// One song's pre-computed render state
#[derive(Debug, Clone)]
pub struct LyricsRenderEntry {
    /// Phase 1: engine lines (word-split lyric data)
    pub engine_lines: Option<Arc<Vec<LyricLineData>>>,
    /// Phase 2: shaped lines (text layout + glyph positions)
    pub shaped_lines: Option<Arc<Vec<CachedShapedLine>>>,
    /// Content width used for shaping
    pub content_width: f32,
    /// Font size used for shaping
    pub font_size: f32,
    /// Font family used for shaping (None = auto-detected)
    pub font_family: Option<String>,
    /// Monotonic generation counter to reject stale results
    pub shape_generation: u64,
}

impl LyricsRenderEntry {
    pub fn shaped_viewport_matches(
        &self,
        content_width: f32,
        font_size: f32,
        font_family: Option<&str>,
    ) -> bool {
        (self.content_width - content_width).abs() < 0.5
            && (self.font_size - font_size).abs() < 0.5
            && self.font_family.as_deref() == font_family
    }
}

/// Manages pre-computed render entries for multiple songs.
///
/// Typically holds up to 3 entries (current / next / prev window).
/// The SDF glyph bitmaps live in the global cache and are NOT stored here.
#[derive(Debug, Default)]
pub struct LyricsRenderManager {
    entries: HashMap<i64, LyricsRenderEntry>,
}

impl LyricsRenderManager {
    pub fn get(&self, song_id: i64) -> Option<&LyricsRenderEntry> {
        self.entries.get(&song_id)
    }

    pub fn entry_mut(&mut self, song_id: i64) -> &mut LyricsRenderEntry {
        self.entries.entry(song_id).or_default()
    }

    pub fn store_engine_lines(&mut self, song_id: i64, lines: Arc<Vec<LyricLineData>>) {
        self.entry_mut(song_id).engine_lines = Some(lines);
    }

    pub fn store_shaped_lines(
        &mut self,
        song_id: i64,
        lines: Arc<Vec<CachedShapedLine>>,
        generation: u64,
        content_width: f32,
        font_size: f32,
        font_family: Option<String>,
    ) {
        let entry = self.entry_mut(song_id);
        entry.shaped_lines = Some(lines);
        entry.shape_generation = generation;
        entry.content_width = content_width;
        entry.font_size = font_size;
        entry.font_family = font_family;
    }

    /// Check if shaped lines are ready and match the given viewport
    pub fn is_render_ready(
        &self,
        song_id: i64,
        content_width: f32,
        font_size: f32,
        font_family: Option<&str>,
    ) -> bool {
        self.entries.get(&song_id).is_some_and(|e| {
            e.shaped_lines.is_some()
                && e.shaped_viewport_matches(content_width, font_size, font_family)
        })
    }

    /// Remove entries for songs not in the given set
    pub fn retain(&mut self, song_ids: &[i64]) {
        let ids: Vec<i64> = song_ids.to_vec();
        self.entries.retain(|id, _| ids.contains(id));
    }
}

impl Default for LyricsRenderEntry {
    fn default() -> Self {
        Self {
            engine_lines: None,
            shaped_lines: None,
            content_width: 0.0,
            font_size: 0.0,
            font_family: None,
            shape_generation: 0,
        }
    }
}
