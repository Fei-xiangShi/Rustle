//! GPU-accelerated Lyrics Render Engine
//!
//! ## Architecture
//!
//! This engine bypasses glyphon's limitations for dynamic lyrics:
//! - Uses cosmic-text directly for text shaping
//! - Custom SDF glyph atlas management (MSDF)
//! - Custom vertex structure with timing data
//! - GPU-based word-by-word highlighting
//!
//! ## Key Components
//!
//! - `LyricsEngine`: Main engine managing state, scrolling, and line tracking
//! - `LyricsGpuPipeline`: GPU pipeline for text rendering with SDF shader
//! - `SdfCache`: MSDF texture atlas for glyph storage
//! - `TextShaper`: cosmic-text based text shaping
//! - `ScrollPhysics`: Physics simulation for smooth scrolling
//! - `Spring`: Spring-based animation system

// Core modules
pub mod gpu_pipeline;
pub mod interlude_dots;
pub mod line_animation;
pub mod per_line_blur;
pub mod physics;
pub mod pipeline;
pub mod program;
pub mod sdf_cache;
pub mod sdf_generator;
pub mod spring;
pub mod text_shaper;
pub mod types;
pub mod vertex;

// Re-exports for convenience
pub use interlude_dots::InterludeDots;
pub use line_animation::{AnimationBuffers, LineAnimationManager};
pub use physics::{ScrollPhysics, ScrollState};
pub use sdf_cache::{SharedFontSystem, pre_generate_sdf_batch};
pub use text_shaper::TextShaper;
pub use types::{FontConfig, FontSizeConfig, LyricLineData, LyricsLineTraits, WordData};

use std::sync::Arc;
use std::time::Instant;

/// Configuration for the lyrics engine
///
/// All timing values are in seconds. Dimension fields are authored in 1080P
/// reference pixels and resolved with the caller-provided visual scale.
///
/// ## Features
///
/// This configuration supports visual effects:
/// - Per-line spring animations with configurable parameters
/// - Distance-based blur (increases with distance from active line)
/// - Staggered animation delays for "waterfall" effect
/// - Emphasis effects for long words (scale, glow, float)
/// - Interlude dots animation
/// - Translation and romanized text support
#[derive(Debug, Clone)]
pub struct LyricsEngineConfig {
    /// Application-owned visual scale. Rustle supplies its root-rem scale;
    /// the renderer does not derive a second scale from its widget bounds.
    visual_scale: f32,

    // === Font Size ===
    /// Font size configuration for lyrics rendering
    pub font_size_config: FontSizeConfig,

    // === Layout ===
    /// Base line height in logical pixels
    pub line_height: f32,
    /// Translation line height ratio (relative to main line)
    pub trans_height_ratio: f32,
    /// Romanized text height ratio (relative to main line)
    pub roman_height_ratio: f32,
    /// Alignment position (0.0 = top, 0.5 = center, 1.0 = bottom)
    /// Default: 0.35
    pub align_position: f32,
    /// Alignment anchor for current line
    pub align_anchor: AlignAnchor,

    // === Visual Effects ===
    /// Enable GPU blur effects (distance-based blur)
    pub enable_blur: bool,
    /// Enable scale effect for non-active lines
    pub enable_scale: bool,
    /// Scale factor for non-active lines (default: 0.97)
    pub inactive_scale: f32,
    /// Scale factor for background lyrics (default: 0.75)
    pub bg_line_scale: f32,
    /// Base font size ratio for background lyrics (default: 0.7em)
    pub bg_font_size_ratio: f32,
    /// Word fade width in em units (default: 0.5)
    /// - 0.5 for iPad-like effect
    /// - 1.0 for Android-like effect
    pub word_fade_width: f32,

    // === Physics ===
    /// Scroll timeout before auto-return (seconds)
    /// Default: 5.0
    pub scroll_timeout: f32,

    // === Spring Parameters ===
    // Position Y: mass=0.9, damping=15, stiffness=90
    /// Spring mass for Y position
    pub spring_mass: f32,
    /// Spring damping for Y position
    pub spring_damping: f32,
    /// Spring stiffness for Y position
    pub spring_stiffness: f32,

    // === Staggered Animation ===
    /// Base delay for staggered animation (seconds)
    /// Default: 0.05
    pub stagger_base_delay: f32,
    /// Delay reduction factor for lines after target
    /// Default: 1.05 (delay /= 1.05 for each line)
    pub stagger_reduction_factor: f32,

    // === Interlude ===
    /// Minimum interlude duration to show dots (ms)
    pub interlude_min_duration: u64,

    // === Rendering ===
    /// Hide passed lines (scroll them out of view)
    pub hide_passed_lines: bool,
    /// Overscan distance for virtualization (pixels)
    /// Default: 300
    pub overscan_px: f32,
}

/// Alignment anchor for current lyric line
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignAnchor {
    Top,
    #[default]
    Center,
    Bottom,
}

impl Default for LyricsEngineConfig {
    fn default() -> Self {
        Self {
            visual_scale: 1.0,

            // Font Size
            font_size_config: FontSizeConfig::default(),

            // Layout
            line_height: 48.0,
            trans_height_ratio: 0.5,
            roman_height_ratio: 0.5,
            align_position: 0.35,
            align_anchor: AlignAnchor::Center,

            // Visual Effects
            enable_blur: true,
            enable_scale: true,
            inactive_scale: 0.97,
            bg_line_scale: 0.75,
            bg_font_size_ratio: 0.7,
            word_fade_width: 0.5,

            // Physics
            scroll_timeout: 5.0,

            // Spring (defaults for position Y)
            spring_mass: 0.9,
            spring_damping: 15.0,
            spring_stiffness: 90.0,

            // Staggered Animation (defaults)
            stagger_base_delay: 0.05,
            stagger_reduction_factor: 1.05,

            // Interlude
            interlude_min_duration: 4000,

            // Rendering
            hide_passed_lines: false,
            overscan_px: 300.0,
        }
    }
}

impl LyricsEngineConfig {
    /// Resolve reference-space renderer metrics with a caller-owned scale.
    pub fn with_visual_scale(mut self, visual_scale: f32) -> Self {
        self.set_visual_scale(visual_scale);
        self
    }

    fn set_visual_scale(&mut self, visual_scale: f32) {
        self.visual_scale = if visual_scale.is_finite() && visual_scale > 0.0 {
            visual_scale
        } else {
            1.0
        };
    }

    #[inline]
    pub fn resolved_font_size(&self) -> f32 {
        self.font_size_config.calculate_font_size(self.visual_scale)
    }

    #[inline]
    pub fn visual_scale(&self) -> f32 {
        self.visual_scale
    }

    #[inline]
    pub fn resolved_line_height(&self) -> f32 {
        self.line_height * self.visual_scale
    }

    #[inline]
    pub fn resolved_overscan(&self) -> f32 {
        self.overscan_px * self.visual_scale
    }
}

/// 缓存的 shaped line 数据
/// 文本布局的唯一数据源
#[derive(Debug, Clone)]
pub struct CachedShapedLine {
    /// Main text shaped result
    pub main: text_shaper::ShapedLine,
    /// Main text logical font size
    pub main_font_size: f32,
    /// Translation text shaped result (if any)
    pub translation: Option<text_shaper::ShapedLine>,
    /// Translation logical font size
    pub translation_font_size: f32,
    /// Romanized text shaped result (if any)
    pub romanized: Option<text_shaper::ShapedLine>,
    /// Romanized logical font size
    pub romanized_font_size: f32,
    /// Total height of this line (main + translation + romanized)
    pub total_height: f32,
}

/// Main lyrics engine - manages scrolling, layout, and rendering
pub struct LyricsEngine {
    /// Configuration
    config: LyricsEngineConfig,
    /// Physics simulation for user scrolling (not used for auto-scroll anymore)
    physics: ScrollPhysics,
    /// Interlude dots animation
    interlude_dots: InterludeDots,
    /// Per-line animation manager
    line_animations: LineAnimationManager,
    /// Pre-allocated animation buffers for efficient rendering
    /// Avoids per-frame Vec allocations in LyricsEnginePrimitive::from_engine
    animation_buffers: AnimationBuffers,
    /// Current time in milliseconds
    current_time_ms: f64,
    /// Whether mouse is hovering
    is_hovering: bool,
    /// Whether playback is active
    is_playing: bool,
    /// Current scroll target index (for auto-scroll)
    scroll_to_index: usize,
    /// Buffered (active) line indices
    buffered_lines: std::collections::HashSet<usize>,
    /// Hot (currently playing) line indices
    hot_lines: std::collections::HashSet<usize>,
    /// Last update time
    last_update: Instant,
    /// Cached shaped lines (Single Source of Truth for text layout)
    /// Contains all glyph positions, heights, and word bounds
    cached_shaped_lines: Arc<Vec<CachedShapedLine>>,
    /// Cached line heights (derived from cached_shaped_lines for convenience)
    cached_line_heights: Arc<Vec<f32>>,
    /// Static traits derived from the current lyrics.
    line_traits: LyricsLineTraits,
    /// Pointer used to know when the current lyrics slice changed.
    line_traits_source_ptr: usize,
    /// Length paired with line_traits_source_ptr.
    line_traits_source_len: usize,
    /// Last known content width for invalidation
    last_content_width: f32,
    /// Last known font size for invalidation
    last_font_size: f32,
    /// Force layout recalculation on the next timing update.
    layout_dirty: bool,
    /// Viewport height for layout calculations
    viewport_height: f32,
    /// Viewport width for layout calculations
    viewport_width: f32,
    /// Last blur-disable state used by the current layout.
    last_disable_blur: bool,
    /// Last target align index for dynamic spring params
    last_target_align_index: usize,
    /// Last interlude state for dynamic spring params
    last_interlude_state: bool,
    /// Current interlude insert position (gap is after this line index)
    interlude_insert_after: Option<usize>,
    /// Extra vertical height reserved for the current interlude
    interlude_extra_height: f32,
    /// Whether the next line after the interlude is a duet line
    interlude_align_right: bool,
}

impl LyricsEngine {
    fn pos_y_spring_params(&self) -> crate::features::lyrics::engine::spring::SpringParams {
        crate::features::lyrics::engine::spring::SpringParams {
            mass: self.config.spring_mass as f64,
            damping: self.config.spring_damping as f64,
            stiffness: self.config.spring_stiffness as f64,
            soft: false,
        }
    }

    /// Animation consumes asynchronously prepared layout and never loads fonts.
    pub fn new(config: LyricsEngineConfig) -> Self {
        let physics = ScrollPhysics::new();
        debug_assert!(
            [AlignAnchor::Top, AlignAnchor::Center, AlignAnchor::Bottom]
                .contains(&config.align_anchor)
        );

        // Create line animation manager with config
        let mut line_animations = LineAnimationManager::new();
        line_animations.set_align_position(config.align_position);
        // Convert AlignAnchor from config to line_animation's AlignAnchor
        line_animations.set_align_anchor(match config.align_anchor {
            AlignAnchor::Top => line_animation::AlignAnchor::Top,
            AlignAnchor::Center => line_animation::AlignAnchor::Center,
            AlignAnchor::Bottom => line_animation::AlignAnchor::Bottom,
        });
        line_animations.set_hide_passed_lines(config.hide_passed_lines);

        Self {
            physics,
            interlude_dots: InterludeDots::new(),
            line_animations,
            animation_buffers: AnimationBuffers::new(),
            config,
            current_time_ms: 0.0,
            is_hovering: false,
            is_playing: true,
            scroll_to_index: 0,
            buffered_lines: std::collections::HashSet::new(),
            hot_lines: std::collections::HashSet::new(),
            last_update: Instant::now(),
            cached_shaped_lines: Arc::new(Vec::new()),
            cached_line_heights: Arc::new(Vec::new()),
            line_traits: LyricsLineTraits::default(),
            line_traits_source_ptr: 0,
            line_traits_source_len: 0,
            last_content_width: 0.0,
            last_font_size: 0.0,
            layout_dirty: true,
            // Non-zero renderer bootstrap bounds. Sensor measurements replace
            // them before viewport-owned layout is considered initialized;
            // they are not visual sizing policy.
            viewport_height: 800.0,
            viewport_width: 1200.0,
            last_disable_blur: false,
            last_target_align_index: 0,
            last_interlude_state: false,
            interlude_insert_after: None,
            interlude_extra_height: 0.0,
            interlude_align_right: false,
        }
    }

    /// Update the engine state
    ///
    /// This must be called every frame with delta time in seconds.
    pub fn update(&mut self, dt: f32) {
        // Update physics simulation (for user scrolling)
        self.physics.update(dt, self.is_hovering);

        // Check if we should return to auto-play using configured timeout
        if self.physics.state() == ScrollState::Idle
            && self.physics.time_since_interaction() > self.config.scroll_timeout
            && !self.is_hovering
        {
            self.physics.start_auto_play();
        }

        // Update interlude dots animation
        self.interlude_dots.update(dt);

        // Update line animations
        self.line_animations.update(dt);

        // Keep the dots aligned with animated line movement.
        self.update_interlude_transform();

        // Update animation buffers from line animations (in-place, no allocations)
        // This prepares the data for LyricsEnginePrimitive::from_engine
        self.animation_buffers
            .update_from_manager(&self.line_animations);

        // Update current time for animations
        self.current_time_ms += (dt * 1000.0) as f64;
        self.last_update = Instant::now();
    }

    /// Get the current configuration
    pub fn config(&self) -> &LyricsEngineConfig {
        &self.config
    }

    /// Update application-owned responsive scaling without deriving layout
    /// policy from renderer bounds. Existing shaped metrics remain active
    /// until their replacement is ready, avoiding a one-frame size flash.
    pub fn set_visual_scale(&mut self, visual_scale: f32) {
        let previous = self.config.visual_scale;
        self.config.set_visual_scale(visual_scale);
        if (self.config.visual_scale - previous).abs() > f32::EPSILON {
            self.layout_dirty = true;
        }
    }

    /// Font size currently matching the shaped cache, with a responsive
    /// fallback for the first frame before shaping completes.
    pub fn resolved_font_size(&self) -> f32 {
        if self.last_font_size > 0.0 {
            self.last_font_size
        } else {
            self.config.resolved_font_size()
        }
    }

    /// Handle mouse wheel event
    pub fn handle_wheel(&mut self, delta: f32) {
        if delta.is_finite() && delta != 0.0 {
            self.physics.scroll_by(delta);
        }
    }

    /// Lyric presses are available only while browsing after a manual scroll.
    pub fn is_manually_scrolling(&self) -> bool {
        self.physics.state() == ScrollState::Idle
    }

    /// Resume playback tracking without snapping the existing line springs.
    /// The next playback tick animates to the new time just like a slider seek.
    pub fn resume_auto_follow(&mut self) {
        self.physics.reset_manual_scroll();
        self.layout_dirty = true;
    }

    /// Get mutable line animation manager
    pub fn line_animations_mut(&mut self) -> &mut LineAnimationManager {
        &mut self.line_animations
    }

    /// Recalculate scroll layout on the next update while retaining shaped text.
    pub fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
    }

    /// Get animation buffers reference (for efficient rendering)
    ///
    /// Returns pre-allocated buffers containing positions, scales, blur levels,
    /// and opacities. These are updated in-place during `update()` to avoid
    /// per-frame allocations.
    pub fn animation_buffers(&self) -> &AnimationBuffers {
        &self.animation_buffers
    }

    /// Reset state that must not leak across lyric loads, even if line counts match.
    pub fn reset_for_new_lyrics(&mut self) {
        self.hot_lines.clear();
        self.buffered_lines.clear();
        self.scroll_to_index = 0;
        self.current_time_ms = 0.0;
        self.last_target_align_index = 0;
        self.last_interlude_state = false;
        self.physics.reset_manual_scroll();
        self.interlude_dots.set_interlude(None);
        self.interlude_insert_after = None;
        self.interlude_extra_height = 0.0;
        self.interlude_align_right = false;
        self.line_animations.reset();
        self.animation_buffers.clear();
        self.line_traits = LyricsLineTraits::default();
        self.line_traits_source_ptr = 0;
        self.line_traits_source_len = 0;
        self.layout_dirty = true;
        self.last_disable_blur = false;
    }

    /// 设置当前播放时间并更新行状态
    /// 与播放同步的主入口
    ///
    /// For accurate scroll positioning with text wrapping, call
    /// `set_viewport_info` first to update line height calculations.
    pub fn set_current_time(&mut self, time_ms: f64, lines: &[LyricLineData], is_seek: bool) {
        self.current_time_ms = time_ms;

        // 检查歌词是否变化（切歌或首次加载）
        // 如果歌词数量变化，需要重新初始化动画并立即排版
        let lyrics_changed = self.line_animations.len() != lines.len();
        self.sync_line_traits(lines);

        if is_seek || lyrics_changed {
            self.physics.reset_manual_scroll();
        }

        // Update hot lines (currently playing)
        let scroll_changed = self.update_hot_lines(time_ms, lines, is_seek);

        // Check for interlude
        self.update_interlude(time_ms, lines);

        // Update scroll target if needed (auto-scroll)
        // 触发条件：
        // 1. buffered_lines 发生变化（scroll_changed）
        // 2. 显式 seek 操作（is_seek）
        // 3. 歌词变化（lyrics_changed）- 切歌或首次加载时立即排版
        // 4. 用户正在手动滚动（物理状态不是 AutoPlay）
        // 5. 正在平滑回弹到中心（position 不为 0）
        let disable_blur = self.physics.state()
            != crate::features::lyrics::engine::physics::ScrollState::AutoPlay
            || self.physics.position().abs() > 0.5;
        let blur_state_changed = disable_blur != self.last_disable_blur;

        if scroll_changed
            || is_seek
            || lyrics_changed
            || self.layout_dirty
            || blur_state_changed
            || self.physics.state()
                != crate::features::lyrics::engine::physics::ScrollState::AutoPlay
            || self.physics.position().abs() > 0.5
        {
            self.calc_scroll_target(lines, is_seek || lyrics_changed);
        } else {
            self.last_disable_blur = disable_blur;
        }
    }

    /// Set viewport information without reshaping the prepared text.
    /// Call this before set_current_time when viewport size changes
    ///
    /// Parameters:
    /// - viewport_height: Viewport height (in logical pixels)
    /// - viewport_width: Viewport width (in logical pixels)
    pub fn set_viewport_info(&mut self, viewport_height: f32, viewport_width: f32) {
        if (self.viewport_height - viewport_height).abs() > 0.5
            || (self.viewport_width - viewport_width).abs() > 0.5
        {
            self.layout_dirty = true;
        }
        self.viewport_height = viewport_height;
        self.viewport_width = viewport_width;
        self.line_animations.set_viewport_height(viewport_height);
    }

    fn update_dynamic_spring_params(&mut self, lines: &[LyricLineData]) {
        if lines.is_empty() {
            return;
        }

        let current_index = self.scroll_to_index;
        if current_index == 0 || current_index >= lines.len() {
            return;
        }

        let current_line = &lines[current_index];
        let prev_line = &lines[current_index - 1];

        let prev_start = prev_line
            .words
            .first()
            .map(|w| w.start_ms)
            .unwrap_or(prev_line.start_ms);
        let interval = current_line.start_ms.saturating_sub(prev_start) as f32;

        let min_interval = 100.0;
        let max_interval = 800.0;
        let clamped_interval = interval.clamp(min_interval, max_interval);

        let max_stiffness = 220.0;
        let min_stiffness = 170.0;

        let mut ratio = 1.0 - (clamped_interval - min_interval) / (max_interval - min_interval);
        ratio = ratio.clamp(0.0, 1.0).powf(0.2);

        let target_stiffness = min_stiffness + ratio * (max_stiffness - min_stiffness);
        let damping_multiplier = 2.2;
        let target_damping = target_stiffness.sqrt() * damping_multiplier;

        let mut params = self.pos_y_spring_params();
        params.stiffness = target_stiffness as f64;
        params.damping = target_damping as f64;

        self.line_animations.set_pos_y_spring_params(params);
    }

    /// Calculate and set the scroll target position
    ///
    /// This now uses per-line animations instead of global scroll position.
    /// Each line has its own Spring for smooth, independent animation.
    ///
    /// Features:
    /// - Per-line spring animations with configurable parameters
    /// - Staggered delays for "waterfall" effect
    /// - Distance-based blur calculation
    fn calc_scroll_target(&mut self, lines: &[LyricLineData], is_seek: bool) {
        if lines.is_empty() {
            return;
        }

        let is_non_dynamic = self.line_traits.is_non_dynamic;

        // Ensure we have animations for all lines
        let is_bg_flags: Vec<bool> = lines.iter().map(|l| l.is_bg).collect();
        let was_reset = self
            .line_animations
            .ensure_capacity(lines.len(), &is_bg_flags);

        // If animations were reset (new song), force seek behavior
        let is_seek = is_seek || was_reset;

        let is_interlude_active = self.interlude_dots.enabled;

        let index_changed = self.last_target_align_index != self.scroll_to_index;

        if index_changed || self.last_interlude_state != is_interlude_active {
            self.last_interlude_state = is_interlude_active;

            if is_seek || is_interlude_active {
                self.line_animations
                    .set_pos_y_spring_params(self.pos_y_spring_params());
            } else {
                self.update_dynamic_spring_params(lines);
            }
        }
        self.last_target_align_index = self.scroll_to_index;

        // Use the same line_spacing formula as GPU pipeline
        let line_spacing = self.config.resolved_line_height() * 0.5;

        // Get line heights (use cached if available)
        let line_heights: Vec<f32> = lines
            .iter()
            .enumerate()
            .map(|(idx, _)| {
                if idx < self.cached_line_heights.len() {
                    self.cached_line_heights[idx]
                } else {
                    self.config.resolved_line_height() * 1.4
                }
            })
            .collect();

        // Extract manual scroll offset from physics state.
        // During auto-play, this is naturally 0.0.
        let mut manual_scroll_offset = self.physics.position();
        let disable_blur =
            self.physics.state() != ScrollState::AutoPlay || manual_scroll_offset.abs() > 0.5;

        let effective_stagger_delay = if index_changed {
            self.config.stagger_base_delay
        } else {
            0.0
        };

        // Blur intensity is width-independent; root-rem scaling is applied at
        // the renderer boundary together with the physical scale factor.
        let mut bounds = self.line_animations.calc_layout_full(
            &line_heights,
            line_spacing,
            self.scroll_to_index,
            &self.buffered_lines,
            self.is_playing,
            is_seek,
            self.config.enable_scale,
            self.config.inactive_scale,
            self.config.bg_line_scale,
            effective_stagger_delay,
            self.config.stagger_reduction_factor,
            is_non_dynamic,
            self.viewport_width,
            manual_scroll_offset,
            disable_blur,
            self.interlude_insert_after,
            self.interlude_extra_height,
            interlude_dots::dot_margin(self.last_font_size.max(self.config.resolved_line_height())),
        );

        self.physics.set_scroll_bounds(bounds.min, bounds.max);
        let clamped_offset = self.physics.clamp_position();

        if (clamped_offset - manual_scroll_offset).abs() > 0.1 {
            manual_scroll_offset = clamped_offset;
            bounds = self.line_animations.calc_layout_full(
                &line_heights,
                line_spacing,
                self.scroll_to_index,
                &self.buffered_lines,
                self.is_playing,
                is_seek,
                self.config.enable_scale,
                self.config.inactive_scale,
                self.config.bg_line_scale,
                effective_stagger_delay,
                self.config.stagger_reduction_factor,
                is_non_dynamic,
                self.viewport_width,
                manual_scroll_offset,
                disable_blur,
                self.interlude_insert_after,
                self.interlude_extra_height,
                interlude_dots::dot_margin(
                    self.last_font_size.max(self.config.resolved_line_height()),
                ),
            );
            self.physics.set_scroll_bounds(bounds.min, bounds.max);
        }

        self.layout_dirty = false;
        self.update_interlude_transform();
        self.last_disable_blur =
            self.physics.state() != ScrollState::AutoPlay || manual_scroll_offset.abs() > 0.5;
    }

    /// Get cached line heights as an O(1) snapshot.
    pub fn cached_line_heights_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.cached_line_heights)
    }

    /// Get cached shaped lines (Single Source of Truth for GPU rendering)
    pub fn cached_shaped_lines(&self) -> Arc<Vec<CachedShapedLine>> {
        Arc::clone(&self.cached_shaped_lines)
    }

    /// 设置异步任务预计算的 shaped lines
    /// 允许在后台线程进行文本 shaping
    pub fn set_cached_shaped_lines(&mut self, shaped_lines: Vec<CachedShapedLine>) {
        self.set_cached_shaped_lines_with_metrics(shaped_lines, 0.0, 0.0);
    }

    /// Sets async pre-computed shaped lines together with the metrics used to
    /// produce them so we can avoid immediately reshaping with the same inputs.
    pub fn set_cached_shaped_lines_with_metrics(
        &mut self,
        shaped_lines: Vec<CachedShapedLine>,
        content_width: f32,
        font_size: f32,
    ) {
        self.set_cached_shaped_lines_arc_with_metrics(
            Arc::new(shaped_lines),
            content_width,
            font_size,
        );
    }

    /// Sets async pre-computed shaped lines without copying the shaped glyph data.
    pub fn set_cached_shaped_lines_arc_with_metrics(
        &mut self,
        shaped_lines: Arc<Vec<CachedShapedLine>>,
        content_width: f32,
        font_size: f32,
    ) {
        if Arc::ptr_eq(&self.cached_shaped_lines, &shaped_lines)
            && self.last_content_width == content_width
            && self.last_font_size == font_size
        {
            return;
        }
        // Update cached line heights from shaped lines
        self.cached_line_heights = Arc::new(shaped_lines.iter().map(|s| s.total_height).collect());

        self.cached_shaped_lines = shaped_lines;
        self.layout_dirty = true;
        if content_width > 0.0 {
            self.last_content_width = content_width;
        }
        if font_size > 0.0 {
            self.last_font_size = font_size;
        }
    }

    /// Update hot lines based on current time
    /// Returns true if buffered_lines changed (need to recalculate layout/blur)
    ///
    /// 歌词选择状态定义：
    /// - 普通行：当前不处于时间范围内的歌词行
    /// - 热行(hotLines)：当前绝对处于播放时间内的歌词行
    /// - 缓冲行(bufferedLines)：一般处于播放时间后的歌词行，会因为当前播放状态的缘故推迟解除状态
    ///
    /// 关键行为：
    /// - 如果当前仍有缓冲行的情况下加入新热行，则不会解除当前缓冲行，且也不会修改当前滚动位置
    /// - 如果当前所有缓冲行都将被删除且没有新热行加入，则删除所有缓冲行
    /// - 如果当前所有缓冲行都将被删除且有新热行加入，则删除所有缓冲行并加入新热行
    fn update_hot_lines(&mut self, time_ms: f64, lines: &[LyricLineData], is_seek: bool) -> bool {
        let time = time_ms as u64;
        let old_buffered_lines = self.buffered_lines.clone();

        // Step 1: 先检索当前已经超出时间范围的热行，从 hot_lines 中移除
        let mut removed_hot_ids: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        let hot_lines_snapshot: Vec<usize> = self.hot_lines.iter().copied().collect();
        for last_hot_id in hot_lines_snapshot {
            if let Some(line) = lines.get(last_hot_id) {
                if line.is_bg {
                    continue;
                }
                // 检查是否有背景行
                let next_line = lines.get(last_hot_id + 1);
                if next_line.map(|l| l.is_bg).unwrap_or(false) {
                    // 有背景行的情况
                    let next_main_line = lines.get(last_hot_id + 2);
                    let start_time = line
                        .start_ms
                        .min(next_line.map(|l| l.start_ms).unwrap_or(line.start_ms));
                    let end_time = line
                        .end_ms
                        .max(next_main_line.map(|l| l.start_ms).unwrap_or(u64::MAX))
                        .min(
                            line.end_ms
                                .max(next_line.map(|l| l.end_ms).unwrap_or(line.end_ms)),
                        );

                    if start_time > time || end_time <= time {
                        self.hot_lines.remove(&last_hot_id);
                        removed_hot_ids.insert(last_hot_id);
                        self.hot_lines.remove(&(last_hot_id + 1));
                        removed_hot_ids.insert(last_hot_id + 1);
                    }
                } else if line.start_ms > time || line.end_ms <= time {
                    self.hot_lines.remove(&last_hot_id);
                    removed_hot_ids.insert(last_hot_id);
                }
            } else {
                self.hot_lines.remove(&last_hot_id);
                removed_hot_ids.insert(last_hot_id);
            }
        }

        // Step 2: 找到新的热行（当前时间范围内的行）
        let mut added_ids: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for (i, line) in lines.iter().enumerate() {
            if !line.is_bg
                && line.start_ms <= time
                && line.end_ms > time
                && !self.hot_lines.contains(&i)
            {
                self.hot_lines.insert(i);
                added_ids.insert(i);
                // 如果下一行是背景行，也加入
                if let Some(next) = lines.get(i + 1)
                    && next.is_bg
                {
                    self.hot_lines.insert(i + 1);
                    added_ids.insert(i + 1);
                }
            }
        }

        // Step 3: 计算需要从 buffered_lines 中移除的行
        let removed_ids: std::collections::HashSet<usize> = self
            .buffered_lines
            .iter()
            .filter(|&&idx| !self.hot_lines.contains(&idx))
            .copied()
            .collect();

        // Step 4: 根据逻辑更新 buffered_lines
        if is_seek {
            // Seek 操作：直接同步
            if !self.buffered_lines.is_empty() {
                self.scroll_to_index = *self.buffered_lines.iter().min().unwrap_or(&0);
            } else {
                self.scroll_to_index = lines.iter().position(|l| l.start_ms >= time).unwrap_or(0);
            }
            self.buffered_lines.clear();
            for &v in &self.hot_lines {
                self.buffered_lines.insert(v);
            }
        } else if !removed_ids.is_empty() || !added_ids.is_empty() {
            if removed_ids.is_empty() && !added_ids.is_empty() {
                // 只有新增，没有删除 -> 直接添加到 bufferedLines
                for &v in &added_ids {
                    self.buffered_lines.insert(v);
                }
                self.scroll_to_index = *self.buffered_lines.iter().min().unwrap_or(&0);
            } else if added_ids.is_empty() && !removed_ids.is_empty() {
                // 只有删除，没有新增
                // 关键逻辑：只有当 removedIds 等于 bufferedLines 时才删除
                // 这意味着如果还有其他行在 bufferedLines 中，不会删除任何行
                if removed_ids == self.buffered_lines {
                    for v in self.buffered_lines.clone() {
                        if !self.hot_lines.contains(&v) {
                            self.buffered_lines.remove(&v);
                        }
                    }
                }
                // 注意：这种情况下不更新 scroll_to_index
            } else {
                // 既有新增又有删除
                for &v in &added_ids {
                    self.buffered_lines.insert(v);
                }
                for &v in &removed_ids {
                    self.buffered_lines.remove(&v);
                }
                if !self.buffered_lines.is_empty() {
                    self.scroll_to_index = *self.buffered_lines.iter().min().unwrap_or(&0);
                }
            }
        }

        // 如果 buffered_lines 为空且没有热行，更新 scroll_to_index 到下一行
        if self.buffered_lines.is_empty() && self.hot_lines.is_empty() {
            self.scroll_to_index = self.find_next_line_index(time, lines);
        }

        // Return whether buffered_lines changed (need to recalculate layout/blur)
        // 关键修复：当 buffered_lines 发生任何变化时都需要重新计算布局
        // 这样当第二行开始播放时，即使 scroll_to_index 没变，也会更新模糊级别
        self.buffered_lines != old_buffered_lines
    }

    /// Find the index of the next line that will play after the given time
    /// Used during interludes to determine which line should be "active" (no blur)
    fn find_next_line_index(&self, time: u64, lines: &[LyricLineData]) -> usize {
        // Find the first non-BG line that starts after the current time
        for (i, line) in lines.iter().enumerate() {
            if !line.is_bg && line.start_ms > time {
                return i;
            }
        }
        // If no future line found, find the last line that ended before current time
        // This handles the case where we're past all lyrics
        for (i, line) in lines.iter().enumerate().rev() {
            if !line.is_bg && line.end_ms <= time {
                return i;
            }
        }
        // Fallback to 0
        0
    }

    /// Update interlude dots state
    ///
    /// - Checks gaps at (scrollToIndex - 1), scrollToIndex, and (scrollToIndex + 1)
    /// - Gap end is adjusted by -250ms (early end before next line starts)
    /// - Sets dots position based on the interlude's location in the lyrics
    fn update_interlude(&mut self, time_ms: f64, lines: &[LyricLineData]) {
        let current_time = time_ms as u64 + 20;
        let old_insert_after = self.interlude_insert_after;
        let old_extra_height = self.interlude_extra_height;
        let old_align_right = self.interlude_align_right;
        let old_enabled = self.interlude_dots.enabled;

        // Check if we're in an interlude (no active lines)
        if !self.buffered_lines.is_empty() {
            self.interlude_dots.set_interlude(None);
            self.interlude_insert_after = None;
            self.interlude_extra_height = 0.0;
            self.interlude_align_right = false;
            let layout_changed = old_insert_after.is_some()
                || old_enabled
                || old_align_right
                || old_extra_height.abs() > 0.5;
            if layout_changed {
                self.layout_dirty = true;
            }
            return;
        }

        let idx = self.scroll_to_index as i64;
        let min_duration = self.config.interlude_min_duration;

        // Returns (gap_start, gap_end, k_index) if we're in a valid gap
        let check_gap = |k: i64| -> Option<(u64, u64, i64)> {
            if k < -1 || k >= lines.len() as i64 - 1 {
                return None;
            }

            let gap_start = if k == -1 {
                0u64
            } else {
                lines[k as usize].end_ms
            };

            let next_line = &lines[(k + 1) as usize];
            let gap_end = gap_start.max(next_line.start_ms.saturating_sub(250));

            if gap_end.saturating_sub(gap_start) < min_duration {
                return None;
            }

            if gap_end > current_time && gap_start < current_time {
                Some((gap_start, gap_end, k))
            } else {
                None
            }
        };

        let interlude = check_gap(idx - 1)
            .or_else(|| check_gap(idx))
            .or_else(|| check_gap(idx + 1));

        if let Some((gap_start, gap_end, interlude_line_idx)) = interlude {
            // Use FIXED gap boundaries as the interlude range.
            self.interlude_dots
                .set_interlude(Some((gap_start as f32, gap_end as f32)));
            self.interlude_dots.sync_time(current_time as f32);
            let font_size = self.last_font_size.max(self.config.resolved_line_height());
            // `interlude_line_idx == -1` means the gap is before the first lyric line.
            // Keep that state explicit with a sentinel instead of casting directly and overflowing later.
            self.interlude_insert_after = Some(if interlude_line_idx < 0 {
                usize::MAX
            } else {
                interlude_line_idx as usize
            });
            self.interlude_extra_height = interlude_dots::dot_total_height(
                font_size,
                self.viewport_width,
                self.viewport_height,
            );
            self.interlude_align_right = lines
                .get((interlude_line_idx + 1) as usize)
                .map(|line| line.is_duet)
                .unwrap_or(false);
        } else {
            self.interlude_dots.set_interlude(None);
            self.interlude_insert_after = None;
            self.interlude_extra_height = 0.0;
            self.interlude_align_right = false;
        }

        let layout_changed = old_insert_after != self.interlude_insert_after
            || (old_extra_height - self.interlude_extra_height).abs() > 0.5
            || old_align_right != self.interlude_align_right
            || old_enabled != self.interlude_dots.enabled;

        if layout_changed {
            self.layout_dirty = true;
        }
    }

    fn update_interlude_transform(&mut self) {
        let Some(top) = self.line_animations.current_interlude_top() else {
            return;
        };

        let font_size = self.last_font_size.max(self.config.resolved_line_height());
        let dot_width = interlude_dots::dot_container_width(
            font_size,
            self.viewport_width,
            self.viewport_height,
        );
        let pad_x = interlude_dots::dot_padding_x(font_size);
        let trailing_margin = interlude_dots::dot_trailing_margin_px(font_size);
        let base_padding = self.viewport_width * 0.05;
        let left = if self.interlude_align_right {
            (self.viewport_width - base_padding - dot_width + pad_x + trailing_margin).max(0.0)
        } else {
            (base_padding - pad_x).max(0.0)
        };

        self.interlude_dots.set_transform(left, top);
    }

    /// Pause playback effects
    pub fn pause(&mut self) {
        self.is_playing = false;
        self.interlude_dots.pause();
    }

    /// Resume playback effects
    pub fn resume(&mut self) {
        self.is_playing = true;
        self.interlude_dots.resume();
    }

    /// Check if playing
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Get interlude dots state for rendering
    pub fn interlude_dots(&self) -> &InterludeDots {
        &self.interlude_dots
    }

    /// Get buffered line indices
    pub fn buffered_lines(&self) -> &std::collections::HashSet<usize> {
        &self.buffered_lines
    }

    /// Get scroll target index
    pub fn scroll_to_index(&self) -> usize {
        self.scroll_to_index
    }

    /// Keep cached static traits aligned with the current lyrics slice.
    pub fn sync_line_traits(&mut self, lines: &[LyricLineData]) {
        let source_ptr = lines.as_ptr() as usize;
        if source_ptr != self.line_traits_source_ptr || lines.len() != self.line_traits_source_len {
            self.line_traits = LyricsLineTraits::from_lines(lines);
            self.line_traits_source_ptr = source_ptr;
            self.line_traits_source_len = lines.len();
        }
    }

    /// Static traits for the current lyrics.
    pub fn line_traits(&self) -> LyricsLineTraits {
        self.line_traits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_browsing_ends_on_resume_timeout_and_new_lyrics() {
        let mut engine = LyricsEngine::new(LyricsEngineConfig::default());
        assert!(!engine.is_manually_scrolling());
        engine.handle_wheel(0.0);
        engine.handle_wheel(f32::NAN);
        assert!(!engine.is_manually_scrolling());

        engine.handle_wheel(40.0);
        assert!(engine.is_manually_scrolling());
        engine.resume_auto_follow();
        assert!(!engine.is_manually_scrolling());

        engine.handle_wheel(40.0);
        engine.config.scroll_timeout = 0.0;
        engine.update(1.0 / 60.0);
        assert!(!engine.is_manually_scrolling());

        engine.handle_wheel(40.0);
        engine.reset_for_new_lyrics();
        assert!(!engine.is_manually_scrolling());
    }

    #[test]
    fn lyric_seek_keeps_drawn_positions_then_animates_to_playback() {
        let lines: Vec<_> = (0..8)
            .map(|index| LyricLineData {
                text: format!("Line {index}"),
                start_ms: index * 2000,
                end_ms: (index + 1) * 2000,
                ..Default::default()
            })
            .collect();
        let positions = |engine: &LyricsEngine| {
            engine
                .line_animations
                .animations_slice()
                .iter()
                .map(|line| line.current_y())
                .collect::<Vec<_>>()
        };

        for (initial_time, target_time) in [(0.0, 6000.0), (10000.0, 2000.0), (4000.0, 4000.0)] {
            let mut engine = LyricsEngine::new(LyricsEngineConfig::default());
            engine.set_viewport_info(800.0, 600.0);
            engine.set_current_time(initial_time, &lines, true);
            engine.handle_wheel(120.0);
            engine.set_current_time(initial_time, &lines, false);
            for _ in 0..180 {
                engine.update(1.0 / 60.0);
            }
            let before = positions(&engine);

            engine.resume_auto_follow();
            engine.set_current_time(target_time, &lines, false);
            assert_eq!(
                positions(&engine),
                before,
                "click must not snap line positions"
            );
            assert!(!engine.is_manually_scrolling());

            for _ in 0..240 {
                engine.update(1.0 / 60.0);
            }
            let after = positions(&engine);
            assert_ne!(
                after, before,
                "lines must move toward the playback position"
            );

            let mut target = LyricsEngine::new(LyricsEngineConfig::default());
            target.set_viewport_info(800.0, 600.0);
            target.set_current_time(target_time, &lines, true);
            for (actual, expected) in after.into_iter().zip(positions(&target)) {
                assert!(
                    (actual - expected).abs() < 0.1,
                    "animation must settle at playback: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn animation_and_resize_never_shape_or_replace_prepared_layout() {
        let mut engine = LyricsEngine::new(LyricsEngineConfig::default());
        let prepared = Arc::new(Vec::new());
        engine.set_cached_shaped_lines_arc_with_metrics(prepared.clone(), 600.0, 48.0);
        let heights = engine.cached_line_heights_arc();
        for frame in 0..120 {
            engine.set_viewport_info(800.0, 900.0 + frame as f32);
            engine.update(1.0 / 60.0);
            engine.set_cached_shaped_lines_arc_with_metrics(prepared.clone(), 600.0, 48.0);
        }
        assert!(Arc::ptr_eq(&engine.cached_shaped_lines(), &prepared));
        assert!(Arc::ptr_eq(&engine.cached_line_heights_arc(), &heights));
        assert_eq!(engine.resolved_font_size(), 48.0);
    }
}
