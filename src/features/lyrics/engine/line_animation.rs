//! Per-line animation system for lyrics
//!
//! Each lyric line has its own Spring for Y position and scale,
//! enabling smooth, independent animations with staggered delays.
//!
//! ## Spring Parameters
//!
//! Different spring parameters are used for different properties:
//!
//! ### Position Y
//! - mass: 0.9, damping: 15, stiffness: 90
//! - Provides responsive but smooth vertical movement
//!
//! ### Scale (normal lines)
//! - mass: 2, damping: 25, stiffness: 100
//! - Heavier mass for more deliberate scale changes
//!
//! ### Scale (background lines)
//! - mass: 1, damping: 20, stiffness: 50
//! - Slower, more subtle animation for background vocals
//!
//! ## Performance Optimization
//!
//! `AnimationBuffers` provides pre-allocated buffers for animation state,
//! avoiding per-frame Vec allocations in the render path.

use super::spring::{Spring, SpringParams};
use std::sync::Arc;

const AMLL_TRANSITION_DURATION: f32 = 0.4;
const AMLL_MASK_INACTIVE_DURATION: f32 = 0.45;
const AMLL_MASK_ACTIVE_DURATION: f32 = 0.3;
const AMLL_EASE_X1: f32 = 0.25;
const AMLL_EASE_Y1: f32 = 0.1;
const AMLL_EASE_X2: f32 = 0.25;
const AMLL_EASE_Y2: f32 = 1.0;
const AMLL_MAX_BLUR: f32 = 5.0;

#[inline]
fn cubic_bezier_axis(t: f32, p1: f32, p2: f32) -> f32 {
    let one_minus_t = 1.0 - t;
    3.0 * one_minus_t * one_minus_t * t * p1 + 3.0 * one_minus_t * t * t * p2 + t * t * t
}

#[inline]
fn cubic_bezier_axis_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let one_minus_t = 1.0 - t;
    3.0 * one_minus_t * one_minus_t * p1
        + 6.0 * one_minus_t * t * (p2 - p1)
        + 3.0 * t * t * (1.0 - p2)
}

/// Evaluate AMLL's CSS `ease` curve (`cubic-bezier(0.25, 0.1, 0.25, 1)`).
#[inline]
fn amll_ease(progress: f32) -> f32 {
    let x = progress.clamp(0.0, 1.0);
    let mut t = x;

    for _ in 0..8 {
        let current_x = cubic_bezier_axis(t, AMLL_EASE_X1, AMLL_EASE_X2);
        let derivative = cubic_bezier_axis_derivative(t, AMLL_EASE_X1, AMLL_EASE_X2);
        if derivative.abs() < 1e-5 {
            break;
        }
        t = (t - (current_x - x) / derivative).clamp(0.0, 1.0);
    }

    cubic_bezier_axis(t, AMLL_EASE_Y1, AMLL_EASE_Y2).clamp(0.0, 1.0)
}

/// Evaluate AMLL's mask transition `cubic-bezier(0, 0, 0.58, 1)`.
#[inline]
fn ease_out_progress(progress: f32) -> f32 {
    let x = progress.clamp(0.0, 1.0);
    let mut t = x;
    for _ in 0..8 {
        let current_x = cubic_bezier_axis(t, 0.0, 0.58);
        let derivative = cubic_bezier_axis_derivative(t, 0.0, 0.58);
        if derivative.abs() < 1e-5 {
            break;
        }
        t = (t - (current_x - x) / derivative).clamp(0.0, 1.0);
    }
    cubic_bezier_axis(t, 0.0, 1.0).clamp(0.0, 1.0)
}

/// Manual scroll bounds for the lyrics viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollBounds {
    pub min: f32,
    pub max: f32,
}

// ============================================================================
// AnimationBuffers - Pre-allocated buffers for animation state
// ============================================================================

/// Pre-allocated buffers for animation state
///
/// This struct avoids per-frame Vec allocations by maintaining
/// pre-sized buffers that are updated in-place.
///
/// ## Usage
///
/// ```ignore
/// let mut buffers = AnimationBuffers::default();
/// buffers.ensure_capacity(line_count);
/// buffers.update_from_manager(&line_animation_manager);
/// // Now use buffers.positions(), buffers.scales(), etc.
/// ```
#[derive(Debug, Clone, Default)]
pub struct AnimationBuffers {
    /// Y positions in logical pixels
    positions: Arc<Vec<f32>>,
    /// Scale values (0.0 - 1.0)
    scales: Arc<Vec<f32>>,
    /// Blur levels
    blur_levels: Arc<Vec<f32>>,
    /// Opacity values (0.0 - 1.0)
    opacities: Arc<Vec<f32>>,
    /// Background wrapper slide positions (-80..0, in percent)
    bg_slide_y: Arc<Vec<f32>>,
    /// Independent AMLL mask alpha values
    bright_mask_alpha: Arc<Vec<f32>>,
    dark_mask_alpha: Arc<Vec<f32>>,
}

impl AnimationBuffers {
    /// Create new empty buffers
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure buffers have enough capacity for the given line count
    ///
    /// This resizes buffers only when necessary, avoiding allocations
    /// when the line count hasn't changed.
    pub fn ensure_capacity(&mut self, line_count: usize) {
        Self::ensure_buffer(&mut self.positions, line_count, 0.0);
        Self::ensure_buffer(&mut self.scales, line_count, 1.0);
        Self::ensure_buffer(&mut self.blur_levels, line_count, 0.0);
        Self::ensure_buffer(&mut self.opacities, line_count, 1.0);
        Self::ensure_buffer(&mut self.bg_slide_y, line_count, 0.0);
        Self::ensure_buffer(&mut self.bright_mask_alpha, line_count, 0.2);
        Self::ensure_buffer(&mut self.dark_mask_alpha, line_count, 0.2);
    }

    fn ensure_buffer(buffer: &mut Arc<Vec<f32>>, line_count: usize, fill: f32) {
        if let Some(values) = Arc::get_mut(buffer) {
            if values.len() != line_count {
                values.resize(line_count, fill);
            }
        } else {
            if buffer.len() != line_count {
                *buffer = Arc::new(vec![fill; line_count]);
            }
        }
    }

    /// Update buffers from LineAnimationManager (in-place, no allocations)
    ///
    /// This copies the current animation state from the manager into
    /// the pre-allocated buffers.
    pub fn update_from_manager(&mut self, manager: &LineAnimationManager) {
        let animations = manager.animations_slice();
        let len = animations.len();

        // Ensure capacity
        self.ensure_capacity(len);

        // Update in-place when unique; otherwise write into a fresh buffer instead of
        // cloning the previous frame only to overwrite every value below.
        Self::ensure_unique(&mut self.positions, len, 0.0);
        Self::ensure_unique(&mut self.scales, len, 1.0);
        Self::ensure_unique(&mut self.blur_levels, len, 0.0);
        Self::ensure_unique(&mut self.opacities, len, 1.0);
        Self::ensure_unique(&mut self.bg_slide_y, len, 0.0);
        Self::ensure_unique(&mut self.bright_mask_alpha, len, 0.2);
        Self::ensure_unique(&mut self.dark_mask_alpha, len, 0.2);

        let positions = Arc::get_mut(&mut self.positions).expect("buffer is unique");
        let scales = Arc::get_mut(&mut self.scales).expect("buffer is unique");
        let blur_levels = Arc::get_mut(&mut self.blur_levels).expect("buffer is unique");
        let opacities = Arc::get_mut(&mut self.opacities).expect("buffer is unique");
        let bg_slide_y = Arc::get_mut(&mut self.bg_slide_y).expect("buffer is unique");
        let bright_mask_alpha =
            Arc::get_mut(&mut self.bright_mask_alpha).expect("buffer is unique");
        let dark_mask_alpha = Arc::get_mut(&mut self.dark_mask_alpha).expect("buffer is unique");

        for (i, anim) in animations.iter().enumerate() {
            positions[i] = anim.current_y();
            scales[i] = anim.current_scale();
            blur_levels[i] = anim.blur;
            opacities[i] = anim.opacity;
            bg_slide_y[i] = anim.current_bg_slide_y();
            bright_mask_alpha[i] = anim.bright_mask_alpha;
            dark_mask_alpha[i] = anim.dark_mask_alpha;
        }
    }

    fn ensure_unique(buffer: &mut Arc<Vec<f32>>, line_count: usize, fill: f32) {
        match Arc::get_mut(buffer) {
            Some(values) => {
                if values.len() != line_count {
                    values.resize(line_count, fill);
                }
            }
            None => {
                *buffer = Arc::new(vec![fill; line_count]);
            }
        }
    }

    /// Clone the current positions snapshot.
    #[inline]
    pub fn positions_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.positions)
    }

    /// Clone the current scales snapshot.
    #[inline]
    pub fn scales_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.scales)
    }

    /// Clone the current blur levels snapshot.
    #[inline]
    pub fn blur_levels_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.blur_levels)
    }

    /// Clone the current opacity snapshot.
    #[inline]
    pub fn opacities_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.opacities)
    }

    #[inline]
    pub fn bg_slide_y_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.bg_slide_y)
    }

    #[inline]
    pub fn bright_mask_alpha_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.bright_mask_alpha)
    }

    #[inline]
    pub fn dark_mask_alpha_arc(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.dark_mask_alpha)
    }

    /// Get positions slice (no allocation)
    #[inline]
    pub fn positions(&self) -> &[f32] {
        &self.positions
    }

    /// Get scales slice (no allocation)
    #[inline]
    pub fn scales(&self) -> &[f32] {
        &self.scales
    }

    /// Get blur levels slice (no allocation)
    #[inline]
    pub fn blur_levels(&self) -> &[f32] {
        &self.blur_levels
    }

    /// Get opacities slice (no allocation)
    #[inline]
    pub fn opacities(&self) -> &[f32] {
        &self.opacities
    }

    #[inline]
    pub fn bg_slide_y(&self) -> &[f32] {
        &self.bg_slide_y
    }

    #[inline]
    pub fn bright_mask_alpha(&self) -> &[f32] {
        &self.bright_mask_alpha
    }

    #[inline]
    pub fn dark_mask_alpha(&self) -> &[f32] {
        &self.dark_mask_alpha
    }

    /// Clear all buffers
    pub fn clear(&mut self) {
        self.positions = Arc::new(Vec::new());
        self.scales = Arc::new(Vec::new());
        self.blur_levels = Arc::new(Vec::new());
        self.opacities = Arc::new(Vec::new());
        self.bg_slide_y = Arc::new(Vec::new());
        self.bright_mask_alpha = Arc::new(Vec::new());
        self.dark_mask_alpha = Arc::new(Vec::new());
    }
}

// ============================================================================
// LineAnimation - Animation state for a single line
// ============================================================================

/// Animation state for a single lyric line
#[derive(Debug, Clone)]
pub struct LineAnimation {
    /// Y position spring (in logical pixels)
    pub pos_y: Spring,
    /// Scale spring (0-100, where 100 = 1.0)
    pub scale: Spring,
    /// Target opacity (0.0 - 1.0) - will be smoothly interpolated
    pub target_opacity: f32,
    /// Current animated opacity (0.0 - 1.0)
    pub opacity: f32,
    /// Target blur level - will be smoothly interpolated
    pub target_blur: f32,
    /// Current animated blur level
    pub blur: f32,
    /// Target/current mask alpha values (AMLL's independent CSS custom properties)
    pub target_bright_mask_alpha: f32,
    pub bright_mask_alpha: f32,
    pub target_dark_mask_alpha: f32,
    pub dark_mask_alpha: f32,
    bright_mask_transition_from: f32,
    dark_mask_transition_from: f32,
    bright_mask_transition_elapsed: f32,
    dark_mask_transition_elapsed: f32,
    /// Background wrapper slide position in percent of the background line height.
    pub bg_slide_y: Spring,
    blur_transition_from: f32,
    blur_transition_elapsed: f32,
    opacity_transition_from: f32,
    opacity_transition_elapsed: f32,
    /// Whether this line is currently active
    pub is_active: bool,
    /// Whether this is a background line
    pub is_bg: bool,
    /// Current delay for staggered animation (seconds)
    pub delay: f32,
}

impl LineAnimation {
    /// Create a new line animation with initial Y position
    pub fn new(initial_y: f32, is_bg: bool) -> Self {
        // Use the spring parameters via SpringParams
        let mut pos_y = Spring::from_params(initial_y as f64, SpringParams::POS_Y);
        pos_y.set_target(initial_y as f64);

        // Scale spring: different parameters for normal vs background lines
        let scale_params = if is_bg {
            SpringParams::SCALE_BG
        } else {
            SpringParams::SCALE
        };
        let mut scale = Spring::from_params(100.0, scale_params);
        scale.set_target(100.0);
        let mut bg_slide_y = Spring::from_params(-80.0, SpringParams::POS_Y);
        bg_slide_y.set_target(-80.0);

        Self {
            pos_y,
            scale,
            target_opacity: 1.0,
            opacity: 1.0,
            target_blur: 0.0,
            blur: 0.0,
            target_bright_mask_alpha: 0.2,
            bright_mask_alpha: 0.2,
            target_dark_mask_alpha: 0.2,
            dark_mask_alpha: 0.2,
            bright_mask_transition_from: 0.2,
            dark_mask_transition_from: 0.2,
            bright_mask_transition_elapsed: AMLL_MASK_INACTIVE_DURATION,
            dark_mask_transition_elapsed: AMLL_MASK_INACTIVE_DURATION,
            bg_slide_y,
            blur_transition_from: 0.0,
            blur_transition_elapsed: AMLL_TRANSITION_DURATION,
            opacity_transition_from: 1.0,
            opacity_transition_elapsed: AMLL_TRANSITION_DURATION,
            is_active: false,
            is_bg,
            delay: 0.0,
        }
    }

    /// Set target Y position with optional delay
    /// delay is in seconds
    pub fn set_target_y(&mut self, target: f32, delay: f32) {
        self.delay = delay;
        if delay > 0.0 {
            self.pos_y
                .set_target_with_delay(target as f64, delay as f64);
        } else {
            self.pos_y.set_target(target as f64);
        }
    }

    fn set_target_bg_slide_y(&mut self, target: f32, delay: f32) {
        if delay > 0.0 {
            self.bg_slide_y
                .set_target_with_delay(target as f64, delay as f64);
        } else {
            self.bg_slide_y.set_target(target as f64);
        }
    }

    /// Set target scale (0-100) with optional delay
    pub fn set_target_scale(&mut self, target: f32) {
        // Scale uses the same delay as position for coordinated animation
        if self.delay > 0.0 {
            self.scale
                .set_target_with_delay(target as f64, self.delay as f64);
        } else {
            self.scale.set_target(target as f64);
        }
    }

    /// Force set Y position (no animation)
    pub fn set_position_y(&mut self, pos: f32) {
        self.pos_y.set_position(pos as f64);
        self.pos_y.set_target(pos as f64);
    }

    fn set_bg_slide_y(&mut self, pos: f32) {
        self.bg_slide_y.set_position(pos as f64);
        self.bg_slide_y.set_target(pos as f64);
    }

    /// Force set scale (no animation)
    pub fn set_scale(&mut self, scale: f32) {
        self.scale.set_position(scale as f64);
        self.scale.set_target(scale as f64);
    }

    /// Get current animated Y position
    pub fn current_y(&self) -> f32 {
        self.pos_y.position() as f32
    }

    /// Get current animated scale (0.0 - 1.0)
    pub fn current_scale(&self) -> f32 {
        self.scale.position() as f32 / 100.0
    }

    #[inline]
    pub fn current_bg_slide_y(&self) -> f32 {
        self.bg_slide_y.position() as f32
    }

    /// Update spring parameters (for runtime configuration)
    pub fn update_pos_y_params(&mut self, params: SpringParams) {
        self.pos_y.update_params(params);
    }

    /// Set a CSS-transition-style blur target in logical/CSS pixels.
    pub fn set_target_blur(&mut self, target: f32) {
        let target = target.clamp(0.0, AMLL_MAX_BLUR);
        if (target - self.target_blur).abs() > 0.0001 {
            self.blur_transition_from = self.blur;
            self.blur_transition_elapsed = 0.0;
            self.target_blur = target;
        }
    }

    /// Set a CSS-transition-style opacity target.
    pub fn set_target_opacity(&mut self, target: f32) {
        let target = target.clamp(0.0, 1.0);
        if (target - self.target_opacity).abs() > 0.0001 {
            self.opacity_transition_from = self.opacity;
            self.opacity_transition_elapsed = 0.0;
            self.target_opacity = target;
        }
    }

    /// Immediately apply both visual targets, matching AMLL's seek behavior.
    pub fn snap_visual_targets(&mut self) {
        self.blur = self.target_blur;
        self.opacity = self.target_opacity;
        self.blur_transition_from = self.blur;
        self.blur_transition_elapsed = AMLL_TRANSITION_DURATION;
        self.opacity_transition_from = self.opacity;
        self.opacity_transition_elapsed = AMLL_TRANSITION_DURATION;
        self.snap_mask_targets();
    }

    /// 更新弹簧和平滑过渡 - 每帧调用
    ///
    /// @param delta 距上次更新的时间（秒）
    pub fn update(&mut self, delta: f32) {
        self.pos_y.update(delta as f64);
        self.scale.update(delta as f64);
        self.bg_slide_y.update(delta as f64);

        // Match AMLL's `filter 0.4s ease` and `opacity 0.4s ease` transitions.
        self.blur_transition_elapsed =
            (self.blur_transition_elapsed + delta).min(AMLL_TRANSITION_DURATION);
        self.opacity_transition_elapsed =
            (self.opacity_transition_elapsed + delta).min(AMLL_TRANSITION_DURATION);
        let bright_duration = if self.target_bright_mask_alpha > 0.2 {
            AMLL_MASK_ACTIVE_DURATION
        } else {
            AMLL_MASK_INACTIVE_DURATION
        };
        let dark_duration = if self.target_dark_mask_alpha > 0.2 {
            AMLL_MASK_ACTIVE_DURATION
        } else {
            AMLL_MASK_INACTIVE_DURATION
        };
        self.bright_mask_transition_elapsed =
            (self.bright_mask_transition_elapsed + delta).min(bright_duration);
        self.dark_mask_transition_elapsed =
            (self.dark_mask_transition_elapsed + delta).min(dark_duration);

        let blur_progress = amll_ease(self.blur_transition_elapsed / AMLL_TRANSITION_DURATION);
        let opacity_progress =
            amll_ease(self.opacity_transition_elapsed / AMLL_TRANSITION_DURATION);

        self.blur = self.blur_transition_from
            + (self.target_blur - self.blur_transition_from) * blur_progress;
        self.opacity = self.opacity_transition_from
            + (self.target_opacity - self.opacity_transition_from) * opacity_progress;

        let bright_progress =
            ease_out_progress(self.bright_mask_transition_elapsed / bright_duration);
        let dark_progress = ease_out_progress(self.dark_mask_transition_elapsed / dark_duration);
        self.bright_mask_alpha = self.bright_mask_transition_from
            + (self.target_bright_mask_alpha - self.bright_mask_transition_from) * bright_progress;
        self.dark_mask_alpha = self.dark_mask_transition_from
            + (self.target_dark_mask_alpha - self.dark_mask_transition_from) * dark_progress;
    }

    fn set_target_mask_alpha(&mut self, bright: f32, dark: f32) {
        if (bright - self.target_bright_mask_alpha).abs() > 0.0001 {
            self.bright_mask_transition_from = self.bright_mask_alpha;
            self.bright_mask_transition_elapsed = 0.0;
            self.target_bright_mask_alpha = bright;
        }
        if (dark - self.target_dark_mask_alpha).abs() > 0.0001 {
            self.dark_mask_transition_from = self.dark_mask_alpha;
            self.dark_mask_transition_elapsed = 0.0;
            self.target_dark_mask_alpha = dark;
        }
    }

    fn snap_mask_targets(&mut self) {
        self.bright_mask_alpha = self.target_bright_mask_alpha;
        self.dark_mask_alpha = self.target_dark_mask_alpha;
        self.bright_mask_transition_from = self.bright_mask_alpha;
        self.dark_mask_transition_from = self.dark_mask_alpha;
        self.bright_mask_transition_elapsed = AMLL_MASK_ACTIVE_DURATION;
        self.dark_mask_transition_elapsed = AMLL_MASK_ACTIVE_DURATION;
    }
}

/// Align anchor for lyric lines (the alignAnchor)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignAnchor {
    /// Align to top of the line (no adjustment)
    Top,
    /// Align to center of the line (default)
    #[default]
    Center,
    /// Align to bottom of the line
    Bottom,
}

/// Manager for all line animations
///
/// Implements layout calculation with:
/// - Per-line spring animations for Y position and scale
/// - Staggered delays for "waterfall" effect
/// - Distance-based blur calculation
/// - Different spring parameters for normal vs background lines
#[derive(Debug, Default)]
pub struct LineAnimationManager {
    /// Animations for each line
    animations: Vec<LineAnimation>,
    /// Viewport height for calculations
    viewport_height: f32,
    /// Align position (0.0 - 1.0, default 0.35)
    align_position: f32,
    /// Align anchor: top/center/bottom (default: center)
    align_anchor: AlignAnchor,
    /// Whether to hide passed lines (the hidePassedLines)
    hide_passed_lines: bool,
    /// Custom position Y spring parameters (optional override)
    pos_y_params: Option<SpringParams>,
    /// Interlude insertion point from the latest layout pass.
    interlude_insert_after: Option<usize>,
    /// Reserved interlude height from the latest layout pass.
    interlude_extra_height: f32,
    /// Top margin inside the reserved interlude slot.
    interlude_top_margin: f32,
}

impl LineAnimationManager {
    /// Create a new animation manager
    pub fn new() -> Self {
        Self {
            animations: Vec::new(),
            viewport_height: 800.0,
            align_position: 0.35,
            align_anchor: AlignAnchor::Center,
            hide_passed_lines: false,
            pos_y_params: None,
            interlude_insert_after: None,
            interlude_extra_height: 0.0,
            interlude_top_margin: 0.0,
        }
    }

    /// Set viewport height
    pub fn set_viewport_height(&mut self, height: f32) {
        self.viewport_height = height;
    }

    /// Set align position (0.0 - 1.0, default: 0.35)
    pub fn set_align_position(&mut self, pos: f32) {
        self.align_position = pos;
    }

    /// Set align anchor (the setAlignAnchor)
    ///
    /// - Top: align to top of the target line
    /// - Center: align to center of the target line (default)
    /// - Bottom: align to bottom of the target line
    pub fn set_align_anchor(&mut self, anchor: AlignAnchor) {
        self.align_anchor = anchor;
    }

    /// Set whether to hide passed lines (the hidePassedLines)
    pub fn set_hide_passed_lines(&mut self, hide: bool) {
        self.hide_passed_lines = hide;
    }

    /// Set custom position Y spring parameters
    pub fn set_pos_y_spring_params(&mut self, params: SpringParams) {
        self.pos_y_params = Some(params);
        for anim in &mut self.animations {
            anim.update_pos_y_params(params);
        }
    }

    /// Ensure we have enough animations for the given line count
    /// Returns true if animations were reset (new song)
    pub fn ensure_capacity(&mut self, line_count: usize, is_bg_flags: &[bool]) -> bool {
        if self.animations.len() != line_count {
            // Reset all animations for new song
            self.animations.clear();
            let initial_y = self.viewport_height * 2.0; // Start off-screen like

            for i in 0..line_count {
                let is_bg = is_bg_flags.get(i).copied().unwrap_or(false);
                let mut anim = LineAnimation::new(initial_y, is_bg);

                // Apply custom spring parameters if set
                if let Some(params) = self.pos_y_params {
                    anim.update_pos_y_params(params);
                }

                self.animations.push(anim);
            }
            return true;
        }
        false
    }

    /// 更新所有弹簧 - 每帧调用
    ///
    /// @param delta 距上次更新的时间（秒）
    pub fn update(&mut self, delta: f32) {
        for anim in &mut self.animations {
            anim.update(delta);
        }
    }

    /// Get the current animated interlude top derived from lyric line springs.
    pub fn current_interlude_top(&self) -> Option<f32> {
        let insert_after = self.interlude_insert_after?;
        let next_line_idx = if insert_after == usize::MAX {
            0
        } else {
            insert_after.saturating_add(1)
        };
        let next_line = self.animations.get(next_line_idx)?;

        Some(next_line.current_y() - self.interlude_extra_height + self.interlude_top_margin)
    }

    /// Calculate and set target positions for all lines (the calcLayout)
    ///
    /// This implements the layout algorithm with:
    /// - Staggered delays for "waterfall" animation effect
    /// - Distance-based blur calculation
    /// - Different handling for background vs normal lines
    ///
    /// Parameters:
    /// - line_heights: Height of each line in logical pixels
    /// - line_spacing: Spacing between lines
    /// - scroll_to_index: Current target line index
    /// - buffered_lines: Set of active line indices
    /// - is_playing: Whether playback is active
    /// - is_seek: Whether this is a seek operation (force immediate position)
    /// - enable_scale: Whether to apply scale effect
    /// - inactive_scale: Scale factor for inactive lines (default: 0.97)
    /// - bg_line_scale: Scale factor for background lines (default: 0.75)
    ///
    /// 完整布局计算
    ///
    /// 移植自 `lyric-player/base.ts` 的 `calcLayout`
    ///
    /// Additional parameters:
    /// - is_non_dynamic: True if all lines have only 1 word (affects opacity)
    /// - viewport_width: Retained for renderer API compatibility; responsive
    ///   visual scaling is supplied by the application at the draw boundary.
    #[expect(
        clippy::too_many_arguments,
        reason = "layout function preserves the upstream lyric-engine parameter contract; remove when a versioned LyricsLayoutInput replaces it"
    )]
    pub fn calc_layout_full(
        &mut self,
        line_heights: &[f32],
        line_spacing: f32,
        scroll_to_index: usize,
        buffered_lines: &std::collections::HashSet<usize>,
        is_playing: bool,
        is_seek: bool,
        enable_scale: bool,
        inactive_scale: f32,
        bg_line_scale: f32,
        stagger_base_delay: f32,
        stagger_reduction_factor: f32,
        is_non_dynamic: bool,
        _viewport_width: f32,
        manual_scroll_offset: f32,
        disable_blur: bool,
        interlude_insert_after: Option<usize>,
        interlude_extra_height: f32,
        interlude_top_margin: f32,
    ) -> ScrollBounds {
        if self.animations.is_empty() {
            self.interlude_insert_after = None;
            self.interlude_extra_height = 0.0;
            self.interlude_top_margin = 0.0;
            return ScrollBounds { min: 0.0, max: 0.0 };
        }

        self.interlude_insert_after = interlude_insert_after;
        self.interlude_extra_height = interlude_extra_height;
        self.interlude_top_margin = interlude_top_margin;

        // default: LINE_HEIGHT_FALLBACK = size[1] / 5
        let line_height_fallback = self.viewport_height / 5.0;

        let blur_multiplier = 1.0;

        // default: targetAlignIndex (may differ from scrollToIndex during interlude)
        // For now, we use scroll_to_index directly. Interlude handling can be added later.
        let target_align_index = scroll_to_index;

        // default: Calculate scroll offset (sum of heights before target line, excluding BG lines when playing)
        // scrollOffset = currentLyricLineObjects.slice(0, targetAlignIndex).reduce(...)
        let interlude_before_line = |line_idx: usize| -> bool {
            match interlude_insert_after {
                Some(insert_after) if insert_after == usize::MAX => line_idx == 0,
                Some(insert_after) => insert_after.saturating_add(1) == line_idx,
                None => false,
            }
        };

        let mut scroll_offset = 0.0f32;
        for idx in 0..target_align_index.min(self.animations.len()) {
            let is_bg = self.animations.get(idx).map(|a| a.is_bg).unwrap_or(false);
            if is_bg && is_playing {
                continue; // default: Skip BG lines in scroll calculation when playing
            }
            let height = line_heights
                .get(idx)
                .copied()
                .unwrap_or(line_height_fallback);
            scroll_offset += height + line_spacing;
        }

        // when dots are inserted after an existing line, the whole layout is lifted first,
        // then the dots height is added back right before the following line.
        let initial_interlude_offset = match interlude_insert_after {
            Some(insert_after) if insert_after != usize::MAX => interlude_extra_height,
            _ => 0.0,
        };

        // curPos = -scrollOffset + size[1] * alignPosition - manualScrollOffset
        let mut cur_pos = -scroll_offset - initial_interlude_offset
            + self.viewport_height * self.align_position
            - manual_scroll_offset;

        // default: Apply alignAnchor adjustment to curPos
        if let Some(cur_line_height) = line_heights.get(target_align_index) {
            match self.align_anchor {
                AlignAnchor::Bottom => cur_pos -= *cur_line_height,
                AlignAnchor::Center => cur_pos -= *cur_line_height / 2.0,
                AlignAnchor::Top => {} // No adjustment
            }
        }

        // default: latestIndex = Math.max(...bufferedLines)
        // 注意：当 buffered_lines 为空时，使用 scroll_to_index 作为 latest_index
        // 但这会导致 is_active 条件 (idx >= scroll_to_index && idx < latest_index) 永远为 false
        // 因此我们需要确保至少 scroll_to_index 对应的行是 active 的
        let latest_index = buffered_lines
            .iter()
            .max()
            .copied()
            .unwrap_or(scroll_to_index);

        // default: delay and baseDelay
        let mut delay = 0.0f32;
        let mut base_delay = if is_seek { 0.0 } else { stagger_base_delay };

        for (idx, anim) in self.animations.iter_mut().enumerate() {
            if interlude_before_line(idx) {
                cur_pos += interlude_extra_height;
            }

            let height = line_heights
                .get(idx)
                .copied()
                .unwrap_or(line_height_fallback);
            let has_buffered = buffered_lines.contains(&idx);

            // default: isActive = hasBuffered || (i >= scrollToIndex && i < latestIndex)
            // 修复：当 buffered_lines 为空时，scroll_to_index 对应的行应该是 active
            // 这发生在歌词间隙期间（interlude），此时没有行正在播放，但我们仍然
            // 希望 scroll_to_index 指向的行（即将播放的行）不被模糊
            let is_active = has_buffered
                || (idx >= scroll_to_index && idx < latest_index)
                || (buffered_lines.is_empty() && idx == scroll_to_index);

            // Update active state
            anim.is_active = is_active;

            // AMLL keeps mask alpha independent from line scale. Gradient/active
            // lines animate to (1.0, 0.4) in 0.3s ease-out; solid lines return to
            // (0.2, 0.2) in 0.45s ease-out.
            if is_active {
                anim.set_target_mask_alpha(1.0, 0.4);
            } else {
                anim.set_target_mask_alpha(0.2, 0.2);
            }

            // Background vocals have a separate wrapper spring in AMLL. Rustle's
            // flattened line list uses the default post-position direction.
            let target_bg_slide_y = if anim.is_bg {
                if is_active || !is_playing { 0.0 } else { -80.0 }
            } else {
                0.0
            };

            // default: Calculate target scale
            // SCALE_ASPECT = enableScale ? 97 : 100
            // targetScale = 100 if active or !playing, else (isBG ? 75 : SCALE_ASPECT)
            let scale_aspect = if enable_scale {
                inactive_scale * 100.0
            } else {
                100.0
            };
            let target_scale = if !is_active && is_playing {
                if anim.is_bg {
                    bg_line_scale * 100.0
                } else {
                    scale_aspect
                }
            } else {
                100.0
            };

            // default: Calculate blur level (distance-based)
            let blur_level = if disable_blur || is_active {
                0.0
            } else {
                let mut level = 1.0;
                if idx < scroll_to_index {
                    // Lines above current: blur increases with distance
                    level += (scroll_to_index - idx) as f32 + 1.0;
                } else {
                    // Lines below current: blur increases with distance from latest active
                    level += (idx as i32 - latest_index.max(scroll_to_index) as i32).abs() as f32;
                }
                level * blur_multiplier
            };
            // AMLL clamps the CSS blur value before its transition starts.
            anim.set_target_blur(blur_level.min(AMLL_MAX_BLUR));
            if disable_blur {
                anim.blur = 0.0;
                anim.blur_transition_from = 0.0;
                anim.blur_transition_elapsed = AMLL_TRANSITION_DURATION;
            }

            // default: Calculate opacity
            // hidePassedLines logic + normal opacity logic
            let target_opacity = if self.hide_passed_lines {
                if idx < scroll_to_index && is_playing {
                    // default: 为了避免浏览器优化，使用极小但不为零的值
                    0.0001
                } else if anim.is_bg {
                    if is_active || !is_playing {
                        0.4
                    } else {
                        0.0001
                    }
                } else if has_buffered {
                    0.85
                } else if is_non_dynamic {
                    0.2
                } else {
                    1.0
                }
            } else {
                // No hidePassedLines
                if anim.is_bg {
                    if is_active || !is_playing {
                        0.4
                    } else {
                        0.0001
                    }
                } else if has_buffered {
                    0.85
                } else if is_non_dynamic {
                    0.2
                } else {
                    1.0
                }
            };
            // Match AMLL's wrapper opacity transition target.
            anim.set_target_opacity(target_opacity);

            // Set targets
            if is_seek {
                // Force immediate position on seek
                anim.set_position_y(cur_pos);
                anim.set_scale(target_scale);
                anim.set_bg_slide_y(target_bg_slide_y);
                // Also force immediate blur and opacity on seek
                anim.snap_visual_targets();
            } else {
                anim.set_target_y(cur_pos, delay);
                anim.set_target_scale(target_scale);
                anim.set_target_bg_slide_y(target_bg_slide_y, delay);
            }

            // default: Advance position for next line
            // BG lines only take space if active or not playing
            let takes_space = if anim.is_bg {
                is_active || !is_playing
            } else {
                true
            };
            if takes_space {
                cur_pos += height + line_spacing;
            }

            // default: Update delay for staggered animation
            // Only apply delay when curPos >= 0 and not seeking
            if cur_pos >= 0.0 && !is_seek {
                if !anim.is_bg {
                    delay += base_delay;
                }
                // default: Reduce baseDelay after scrollToIndex for "waterfall" effect
                if idx >= scroll_to_index {
                    base_delay /= stagger_reduction_factor;
                }
            }
        }

        ScrollBounds {
            min: -scroll_offset,
            max: cur_pos + manual_scroll_offset - self.viewport_height / 2.0,
        }
    }

    /// Get number of animations
    pub fn len(&self) -> usize {
        self.animations.len()
    }

    /// Get animations slice (for AnimationBuffers)
    #[inline]
    pub fn animations_slice(&self) -> &[LineAnimation] {
        &self.animations
    }

    /// Clear all line animations so the next layout rebuilds from scratch.
    pub fn reset(&mut self) {
        self.animations.clear();
        self.interlude_insert_after = None;
        self.interlude_extra_height = 0.0;
        self.interlude_top_margin = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Helper to create a LineAnimationManager with test data
    fn create_test_manager(line_count: usize) -> LineAnimationManager {
        let mut manager = LineAnimationManager::new();
        manager.set_viewport_height(800.0);
        let is_bg_flags: Vec<bool> = vec![false; line_count];
        manager.ensure_capacity(line_count, &is_bg_flags);
        manager
    }

    #[test]
    fn interlude_top_tracks_next_line_current_position() {
        let mut manager = create_test_manager(2);
        let line_heights = vec![48.0, 48.0];
        let buffered_lines = HashSet::new();

        manager.calc_layout_full(
            &line_heights,
            8.0,
            0,
            &buffered_lines,
            true,
            true,
            true,
            0.97,
            0.75,
            0.0,
            1.05,
            false,
            1280.0,
            0.0,
            false,
            Some(0),
            36.0,
            6.0,
        );

        let next_line_y = manager.animations[1].current_y();
        assert!((manager.current_interlude_top().unwrap() - (next_line_y - 30.0)).abs() < 0.001);

        manager.animations[1].set_position_y(next_line_y + 24.0);
        assert!((manager.current_interlude_top().unwrap() - (next_line_y - 6.0)).abs() < 0.001);
    }

    /// Helper to get blur level for a specific line after layout calculation
    fn get_blur_for_line(
        manager: &mut LineAnimationManager,
        line_count: usize,
        line_index: usize,
        scroll_to_index: usize,
        buffered_lines: &HashSet<usize>,
        viewport_width: f32,
    ) -> f32 {
        let line_heights: Vec<f32> = vec![48.0; line_count];
        manager.calc_layout_full(
            &line_heights,
            8.0, // line_spacing
            scroll_to_index,
            buffered_lines,
            true,  // is_playing
            true,  // is_seek (force immediate values)
            true,  // enable_scale
            0.97,  // inactive_scale
            0.75,  // bg_line_scale
            0.05,  // stagger_base_delay
            1.05,  // stagger_reduction_factor
            false, // is_non_dynamic
            viewport_width,
            0.0,   // manual_scroll_offset
            false, // disable_blur
            None,
            0.0,
            0.0,
        );
        manager.animations[line_index].blur
    }

    fn get_opacity_for_line(
        manager: &mut LineAnimationManager,
        line_count: usize,
        line_index: usize,
        scroll_to_index: usize,
        buffered_lines: &HashSet<usize>,
        is_non_dynamic: bool,
    ) -> f32 {
        let line_heights: Vec<f32> = vec![48.0; line_count];
        manager.calc_layout_full(
            &line_heights,
            8.0,
            scroll_to_index,
            buffered_lines,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            is_non_dynamic,
            1920.0,
            0.0,
            false,
            None,
            0.0,
            0.0,
        );
        manager.animations[line_index].opacity
    }

    // ========== Property 1: Active lines have zero blur ==========

    #[test]
    fn test_active_line_has_zero_blur() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(5); // Line 5 is active

        let blur = get_blur_for_line(&mut manager, 10, 5, 5, &buffered, 1920.0);
        assert_eq!(blur, 0.0, "Active line should have zero blur");
    }

    #[test]
    fn test_multiple_active_lines_have_zero_blur() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(3);
        buffered.insert(4);
        buffered.insert(5);

        for idx in 3..=5 {
            let blur = get_blur_for_line(&mut manager, 10, idx, 3, &buffered, 1920.0);
            assert_eq!(blur, 0.0, "Active line {} should have zero blur", idx);
        }
    }

    // ========== Property 2: Blur level calculation for inactive lines ==========

    #[test]
    fn test_blur_level_for_lines_before_current() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(5);

        // Line 3 is before scroll_to_index (5)
        // Formula: 1 + abs(scrollToIndex - lineIndex) + 1 = 1 + abs(5 - 3) + 1 = 4
        let blur = get_blur_for_line(&mut manager, 10, 3, 5, &buffered, 1920.0);
        assert_eq!(
            blur, 4.0,
            "Line before current should have blur = 1 + distance + 1"
        );

        // Line 0's raw value is 7, then AMLL's 5px cap is applied.
        let blur = get_blur_for_line(&mut manager, 10, 0, 5, &buffered, 1920.0);
        assert_eq!(blur, 5.0, "Line 0 should be capped at blur = 5");
    }

    #[test]
    fn test_blur_level_for_lines_after_current() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(5);

        // Line 7 is after scroll_to_index (5), latest_index = 5
        // Formula: 1 + abs(lineIndex - max(scrollToIndex, latestIndex)) = 1 + abs(7 - 5) = 3
        let blur = get_blur_for_line(&mut manager, 10, 7, 5, &buffered, 1920.0);
        assert_eq!(
            blur, 3.0,
            "Line after current should have blur = 1 + distance"
        );

        // Line 9: 1 + abs(9 - 5) = 5
        let blur = get_blur_for_line(&mut manager, 10, 9, 5, &buffered, 1920.0);
        assert_eq!(blur, 5.0, "Line 9 should have blur = 5");
    }

    // ========== Property 3: Blur level is independent of local width ==========

    #[test]
    fn test_blur_is_not_scaled_by_renderer_width() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(5);

        let blur_large = get_blur_for_line(&mut manager, 10, 3, 5, &buffered, 1920.0);
        let blur_small = get_blur_for_line(&mut manager, 10, 3, 5, &buffered, 1024.0);

        assert_eq!(blur_large, 4.0, "Large viewport should have full blur");
        assert_eq!(blur_small, blur_large);
    }

    #[test]
    fn test_blur_is_stable_across_renderer_widths() {
        let mut manager = create_test_manager(10);
        let mut buffered = HashSet::new();
        buffered.insert(5);

        let blur_1025 = get_blur_for_line(&mut manager, 10, 3, 5, &buffered, 1025.0);
        let blur_1920 = get_blur_for_line(&mut manager, 10, 3, 5, &buffered, 1920.0);

        assert_eq!(blur_1025, blur_1920);
    }

    #[test]
    fn test_non_dynamic_inactive_line_uses_opacity() {
        let mut manager = create_test_manager(4);
        let mut buffered = HashSet::new();
        buffered.insert(1);

        let non_dynamic_opacity = get_opacity_for_line(&mut manager, 4, 3, 1, &buffered, true);
        let dynamic_opacity = get_opacity_for_line(&mut manager, 4, 3, 1, &buffered, false);

        assert_eq!(non_dynamic_opacity, 0.2);
        assert_eq!(dynamic_opacity, 1.0);
    }

    // ========== Property 4: Blur level maximum cap ==========

    #[test]
    fn test_blur_is_capped_before_animation() {
        let mut manager = create_test_manager(50);
        let mut buffered = HashSet::new();
        buffered.insert(25);

        // Line 0 with scroll_to_index = 25 is clamped to AMLL's 5px cap.
        let blur = get_blur_for_line(&mut manager, 50, 0, 25, &buffered, 1920.0);
        assert_eq!(
            blur, 5.0,
            "Distant line blur should be capped at AMLL's 5px"
        );

        // The cap also applies when the raw distance is even larger.
        let mut buffered2 = HashSet::new();
        buffered2.insert(40);
        let blur = get_blur_for_line(&mut manager, 50, 0, 40, &buffered2, 1920.0);
        assert_eq!(blur, 5.0, "Blur should remain capped at 5px");
    }

    #[test]
    fn test_amll_ease_curve_endpoints_and_midpoint() {
        assert!((amll_ease(0.0) - 0.0).abs() < 0.0001);
        assert!((amll_ease(1.0) - 1.0).abs() < 0.0001);
        assert!((amll_ease(0.5) - 0.802).abs() < 0.002);
    }

    #[test]
    fn test_mask_alpha_transition_is_independent_from_scale() {
        let mut animation = LineAnimation::new(0.0, false);
        animation.set_target_mask_alpha(1.0, 0.4);
        animation.set_scale(0.97 * 100.0);
        animation.update(0.3);

        assert!((animation.bright_mask_alpha - 1.0).abs() < 0.0001);
        assert!((animation.dark_mask_alpha - 0.4).abs() < 0.0001);
        assert_eq!(animation.current_scale(), 0.97);
    }

    #[test]
    fn test_manual_scroll_offset_matches_direction() {
        let mut manager = create_test_manager(3);
        let line_heights = vec![48.0; 3];
        let buffered = HashSet::new();

        manager.calc_layout_full(
            &line_heights,
            8.0,
            0,
            &buffered,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            0.0,
            false,
            None,
            0.0,
            0.0,
        );
        let base_y = manager.animations[0].current_y();

        manager.calc_layout_full(
            &line_heights,
            8.0,
            0,
            &buffered,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            50.0,
            false,
            None,
            0.0,
            0.0,
        );
        let scrolled_y = manager.animations[0].current_y();

        assert_eq!(
            scrolled_y,
            base_y - 50.0,
            "Positive manual scroll should move lyrics upward scrollOffset"
        );
    }

    #[test]
    fn test_scroll_bounds_are_independent_from_current_manual_offset() {
        let mut manager = create_test_manager(6);
        let line_heights = vec![48.0; 6];
        let mut buffered = HashSet::new();
        buffered.insert(2);

        let base_bounds = manager.calc_layout_full(
            &line_heights,
            8.0,
            2,
            &buffered,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            0.0,
            false,
            None,
            0.0,
            0.0,
        );

        let shifted_bounds = manager.calc_layout_full(
            &line_heights,
            8.0,
            2,
            &buffered,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            120.0,
            false,
            None,
            0.0,
            0.0,
        );

        assert_eq!(base_bounds, shifted_bounds);
    }

    #[test]
    fn test_disable_blur_forces_zero_immediately() {
        let mut manager = create_test_manager(5);
        let line_heights = vec![48.0; 5];
        let mut buffered = HashSet::new();
        buffered.insert(2);

        manager.calc_layout_full(
            &line_heights,
            8.0,
            2,
            &buffered,
            true,
            true,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            0.0,
            false,
            None,
            0.0,
            0.0,
        );
        assert!(manager.animations[0].blur > 0.0);

        manager.calc_layout_full(
            &line_heights,
            8.0,
            2,
            &buffered,
            true,
            false,
            true,
            0.97,
            0.75,
            0.05,
            1.05,
            false,
            1920.0,
            80.0,
            true,
            None,
            0.0,
            0.0,
        );

        assert_eq!(manager.animations[0].target_blur, 0.0);
        assert_eq!(manager.animations[0].blur, 0.0);
    }
}
