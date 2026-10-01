//! Mesh Gradient 背景渲染器
//!
//! 完整实现 mesh-renderer 的效果:
//! - Bicubic Hermite Patch (BHP) 网格渲染
//! - 顶点颜色与纹理颜色混合
//! - UV 旋转和缩放动画
//! - Vignette 效果
//! - Gradient noise dithering
//! - 多 mesh 过渡动画 (新旧 mesh 淡入淡出)
//! - 音量平滑

use bytemuck::{Pod, Zeroable};
use iced::Rectangle;
use iced::advanced::graphics::Viewport;
use iced::wgpu;
use iced::widget::shader;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::image_processing::{ProcessedImage, process_image_for_background};
use super::mesh::{BhpMesh, ControlPointPreset, MeshVertex, choose_preset_or_random};

/// Mesh 顶点数据 (GPU 格式)
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct GpuVertex {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub uv: [f32; 2],
    pub _padding: f32,
}

impl From<&MeshVertex> for GpuVertex {
    fn from(v: &MeshVertex) -> Self {
        Self {
            position: v.position,
            color: v.color,
            uv: v.uv,
            _padding: 0.0,
        }
    }
}

/// Uniform 数据
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct MeshUniforms {
    pub time: f32,
    pub volume: f32,
    pub alpha: f32,
    pub aspect: f32,
}

impl Default for MeshUniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            volume: 0.0,
            alpha: 1.0,
            aspect: 1.0,
        }
    }
}

/// WGSL Shader - 完整实现 mesh.vert.glsl + mesh.frag.glsl
pub(super) const MESH_SHADER: &str = r#"
struct Uniforms {
    time: f32,
    volume: f32,
    alpha: f32,
    aspect: f32,
}

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) color: vec3f,
    @location(2) uv: vec2f,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) color: vec3f,
    @location(1) uv: vec2f,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var t_texture: texture_2d<f32>;
@group(0) @binding(2) var s_texture: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = in.color;
    out.uv = in.uv;
    
    // 一致: 根据 aspect ratio 调整顶点位置
    var pos = in.position;
    if (uniforms.aspect > 1.0) {
        pos.y = pos.y * uniforms.aspect;
    } else {
        pos.x = pos.x / uniforms.aspect;
    }
    out.clip_position = vec4f(pos, 0.0, 1.0);
    return out;
}

const INV_255: f32 = 1.0 / 255.0;
const HALF_INV_255: f32 = 0.5 / 255.0;
const GRADIENT_NOISE_A: f32 = 52.9829189;
const GRADIENT_NOISE_B: vec2f = vec2f(0.06711056, 0.00583715);

fn gradient_noise(uv: vec2f) -> f32 {
    return fract(GRADIENT_NOISE_A * fract(dot(uv, GRADIENT_NOISE_B)));
}

fn rot(v: vec2f, angle: f32) -> vec2f {
    let s = sin(angle);
    let c = cos(angle);
    return vec2f(c * v.x - s * v.y, s * v.x + c * v.y);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let volume_effect = uniforms.volume * 2.0;
    let time_volume = uniforms.time + uniforms.volume;
    
    let dither = INV_255 * gradient_noise(in.clip_position.xy) - HALF_INV_255;
    
    let centered_uv = in.uv - vec2f(0.2);
    let rotated_uv = rot(centered_uv, time_volume * 2.0);
    let scale_factor = max(0.001, 1.0 - volume_effect);
    let final_uv = rotated_uv * scale_factor + vec2f(0.5);
    
    var result = textureSample(t_texture, s_texture, clamp(final_uv, vec2f(0.0), vec2f(1.0)));
    
    // The sRGB texture is already decoded to linear RGB by textureSample.
    // Mesh white modulation is a linear identity; opacity is straight alpha.
    let volume_brightness = max(0.5, 1.0 - uniforms.volume * 0.5);
    let dist = distance(in.uv, vec2f(0.5));
    let mask = 0.6 + (1.0 - smoothstep(0.3, 0.8, dist)) * 0.4;
    return color_output(result.rgb * in.color * volume_brightness * mask,
                        result.a * uniforms.alpha, dither);
}
"#;

/// 单个 Mesh 状态 (用于过渡动画)
#[derive(Clone)]
pub struct MeshState {
    pub mesh: Arc<BhpMesh>,
    pub image: Arc<ProcessedImage>,
    pub texture_id: u64,
    pub alpha: f32,
}

impl std::fmt::Debug for MeshState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshState")
            .field("texture_id", &self.texture_id)
            .field("alpha", &self.alpha)
            .finish()
    }
}

/// 缓存的 GPU 资源
struct CachedMeshState {
    texture_id: u64,
    #[expect(
        dead_code,
        reason = "keeps the cached GPU texture alive with its bind group"
    )]
    texture: wgpu::Texture,
    #[expect(
        dead_code,
        reason = "keeps the cached GPU texture view alive with its bind group"
    )]
    texture_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

/// Mesh Gradient Pipeline
pub struct MeshGradientPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    cached_states: Vec<CachedMeshState>,
}

impl MeshGradientPipeline {
    /// 确保 mesh state 已缓存到 GPU，并更新 uniform
    fn ensure_state_cached(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &MeshState,
        uniforms: &MeshUniforms,
    ) -> usize {
        // 查找已缓存的状态
        if let Some(idx) = self
            .cached_states
            .iter()
            .position(|c| c.texture_id == state.texture_id)
        {
            // 更新 uniform
            queue.write_buffer(
                &self.cached_states[idx].uniform_buffer,
                0,
                bytemuck::bytes_of(uniforms),
            );
            return idx;
        }

        // 创建新的缓存状态
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Mesh Texture"),
            size: wgpu::Extent3d {
                width: state.image.width,
                height: state.image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            state.image.as_rgba(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * state.image.width),
                rows_per_image: Some(state.image.height),
            },
            wgpu::Extent3d {
                width: state.image.width,
                height: state.image.height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 每个 mesh state 有独立的 uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Uniform Buffer"),
            size: std::mem::size_of::<MeshUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mesh Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let vertices: Vec<GpuVertex> = state.mesh.vertices.iter().map(GpuVertex::from).collect();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Vertex Buffer"),
            size: (vertices.len() * std::mem::size_of::<GpuVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Index Buffer"),
            size: (state.mesh.indices.len() * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&state.mesh.indices));

        let cached = CachedMeshState {
            texture_id: state.texture_id,
            texture,
            texture_view,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
            index_count: state.mesh.indices.len() as u32,
        };

        self.cached_states.push(cached);
        self.cached_states.len() - 1
    }

    fn cleanup_unused(&mut self, active_ids: &[u64]) {
        self.cached_states
            .retain(|c| active_ids.contains(&c.texture_id));
    }
}

impl shader::Pipeline for MeshGradientPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mesh Gradient Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(super::color_shader(
                MESH_SHADER,
                format,
            ))),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mesh Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mesh Texture Sampler"),
            address_mode_u: wgpu::AddressMode::MirrorRepeat,
            address_mode_v: wgpu::AddressMode::MirrorRepeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 20,
                    shader_location: 2,
                },
            ],
        };

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mesh Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mesh Gradient Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            cached_states: Vec::new(),
        }
    }
}

/// Mesh Gradient Primitive (用于渲染)
#[derive(Debug, Clone)]
pub struct MeshGradientPrimitive {
    pub mesh_states: Vec<MeshState>,
    pub time: f32,
    pub volume: f32,
    pub aspect: f32,
}

impl shader::Primitive for MeshGradientPrimitive {
    type Pipeline = MeshGradientPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        // 确保所有 mesh state 都已缓存，并更新 uniform
        for state in &self.mesh_states {
            let uniforms = MeshUniforms {
                time: self.time,
                volume: self.volume,
                alpha: state.alpha,
                aspect: self.aspect,
            };
            pipeline.ensure_state_cached(device, queue, state, &uniforms);
        }

        // 清理不再使用的缓存
        let active_ids: Vec<u64> = self.mesh_states.iter().map(|s| s.texture_id).collect();
        pipeline.cleanup_unused(&active_ids);
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        if self.mesh_states.is_empty() {
            return false;
        }

        render_pass.set_pipeline(&pipeline.pipeline);

        // 渲染所有 mesh states (从旧到新)
        for state in &self.mesh_states {
            if let Some(cached) = pipeline
                .cached_states
                .iter()
                .find(|c| c.texture_id == state.texture_id)
            {
                render_pass.set_bind_group(0, &cached.bind_group, &[]);
                render_pass.set_vertex_buffer(0, cached.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(cached.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..cached.index_count, 0, 0..1);
            }
        }

        true
    }
}

// ============================================================================
// TexturedBackgroundProgram - 主程序接口
// ============================================================================

/// 全局纹理 ID 计数器
static TEXTURE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_texture_id() -> u64 {
    TEXTURE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// 选择控制点预设 (80% 使用预设，20% 随机生成)
/// 直接使用 mesh 模块的实现
fn choose_control_point_preset() -> ControlPointPreset {
    choose_preset_or_random()
}

/// 生成网格颜色 - 全部设为白色
///
/// 的实现方式：顶点颜色全部为白色 [1,1,1]，
/// 颜色完全来自纹理采样，这样可以利用 GPU 的双线性插值，
/// 避免顶点颜色线性插值产生的色块感。
fn extract_mesh_colors(
    _image: &ProcessedImage,
    cp_width: usize,
    cp_height: usize,
) -> Vec<[f32; 3]> {
    // 原版：顶点颜色默认为白色，不从图像采样
    // 颜色完全由纹理采样提供，利用 GPU 的双线性插值实现平滑过渡
    vec![[1.0, 1.0, 1.0]; cp_width * cp_height]
}

/// 默认背景色 - 用于没有封面时的默认背景
/// 使用柔和的蓝灰色渐变，减少紫色
const DEFAULT_BG_COLORS: [crate::color::Oklcha; 4] = [
    crate::color::Oklcha::new(0.36, 0.045, 185.0, 1.0),
    crate::color::Oklcha::new(0.40, 0.055, 210.0, 1.0),
    crate::color::Oklcha::new(0.28, 0.035, 230.0, 1.0),
    crate::color::Oklcha::new(0.23, 0.025, 185.0, 1.0),
];

/// Textured Background Program
///
/// 管理多个 mesh 状态，实现平滑过渡动画
pub struct TexturedBackgroundProgram {
    mesh_states: Vec<MeshState>,
    /// 当前缓存的图片路径，用于避免重复加载
    current_image_path: Option<PathBuf>,
    smoothed_volume: f32,
    target_volume: f32,
    time: f32,
    has_cover: bool,
    /// 默认背景状态（深色渐变）
    default_state: Option<MeshState>,
}

impl TexturedBackgroundProgram {
    pub fn new() -> Self {
        // 在初始化时就创建默认背景，避免每次 draw 都创建
        let default_state = Self::create_default_background(true);

        Self {
            mesh_states: Vec::new(),
            current_image_path: None,
            smoothed_volume: 0.0,
            target_volume: 0.0,
            time: 0.0,
            has_cover: false,
            default_state,
        }
    }

    /// 创建默认的深色渐变背景
    fn create_default_background(device_available: bool) -> Option<MeshState> {
        if !device_available {
            return None;
        }

        // 使用大纹理尺寸以获得完全平滑的渐变（512x512）
        let size = 512u32;
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);

        let corners = DEFAULT_BG_COLORS.map(|color| color.lab());
        for y in 0..size {
            for x in 0..size {
                // 使用双线性插值在四个角的颜色之间过渡
                let fx = x as f32 / (size - 1) as f32;
                let fy = y as f32 / (size - 1) as f32;

                // 四角颜色
                let tl = corners[0]; // 左上
                let tr = corners[1]; // 右上
                let bl = corners[2]; // 左下
                let br = corners[3]; // 右下

                // 使用平滑的 smoothstep 插值
                let sx = fx * fx * (3.0 - 2.0 * fx);
                let sy = fy * fy * (3.0 - 2.0 * fy);

                // 双线性插值
                let top = [
                    tl[0] * (1.0 - sx) + tr[0] * sx,
                    tl[1] * (1.0 - sx) + tr[1] * sx,
                    tl[2] * (1.0 - sx) + tr[2] * sx,
                ];
                let bottom = [
                    bl[0] * (1.0 - sx) + br[0] * sx,
                    bl[1] * (1.0 - sx) + br[1] * sx,
                    bl[2] * (1.0 - sx) + br[2] * sx,
                ];
                let color = [
                    top[0] * (1.0 - sy) + bottom[0] * sy,
                    top[1] * (1.0 - sy) + bottom[1] * sy,
                    top[2] * (1.0 - sy) + bottom[2] * sy,
                ];

                // Oklab interpolation above; encode only at the texture boundary.
                pixels.extend_from_slice(&crate::color::lab_color(color, 1.0).into_rgba8());
            }
        }

        let processed = ProcessedImage {
            data: pixels,
            width: size,
            height: size,
        };

        // 使用预设生成 mesh
        let preset = choose_control_point_preset();
        let colors = vec![[1.0, 1.0, 1.0]; preset.width * preset.height];
        let mesh = BhpMesh::from_preset(&preset, 20, &colors);

        Some(MeshState {
            mesh: Arc::new(mesh),
            image: Arc::new(processed),
            texture_id: next_texture_id(),
            alpha: 1.0,
        })
    }

    /// 设置专辑封面图像
    ///
    /// # Returns
    /// 返回 `true` 表示生成了新的 Mesh（需要重置时间或触发其他逻辑），
    /// 返回 `false` 表示复用了现有 Mesh。
    pub fn set_album_image(
        &mut self,
        image: image::DynamicImage,
        path_key: Option<PathBuf>,
    ) -> bool {
        // 如果路径一致，直接忽略，避免闪烁和性能浪费
        if let Some(ref path) = path_key
            && self.current_image_path.as_ref() == Some(path)
            && self.has_cover
        {
            tracing::debug!("Cache hit for background image: {:?}", path);
            return false;
        }

        let processed = process_image_for_background(&image, 32);
        // Different cover derivatives/paths can produce the same background.
        // Also reuse a still-retained texture after a transient clear; rebuilding
        // would randomize the mesh and restart the fade for identical pixels.
        if self.mesh_states.last().is_some_and(|state| {
            state.image.width == processed.width
                && state.image.height == processed.height
                && state.image.data == processed.data
        }) {
            self.current_image_path = path_key;
            self.has_cover = true;
            return false;
        }

        tracing::debug!("Creating background texture for changed image content");

        let preset = choose_control_point_preset();
        let colors = extract_mesh_colors(&processed, preset.width, preset.height);
        let mesh = BhpMesh::from_preset(&preset, 20, &colors);

        // 如果是第一次设置封面（没有旧状态），直接设置 alpha=1.0
        // 否则从 alpha=0.0 开始过渡
        let initial_alpha = if self.mesh_states.is_empty() {
            1.0
        } else {
            0.0
        };

        let new_state = MeshState {
            mesh: Arc::new(mesh),
            image: Arc::new(processed),
            texture_id: next_texture_id(),
            alpha: initial_alpha,
        };

        self.mesh_states.push(new_state);
        self.current_image_path = path_key;
        self.has_cover = true;
        true
    }

    /// Check if the given path matches the currently loaded image
    /// Used for async loading to avoid redundant loads
    pub fn is_same_image(&self, path: &std::path::Path) -> bool {
        self.current_image_path.as_deref() == Some(path) && self.has_cover
    }

    /// 清除封面（淡出）
    pub fn clear_cover(&mut self) {
        if self.has_cover {
            self.has_cover = false;
            self.current_image_path = None;
        }
    }

    /// 设置时间
    pub fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    /// 更新状态 (每帧调用)
    ///
    /// 与 的 onRedraw 逻辑保持一致：
    /// - 先更新 alpha 值
    /// - 当最新状态的 alpha 达到 1.0 时，清理旧状态
    /// - 当没有封面时，所有状态的 alpha 逐渐减少到 0
    ///
    /// 注意：时间由外部通过 set_time 设置，这里不再更新时间
    pub fn update(&mut self, delta: f32) {
        // 音量平滑 (一致: lerpFactor = min(1.0, delta / 100.0))
        let lerp_factor = (delta / 100.0).min(1.0);
        self.smoothed_volume += (self.target_volume - self.smoothed_volume) * lerp_factor;

        // Alpha 过渡 (一致: deltaFactor = delta / 500)
        let delta_factor = delta / 500.0;

        if let Some(latest) = self.mesh_states.last_mut()
            && self.has_cover
        {
            // 有封面时，最新状态的 alpha 逐渐增加到 1.0
            latest.alpha = (latest.alpha + delta_factor).min(1.0);
        }

        if self.has_cover {
            // 当最新状态的 alpha 达到 1.0 时，清理旧状态 (一致)
            if self
                .mesh_states
                .last()
                .map(|s| s.alpha >= 1.0)
                .unwrap_or(false)
                && self.mesh_states.len() > 1
            {
                // 保留最后一个，删除其他所有
                let last = self.mesh_states.pop().unwrap();
                self.mesh_states.clear();
                self.mesh_states.push(last);
            }
        } else {
            // 没有封面时，所有状态的 alpha 逐渐减少 (一致)
            // 从后往前遍历，方便删除
            for i in (0..self.mesh_states.len()).rev() {
                self.mesh_states[i].alpha = (self.mesh_states[i].alpha - delta_factor).max(0.0);
                if self.mesh_states[i].alpha <= 0.0 {
                    self.mesh_states.remove(i);
                }
            }
        }

        // 注意：时间由外部通过 set_time 设置，这里不再更新
        // 的 frameTime 是累加的，但我们使用绝对时间
    }

    /// 创建渲染 primitive
    pub fn primitive(&self, aspect: f32) -> MeshGradientPrimitive {
        // 如果没有封面且没有活跃的 mesh，使用默认背景
        let states = if self.mesh_states.is_empty() {
            self.default_state.iter().cloned().collect()
        } else {
            self.mesh_states.clone()
        };

        MeshGradientPrimitive {
            mesh_states: states,
            time: self.time,
            volume: self.smoothed_volume,
            aspect,
        }
    }
}

impl Default for TexturedBackgroundProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TexturedBackgroundProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TexturedBackgroundProgram")
            .field("mesh_states_count", &self.mesh_states.len())
            .field("time", &self.time)
            .field("has_cover", &self.has_cover)
            .finish()
    }
}

impl<Message> shader::Program<Message> for TexturedBackgroundProgram {
    type State = ();
    type Primitive = MeshGradientPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        self.primitive(bounds.width / bounds.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_background_reuses_texture_across_paths_and_transient_clear() {
        let mut program = TexturedBackgroundProgram::new();
        let cover = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([200, 40, 60, 255]),
        ));
        assert!(program.set_album_image(cover.clone(), Some("thumbnail".into())));
        let texture = program.mesh_states[0].texture_id;
        let mesh = program.mesh_states[0].mesh.clone();
        assert!(!program.set_album_image(cover.clone(), Some("hero".into())));
        assert_eq!(program.mesh_states.len(), 1);
        assert_eq!(program.mesh_states[0].texture_id, texture);
        assert!(Arc::ptr_eq(&mesh, &program.mesh_states[0].mesh));
        assert!(program.is_same_image(std::path::Path::new("hero")));

        program.clear_cover();
        program.update(100.0);
        let alpha = program.mesh_states[0].alpha;
        assert!(!program.set_album_image(cover, Some("refreshed".into())));
        assert_eq!(program.mesh_states[0].texture_id, texture);
        assert_eq!(program.mesh_states[0].alpha, alpha);
        assert!(program.has_cover);

        let changed = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([40, 200, 60, 255]),
        ));
        assert!(program.set_album_image(changed, Some("new-song".into())));
        assert_ne!(program.mesh_states.last().unwrap().texture_id, texture);
        assert_eq!(program.mesh_states.len(), 2);
    }
}
