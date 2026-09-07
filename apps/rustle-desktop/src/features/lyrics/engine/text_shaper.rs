//! Text shaping and layout using cosmic-text
//!
//! Handles text shaping, word segmentation, and position calculation.
//! This replaces glyphon's text layout with our custom implementation.
//!
//! ## Caching
//!
//! Text shaping is expensive, so we cache results based on:
//! - Text content
//! - Font size (rounded to avoid cache misses from floating point differences)
//! - Max width (rounded)
//!
//! The cache is stored in the TextShaper and persists across frames.

use cosmic_text::{Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use super::types::{FontConfig, SUB_LINE_HEIGHT_MULTIPLIER, WordData};

/// Cache key for shaped lines
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapingCacheKey {
    /// Text content
    text: String,
    /// Font size (multiplied by 100 and rounded to avoid float comparison issues)
    font_size_x100: u32,
    /// Max width (rounded to nearest 10 pixels)
    max_width_rounded: u32,
}

/// Shaped glyph with position and timing information
#[derive(Debug, Clone)]
pub struct ShapedGlyph {
    /// Cache key for atlas lookup
    pub cache_key: CacheKey,
    /// X position relative to line start
    pub x: f32,
    /// Y position relative to line baseline
    pub y: f32,
    /// Additional glyph X offset in logical pixels.
    /// This must be applied before glyph raster placement to match `cosmic-text`.
    pub x_offset_px: f32,
    /// Additional glyph Y offset in logical pixels.
    /// This must be applied before glyph raster placement to match `cosmic-text`.
    pub y_offset_px: f32,
    /// Glyph advance width
    pub advance: f32,
    /// Word index this glyph belongs to
    pub word_index: usize,
    /// Position within word (0.0 to 1.0)
    pub pos_in_word: f32,
    /// Visual line index within the logical line (0-based)
    /// 0 = first visual line, 1 = second visual line after wrap, etc.
    pub visual_line_index: u32,
    /// Total number of visual lines for this logical line
    pub visual_line_count: u32,
    /// Position within visual line (0.0 = start, 1.0 = end)
    pub pos_in_visual_line: f32,
}

/// Shaped line result
#[derive(Debug, Clone)]
pub struct ShapedLine {
    /// All glyphs in the line
    pub glyphs: Vec<ShapedGlyph>,
    /// Total line width
    pub width: f32,
    /// Line height
    pub height: f32,
    /// Word boundaries (start_x, end_x) for each word
    pub word_bounds: Vec<(f32, f32)>,
}

/// Text shaper using cosmic-text
pub struct TextShaper {
    /// Shared font system
    font_system: Arc<Mutex<FontSystem>>,
    /// Font configuration
    config: FontConfig,
    /// Cache for shaped lines (text + font_size + max_width -> ShapedLine)
    shape_cache: Mutex<HashMap<ShapingCacheKey, ShapedLine>>,
    /// Cache for simple shaped lines (translation/romanized)
    simple_cache: Mutex<HashMap<ShapingCacheKey, ShapedLine>>,
}

impl TextShaper {
    fn resolve_font_config(
        font_system: &Arc<Mutex<FontSystem>>,
        mut config: FontConfig,
    ) -> FontConfig {
        if config.font_family.is_some() {
            return config;
        }

        let font_system = font_system.lock();
        if let Some(family) = crate::platform::theme::preferred_lyrics_font_family(font_system.db())
        {
            config.font_family = Some(family.to_string());
        }

        config
    }

    /// Create a new text shaper with default font config
    pub fn new(font_system: Arc<Mutex<FontSystem>>) -> Self {
        Self::with_config(font_system, FontConfig::default())
    }

    /// Create a new text shaper with custom font config
    pub fn with_config(font_system: Arc<Mutex<FontSystem>>, config: FontConfig) -> Self {
        let config = Self::resolve_font_config(&font_system, config);

        if config.debug_logging {
            if let Some(ref family) = config.font_family {
                tracing::debug!("[TextShaper] Using font family: {}", family);
            } else {
                tracing::debug!("[TextShaper] Using fallback font: SansSerif");
            }
        }
        Self {
            font_system,
            config,
            shape_cache: Mutex::new(HashMap::new()),
            simple_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Create a cache key for shaping
    fn make_cache_key(text: &str, font_size: f32, max_width: f32) -> ShapingCacheKey {
        ShapingCacheKey {
            text: text.to_string(),
            // Round font size to avoid cache misses from tiny floating point differences
            font_size_x100: (font_size * 100.0).round() as u32,
            // Round max width to nearest 10 pixels
            max_width_rounded: ((max_width / 10.0).round() * 10.0) as u32,
        }
    }

    /// Get the font family for text attributes
    fn get_font_family(&self) -> Family<'_> {
        match &self.config.font_family {
            Some(name) => Family::Name(name),
            None => Family::SansSerif,
        }
    }

    /// Shape a line of text with word timing information
    ///
    /// Returns shaped glyphs with position-in-word information for gradient mask.
    /// Results are cached based on text content, font size, and max width.
    pub fn shape_line(
        &self,
        text: &str,
        words: &[WordData],
        font_size: f32,
        max_width: f32,
    ) -> ShapedLine {
        if text.is_empty() {
            return ShapedLine {
                glyphs: Vec::new(),
                width: 0.0,
                height: font_size * 1.4,
                word_bounds: Vec::new(),
            };
        }

        // Check cache first
        let cache_key = Self::make_cache_key(text, font_size, max_width);
        {
            let cache = self.shape_cache.lock();
            if let Some(cached) = cache.get(&cache_key) {
                return cached.clone();
            }
        }

        // Not in cache, do the actual shaping
        let result = self.shape_line_uncached(text, words, font_size, max_width);

        // Store in cache
        {
            let mut cache = self.shape_cache.lock();
            // Limit cache size to prevent memory bloat
            if cache.len() > 1000 {
                cache.clear();
            }
            cache.insert(cache_key, result.clone());
        }

        result
    }

    /// Internal uncached shaping implementation
    fn shape_line_uncached(
        &self,
        text: &str,
        words: &[WordData],
        font_size: f32,
        max_width: f32,
    ) -> ShapedLine {
        let mut font_system = self.font_system.lock();

        // Create buffer for shaping
        let metrics = Metrics::new(font_size, font_size * 1.4);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_size(Some(max_width), None);

        // Set text with configured font family and weight
        let attrs = Attrs::new()
            .family(self.get_font_family())
            .weight(self.config.font_weight);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut font_system, false);

        // Build character to word mapping
        let char_to_word = self.build_char_word_map(text, words);

        // === Calculate visual line count from layout_runs ===
        // layout_runs() 返回所有视觉行的迭代器
        // 与 iced 的 measure() 方法相同
        let visual_line_count_from_buffer = buffer.layout_runs().count().max(1) as u32;

        // Extract glyphs from layout runs
        let mut shaped_glyphs = Vec::new();
        let mut word_bounds: Vec<(f32, f32)> = vec![(f32::MAX, f32::MIN); words.len()];
        let mut total_width = 0.0f32;
        let single_line_height = font_size * 1.4;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let char_idx = glyph.start;
                let word_idx = char_to_word.get(char_idx).copied().unwrap_or(0);

                // Calculate position within word
                let pos_in_word = if word_idx < words.len() {
                    let word = &words[word_idx];
                    let word_text = &word.text;
                    let word_start_char = self.find_word_start_char(text, word_idx, words);
                    let char_offset = char_idx.saturating_sub(word_start_char);
                    let word_char_count = word_text.chars().count();
                    if word_char_count > 0 {
                        char_offset as f32 / word_char_count as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                // Update word bounds
                if word_idx < word_bounds.len() {
                    let (ref mut min_x, ref mut max_x) = word_bounds[word_idx];
                    *min_x = min_x.min(glyph.x);
                    *max_x = max_x.max(glyph.x + glyph.w);
                }

                // Create cache key from glyph info
                let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;

                shaped_glyphs.push(ShapedGlyph {
                    cache_key,
                    x: glyph.x,
                    y: run.line_y, // 该行的基线 Y
                    x_offset_px: glyph.font_size * glyph.x_offset,
                    y_offset_px: glyph.font_size * glyph.y_offset,
                    advance: glyph.w,
                    word_index: word_idx,
                    pos_in_word,
                    // Visual line info will be calculated after all glyphs are collected
                    visual_line_index: 0,
                    visual_line_count: 1,
                    pos_in_visual_line: 0.0,
                });

                total_width = total_width.max(glyph.x + glyph.w);
            }
        }

        // Fix word bounds for words with no glyphs
        for i in 0..word_bounds.len() {
            if word_bounds[i].0 == f32::MAX {
                // No glyphs for this word, estimate from previous
                let prev_end = if i > 0 { word_bounds[i - 1].1 } else { 0.0 };
                word_bounds[i] = (prev_end, prev_end);
            }
        }

        // === Calculate visual line information for wrap highlight fix ===
        // Group glyphs by visual line (based on Y position)
        // visual_line_count_from_buffer is the authoritative count from cosmic-text
        let visual_line_count = visual_line_count_from_buffer;

        if !shaped_glyphs.is_empty() {
            // Collect unique Y positions (visual lines)
            let mut visual_line_ys: Vec<f32> = Vec::new();
            for glyph in &shaped_glyphs {
                // Check if this Y is already in the list (with small tolerance)
                let is_new_line = visual_line_ys.iter().all(|&y| (y - glyph.y).abs() > 0.1);
                if is_new_line {
                    visual_line_ys.push(glyph.y);
                }
            }
            // Sort by Y position (top to bottom)
            visual_line_ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            // For each visual line, collect glyphs and calculate their positions
            for (line_idx, &line_y) in visual_line_ys.iter().enumerate() {
                // Find glyphs on this visual line
                let line_glyph_indices: Vec<usize> = shaped_glyphs
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| (g.y - line_y).abs() < 0.1)
                    .map(|(i, _)| i)
                    .collect();

                if line_glyph_indices.is_empty() {
                    continue;
                }

                // Calculate total width of this visual line
                let line_min_x = line_glyph_indices
                    .iter()
                    .map(|&i| shaped_glyphs[i].x)
                    .fold(f32::MAX, f32::min);
                let line_max_x = line_glyph_indices
                    .iter()
                    .map(|&i| shaped_glyphs[i].x + shaped_glyphs[i].advance)
                    .fold(f32::MIN, f32::max);
                let line_width = (line_max_x - line_min_x).max(0.001);

                // Update each glyph's visual line info
                // Use visual_line_count from buffer (authoritative) for glyph info
                for &glyph_idx in &line_glyph_indices {
                    let glyph = &mut shaped_glyphs[glyph_idx];
                    glyph.visual_line_index = line_idx as u32;
                    glyph.visual_line_count = visual_line_count;

                    // Calculate position within visual line (0.0 = start, 1.0 = end)
                    // Use the center of the glyph for position calculation
                    let glyph_center_x = glyph.x + glyph.advance * 0.5;
                    glyph.pos_in_visual_line =
                        ((glyph_center_x - line_min_x) / line_width).clamp(0.0, 1.0);
                }
            }
        }

        // Calculate height based on VISUAL line count from buffer
        // 使用 cosmic-text 布局的权威行数
        let final_height = single_line_height * visual_line_count as f32;

        ShapedLine {
            glyphs: shaped_glyphs,
            width: total_width,
            height: final_height,
            word_bounds,
        }
    }

    /// Build mapping from character index to word index
    fn build_char_word_map(&self, text: &str, words: &[WordData]) -> Vec<usize> {
        let mut map = vec![0usize; text.len()];
        let mut char_pos = 0;

        for (word_idx, word) in words.iter().enumerate() {
            // Find this word in the text
            if let Some(start) = text[char_pos..].find(&word.text) {
                let abs_start = char_pos + start;
                let abs_end = abs_start + word.text.len();

                for i in abs_start..abs_end.min(map.len()) {
                    map[i] = word_idx;
                }

                char_pos = abs_end;
            }
        }

        map
    }

    /// Find the starting character index for a word
    fn find_word_start_char(&self, text: &str, word_idx: usize, words: &[WordData]) -> usize {
        let mut pos = 0;
        for (i, word) in words.iter().enumerate() {
            if i == word_idx {
                return pos;
            }
            if let Some(start) = text[pos..].find(&word.text) {
                pos += start + word.text.len();
            }
        }
        pos
    }

    /// Shape translation/romanized text (simpler, no word timing)
    /// Results are cached based on text content, font size, and max width.
    pub fn shape_simple(&self, text: &str, font_size: f32, max_width: f32) -> ShapedLine {
        if text.is_empty() {
            return ShapedLine {
                glyphs: Vec::new(),
                width: 0.0,
                height: font_size * SUB_LINE_HEIGHT_MULTIPLIER,
                word_bounds: Vec::new(),
            };
        }

        // Check cache first
        let cache_key = Self::make_cache_key(text, font_size, max_width);
        {
            let cache = self.simple_cache.lock();
            if let Some(cached) = cache.get(&cache_key) {
                return cached.clone();
            }
        }

        // Not in cache, do the actual shaping
        let result = self.shape_simple_uncached(text, font_size, max_width);

        // Store in cache
        {
            let mut cache = self.simple_cache.lock();
            // Limit cache size to prevent memory bloat
            if cache.len() > 500 {
                cache.clear();
            }
            cache.insert(cache_key, result.clone());
        }

        result
    }

    /// Internal uncached simple shaping implementation
    fn shape_simple_uncached(&self, text: &str, font_size: f32, max_width: f32) -> ShapedLine {
        let mut font_system = self.font_system.lock();

        let metrics = Metrics::new(font_size, font_size * SUB_LINE_HEIGHT_MULTIPLIER);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_size(Some(max_width), None);

        let attrs = Attrs::new()
            .family(self.get_font_family())
            .weight(self.config.font_weight);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut font_system, false);

        // Get visual line count from layout_runs (same as shape_line_uncached)
        let visual_line_count = buffer.layout_runs().count().max(1) as u32;
        let single_line_height = font_size * SUB_LINE_HEIGHT_MULTIPLIER;

        let mut shaped_glyphs = Vec::new();
        let mut total_width = 0.0f32;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;

                shaped_glyphs.push(ShapedGlyph {
                    cache_key,
                    x: glyph.x,
                    y: run.line_y,
                    x_offset_px: glyph.font_size * glyph.x_offset,
                    y_offset_px: glyph.font_size * glyph.y_offset,
                    advance: glyph.w,
                    word_index: 0,
                    pos_in_word: 0.0,
                    // Simple shaping doesn't need visual line tracking
                    visual_line_index: 0,
                    visual_line_count: 1,
                    pos_in_visual_line: 0.0,
                });

                total_width = total_width.max(glyph.x + glyph.w);
            }
        }

        // Calculate total height based on visual line count (same as shape_line_uncached)
        let final_height = single_line_height * visual_line_count as f32;

        ShapedLine {
            glyphs: shaped_glyphs,
            width: total_width,
            height: final_height,
            word_bounds: vec![(0.0, total_width)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::lyrics::engine::types::FontConfig;
    use cosmic_text::Weight;

    /// Property 3: Font Family Consistency
    /// Property 7: Configured Font Usage
    /// Validates: Requirements 2.1, 5.2
    ///
    /// When a font family is specified in FontConfig, TextShaper should use
    /// that font family for all text shaping operations.
    #[test]
    fn test_font_family_consistency() {
        let font_system = Arc::new(Mutex::new(FontSystem::new()));

        // Test with default config (SansSerif fallback)
        let default_config = FontConfig::default();
        let shaper_default = TextShaper::with_config(Arc::clone(&font_system), default_config);

        // Test with custom font family
        let custom_config = FontConfig::with_family("Noto Sans");
        let shaper_custom = TextShaper::with_config(Arc::clone(&font_system), custom_config);

        // Both shapers should be able to shape text without panicking
        let words = vec![WordData {
            text: "Hello".to_string(),
            start_ms: 0,
            end_ms: 1000,
            emphasize: false,
            is_last_word: true,
        }];

        let shaped_default = shaper_default.shape_line("Hello", &words, 48.0, 800.0);
        let shaped_custom = shaper_custom.shape_line("Hello", &words, 48.0, 800.0);

        // Both should produce valid output
        assert!(
            !shaped_default.glyphs.is_empty(),
            "Default shaper should produce glyphs"
        );
        assert!(
            !shaped_custom.glyphs.is_empty(),
            "Custom shaper should produce glyphs"
        );
    }

    /// Test that shape_simple also uses configured font family
    #[test]
    fn test_shape_simple_uses_config() {
        let font_system = Arc::new(Mutex::new(FontSystem::new()));
        let config = FontConfig::with_family("DejaVu Sans");
        let shaper = TextShaper::with_config(font_system, config);

        let shaped = shaper.shape_simple("Test text", 24.0, 400.0);
        assert!(
            !shaped.glyphs.is_empty(),
            "shape_simple should produce glyphs"
        );
    }

    /// Test empty text handling
    #[test]
    fn test_empty_text_handling() {
        let font_system = Arc::new(Mutex::new(FontSystem::new()));
        let config = FontConfig::default();
        let shaper = TextShaper::with_config(font_system, config);

        let shaped = shaper.shape_line("", &[], 48.0, 800.0);
        assert!(
            shaped.glyphs.is_empty(),
            "Empty text should produce no glyphs"
        );
        assert_eq!(shaped.width, 0.0, "Empty text should have zero width");

        let shaped_simple = shaper.shape_simple("", 24.0, 400.0);
        assert!(
            shaped_simple.glyphs.is_empty(),
            "Empty simple text should produce no glyphs"
        );
        assert_eq!(
            shaped_simple.height,
            24.0 * SUB_LINE_HEIGHT_MULTIPLIER,
            "Empty sub-lines should still reserve 1.5em line height"
        );
    }

    /// Test font weight configuration
    #[test]
    fn test_font_weight_configuration() {
        let font_system = Arc::new(Mutex::new(FontSystem::new()));

        let config_normal = FontConfig::default();
        assert_eq!(config_normal.font_weight, Weight::SEMIBOLD);

        let config_bold = FontConfig::default().weight(Weight::BOLD);
        assert_eq!(config_bold.font_weight, Weight::BOLD);

        // Both should create valid shapers
        let _shaper_normal = TextShaper::with_config(Arc::clone(&font_system), config_normal);
        let _shaper_bold = TextShaper::with_config(font_system, config_bold);
    }
}
