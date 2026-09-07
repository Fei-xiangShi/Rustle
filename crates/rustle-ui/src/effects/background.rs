//! Animated background shader
//!
//! Implements lyrics background effects using WGPU:
//! - Mesh gradient with animated control points
//! - Gaussian blur on album artwork
//! - Vignette effect
//! - Gradient noise/dithering for banding reduction
//! - Time and volume-based flow animations

use bytemuck::{Pod, Zeroable};
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{Element, Length, Rectangle, mouse};

/// Uniform data passed to the background shader
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct BackgroundUniforms {
    /// Viewport resolution (width, height)
    pub resolution: [f32; 2],
    /// Current animation time
    pub time: f32,
    /// Audio volume for reactive effects (0.0-1.0)
    pub volume: f32,
    /// Primary color from album (RGB + alpha)
    pub color_primary: [f32; 4],
    /// Secondary color from album (RGB + alpha)
    pub color_secondary: [f32; 4],
    /// Tertiary color from album (RGB + alpha)
    pub color_tertiary: [f32; 4],
    /// Flow speed multiplier
    pub flow_speed: f32,
    /// Blur amount (0.0-1.0)
    pub blur_amount: f32,
    /// Vignette intensity (0.0-1.0)
    pub vignette_intensity: f32,
    /// Overall opacity (for fade transitions)
    pub opacity: f32,
}

impl Default for BackgroundUniforms {
    fn default() -> Self {
        Self {
            resolution: [1920.0, 1080.0],
            time: 0.0,
            volume: 1.0,
            color_primary: [0.15, 0.08, 0.25, 1.0],
            color_secondary: [0.08, 0.12, 0.20, 1.0],
            color_tertiary: [0.05, 0.05, 0.10, 1.0],
            flow_speed: 4.0,
            blur_amount: 0.8,
            vignette_intensity: 0.6,
            opacity: 1.0,
        }
    }
}

/// WGSL shader source for the animated background
const BACKGROUND_SHADER: &str = r#"
struct Uniforms {
    resolution: vec2f,
    time: f32,
    volume: f32,
    color_primary: vec4f,
    color_secondary: vec4f,
    color_tertiary: vec4f,
    flow_speed: f32,
    blur_amount: f32,
    vignette_intensity: f32,
    opacity: f32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

// Vertex shader: generates a fullscreen triangle
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let uv = vec2f(
        f32((vertex_index << 1) & 2),
        f32(vertex_index & 2)
    );
    let position = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
    return VertexOut(position, uv);
}

// ============ 常量和函数 ============

// Gradient noise 常量
const INV_255: f32 = 1.0 / 255.0;
const HALF_INV_255: f32 = 0.5 / 255.0;
const GRADIENT_NOISE_A: f32 = 52.9829189;
const GRADIENT_NOISE_B: vec2f = vec2f(0.06711056, 0.00583715);

// Gradient noise for dithering (Jorge Jimenez's presentation)
// http://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare
fn gradient_noise(uv: vec2f) -> f32 {
    return fract(GRADIENT_NOISE_A * fract(dot(uv, GRADIENT_NOISE_B)));
}

// 旋转函数
fn rot(v: vec2f, angle: f32) -> vec2f {
    let s = sin(angle);
    let c = cos(angle);
    return vec2f(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Cubic Hermite 插值 (用于平滑颜色过渡)
fn hermite(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

// Quintic Hermite 插值 (更平滑)
fn quintic(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// ============ Mesh Gradient 控制点系统 ============

// 3x3 控制点网格颜色插值
// 使用 Bicubic Hermite 风格的平滑插值
fn mesh_color(uv: vec2f) -> vec3f {
    // 控制点颜色布局 (3x3 网格):
    // [primary]   [mix1]      [secondary]
    // [mix2]      [center]    [mix3]
    // [secondary] [mix4]      [tertiary]
    
    let c00 = uniforms.color_primary.rgb;
    let c20 = uniforms.color_secondary.rgb;
    let c02 = uniforms.color_secondary.rgb;
    let c22 = uniforms.color_tertiary.rgb;
    let c11 = mix(mix(c00, c20, 0.5), mix(c02, c22, 0.5), 0.5); // 中心点
    
    // 使用 Hermite 插值在控制点之间平滑过渡
    let tx = hermite(uv.x);
    let ty = hermite(uv.y);
    
    // 双线性插值 + Hermite 平滑
    let top = mix(c00, c20, tx);
    let mid = mix(mix(c00, c11, 0.5 + tx * 0.5), mix(c11, c20, tx * 0.5), tx);
    let bot = mix(c02, c22, tx);
    
    return mix(mix(top, mid, ty * 2.0), mix(mid, bot, (ty - 0.5) * 2.0), step(0.5, ty));
}

// ============ 渐变生成 ============

fn mesh_gradient(uv: vec2f, time: f32, volume: f32) -> vec3f {
    // 时间和音量效果
    let volume_effect = volume * 2.0;
    let time_volume = time * 0.001 + volume;  // 转换为秒级时间
    
    // UV 旋转和缩放
    let centered_uv = uv - vec2f(0.2);
    let rotated_uv = rot(centered_uv, time_volume * 2.0 * uniforms.flow_speed * 0.1);
    let scale_factor = max(0.001, 1.0 - volume_effect);
    let final_uv = rotated_uv * scale_factor + vec2f(0.5);
    
    // 从控制点网格获取颜色
    let base_color = mesh_color(clamp(final_uv, vec2f(0.0), vec2f(1.0)));
    
    // Alpha 和音量因子
    let alpha_volume_factor = uniforms.opacity * max(0.5, 1.0 - volume * 0.5);
    
    return base_color * alpha_volume_factor;
}

// Vignette 效果
fn vignette_effect(uv: vec2f) -> f32 {
    let dist = distance(uv, vec2f(0.5));
    let vignette = smoothstep(0.8, 0.3, dist);
    return 0.6 + vignette * 0.4;
}

// ============ Fragment Shader ============

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    let uv = in.uv;
    
    // 生成渐变
    var color = mesh_gradient(uv, uniforms.time, uniforms.volume);
    
    // 应用 dithering (减少色带)
    let dither = INV_255 * gradient_noise(in.position.xy) - HALF_INV_255;
    color += vec3f(dither);
    
    // 应用 vignette 效果
    let mask = vignette_effect(uv);
    color *= mask;
    
    return vec4f(color, uniforms.opacity);
}
"#;

/// WGPU pipeline for the background shader - implements iced's Pipeline trait
pub struct BackgroundPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    uniforms: BackgroundUniforms,
}

impl shader::Pipeline for BackgroundPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Background Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(BACKGROUND_SHADER)),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Background Uniform Buffer"),
            size: std::mem::size_of::<BackgroundUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Background Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Background Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Background Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Background Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
            uniform_buffer,
            bind_group,
            uniforms: BackgroundUniforms::default(),
        }
    }
}

impl BackgroundPipeline {
    fn update(&mut self, queue: &wgpu::Queue, uniforms: BackgroundUniforms) {
        self.uniforms = uniforms;
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }
}

/// Shader primitive for background rendering
#[derive(Debug, Clone)]
pub struct BackgroundPrimitive {
    uniforms: BackgroundUniforms,
}

impl BackgroundPrimitive {
    pub fn new(uniforms: BackgroundUniforms) -> Self {
        Self { uniforms }
    }
}

impl shader::Primitive for BackgroundPrimitive {
    type Pipeline = BackgroundPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        let mut uniforms = self.uniforms;
        uniforms.resolution = [bounds.width, bounds.height];
        pipeline.update(queue, uniforms);
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &pipeline.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

/// State for background shader interaction
#[derive(Debug, Default)]
pub struct BackgroundState;

/// Animated background shader program
#[derive(Debug, Clone)]
pub struct LyricsBackgroundProgram {
    uniforms: BackgroundUniforms,
}

impl LyricsBackgroundProgram {
    pub fn new() -> Self {
        Self {
            uniforms: BackgroundUniforms::default(),
        }
    }

    pub fn with_colors(
        mut self,
        primary: [f32; 4],
        secondary: [f32; 4],
        tertiary: [f32; 4],
    ) -> Self {
        self.uniforms.color_primary = primary;
        self.uniforms.color_secondary = secondary;
        self.uniforms.color_tertiary = tertiary;
        self
    }

    pub fn with_time(mut self, time: f32) -> Self {
        self.uniforms.time = time;
        self
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.uniforms.volume = volume.clamp(0.0, 1.0);
        self
    }

    pub fn with_flow_speed(mut self, speed: f32) -> Self {
        self.uniforms.flow_speed = speed;
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.uniforms.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn with_vignette(mut self, intensity: f32) -> Self {
        self.uniforms.vignette_intensity = intensity.clamp(0.0, 1.0);
        self
    }

    pub fn set_colors(&mut self, primary: [f32; 4], secondary: [f32; 4], tertiary: [f32; 4]) {
        self.uniforms.color_primary = primary;
        self.uniforms.color_secondary = secondary;
        self.uniforms.color_tertiary = tertiary;
    }

    pub fn set_time(&mut self, time: f32) {
        self.uniforms.time = time;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.uniforms.volume = volume.clamp(0.0, 1.0);
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.uniforms.opacity = opacity.clamp(0.0, 1.0);
    }
}

impl Default for LyricsBackgroundProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> shader::Program<Message> for LyricsBackgroundProgram {
    type State = BackgroundState;
    type Primitive = BackgroundPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        BackgroundPrimitive::new(self.uniforms)
    }
}

/// Widget for animated background
pub struct LyricsBackground<'a, Message> {
    program: &'a LyricsBackgroundProgram,
    width: Length,
    height: Length,
    _phantom: std::marker::PhantomData<Message>,
}

impl<'a, Message> LyricsBackground<'a, Message> {
    pub fn new(program: &'a LyricsBackgroundProgram) -> Self {
        Self {
            program,
            width: Length::Fill,
            height: Length::Fill,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<'a, Message: 'a> From<LyricsBackground<'a, Message>> for Element<'a, Message> {
    fn from(background: LyricsBackground<'a, Message>) -> Self {
        iced::widget::shader(background.program)
            .width(background.width)
            .height(background.height)
            .into()
    }
}

/// Helper to convert iced::Color to shader color array
pub fn color_to_array(color: iced::Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}
