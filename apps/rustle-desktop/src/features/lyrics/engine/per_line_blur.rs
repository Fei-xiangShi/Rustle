//! 逐行离屏模糊渲染器
//!
//! 真实的歌词模糊不应该直接在 SDF 边缘上做 softening，
//! 而应该把每一行歌词先渲染到独立纹理，再对该纹理做 blur，最后再合成回主目标。

use bytemuck::{Pod, Zeroable};
use iced::wgpu;
use iced::wgpu::util::DeviceExt;
use iced::wgpu::{Device, TextureFormat};

use super::vertex::{GlobalUniform, LineUniform};

pub const GLOW_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const GLOW_DOWNSAMPLE_FACTOR: f32 = 1.0;

struct BlurPassInput<'a> {
    device: &'a Device,
    layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    source_view: &'a wgpu::TextureView,
    texture_size: [u32; 2],
    direction: [f32; 2],
    radius: f32,
    label: &'a str,
}

struct BlurPassResourcesInput<'a> {
    device: &'a Device,
    layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    source_view: &'a wgpu::TextureView,
    scratch_view: &'a wgpu::TextureView,
    texture_size: [u32; 2],
    radius: f32,
    label_prefix: &'a str,
}

pub(super) struct PreparedRenderInput<'a> {
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub clip_bounds: &'a iced::Rectangle<u32>,
    pub text_pipeline: &'a wgpu::RenderPipeline,
    pub glow_pipeline: &'a wgpu::RenderPipeline,
    pub vertex_buffer: &'a wgpu::Buffer,
    pub index_buffer: &'a wgpu::Buffer,
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

/// 单行渲染纹理
struct LineTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    in_use: bool,
}

impl LineTexture {
    fn new(device: &Device, width: u32, height: u32, format: TextureFormat, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            width,
            height,
            in_use: false,
        }
    }

    fn matches_size(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }
}

/// 行渲染信息
#[derive(Debug, Clone)]
pub struct LineRenderInfo {
    pub line_index: usize,
    pub blur_level: f32,
    pub glow_blur_level: f32,
    pub glow_bounds: Option<GlowBounds>,
    pub y_position: f32,
    pub height: f32,
    pub visible: bool,
    pub index_range: (u32, u32),
}

#[derive(Debug, Clone, Copy)]
pub struct GlowBounds {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BlurPassUniform {
    texture_size_and_direction: [f32; 4],
    radius_and_padding: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LineCompositeUniform {
    target_size: [f32; 2],
    dest_origin: [f32; 2],
    dest_size: [f32; 2],
    src_uv_min: [f32; 2],
    src_uv_max: [f32; 2],
    _padding: [f32; 2],
}

struct PreparedLine {
    info: LineRenderInfo,
    texture_size: (u32, u32),
    source_texture: usize,
    scratch_texture: usize,
    _text_global_buffer: wgpu::Buffer,
    _text_line_buffer: wgpu::Buffer,
    text_bind_group: wgpu::BindGroup,
    _blur_horizontal_uniform: Option<wgpu::Buffer>,
    blur_horizontal_bind_group: Option<wgpu::BindGroup>,
    _blur_vertical_uniform: Option<wgpu::Buffer>,
    blur_vertical_bind_group: Option<wgpu::BindGroup>,
    _composite_uniform: wgpu::Buffer,
    composite_bind_group: wgpu::BindGroup,
    glow: Option<PreparedGlow>,
}

struct PreparedGlow {
    mask_texture: usize,
    blur_texture_a: usize,
    blur_texture_b: usize,
    full_size: (u32, u32),
    blur_size: (u32, u32),
    _global_buffer: wgpu::Buffer,
    _line_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _first_blur_uniform: wgpu::Buffer,
    first_blur_bind_group: wgpu::BindGroup,
    _second_blur_uniform: wgpu::Buffer,
    second_blur_bind_group: wgpu::BindGroup,
    _composite_uniform: wgpu::Buffer,
    composite_bind_group: wgpu::BindGroup,
}

/// 逐行模糊渲染器
pub struct PerLineBlurRenderer {
    texture_pool: Vec<LineTexture>,
    glow_texture_pool: Vec<LineTexture>,
    blur_pipeline: wgpu::RenderPipeline,
    glow_blur_pipeline: wgpu::RenderPipeline,
    blur_bind_group_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    composite_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: TextureFormat,
    viewport_size: (u32, u32),
    prepared_lines: Vec<PreparedLine>,
}

impl PerLineBlurRenderer {
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Per-Line Blur Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let blur_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Per-Line Blur Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Per-Line Blur Pipeline Layout"),
            bind_group_layouts: &[Some(&blur_bind_group_layout)],
            immediate_size: 0,
        });

        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Per-Line Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/line_blur.wgsl").into()),
        });

        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Per-Line Blur Pipeline"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let glow_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Per-Line Glow Blur Pipeline"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: GLOW_TEXTURE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let composite_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Per-Line Composite Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Per-Line Composite Pipeline Layout"),
                bind_group_layouts: &[Some(&composite_bind_group_layout)],
                immediate_size: 0,
            });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Per-Line Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/layer_composite.wgsl").into()),
        });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Per-Line Composite Pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Line textures stay in premultiplied space, but the final composite back
                    // to the lyrics layer uses additive color to approximate plus-lighter.
                    blend: Some(lyrics_plus_lighter_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            texture_pool: Vec::new(),
            glow_texture_pool: Vec::new(),
            blur_pipeline,
            glow_blur_pipeline,
            blur_bind_group_layout,
            composite_pipeline,
            composite_bind_group_layout,
            sampler,
            format,
            viewport_size: (0, 0),
            prepared_lines: Vec::new(),
        }
    }

    fn acquire_texture(
        pool: &mut Vec<LineTexture>,
        device: &Device,
        width: u32,
        height: u32,
        format: TextureFormat,
        label_prefix: &str,
    ) -> usize {
        for (i, tex) in pool.iter_mut().enumerate() {
            if !tex.in_use && tex.matches_size(width, height) {
                tex.in_use = true;
                return i;
            }
        }

        for (i, tex) in pool.iter_mut().enumerate() {
            if !tex.in_use {
                *tex = LineTexture::new(
                    device,
                    width,
                    height,
                    format,
                    &format!("{} {}", label_prefix, i),
                );
                tex.in_use = true;
                return i;
            }
        }

        let idx = pool.len();
        let mut tex = LineTexture::new(
            device,
            width,
            height,
            format,
            &format!("{} {}", label_prefix, idx),
        );
        tex.in_use = true;
        pool.push(tex);
        idx
    }

    fn texture_view_from_pool(pool: &[LineTexture], index: usize) -> Option<&wgpu::TextureView> {
        pool.get(index).map(|texture| &texture.view)
    }

    fn texture_view(&self, index: usize) -> Option<&wgpu::TextureView> {
        Self::texture_view_from_pool(&self.texture_pool, index)
    }

    fn glow_texture_view(&self, index: usize) -> Option<&wgpu::TextureView> {
        Self::texture_view_from_pool(&self.glow_texture_pool, index)
    }

    fn release_pool_textures(pool: &mut Vec<LineTexture>) {
        for tex in pool {
            tex.in_use = false;
        }
    }

    fn release_all_textures(&mut self) {
        Self::release_pool_textures(&mut self.texture_pool);
        Self::release_pool_textures(&mut self.glow_texture_pool);
    }

    pub fn clear_prepared(&mut self) {
        self.prepared_lines.clear();
        self.release_all_textures();
    }

    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.viewport_size = (width, height);
    }

    fn create_blur_pass_bind_group(input: BlurPassInput<'_>) -> (wgpu::Buffer, wgpu::BindGroup) {
        let BlurPassInput {
            device,
            layout,
            sampler,
            source_view,
            texture_size: [texture_width, texture_height],
            direction,
            radius,
            label,
        } = input;
        let uniform = BlurPassUniform {
            texture_size_and_direction: [
                texture_width as f32,
                texture_height as f32,
                direction[0],
                direction[1],
            ],
            radius_and_padding: [radius, 0.0, 0.0, 0.0],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        (buffer, bind_group)
    }

    fn create_blur_pass_resources(
        input: BlurPassResourcesInput<'_>,
    ) -> (
        Option<wgpu::Buffer>,
        Option<wgpu::BindGroup>,
        Option<wgpu::Buffer>,
        Option<wgpu::BindGroup>,
    ) {
        let BlurPassResourcesInput {
            device,
            layout,
            sampler,
            source_view,
            scratch_view,
            texture_size,
            radius,
            label_prefix,
        } = input;
        if radius < 0.5 {
            return (None, None, None, None);
        }

        let (horizontal_buffer, horizontal_bind_group) =
            Self::create_blur_pass_bind_group(BlurPassInput {
                device,
                layout,
                sampler,
                source_view,
                texture_size,
                direction: [1.0, 0.0],
                radius,
                label: &format!("{label_prefix} Blur Horizontal Bind Group"),
            });

        let (vertical_buffer, vertical_bind_group) =
            Self::create_blur_pass_bind_group(BlurPassInput {
                device,
                layout,
                sampler,
                source_view: scratch_view,
                texture_size,
                direction: [0.0, 1.0],
                radius,
                label: &format!("{label_prefix} Blur Vertical Bind Group"),
            });

        (
            Some(horizontal_buffer),
            Some(horizontal_bind_group),
            Some(vertical_buffer),
            Some(vertical_bind_group),
        )
    }

    pub fn prepare_lines(
        &mut self,
        device: &Device,
        lyrics_bind_group_layout: &wgpu::BindGroupLayout,
        atlas_view: &wgpu::TextureView,
        lines: &[LineRenderInfo],
        globals: &GlobalUniform,
        line_uniforms: &[LineUniform],
    ) {
        self.clear_prepared();

        let line_texture_width = globals.bounds_size[0].max(1.0).ceil() as u32;
        for line in lines {
            if !line.visible || line.index_range.1 == 0 || line.line_index >= line_uniforms.len() {
                continue;
            }

            let text_blur_margin = ((line.blur_level.max(0.0) * 3.0) + 6.0).ceil();
            let line_texture_height = (line.height + text_blur_margin * 2.0).max(1.0).ceil() as u32;
            let source_texture = Self::acquire_texture(
                &mut self.texture_pool,
                device,
                line_texture_width,
                line_texture_height,
                self.format,
                "Per-Line Source Texture",
            );
            let scratch_texture = Self::acquire_texture(
                &mut self.texture_pool,
                device,
                line_texture_width,
                line_texture_height,
                self.format,
                "Per-Line Scratch Texture",
            );

            let glow_setup = if line.glow_blur_level >= 0.5 {
                line.glow_bounds.map(|bounds| {
                    let glow_padding = ((line.glow_blur_level * 3.0) + 6.0).ceil();
                    let full_width = (bounds.width + glow_padding * 2.0).max(1.0).ceil() as u32;
                    let full_height = (bounds.height + glow_padding * 2.0).max(1.0).ceil() as u32;
                    let blur_width =
                        ((full_width as f32 * GLOW_DOWNSAMPLE_FACTOR).ceil()).max(1.0) as u32;
                    let blur_height =
                        ((full_height as f32 * GLOW_DOWNSAMPLE_FACTOR).ceil()).max(1.0) as u32;
                    let mask_texture = Self::acquire_texture(
                        &mut self.glow_texture_pool,
                        device,
                        full_width,
                        full_height,
                        GLOW_TEXTURE_FORMAT,
                        "Per-Line Glow Mask Texture",
                    );
                    let blur_texture_a = Self::acquire_texture(
                        &mut self.glow_texture_pool,
                        device,
                        blur_width,
                        blur_height,
                        GLOW_TEXTURE_FORMAT,
                        "Per-Line Glow Blur Texture A",
                    );
                    let blur_texture_b = Self::acquire_texture(
                        &mut self.glow_texture_pool,
                        device,
                        blur_width,
                        blur_height,
                        GLOW_TEXTURE_FORMAT,
                        "Per-Line Glow Blur Texture B",
                    );

                    (
                        bounds,
                        glow_padding,
                        mask_texture,
                        blur_texture_a,
                        blur_texture_b,
                        full_width,
                        full_height,
                        blur_width,
                        blur_height,
                    )
                })
            } else {
                None
            };

            let mut local_globals = *globals;
            local_globals.viewport_size = [globals.bounds_size[0], line_texture_height as f32];
            local_globals.bounds_offset = [0.0, 0.0];
            local_globals.bounds_size = [globals.bounds_size[0], line_texture_height as f32];

            let mut local_line_uniforms = line_uniforms.to_vec();
            if let Some(uniform) = local_line_uniforms.get_mut(line.line_index) {
                uniform.y_position = text_blur_margin;
                uniform.blur = 0.0;
            }

            let text_global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Per-Line Text Global Buffer"),
                contents: bytemuck::bytes_of(&local_globals),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let text_line_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Per-Line Text Line Buffer"),
                contents: bytemuck::cast_slice(&local_line_uniforms),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });

            let text_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Per-Line Text Bind Group"),
                layout: lyrics_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: text_global_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: text_line_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(atlas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            let source_view = match self.texture_view(source_texture) {
                Some(view) => view,
                None => continue,
            };
            let scratch_view = match self.texture_view(scratch_texture) {
                Some(view) => view,
                None => continue,
            };

            let (
                blur_horizontal_uniform,
                blur_horizontal_bind_group,
                blur_vertical_uniform,
                blur_vertical_bind_group,
            ) = Self::create_blur_pass_resources(BlurPassResourcesInput {
                device,
                layout: &self.blur_bind_group_layout,
                sampler: &self.sampler,
                source_view,
                scratch_view,
                texture_size: [line_texture_width, line_texture_height],
                radius: line.blur_level,
                label_prefix: "Per-Line Text",
            });

            let composite_uniform = LineCompositeUniform {
                target_size: globals.viewport_size,
                dest_origin: [
                    globals.bounds_offset[0],
                    globals.bounds_offset[1] + line.y_position - text_blur_margin,
                ],
                dest_size: [globals.bounds_size[0], line_texture_height as f32],
                src_uv_min: [0.0, 0.0],
                src_uv_max: [1.0, 1.0],
                _padding: [0.0, 0.0],
            };
            let composite_uniform_buffer =
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Per-Line Composite Uniform"),
                    contents: bytemuck::bytes_of(&composite_uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
            let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Per-Line Composite Bind Group"),
                layout: &self.composite_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: composite_uniform_buffer.as_entire_binding(),
                    },
                ],
            });

            let glow = glow_setup.and_then(
                |(
                    bounds,
                    glow_padding,
                    mask_texture,
                    blur_texture_a,
                    blur_texture_b,
                    full_width,
                    full_height,
                    blur_width,
                    blur_height,
                )| {
                    let mask_view = self.glow_texture_view(mask_texture)?;
                    let blur_view_a = self.glow_texture_view(blur_texture_a)?;
                    let blur_view_b = self.glow_texture_view(blur_texture_b)?;

                    let mut glow_globals = *globals;
                    glow_globals.viewport_size = [full_width as f32, full_height as f32];
                    glow_globals.bounds_offset = [glow_padding - bounds.left, 0.0];
                    glow_globals.bounds_size = [full_width as f32, full_height as f32];

                    let mut glow_line_uniforms = line_uniforms.to_vec();
                    if let Some(uniform) = glow_line_uniforms.get_mut(line.line_index) {
                        uniform.y_position = glow_padding - bounds.top;
                        uniform.blur = 0.0;
                    }

                    let global_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Per-Line Glow Global Buffer"),
                            contents: bytemuck::bytes_of(&glow_globals),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        });
                    let line_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Per-Line Glow Line Buffer"),
                            contents: bytemuck::cast_slice(&glow_line_uniforms),
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        });
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Per-Line Glow Bind Group"),
                        layout: lyrics_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: global_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: line_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(atlas_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                        ],
                    });

                    let (first_blur_uniform, first_blur_bind_group) =
                        Self::create_blur_pass_bind_group(BlurPassInput {
                            device,
                            layout: &self.blur_bind_group_layout,
                            sampler: &self.sampler,
                            source_view: mask_view,
                            texture_size: [full_width, full_height],
                            direction: [1.0, 0.0],
                            radius: line.glow_blur_level,
                            label: "Per-Line Glow Blur Horizontal Bind Group",
                        });
                    let (second_blur_uniform, second_blur_bind_group) =
                        Self::create_blur_pass_bind_group(BlurPassInput {
                            device,
                            layout: &self.blur_bind_group_layout,
                            sampler: &self.sampler,
                            source_view: blur_view_a,
                            texture_size: [blur_width, blur_height],
                            direction: [0.0, 1.0],
                            radius: line.glow_blur_level * GLOW_DOWNSAMPLE_FACTOR,
                            label: "Per-Line Glow Blur Vertical Bind Group",
                        });

                    let composite_uniform = LineCompositeUniform {
                        target_size: globals.viewport_size,
                        dest_origin: [
                            globals.bounds_offset[0] + bounds.left - glow_padding,
                            globals.bounds_offset[1] + line.y_position + bounds.top - glow_padding,
                        ],
                        dest_size: [full_width as f32, full_height as f32],
                        src_uv_min: [0.0, 0.0],
                        src_uv_max: [1.0, 1.0],
                        _padding: [0.0, 0.0],
                    };
                    let composite_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Per-Line Glow Composite Uniform"),
                            contents: bytemuck::bytes_of(&composite_uniform),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        });
                    let composite_bind_group =
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("Per-Line Glow Composite Bind Group"),
                            layout: &self.composite_bind_group_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(blur_view_b),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: composite_uniform_buffer.as_entire_binding(),
                                },
                            ],
                        });

                    Some(PreparedGlow {
                        mask_texture,
                        blur_texture_a,
                        blur_texture_b,
                        full_size: (full_width, full_height),
                        blur_size: (blur_width, blur_height),
                        _global_buffer: global_buffer,
                        _line_buffer: line_buffer,
                        bind_group,
                        _first_blur_uniform: first_blur_uniform,
                        first_blur_bind_group,
                        _second_blur_uniform: second_blur_uniform,
                        second_blur_bind_group,
                        _composite_uniform: composite_uniform_buffer,
                        composite_bind_group,
                    })
                },
            );

            self.prepared_lines.push(PreparedLine {
                info: line.clone(),
                texture_size: (line_texture_width, line_texture_height),
                source_texture,
                scratch_texture,
                _text_global_buffer: text_global_buffer,
                _text_line_buffer: text_line_buffer,
                text_bind_group,
                _blur_horizontal_uniform: blur_horizontal_uniform,
                blur_horizontal_bind_group,
                _blur_vertical_uniform: blur_vertical_uniform,
                blur_vertical_bind_group,
                _composite_uniform: composite_uniform_buffer,
                composite_bind_group,
                glow,
            });
        }
    }

    pub(super) fn render_prepared(&self, input: PreparedRenderInput<'_>) {
        let PreparedRenderInput {
            encoder,
            target,
            clip_bounds,
            text_pipeline,
            glow_pipeline,
            vertex_buffer,
            index_buffer,
        } = input;
        if self.prepared_lines.is_empty() {
            return;
        }

        let mut sorted_lines: Vec<_> = self.prepared_lines.iter().collect();
        sorted_lines.sort_by(|a, b| {
            b.info
                .blur_level
                .max(b.info.glow_blur_level)
                .partial_cmp(&a.info.blur_level.max(a.info.glow_blur_level))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for line in sorted_lines {
            let Some(source_view) = self.texture_view(line.source_texture) else {
                continue;
            };
            let Some(scratch_view) = self.texture_view(line.scratch_texture) else {
                continue;
            };

            if let Some(glow) = &line.glow {
                let Some(mask_view) = self.glow_texture_view(glow.mask_texture) else {
                    continue;
                };
                let Some(blur_view_a) = self.glow_texture_view(glow.blur_texture_a) else {
                    continue;
                };
                let Some(blur_view_b) = self.glow_texture_view(glow.blur_texture_b) else {
                    continue;
                };

                let mut glow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Per-Line Glow Mask Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: mask_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                glow_pass.set_scissor_rect(0, 0, glow.full_size.0, glow.full_size.1);
                glow_pass.set_pipeline(glow_pipeline);
                glow_pass.set_bind_group(0, &glow.bind_group, &[]);
                glow_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                glow_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let (start, count) = line.info.index_range;
                glow_pass.draw_indexed(start..(start + count), 0, 0..1);
                drop(glow_pass);

                {
                    let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Per-Line Glow Blur Horizontal Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: blur_view_a,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    blur_pass.set_scissor_rect(0, 0, glow.blur_size.0, glow.blur_size.1);
                    blur_pass.set_pipeline(&self.glow_blur_pipeline);
                    blur_pass.set_bind_group(0, &glow.first_blur_bind_group, &[]);
                    blur_pass.draw(0..3, 0..1);
                    drop(blur_pass);
                }

                {
                    let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Per-Line Glow Blur Vertical Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: blur_view_b,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    blur_pass.set_scissor_rect(0, 0, glow.blur_size.0, glow.blur_size.1);
                    blur_pass.set_pipeline(&self.glow_blur_pipeline);
                    blur_pass.set_bind_group(0, &glow.second_blur_bind_group, &[]);
                    blur_pass.draw(0..3, 0..1);
                    drop(blur_pass);
                }

                let mut composite_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Per-Line Glow Composite Pass"),
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
                composite_pass.set_scissor_rect(
                    clip_bounds.x,
                    clip_bounds.y,
                    clip_bounds.width,
                    clip_bounds.height,
                );
                composite_pass.set_pipeline(&self.composite_pipeline);
                composite_pass.set_bind_group(0, &glow.composite_bind_group, &[]);
                composite_pass.draw(0..6, 0..1);
                drop(composite_pass);
            }

            let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Per-Line Text Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: source_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            text_pass.set_scissor_rect(0, 0, line.texture_size.0, line.texture_size.1);
            text_pass.set_pipeline(text_pipeline);
            text_pass.set_bind_group(0, &line.text_bind_group, &[]);
            text_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            text_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            let (start, count) = line.info.index_range;
            text_pass.draw_indexed(start..(start + count), 0, 0..1);
            drop(text_pass);

            if let Some(horizontal_bind_group) = &line.blur_horizontal_bind_group {
                let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Per-Line Blur Horizontal Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: scratch_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                blur_pass.set_scissor_rect(0, 0, line.texture_size.0, line.texture_size.1);
                blur_pass.set_pipeline(&self.blur_pipeline);
                blur_pass.set_bind_group(0, horizontal_bind_group, &[]);
                blur_pass.draw(0..3, 0..1);
                drop(blur_pass);
            }

            if let Some(vertical_bind_group) = &line.blur_vertical_bind_group {
                let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Per-Line Blur Vertical Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: source_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                blur_pass.set_scissor_rect(0, 0, line.texture_size.0, line.texture_size.1);
                blur_pass.set_pipeline(&self.blur_pipeline);
                blur_pass.set_bind_group(0, vertical_bind_group, &[]);
                blur_pass.draw(0..3, 0..1);
                drop(blur_pass);
            }

            let mut composite_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Per-Line Composite Pass"),
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
            composite_pass.set_scissor_rect(
                clip_bounds.x,
                clip_bounds.y,
                clip_bounds.width,
                clip_bounds.height,
            );
            composite_pass.set_pipeline(&self.composite_pipeline);
            composite_pass.set_bind_group(0, &line.composite_bind_group, &[]);
            composite_pass.draw(0..6, 0..1);
            drop(composite_pass);
        }
    }
}
