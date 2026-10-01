//! GPU Pipeline for lyrics rendering (SDF Version)
//!
//! Custom rendering pipeline that:
//! 1. Uses cosmic-text for text shaping
//! 2. Manages SDF glyph atlas (SDF)
//! 3. Passes timing data per-vertex to GPU
//! 4. Implements word-by-word highlighting in shader
//! 5. Renders interlude dots with breathing animation
//! 6. Supports translation and romanized text
//! 7. Implements virtualization (isInSight) for performance
//! 8. Single-pass SDF rendering with built-in blur effects
//!
//! ## SDF Rendering Architecture
//!
//! ```text
//! cosmic-text (text shaping)
//!     ↓
//! SDF generator (8SSEDT algorithm)
//!     ↓
//! SDF Atlas (4096x4096 RGBA texture)
//!     ↓
//! lyrics_sdf.wgsl (SDF math + fwidth AA)
//!     ↓
//! Single pass with all effects
//! ```

use bytemuck::{Pod, Zeroable};
use iced::wgpu;
use iced::wgpu::{Device, Queue, TextureFormat};
use parking_lot::RwLock;

use super::CachedShapedLine;
use super::interlude_dots::{InterludeDots, dot_padding_x, dot_padding_y, dot_size, dot_spacing};
use super::per_line_blur::{
    GLOW_TEXTURE_FORMAT, GlowBounds, LineRenderInfo, PerLineBlurRenderer, PreparedRenderInput,
};
use super::sdf_cache::SdfCache;
use super::text_shaper::ShapedLine;
use super::types::{
    ComputedLineStyle, FontConfig, LyricLineData, LyricsLineTraits, emphasis_easing,
};
use super::vertex::{GlobalUniform, LineUniform, LyricGlyphVertex};

pub struct InterludeDotsPreparation<'a> {
    pub device: &'a Device,
    pub queue: &'a Queue,
    pub dots: &'a InterludeDots,
    pub viewport_size: [f32; 2],
    pub bounds_origin: [f32; 2],
    pub scale_factor: f32,
    pub logical_font_size: f32,
}

fn lyrics_plus_lighter_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn premultiplied_alpha_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// Uniform data for interlude dots rendering
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DotsUniform {
    /// Position in physical pixels (relative to widget)
    pub position: [f32; 2],
    /// Overall scale (0.0 - 1.0, includes breathing animation)
    pub scale: f32,
    /// Dot size in pixels
    pub dot_size: f32,
    /// Dot spacing in pixels
    pub dot_spacing: f32,
    /// Individual dot opacities (0.0 - 1.0)
    pub dot0_opacity: f32,
    pub dot1_opacity: f32,
    pub dot2_opacity: f32,
    /// Whether dots are enabled
    pub enabled: f32,
    /// Padding to align viewport_size to 8 bytes (WGSL vec2 alignment)
    pub _pad1: f32,
    /// Viewport info
    pub viewport_size: [f32; 2],
    pub bounds_offset: [f32; 2],
    /// Padding to align _padding to 16 bytes (WGSL vec4 alignment)
    pub _pad2: [f32; 2],
    /// Final padding (vec4<f32>)
    pub _padding: [f32; 4],
}

impl Default for DotsUniform {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            scale: 0.0,
            dot_size: 12.0,
            dot_spacing: 20.0,
            dot0_opacity: 0.0,
            dot1_opacity: 0.0,
            dot2_opacity: 0.0,
            enabled: 0.0,
            _pad1: 0.0,
            viewport_size: [800.0, 600.0],
            bounds_offset: [0.0, 0.0],
            _pad2: [0.0, 0.0],
            _padding: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

impl DotsUniform {
    /// Create from InterludeDots state
    pub fn from_interlude_dots(
        dots: &InterludeDots,
        viewport_size: [f32; 2],
        bounds_offset: [f32; 2],
        scale_factor: f32,
        logical_font_size: f32,
    ) -> Self {
        let logical_viewport_height = viewport_size[1] / scale_factor.max(0.001);
        let dot_size = dot_size(logical_font_size, logical_viewport_height) * scale_factor;
        let dot_spacing = dot_spacing(logical_font_size) * scale_factor;
        let padding_x = dot_padding_x(logical_font_size);
        let padding_y = dot_padding_y(logical_font_size);

        Self {
            position: [
                (dots.left + padding_x) * scale_factor,
                (dots.top + padding_y) * scale_factor,
            ],
            scale: dots.scale,
            dot_size,
            dot_spacing,
            dot0_opacity: dots.dot_opacities[0],
            dot1_opacity: dots.dot_opacities[1],
            dot2_opacity: dots.dot_opacities[2],
            enabled: if dots.enabled { 1.0 } else { 0.0 },
            _pad1: 0.0,
            viewport_size,
            bounds_offset,
            _pad2: [0.0, 0.0],
            _padding: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Maximum number of glyphs per frame
const MAX_GLYPHS: usize = 8192;
/// Maximum number of lines
const MAX_LINES: usize = 128;

/// GPU resources for lyrics rendering
pub struct LyricsGpuPipeline {
    // === Direct rendering pipeline (single output) ===
    pipeline: wgpu::RenderPipeline,
    offscreen_pipeline: wgpu::RenderPipeline,
    glow_mask_pipeline: wgpu::RenderPipeline,

    // Buffers for lyrics
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    global_uniform_buffer: wgpu::Buffer,
    line_uniform_buffer: wgpu::Buffer,

    // Bind groups for lyrics
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,

    // Interlude dots rendering
    dots_pipeline: wgpu::RenderPipeline,
    dots_uniform_buffer: wgpu::Buffer,
    dots_bind_group_layout: wgpu::BindGroupLayout,
    dots_bind_group: Option<wgpu::BindGroup>,
    dots_enabled: bool,

    // Glyph management (SDF)
    sdf_cache: SdfCache,

    // Font configuration
    font_config: FontConfig,

    // === 逐行模糊渲染器 ===
    per_line_blur: RwLock<PerLineBlurRenderer>,

    // State
    index_count: u32,
    vertex_scratch: Vec<LyricGlyphVertex>,
    index_scratch: Vec<u32>,

    // Cached uniforms for per-line offscreen rendering
    cached_global_uniform: RwLock<Option<GlobalUniform>>,
    cached_line_uniforms: RwLock<Vec<LineUniform>>,

    // === 逐行渲染信息 ===
    // 缓存的行渲染信息，用于逐行模糊
    cached_line_render_info: RwLock<Vec<LineRenderInfo>>,
}

impl LyricsGpuPipeline {
    /// Create a new GPU pipeline with default font configuration
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        Self::with_config(device, format, FontConfig::default())
    }

    /// Create a new GPU pipeline with custom font configuration
    pub fn with_config(device: &Device, format: TextureFormat, font_config: FontConfig) -> Self {
        // SdfCache uses the global shared FontSystem so cache keys stay aligned with shaping.
        let sdf_cache = SdfCache::with_debug(device, font_config.debug_logging);

        // Create bind group layout for lyrics rendering
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Lyrics Bind Group Layout"),
            entries: &[
                // Global uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Line uniforms (storage buffer)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Glyph atlas texture
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lyrics Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Load SDF shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lyrics SDF Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lyrics_sdf.wgsl").into()),
        });

        // Create direct render pipeline (single output)
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lyrics Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[LyricGlyphVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(lyrics_plus_lighter_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let offscreen_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lyrics Offscreen Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[LyricGlyphVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(premultiplied_alpha_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let glow_mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lyrics Glow Mask Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[LyricGlyphVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_glow_mask"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: GLOW_TEXTURE_FORMAT,
                    blend: Some(premultiplied_alpha_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Create buffers
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lyrics Vertex Buffer"),
            size: (std::mem::size_of::<LyricGlyphVertex>() * MAX_GLYPHS * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lyrics Index Buffer"),
            size: (std::mem::size_of::<u32>() * MAX_GLYPHS * 6) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let global_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lyrics Global Uniform Buffer"),
            size: std::mem::size_of::<GlobalUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let line_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lyrics Line Uniform Buffer"),
            size: (std::mem::size_of::<LineUniform>() * MAX_LINES) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // === Interlude Dots Pipeline ===
        let (dots_pipeline, dots_bind_group_layout, dots_uniform_buffer) =
            Self::create_dots_pipeline(device, format);

        Self {
            pipeline,
            offscreen_pipeline,
            glow_mask_pipeline,
            vertex_buffer,
            index_buffer,
            global_uniform_buffer,
            line_uniform_buffer,
            bind_group_layout,
            bind_group: None,
            dots_pipeline,
            dots_uniform_buffer,
            dots_bind_group_layout,
            dots_bind_group: None,
            dots_enabled: false,
            sdf_cache,
            font_config,
            per_line_blur: RwLock::new(PerLineBlurRenderer::new(device, format)),
            index_count: 0,
            vertex_scratch: Vec::new(),
            index_scratch: Vec::new(),
            cached_global_uniform: RwLock::new(None),
            cached_line_uniforms: RwLock::new(Vec::new()),
            cached_line_render_info: RwLock::new(Vec::new()),
        }
    }

    /// Create interlude dots pipeline
    fn create_dots_pipeline(
        device: &Device,
        format: TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Buffer) {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Interlude Dots Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Interlude Dots Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Interlude Dots Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/interlude_dots.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Interlude Dots Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(lyrics_plus_lighter_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Interlude Dots Uniform Buffer"),
            size: std::mem::size_of::<DotsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        (pipeline, bind_group_layout, uniform_buffer)
    }

    /// 使用预 shaped lines 准备渲染数据
    ///
    /// 首选方法，使用 LyricsEngine 的 CachedShapedLine
    /// 确保 CPU 和 GPU 之间文本布局一致
    ///
    /// The shaped_lines contain all glyph positions calculated by LyricsEngine,
    /// so we don't need to call shape_line again here.
    #[expect(
        clippy::too_many_arguments,
        reason = "GPU frame preparation borrows renderer-owned data without cloning; remove when LyricsFrameInput owns the complete frame snapshot"
    )]
    pub fn prepare_with_shaped_lines(
        &mut self,
        device: &Device,
        queue: &Queue,
        viewport_width: f32,
        viewport_height: f32,
        bounds_x: f32,
        bounds_y: f32,
        bounds_width: f32,
        bounds_height: f32,
        lines: &[LyricLineData],
        shaped_lines: &[CachedShapedLine],
        line_styles: &[ComputedLineStyle],
        line_heights: &[f32], // Pre-calculated by LyricsEngine (in LOGICAL pixels)
        line_traits: LyricsLineTraits,
        current_time_ms: f32,
        scroll_y: f32,
        font_size: f32, // Physical pixels
        word_fade_width: f32,
        overscan_px: f32,
        scale: f32, // Scale factor for logical to physical conversion
        trans_ratio: f32,
        roman_ratio: f32,
    ) {
        // Update global uniforms
        let globals = GlobalUniform {
            viewport_size: [viewport_width, viewport_height],
            bounds_offset: [bounds_x, bounds_y],
            bounds_size: [bounds_width, bounds_height],
            current_time_ms,
            word_fade_width,
            font_size,
            scroll_y,
            align_position: 0.35,
            sdf_range: 4.0, // Default SDF range for distance extrapolation
        };
        *self.cached_global_uniform.write() = Some(globals);
        queue.write_buffer(&self.global_uniform_buffer, 0, bytemuck::bytes_of(&globals));

        // Update line uniforms
        let mut line_uniforms = std::mem::take(self.cached_line_uniforms.get_mut());
        line_uniforms.clear();
        line_uniforms.extend(
            line_styles
                .iter()
                .enumerate()
                .take(MAX_LINES)
                .map(|(idx, style)| {
                    let actual_height = line_heights
                        .get(idx)
                        .map(|height| height * scale)
                        .unwrap_or(font_size * 1.4);
                    LineUniform {
                        y_position: style.y_position,
                        scale: style.scale,
                        blur: style.blur,
                        opacity: style.opacity,
                        glow: style.glow,
                        is_active: if style.is_active { 1 } else { 0 },
                        line_height: actual_height,
                        bright_mask_alpha: style.bright_mask_alpha,
                        dark_mask_alpha: style.dark_mask_alpha,
                        _padding: [0.0; 3],
                    }
                }),
        );

        if !line_uniforms.is_empty() {
            queue.write_buffer(
                &self.line_uniform_buffer,
                0,
                bytemuck::cast_slice(&line_uniforms),
            );
        }
        *self.cached_line_uniforms.get_mut() = line_uniforms;

        // Build geometry from pre-shaped lines (Single Source of Truth)
        // No more duplicate shape_line calls!
        let (vertices, indices) = self.build_geometry_from_shaped(
            queue,
            lines,
            shaped_lines,
            line_styles,
            bounds_width,
            bounds_height,
            font_size,
            line_traits,
            current_time_ms,
            overscan_px,
            scale,
            trans_ratio,
            roman_ratio,
        );

        self.index_count = indices.len() as u32;

        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
        if !indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));
        }

        self.vertex_scratch = vertices;
        self.index_scratch = indices;

        // Update bind group
        self.update_bind_group(device);

        // 更新逐行模糊渲染器的视口尺寸
        self.per_line_blur
            .write()
            .set_viewport_size(bounds_width as u32, bounds_height as u32);
    }

    /// Build geometry from pre-shaped lines (Single Source of Truth)
    ///
    /// This method uses CachedShapedLine from LyricsEngine instead of calling shape_line.
    /// 确保 CPU 布局和 GPU 渲染之间文本布局一致
    ///
    /// Key differences from build_geometry_per_line:
    /// - No shape_line calls - uses pre-computed glyph positions
    /// - Glyph positions are in LOGICAL pixels, scaled to physical for rendering
    /// - Translation and romanized text also use pre-shaped data
    #[expect(
        clippy::too_many_arguments,
        reason = "geometry builder consumes a normalized shaped-frame contract; remove when ShapedGeometryInput groups these immutable values"
    )]
    fn build_geometry_from_shaped(
        &mut self,
        queue: &Queue,
        lines: &[LyricLineData],
        shaped_lines: &[CachedShapedLine],
        line_styles: &[ComputedLineStyle],
        viewport_width: f32,  // Physical pixels
        viewport_height: f32, // Physical pixels
        font_size: f32,       // Physical pixels
        line_traits: LyricsLineTraits,
        current_time_ms: f32,
        overscan_px: f32,
        scale: f32, // Scale factor (physical / logical)
        _trans_ratio: f32,
        _roman_ratio: f32,
    ) -> (Vec<LyricGlyphVertex>, Vec<u32>) {
        let mut all_vertices = std::mem::take(&mut self.vertex_scratch);
        let mut all_indices = std::mem::take(&mut self.index_scratch);
        let mut line_render_info = std::mem::take(self.cached_line_render_info.get_mut());
        all_vertices.clear();
        all_indices.clear();
        line_render_info.clear();

        let has_duet_line = line_traits.has_duet_line;
        let is_non_dynamic = line_traits.is_non_dynamic;
        let base_padding = viewport_width * 0.05;
        // SDF base size for scaling
        let sdf_base_size = 64.0_f32;

        for (line_idx, line) in lines.iter().enumerate().take(MAX_LINES) {
            let style = line_styles.get(line_idx).cloned().unwrap_or_default();
            let mut glow_bounds = None;

            // Get pre-shaped data for this line
            let shaped_line = shaped_lines.get(line_idx);

            let line_height = shaped_line
                .map(|s| s.total_height * scale)
                .unwrap_or(font_size * 1.4);
            let glow_blur_level = if is_non_dynamic {
                0.0
            } else {
                line.words
                    .iter()
                    .filter(|word| word.emphasize || word.should_emphasize())
                    .map(|word| {
                        let glow_radius_em = (word.emphasis_blur() * 0.3).min(0.3);
                        let emphasis_scale = 1.0 + 0.1 * word.emphasis_amount();
                        glow_radius_em * font_size * style.scale * emphasis_scale
                    })
                    .fold(0.0, f32::max)
            };

            let visible = style.opacity >= 0.01
                && LyricLineData::is_in_sight(
                    style.y_position,
                    line_height,
                    viewport_height,
                    overscan_px,
                );

            // 记录这行的起始索引
            let start_index = all_indices.len() as u32;

            if visible && let Some(cached) = shaped_line {
                let mut glow_left = f32::INFINITY;
                let mut glow_top = f32::INFINITY;
                let mut glow_right = f32::NEG_INFINITY;
                let mut glow_bottom = f32::NEG_INFINITY;
                let main_font_size = cached.main_font_size * scale;
                let main_sdf_scale = main_font_size / sdf_base_size;

                // Calculate padding in physical pixels
                let (padding_left, padding_right) = if has_duet_line {
                    if line.is_duet {
                        (viewport_width * 0.15, base_padding)
                    } else {
                        (base_padding, viewport_width * 0.15)
                    }
                } else {
                    (base_padding, base_padding)
                };

                // Line X position in physical pixels
                // shaped.width is in logical pixels, multiply by scale
                let line_x = if line.is_duet {
                    viewport_width - cached.main.width * scale - padding_right
                } else {
                    padding_left
                };

                // Debug logging for first line only
                let should_log_debug = self.font_config.debug_logging && line_idx == 0;

                // Add glyphs for main text using pre-shaped data
                for glyph in &cached.main.glyphs {
                    if all_vertices.len() / 4 >= MAX_GLYPHS {
                        break;
                    }
                    let glyph_info = match self.sdf_cache.get_glyph(queue, glyph.cache_key) {
                        Some(info) => info,
                        None => continue,
                    };

                    if glyph_info.width == 0 || glyph_info.height == 0 {
                        continue;
                    }

                    // SDF metrics scaled to actual font size
                    let scaled_width = glyph_info.width as f32 * main_sdf_scale;
                    let scaled_height = glyph_info.height as f32 * main_sdf_scale;
                    let scaled_bearing_x = glyph_info.offset_x as f32 * main_sdf_scale;
                    let scaled_bearing_y = glyph_info.offset_y as f32 * main_sdf_scale;

                    // Glyph position: logical pixels * scale = physical pixels
                    let glyph_x = line_x + (glyph.x + glyph.x_offset_px) * scale + scaled_bearing_x;
                    let glyph_y = (glyph.y - glyph.y_offset_px) * scale - scaled_bearing_y;

                    if should_log_debug {
                        tracing::debug!(
                            "[build_from_shaped] glyph.x={:.2}, bearing_x={:.2}, glyph_x={:.2}",
                            glyph.x,
                            scaled_bearing_x,
                            glyph_x
                        );
                    }

                    let word = line.words.get(glyph.word_index);
                    let (word_start, word_end) = word
                        .map(|w| (w.start_ms as f32, w.end_ms as f32))
                        .unwrap_or((0.0, 0.0));

                    let (word_pixel_width, word_start_x) =
                        if glyph.word_index < cached.main.word_bounds.len() {
                            let (start, end) = cached.main.word_bounds[glyph.word_index];
                            (end - start, start)
                        } else {
                            (glyph.advance, glyph.x)
                        };

                    let emphasize = !is_non_dynamic
                        && word
                            .map(|w| w.emphasize || w.should_emphasize())
                            .unwrap_or(false);
                    let is_last_word = word.map(|w| w.is_last_word).unwrap_or(false);
                    let emphasis_amount = word.map(|w| w.emphasis_amount()).unwrap_or_default();

                    let emphasis_progress = if emphasize && word_end > word_start {
                        let progress = (current_time_ms - word_start) / (word_end - word_start);
                        progress.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    if emphasize && emphasis_progress > 0.0 {
                        let glow_intensity = emphasis_easing(emphasis_progress)
                            * word.map(|w| w.emphasis_blur()).unwrap_or_default();
                        if glow_intensity > 0.01 {
                            let glow_radius_em = word
                                .map(|w| (w.emphasis_blur() * 0.3).min(0.3))
                                .unwrap_or(0.0);
                            let emphasis_scale = 1.0 + 0.1 * emphasis_amount;
                            let glow_radius_px =
                                glow_radius_em * font_size * style.scale * emphasis_scale;
                            let glow_padding = glow_radius_px * 3.0 + 8.0;
                            glow_left = glow_left.min(glyph_x - glow_padding);
                            glow_top = glow_top.min(glyph_y - glow_padding);
                            glow_right = glow_right.max(glyph_x + scaled_width + glow_padding);
                            glow_bottom = glow_bottom.max(glyph_y + scaled_height + glow_padding);
                        }
                    }

                    let base_vertex = all_vertices.len() as u32;

                    let word_text = word.map(|w| &w.text).map(|t| t.as_str()).unwrap_or("");
                    let char_count = word_text.chars().count().max(1) as f32;
                    let char_index = (glyph.pos_in_word * char_count).floor();
                    let word_duration = word_end - word_start;
                    let word_delay = word_start;
                    let effective_duration = word_duration.max(1000.0);
                    let char_delay_offset = if char_count > 1.0 {
                        (effective_duration / 2.5 / char_count) * char_index
                    } else {
                        0.0
                    };
                    let char_delay_ms = word_delay + char_delay_offset;

                    let glyph_left_x = glyph.x;
                    let glyph_start_in_word = if word_pixel_width > 0.0 {
                        ((glyph_left_x - word_start_x) / word_pixel_width).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let glyph_width_ratio = if word_pixel_width > 0.0 {
                        (glyph.advance / word_pixel_width).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };

                    let mut base = LyricGlyphVertex {
                        pos_x: glyph_x,
                        pos_y: glyph_y,
                        width: scaled_width,
                        height: scaled_height,
                        uv_min: glyph_info.uv_min,
                        uv_max: glyph_info.uv_max,
                        word_start_ms: word_start,
                        word_end_ms: word_end,
                        glyph_start_in_word,
                        glyph_width_ratio,
                        line_index: line_idx as u32,
                        flags: 0,
                        color: 0xFFFFFFFF,
                        emphasis_progress,
                        corner_x: 0.0,
                        corner_y: 0.0,
                        char_index,
                        char_count,
                        char_delay_ms,
                        word_duration_ms: word_duration,
                        visual_line_info: (glyph.visual_line_index & 0xFFFF)
                            | ((glyph.visual_line_count & 0xFFFF) << 16),
                        pos_in_visual_line: glyph.pos_in_visual_line,
                    };

                    base.set_active(style.is_active);
                    base.set_emphasize(emphasize);
                    base.set_last_word(is_last_word);
                    base.set_non_dynamic(is_non_dynamic);
                    base.set_bg(line.is_bg);
                    base.set_duet(line.is_duet);

                    for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                        let mut v = base;
                        v.corner_x = cx;
                        v.corner_y = cy;
                        all_vertices.push(v);
                    }

                    all_indices.extend_from_slice(&[
                        base_vertex,
                        base_vertex + 1,
                        base_vertex + 2,
                        base_vertex,
                        base_vertex + 2,
                        base_vertex + 3,
                    ]);
                }

                // Add translation text using pre-shaped data
                if let Some(ref trans_shaped) = cached.translation {
                    let trans_y_offset = cached.main.height * scale;
                    let trans_x = if line.is_duet {
                        viewport_width - trans_shaped.width * scale - padding_right
                    } else {
                        padding_left
                    };

                    let trans_font_size = cached.translation_font_size * scale;
                    let trans_sdf_scale = trans_font_size / sdf_base_size;

                    self.add_shaped_glyphs_to_line(
                        queue,
                        &mut all_vertices,
                        &mut all_indices,
                        trans_shaped,
                        trans_x,
                        trans_y_offset,
                        line_idx,
                        &style,
                        true,
                        false,
                        is_non_dynamic,
                        trans_sdf_scale,
                        scale,
                    );
                }

                // Add romanized text using pre-shaped data
                if let Some(ref roman_shaped) = cached.romanized {
                    let trans_height = cached
                        .translation
                        .as_ref()
                        .map(|t| t.height * scale)
                        .unwrap_or(0.0);
                    let roman_y_offset = cached.main.height * scale + trans_height;
                    let roman_x = if line.is_duet {
                        viewport_width - roman_shaped.width * scale - padding_right
                    } else {
                        padding_left
                    };

                    let roman_font_size = cached.romanized_font_size * scale;
                    let roman_sdf_scale = roman_font_size / sdf_base_size;

                    self.add_shaped_glyphs_to_line(
                        queue,
                        &mut all_vertices,
                        &mut all_indices,
                        roman_shaped,
                        roman_x,
                        roman_y_offset,
                        line_idx,
                        &style,
                        false,
                        true,
                        is_non_dynamic,
                        roman_sdf_scale,
                        scale,
                    );
                }

                if glow_left.is_finite()
                    && glow_top.is_finite()
                    && glow_right.is_finite()
                    && glow_bottom.is_finite()
                {
                    glow_bounds = Some(GlowBounds {
                        left: glow_left,
                        top: glow_top,
                        width: (glow_right - glow_left).max(1.0),
                        height: (glow_bottom - glow_top).max(1.0),
                    });
                }
            }

            // 记录这行的索引范围
            let index_count = all_indices.len() as u32 - start_index;

            // 记录行渲染信息
            line_render_info.push(LineRenderInfo {
                line_index: line_idx,
                blur_level: style.blur,
                glow_blur_level,
                glow_bounds,
                y_position: style.y_position,
                height: line_height,
                visible,
                index_range: (start_index, index_count),
            });
        }

        // 更新缓存
        *self.cached_line_render_info.write() = line_render_info;

        (all_vertices, all_indices)
    }

    /// Add glyphs from pre-shaped line data (for translation/romanized)
    #[expect(
        clippy::too_many_arguments,
        reason = "glyph emission is a private renderer leaf over one shaped run; remove when ShapedGlyphRun owns the run metadata"
    )]
    fn add_shaped_glyphs_to_line(
        &mut self,
        queue: &Queue,
        vertices: &mut Vec<LyricGlyphVertex>,
        indices: &mut Vec<u32>,
        shaped: &ShapedLine,
        base_x: f32,   // Physical pixels
        y_offset: f32, // Physical pixels
        line_idx: usize,
        style: &ComputedLineStyle,
        is_translation: bool,
        is_romanized: bool,
        is_non_dynamic: bool,
        sdf_scale: f32, // SDF scale factor (font_size / 64.0)
        scale: f32,     // Logical to physical scale factor
    ) {
        for glyph in &shaped.glyphs {
            if vertices.len() / 4 >= MAX_GLYPHS {
                break;
            }
            let glyph_info = match self.sdf_cache.get_glyph(queue, glyph.cache_key) {
                Some(info) => info,
                None => continue,
            };

            if glyph_info.width == 0 || glyph_info.height == 0 {
                continue;
            }

            let scaled_width = glyph_info.width as f32 * sdf_scale;
            let scaled_height = glyph_info.height as f32 * sdf_scale;
            let scaled_bearing_x = glyph_info.offset_x as f32 * sdf_scale;
            let scaled_bearing_y = glyph_info.offset_y as f32 * sdf_scale;

            // Glyph position: logical pixels * scale = physical pixels
            let glyph_x = base_x + (glyph.x + glyph.x_offset_px) * scale + scaled_bearing_x;
            let glyph_y = y_offset + (glyph.y - glyph.y_offset_px) * scale - scaled_bearing_y;

            let base_vertex = vertices.len() as u32;

            let mut base = LyricGlyphVertex {
                pos_x: glyph_x,
                pos_y: glyph_y,
                width: scaled_width,
                height: scaled_height,
                uv_min: glyph_info.uv_min,
                uv_max: glyph_info.uv_max,
                word_start_ms: 0.0,
                word_end_ms: 0.0,
                glyph_start_in_word: 0.0,
                glyph_width_ratio: 1.0,
                line_index: line_idx as u32,
                flags: 0,
                color: 0xFFFFFFFF,
                emphasis_progress: 0.0,
                corner_x: 0.0,
                corner_y: 0.0,
                char_index: 0.0,
                char_count: 1.0,
                char_delay_ms: 0.0,
                word_duration_ms: 0.0,
                visual_line_info: (glyph.visual_line_index & 0xFFFF)
                    | ((glyph.visual_line_count & 0xFFFF) << 16),
                pos_in_visual_line: glyph.pos_in_visual_line,
            };

            base.set_active(style.is_active);
            base.set_translation(is_translation);
            base.set_romanized(is_romanized);
            base.set_non_dynamic(is_non_dynamic);

            for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                let mut v = base;
                v.corner_x = cx;
                v.corner_y = cy;
                vertices.push(v);
            }

            indices.extend_from_slice(&[
                base_vertex,
                base_vertex + 1,
                base_vertex + 2,
                base_vertex,
                base_vertex + 2,
                base_vertex + 3,
            ]);
        }
    }

    /// Update bind group with current atlas
    fn update_bind_group(&mut self, device: &Device) {
        // Atlas contents change in-place; the texture and bound buffers do not.
        if self.bind_group.is_some() {
            return;
        }
        let atlas_view = self.sdf_cache.atlas_view();

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SDF Glyph Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lyrics Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.global_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.line_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }));
    }

    /// Direct render to target (no blur)
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if self.index_count == 0 {
            return;
        }

        let Some(bind_group) = &self.bind_group else {
            return;
        };

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Prepare interlude dots for rendering
    pub fn prepare_interlude_dots(&mut self, preparation: InterludeDotsPreparation<'_>) {
        let InterludeDotsPreparation {
            device,
            queue,
            dots,
            viewport_size,
            bounds_origin,
            scale_factor,
            logical_font_size,
        } = preparation;
        self.dots_enabled = dots.enabled && dots.scale > 0.01;

        if !self.dots_enabled {
            return;
        }

        let dots_uniform = DotsUniform::from_interlude_dots(
            dots,
            viewport_size,
            bounds_origin,
            scale_factor,
            logical_font_size,
        );
        queue.write_buffer(
            &self.dots_uniform_buffer,
            0,
            bytemuck::bytes_of(&dots_uniform),
        );

        self.dots_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Interlude Dots Bind Group"),
            layout: &self.dots_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.dots_uniform_buffer.as_entire_binding(),
            }],
        }));
    }

    pub fn clear_interlude_dots(&mut self) {
        self.dots_enabled = false;
        self.dots_bind_group = None;
    }

    pub fn has_preparable_blur(&self) -> bool {
        self.cached_line_render_info.read().iter().any(|line| {
            line.visible
                && line.index_range.1 > 0
                && (line.blur_level >= 0.5
                    || (line.glow_blur_level >= 0.5 && line.glow_bounds.is_some()))
        })
    }

    pub fn clear_prepared_blur(&mut self) {
        self.per_line_blur.write().clear_prepared();
    }

    /// Render interlude dots
    pub fn render_interlude_dots<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if !self.dots_enabled {
            return;
        }

        let Some(bind_group) = &self.dots_bind_group else {
            return;
        };

        render_pass.set_pipeline(&self.dots_pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..4, 0..3);
    }

    /// Prepare blur rendering resources
    pub fn prepare_blur(&mut self, device: &Device, queue: &Queue) {
        let line_render_info = self.cached_line_render_info.read();
        let Some(globals) = *self.cached_global_uniform.read() else {
            self.per_line_blur.write().clear_prepared();
            return;
        };
        let line_uniforms = self.cached_line_uniforms.read();

        if line_render_info.is_empty() || line_uniforms.is_empty() {
            self.per_line_blur.write().clear_prepared();
            return;
        }

        let atlas_view = self.sdf_cache.atlas_view();
        self.per_line_blur
            .write()
            .prepare_lines(super::per_line_blur::LinePreparationInput {
                device,
                queue,
                lyrics_bind_group_layout: &self.bind_group_layout,
                atlas_view: &atlas_view,
                lines: &line_render_info,
                globals: &globals,
                line_uniforms: &line_uniforms,
            });
    }

    /// Render with per-line blur effect
    ///
    /// 逐行歌词渲染：
    /// 1. 每行歌词先离屏绘制成锐利文本
    /// 2. 对该行位图做横向 + 纵向 separable blur
    /// 3. 按从远到近的顺序合成回最终目标
    ///
    /// 这样 blur 只作用在歌词层本身，不会像整屏后处理那样把相邻行或背景混在一起。
    pub fn render_with_per_line_blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &iced::Rectangle<u32>,
        viewport_width: u32,
        viewport_height: u32,
    ) {
        if self.index_count == 0 {
            return;
        }

        // 更新逐行模糊渲染器的视口尺寸
        self.per_line_blur
            .write()
            .set_viewport_size(viewport_width, viewport_height);

        // 渲染间奏点（无模糊，保持在歌词后方）
        if self.dots_enabled {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Interlude Dots Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
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

            render_pass.set_scissor_rect(
                clip_bounds.x,
                clip_bounds.y,
                clip_bounds.width,
                clip_bounds.height,
            );

            self.render_interlude_dots(&mut render_pass);
        }

        // 使用逐行模糊渲染器
        self.per_line_blur
            .read()
            .render_prepared(PreparedRenderInput {
                encoder,
                target,
                clip_bounds,
                text_pipeline: &self.offscreen_pipeline,
                glow_pipeline: &self.glow_mask_pipeline,
                vertex_buffer: &self.vertex_buffer,
                index_buffer: &self.index_buffer,
            });
    }
}
