//! iced Pipeline/Primitive 适配层
//!
//! 本模块实现 iced 框架的 `Pipeline` 和 `Primitive` trait，
//! 作为 iced shader widget 和底层 GPU 渲染（gpu_pipeline.rs）之间的桥梁。
//!
//! ## 架构关系
//!
//! ```text
//! lyrics.rs (UI Page)
//!     ↓ 创建 LyricsEngineProgram
//! program.rs (iced Program trait)
//!     ↓ draw() 返回 LyricsEnginePrimitive
//! pipeline.rs (本模块)
//!     ├─ LyricsEnginePrimitive: 收集渲染数据
//!     └─ LyricsEnginePipeline: 调用 GPU 管线
//!         ↓ prepare() / render()
//! gpu_pipeline.rs (LyricsGpuPipeline)
//!     └─ 实际的 wgpu 渲染实现
//! ```

use super::{LyricsEngine, LyricsEngineConfig};
use crate::features::lyrics::engine::{
    CachedShapedLine,
    gpu_pipeline::LyricsGpuPipeline,
    types::{ComputedLineStyle, LyricLineData, LyricsLineTraits},
};
use iced::Rectangle;
use iced::wgpu;
use iced::widget::shader::{Pipeline, Primitive};
use std::collections::HashSet;
use std::sync::Arc;

const AMLL_MAX_BLUR_REFERENCE_PX: f32 = 5.0;
const AMLL_BG_SLIDE_PERCENT: f32 = 80.0;

#[inline]
fn logical_blur_to_physical(logical_blur: f32, renderer_scale: f32, visual_scale: f32) -> f32 {
    logical_blur.clamp(0.0, AMLL_MAX_BLUR_REFERENCE_PX)
        * visual_scale.max(0.0)
        * renderer_scale.max(0.0)
}

#[inline]
fn background_slide_progress(slide_y: f32) -> f32 {
    (1.0 - slide_y.abs() / AMLL_BG_SLIDE_PERCENT).clamp(0.0, 1.0)
}

#[inline]
fn background_slide_scale(slide_y: f32) -> f32 {
    0.8 + background_slide_progress(slide_y) * 0.2
}

/// iced Pipeline 实现，管理 GPU 管线生命周期
pub struct LyricsEnginePipeline {
    /// The GPU pipeline for text rendering
    gpu_pipeline: Option<LyricsGpuPipeline>,
    /// Whether the pipeline is initialized
    initialized: bool,
    /// Cached render parameters for blur pass
    cached_render_params: Option<CachedRenderParams>,
}

/// Cached parameters for render pass (set in prepare, used in render)
#[derive(Clone)]
struct CachedRenderParams {
    viewport_width: u32,
    viewport_height: u32,
    enable_blur: bool,
}

impl LyricsEnginePipeline {
    /// Create a new pipeline (uninitialized)
    pub fn new() -> Self {
        Self {
            gpu_pipeline: None,
            initialized: false,
            cached_render_params: None,
        }
    }
}

impl Default for LyricsEnginePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline for LyricsEnginePipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let gpu_pipeline = LyricsGpuPipeline::new(device, format);
        Self {
            gpu_pipeline: Some(gpu_pipeline),
            initialized: true,
            cached_render_params: None,
        }
    }
}

/// 歌词渲染数据（Primitive）
///
/// 包含一帧渲染所需的所有数据，由 `LyricsEngineProgram::draw()` 创建，
/// 传递给 `LyricsEnginePipeline::prepare()` 进行 GPU 数据准备。
///
/// Features:
/// - Per-line spring animations for Y position and scale
/// - Distance-based blur levels
/// - Staggered animation delays
/// - Interlude dots animation
///
/// Performance optimization:
/// - Uses Arc<Vec<LyricLineData>> to avoid cloning lyrics data each frame
/// - Uses pre-allocated AnimationBuffers for animation state
/// - Uses Arc<Vec<CachedShapedLine>> for Single Source of Truth text layout
#[derive(Debug, Clone)]
pub struct LyricsEnginePrimitive {
    /// Lyrics lines (Arc for O(1) clone, thread-safe)
    pub lines: Arc<Vec<LyricLineData>>,
    /// Cached shaped lines from LyricsEngine (Single Source of Truth)
    /// Contains all glyph positions, heights, and word bounds
    pub shaped_lines: Arc<Vec<CachedShapedLine>>,
    /// Current scroll position (legacy, kept for compatibility)
    pub scroll_position: f32,
    /// Whether the user is browsing lyrics after scrolling manually.
    pub is_manually_scrolling: bool,
    /// Buffered (active) line indices
    pub buffered_lines: HashSet<usize>,
    /// Scroll target index
    pub scroll_to_index: usize,
    /// Current playback time in milliseconds
    pub current_time_ms: f32,
    /// Logical font size matching the shaped cache.
    pub font_size: f32,
    /// Engine configuration
    pub config: LyricsEngineConfig,
    /// Whether playback is active
    pub is_playing: bool,
    /// Interlude dots state
    pub interlude_dots: Option<InterludeDotsState>,
    /// Cached line heights from engine (in logical pixels)
    pub cached_line_heights: Arc<Vec<f32>>,
    /// Static traits derived from the current lyrics.
    pub line_traits: LyricsLineTraits,
    /// Per-line animated Y positions (in logical pixels)
    pub line_positions: Arc<Vec<f32>>,
    /// Per-line animated scales (0.0 - 1.0)
    pub line_scales: Arc<Vec<f32>>,
    /// Per-line blur levels (distance-based blur)
    pub line_blur_levels: Arc<Vec<f32>>,
    /// Per-line opacities
    pub line_opacities: Arc<Vec<f32>>,
    /// Per-line background wrapper slide positions (-80..0 percent)
    pub line_bg_slide_y: Arc<Vec<f32>>,
    /// Per-line independent mask alpha values
    pub line_bright_mask_alpha: Arc<Vec<f32>>,
    pub line_dark_mask_alpha: Arc<Vec<f32>>,
}

/// Serializable interlude dots state for primitive
#[derive(Debug, Clone)]
pub struct InterludeDotsState {
    pub enabled: bool,
    pub left: f32,
    pub scale: f32,
    pub dot_opacities: [f32; 3],
    pub top: f32,
}

impl LyricsEnginePrimitive {
    /// Hit-test the same animated layout used by the GPU, in logical pixels.
    pub(super) fn hit_test(&self, bounds: Rectangle, cursor: iced::mouse::Cursor) -> Option<usize> {
        if !self.is_manually_scrolling {
            return None;
        }
        let position = cursor.position_in(bounds)?;
        let styles = self.compute_line_styles_physical(&bounds, 1.0);

        // Later lines are painted on top when spring animations overlap.
        styles.iter().enumerate().rev().find_map(|(index, style)| {
            let line = self.lines.get(index)?;
            let shaped = self.shaped_lines.get(index)?;
            if style.opacity < 0.01 || line.text.trim().is_empty() {
                return None;
            }

            // The shader scales vertically about the center of the whole line,
            // including wrapped text, translation and romanization.
            let height = shaped.total_height * style.scale;
            let row = Rectangle {
                x: bounds.width * 0.05,
                y: style.y_position + (shaped.total_height - height) * 0.5,
                width: bounds.width * 0.9,
                height,
            };
            row.contains(position).then_some(index)
        })
    }

    /// Create a new primitive from engine state
    ///
    /// Captures all animation state including:
    /// - Per-line Y positions (from spring animations)
    /// - Per-line scales (from spring animations)
    /// - Per-line blur levels (distance-based)
    /// - Per-line opacities
    /// - Interlude dots state
    /// - Cached shaped lines (Single Source of Truth for text layout)
    ///
    /// Performance optimization:
    /// - Uses Arc<Vec<LyricLineData>> for O(1) clone of lyrics data (thread-safe)
    /// - Uses Arc<Vec<CachedShapedLine>> for O(1) clone of shaped lines
    /// - Uses pre-allocated AnimationBuffers from engine instead of creating new Vecs each frame
    pub fn from_engine(
        engine: &mut LyricsEngine,
        lines: Arc<Vec<LyricLineData>>,
        current_time_ms: f32,
    ) -> Self {
        let line_count = lines.len();
        engine.sync_line_traits(&lines);
        let line_traits = engine.line_traits();
        let config = engine.config();

        // Get pre-allocated animation buffers (updated in-place during engine.update())
        // These snapshots are Arc clones so Program::draw() can clone primitives without
        // copying per-line animation arrays.
        let buffers = engine.animation_buffers();
        let line_positions = if buffers.positions().len() < line_count {
            let mut values = buffers.positions().to_vec();
            let default_y = config.align_position * 800.0 * 2.0;
            while values.len() < line_count {
                values.push(default_y + (values.len() as f32 * 100.0));
            }
            Arc::new(values)
        } else {
            buffers.positions_arc()
        };
        let line_scales = if buffers.scales().len() < line_count {
            let mut values = buffers.scales().to_vec();
            while values.len() < line_count {
                values.push(config.inactive_scale);
            }
            Arc::new(values)
        } else {
            buffers.scales_arc()
        };
        let line_blur_levels = if buffers.blur_levels().len() < line_count {
            let mut values = buffers.blur_levels().to_vec();
            while values.len() < line_count {
                values.push(3.0);
            }
            Arc::new(values)
        } else {
            buffers.blur_levels_arc()
        };
        let line_opacities = if buffers.opacities().len() < line_count {
            let mut values = buffers.opacities().to_vec();
            let default_opacity = if line_traits.is_non_dynamic { 0.2 } else { 1.0 };
            while values.len() < line_count {
                values.push(default_opacity);
            }
            Arc::new(values)
        } else {
            buffers.opacities_arc()
        };
        let line_bg_slide_y = if buffers.bg_slide_y().len() < line_count {
            let mut values = buffers.bg_slide_y().to_vec();
            while values.len() < line_count {
                values.push(0.0);
            }
            Arc::new(values)
        } else {
            buffers.bg_slide_y_arc()
        };
        let line_bright_mask_alpha = if buffers.bright_mask_alpha().len() < line_count {
            let mut values = buffers.bright_mask_alpha().to_vec();
            while values.len() < line_count {
                values.push(0.2);
            }
            Arc::new(values)
        } else {
            buffers.bright_mask_alpha_arc()
        };
        let line_dark_mask_alpha = if buffers.dark_mask_alpha().len() < line_count {
            let mut values = buffers.dark_mask_alpha().to_vec();
            while values.len() < line_count {
                values.push(0.2);
            }
            Arc::new(values)
        } else {
            buffers.dark_mask_alpha_arc()
        };

        // Get cached shaped lines (Single Source of Truth)
        let shaped_lines = engine.cached_shaped_lines();

        // Now get immutable data
        let dots = engine.interlude_dots();
        let interlude_dots = if dots.enabled {
            Some(InterludeDotsState {
                enabled: true,
                left: dots.left,
                scale: dots.scale,
                dot_opacities: dots.dot_opacities,
                top: dots.top,
            })
        } else {
            None
        };

        Self {
            lines,                // Arc clone is O(1)
            shaped_lines,         // Arc clone is O(1)
            scroll_position: 0.0, // No longer used with per-line animations
            is_manually_scrolling: engine.is_manually_scrolling(),
            buffered_lines: engine.buffered_lines().clone(),
            scroll_to_index: engine.scroll_to_index(),
            current_time_ms,
            font_size: engine.resolved_font_size(),
            config: engine.config().clone(),
            is_playing: engine.is_playing(),
            interlude_dots,
            cached_line_heights: engine.cached_line_heights_arc(),
            line_traits,
            line_positions,
            line_scales,
            line_blur_levels,
            line_opacities,
            line_bg_slide_y,
            line_bright_mask_alpha,
            line_dark_mask_alpha,
        }
    }

    /// Compute line styles for rendering using physical pixels
    ///
    /// This version uses per-line animated positions from LineAnimationManager
    /// instead of calculating positions from scroll offset.
    ///
    /// Features:
    /// - Per-line spring animations for Y position and scale
    /// - Distance-based blur (increases with distance from active line)
    /// - Staggered animation delays for "waterfall" effect
    /// - Proper opacity handling for background lines (CSS)
    pub fn compute_line_styles_physical(
        &self,
        viewport: &Rectangle<f32>,
        scale: f32,
    ) -> Vec<ComputedLineStyle> {
        let mut styles = Vec::with_capacity(self.lines.len());
        let is_non_dynamic = self.line_traits.is_non_dynamic;

        let physical_height = viewport.height * scale;

        // Calculate alignment position for lens calculations
        let align_y = physical_height * self.config.align_position;

        // Check if we have per-line animations
        let use_line_animations =
            !self.line_positions.is_empty() && self.line_positions.len() == self.lines.len();

        // Find the latest active line index for blur calculation
        let latest_index = self
            .buffered_lines
            .iter()
            .max()
            .copied()
            .unwrap_or(self.scroll_to_index);

        for (idx, line) in self.lines.iter().enumerate() {
            // Get animated Y position (in logical pixels, convert to physical)
            let y_position = if use_line_animations {
                self.line_positions[idx] * scale
            } else {
                // Fallback: use align_y (shouldn't happen in normal operation)
                align_y
            };

            // Get animated scale
            let animated_scale = if use_line_animations && idx < self.line_scales.len() {
                self.line_scales[idx]
            } else {
                1.0
            };

            let bg_slide_y = if line.is_bg && idx < self.line_bg_slide_y.len() {
                self.line_bg_slide_y[idx]
            } else {
                0.0
            };
            let bg_slide_scale = if line.is_bg {
                background_slide_scale(bg_slide_y)
            } else {
                1.0
            };
            let render_scale = animated_scale * bg_slide_scale;
            let line_height_physical = self
                .cached_line_heights
                .get(idx)
                .copied()
                .unwrap_or(self.config.resolved_line_height())
                * scale;

            let is_active = self.buffered_lines.contains(&idx);

            // Use pre-computed blur from LineAnimationManager if available
            // Otherwise calculate distance-based blur
            let blur = if !self.config.enable_blur {
                0.0
            } else if idx < self.line_blur_levels.len() {
                // Use pre-computed blur from LineAnimationManager
                self.line_blur_levels[idx]
            } else if is_active {
                0.0
            } else {
                let mut level = 1.0;
                if idx < self.scroll_to_index {
                    // Lines above current: blur increases with distance
                    level += (self.scroll_to_index - idx) as f32 + 1.0;
                } else {
                    // Lines below current: blur increases with distance from latest active
                    level +=
                        (idx as i32 - latest_index.max(self.scroll_to_index) as i32).abs() as f32;
                }
                level
            };

            // Calculate glow for active lines
            let glow = if is_active { 0.5 } else { 0.0 };

            // Use pre-computed opacity from LineAnimationManager if available
            // This includes proper handling for:
            // - Background lines: 0.0001 (inactive), 0.4 (active or not playing)
            // - Normal lines: 0.85 (active), 1.0 (inactive), 0.2 (non-dynamic)
            let final_opacity = if idx < self.line_opacities.len() {
                // Use pre-computed opacity from LineAnimationManager
                let base_opacity = self.line_opacities[idx];
                // Apply hide passed lines on top
                if self.config.hide_passed_lines && idx < self.scroll_to_index && self.is_playing {
                    0.0001
                } else {
                    base_opacity
                }
            } else {
                // Fallback calculation (shouldn't happen in normal operation)
                if self.config.hide_passed_lines && idx < self.scroll_to_index && self.is_playing {
                    0.0001
                } else if is_active {
                    0.85
                } else if line.is_bg {
                    // CSS: .lyricBgLine { opacity: 0.0001; }
                    // .lyricBgLine.active { opacity: 0.4; }
                    // :not(.playing) > .lyricBgLine { opacity: 0.4; }
                    if !self.is_playing { 0.4 } else { 0.0001 }
                } else if is_non_dynamic {
                    0.2
                } else {
                    1.0
                }
            };

            // Animation state is kept in logical/CSS pixels. Convert blur to
            // physical pixels at the same boundary as positions and glyphs.
            let blur_px = logical_blur_to_physical(blur, scale, self.config.visual_scale());
            let (bright_mask_alpha, dark_mask_alpha) = if idx < self.line_bright_mask_alpha.len()
                && idx < self.line_dark_mask_alpha.len()
            {
                (
                    self.line_bright_mask_alpha[idx],
                    self.line_dark_mask_alpha[idx],
                )
            } else if is_active {
                (1.0, 0.4)
            } else {
                (0.2, 0.2)
            };

            styles.push(ComputedLineStyle {
                y_position: y_position + (bg_slide_y / 100.0) * line_height_physical,
                scale: render_scale,
                blur: blur_px,
                opacity: final_opacity,
                glow,
                is_active,
                bright_mask_alpha,
                dark_mask_alpha,
            });
        }

        styles
    }
}

impl Primitive for LyricsEnginePrimitive {
    type Pipeline = LyricsEnginePipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &iced::widget::shader::Viewport,
    ) {
        if !pipeline.initialized {
            return;
        }

        let Some(gpu_pipeline) = &mut pipeline.gpu_pipeline else {
            return;
        };

        // Get scale factor for physical pixels
        let scale = viewport.scale_factor();

        // Full viewport (window) size in physical pixels
        let full_viewport_width = viewport.physical_width() as f32;
        let full_viewport_height = viewport.physical_height() as f32;

        // Widget bounds in physical pixels
        let bounds_x = bounds.x * scale;
        let bounds_y = bounds.y * scale;
        let bounds_width = bounds.width * scale;
        let bounds_height = bounds.height * scale;

        // Shaping and rendering share one resolved logical font size. The
        // application supplies root-rem scaling to the engine; widget bounds
        // only determine clipping and available layout space.
        let font_size = self.font_size * scale;

        // Compute line styles based on scroll position (using physical pixels)
        let line_styles = self.compute_line_styles_physical(bounds, scale);

        // Prepare GPU pipeline with new data
        // Use cached shaped_lines from LyricsEngine (Single Source of Truth)
        // This avoids duplicate text shaping in GPU pipeline
        gpu_pipeline.prepare_with_shaped_lines(
            device,
            queue,
            full_viewport_width,
            full_viewport_height,
            bounds_x,
            bounds_y,
            bounds_width,
            bounds_height,
            &self.lines,        // Arc<Vec<T>> derefs to &[T]
            &self.shaped_lines, // Pre-shaped lines from LyricsEngine
            &line_styles,
            &self.cached_line_heights, // Logical pixels; GPU pipeline scales on demand.
            self.line_traits,
            self.current_time_ms,
            self.scroll_position,
            font_size,
            self.config.word_fade_width,
            self.config.resolved_overscan(),
            scale, // Scale factor for logical to physical conversion
            self.config.trans_height_ratio,
            self.config.roman_height_ratio,
        );

        gpu_pipeline.clear_interlude_dots();

        // Prepare interlude dots if present
        if let Some(ref dots) = self.interlude_dots {
            let mut dots_state = crate::features::lyrics::engine::InterludeDots::new();
            dots_state.left = dots.left;
            dots_state.top = dots.top;
            dots_state.enabled = dots.enabled;
            dots_state.scale = dots.scale;
            dots_state.dot_opacities = dots.dot_opacities;

            gpu_pipeline.prepare_interlude_dots(
                crate::features::lyrics::engine::gpu_pipeline::InterludeDotsPreparation {
                    device,
                    queue,
                    dots: &dots_state,
                    viewport_size: [full_viewport_width, full_viewport_height],
                    bounds_origin: [bounds_x, bounds_y],
                    scale_factor: scale,
                    logical_font_size: font_size / scale,
                },
            );
        }

        // Prepare blur rendering resources only when the current frame has
        // visible line blur or emphasis glow work.
        let enable_blur = self.config.enable_blur && gpu_pipeline.has_preparable_blur();
        if enable_blur {
            gpu_pipeline.prepare_blur(device, queue);
        } else {
            gpu_pipeline.clear_prepared_blur();
        }

        // Cache render parameters
        pipeline.cached_render_params = Some(CachedRenderParams {
            viewport_width: viewport.physical_width(),
            viewport_height: viewport.physical_height(),
            enable_blur,
        });
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        if !pipeline.initialized {
            return;
        }

        let Some(gpu_pipeline) = &pipeline.gpu_pipeline else {
            return;
        };

        // 检查是否启用模糊效果并且有缓存的渲染参数
        let use_blur = pipeline
            .cached_render_params
            .as_ref()
            .map(|p| p.enable_blur)
            .unwrap_or(false);

        if use_blur {
            // 逐行模糊渲染模式
            // 每行歌词独立渲染和模糊，避免不同行之间的模糊混合
            if let Some(ref params) = pipeline.cached_render_params {
                gpu_pipeline.render_with_per_line_blur(
                    encoder,
                    target,
                    clip_bounds,
                    params.viewport_width,
                    params.viewport_height,
                );
                return;
            }
        }

        // 直接渲染模式（不使用多 pass 模糊）
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lyrics Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Don't clear - we're rendering on top of background
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // Set scissor rect to clip bounds
        render_pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );

        // Render interlude dots first (behind text)
        gpu_pipeline.render_interlude_dots(&mut render_pass);

        // Render lyrics text
        gpu_pipeline.render(&mut render_pass);
    }
}

#[cfg(test)]
mod tests {
    use super::{background_slide_progress, background_slide_scale, logical_blur_to_physical};

    #[test]
    fn blur_cap_is_applied_before_physical_scale() {
        assert_eq!(logical_blur_to_physical(4.0, 2.0, 1.0), 8.0);
        assert_eq!(logical_blur_to_physical(8.0, 2.0, 1.0), 10.0);
        assert_eq!(logical_blur_to_physical(-1.0, 2.0, 1.0), 0.0);
        assert!((logical_blur_to_physical(5.0, 1.0, 4.0 / 3.0) - 20.0 / 3.0).abs() < 0.0001);
    }

    #[test]
    fn background_slide_uses_amll_progress_and_scale() {
        assert_eq!(background_slide_progress(-80.0), 0.0);
        assert_eq!(background_slide_progress(0.0), 1.0);
        assert_eq!(background_slide_scale(-80.0), 0.8);
        assert_eq!(background_slide_scale(0.0), 1.0);
    }
}
