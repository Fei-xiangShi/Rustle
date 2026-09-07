//! Custom vertex structures for lyrics rendering
//!
//! These structures carry timing data to the GPU for word-by-word highlighting.
//! This is the key difference from glyphon - we can pass time information per-glyph.

use bytemuck::{Pod, Zeroable};
use iced::wgpu;

/// 单个字形的顶点数据，包含时间信息
///
/// 核心数据结构，支持逐字高亮
/// Each glyph knows its timing, allowing the shader to calculate highlight progress.
///
/// ## Features
///
/// - Per-character animation delay for wave effects
/// - Per-character X offset for emphasis wave
/// - Gradient mask parameters for smooth highlighting
/// - Support for translation and romanized text (via line_type flag)
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LyricGlyphVertex {
    // === Position (8 bytes) ===
    /// Screen position X (pixels)
    pub pos_x: f32,
    /// Screen position Y (pixels)
    pub pos_y: f32,

    // === Dimensions (8 bytes) ===
    /// Glyph width (pixels)
    pub width: f32,
    /// Glyph height (pixels)
    pub height: f32,

    // === UV coordinates (16 bytes) ===
    /// UV min in atlas
    pub uv_min: [f32; 2],
    /// UV max in atlas
    pub uv_max: [f32; 2],

    // === Timing (8 bytes) ===
    /// Word start time (milliseconds)
    pub word_start_ms: f32,
    /// Word end time (milliseconds)
    pub word_end_ms: f32,

    // === Word position (8 bytes) ===
    /// Glyph left edge position within word (0.0 = word start, 1.0 = word end)
    /// Used for pixel-level gradient mask calculation
    pub glyph_start_in_word: f32,
    /// Glyph width relative to word width (glyph_width / word_width)
    /// Used with local_x to calculate pixel position in word
    pub glyph_width_ratio: f32,

    // === Line info (8 bytes) ===
    /// Line index (for per-line effects)
    pub line_index: u32,
    /// Flags: bit 0 = is_active, bit 1 = emphasize, bit 2 = is_bg, bit 3 = is_duet
    ///        bit 4 = is_translation, bit 5 = is_romanized,
    ///        bit 6 = is_last_word, bit 7 = is_non_dynamic
    pub flags: u32,

    // === Visual properties (8 bytes) ===
    /// Base color (packed RGBA)
    pub color: u32,
    /// Emphasis progress (0.0-1.0, for glow/scale effects)
    pub emphasis_progress: f32,

    // === Corner info (8 bytes) ===
    /// Corner X (0.0 = left, 1.0 = right)
    pub corner_x: f32,
    /// Corner Y (0.0 = top, 1.0 = bottom)
    pub corner_y: f32,

    // === Per-character animation (16 bytes) ===
    /// Character index within word (for wave effect)
    pub char_index: f32,
    /// Total character count in word (for wave calculation)
    pub char_count: f32,
    /// Per-character delay offset in milliseconds (default: wordDe = de + (du / 2.5 / arr.length) * i)
    pub char_delay_ms: f32,
    /// Word duration in milliseconds (for emphasis calculations)
    pub word_duration_ms: f32,

    // === Visual line info for wrap highlight fix (8 bytes) ===
    /// Visual line info packed: lower 16 bits = visual_line_index, upper 16 bits = visual_line_count
    pub visual_line_info: u32,
    /// Position within visual line (0.0 = start, 1.0 = end)
    pub pos_in_visual_line: f32,
}

impl LyricGlyphVertex {
    /// Vertex buffer layout for wgpu
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // pos_x, pos_y
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // width, height
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // uv_min
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // uv_max
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // word_start_ms, word_end_ms
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // glyph_start_in_word, glyph_width_ratio
                wgpu::VertexAttribute {
                    offset: 40,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // line_index, flags
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Uint32x2,
                },
                // color, emphasis_progress
                wgpu::VertexAttribute {
                    offset: 56,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 60,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32,
                },
                // corner_x, corner_y
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // char_index, char_count
                wgpu::VertexAttribute {
                    offset: 72,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // char_delay_ms, word_duration_ms
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // visual_line_info, pos_in_visual_line
                wgpu::VertexAttribute {
                    offset: 88,
                    shader_location: 12,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 92,
                    shader_location: 13,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }

    /// Create vertex with default values
    pub fn new() -> Self {
        Self {
            pos_x: 0.0,
            pos_y: 0.0,
            width: 0.0,
            height: 0.0,
            uv_min: [0.0, 0.0],
            uv_max: [0.0, 0.0],
            word_start_ms: 0.0,
            word_end_ms: 0.0,
            glyph_start_in_word: 0.0,
            glyph_width_ratio: 0.0,
            line_index: 0,
            flags: 0,
            color: 0xFFFFFFFF,
            emphasis_progress: 0.0,
            corner_x: 0.0,
            corner_y: 0.0,
            char_index: 0.0,
            char_count: 1.0,
            char_delay_ms: 0.0,
            word_duration_ms: 0.0,
            visual_line_info: 0x00010000, // index=0, count=1
            pos_in_visual_line: 0.0,
        }
    }

    /// Set active flag
    pub fn set_active(&mut self, active: bool) {
        if active {
            self.flags |= 1;
        } else {
            self.flags &= !1;
        }
    }

    /// Set emphasize flag
    pub fn set_emphasize(&mut self, emphasize: bool) {
        if emphasize {
            self.flags |= 2;
        } else {
            self.flags &= !2;
        }
    }

    /// Set background line flag
    pub fn set_bg(&mut self, is_bg: bool) {
        if is_bg {
            self.flags |= 4;
        } else {
            self.flags &= !4;
        }
    }

    /// Set duet line flag
    pub fn set_duet(&mut self, is_duet: bool) {
        if is_duet {
            self.flags |= 8;
        } else {
            self.flags &= !8;
        }
    }

    /// Set translation line flag
    pub fn set_translation(&mut self, is_translation: bool) {
        if is_translation {
            self.flags |= 16;
        } else {
            self.flags &= !16;
        }
    }

    /// Set romanized line flag
    pub fn set_romanized(&mut self, is_romanized: bool) {
        if is_romanized {
            self.flags |= 32;
        } else {
            self.flags &= !32;
        }
    }

    /// Set last-word flag
    pub fn set_last_word(&mut self, is_last_word: bool) {
        if is_last_word {
            self.flags |= 64;
        } else {
            self.flags &= !64;
        }
    }

    /// Set non-dynamic lyrics flag
    pub fn set_non_dynamic(&mut self, is_non_dynamic: bool) {
        if is_non_dynamic {
            self.flags |= 128;
        } else {
            self.flags &= !128;
        }
    }
}

impl Default for LyricGlyphVertex {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-line uniform data
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineUniform {
    /// Y position (after scroll)
    pub y_position: f32,
    /// Scale factor (0.97 for inactive, 1.0 for active)
    pub scale: f32,
    /// Blur amount (0-32, style)
    pub blur: f32,
    /// Opacity (0-1)
    pub opacity: f32,
    /// Glow intensity
    pub glow: f32,
    /// Is active (1 or 0)
    pub is_active: u32,
    /// Line height
    pub line_height: f32,
    /// Independent AMLL mask alpha values
    pub bright_mask_alpha: f32,
    pub dark_mask_alpha: f32,
    /// Padding to keep a stable 48-byte storage stride
    pub _padding: [f32; 3],
}

impl Default for LineUniform {
    fn default() -> Self {
        Self {
            y_position: 0.0,
            scale: 1.0,
            blur: 0.0,
            opacity: 1.0,
            glow: 0.0,
            is_active: 0,
            line_height: 48.0,
            bright_mask_alpha: 0.2,
            dark_mask_alpha: 0.2,
            _padding: [0.0; 3],
        }
    }
}

/// Global uniforms for the lyrics shader
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GlobalUniform {
    /// Viewport size (full window size in physical pixels)
    pub viewport_size: [f32; 2],
    /// Bounds offset (widget position in window, physical pixels)
    pub bounds_offset: [f32; 2],
    /// Bounds size (widget size in physical pixels)
    pub bounds_size: [f32; 2],
    /// Current playback time (milliseconds)
    pub current_time_ms: f32,
    /// Word fade width (em units, default: 0.5)
    pub word_fade_width: f32,
    /// Base font size (pixels)
    pub font_size: f32,
    /// Scroll position
    pub scroll_y: f32,
    /// Alignment position (0.35 default)
    pub align_position: f32,
    /// SDF distance range in pixels (typically 4.0-8.0, used for distance extrapolation)
    pub sdf_range: f32,
}

impl Default for GlobalUniform {
    fn default() -> Self {
        Self {
            viewport_size: [800.0, 600.0],
            bounds_offset: [0.0, 0.0],
            bounds_size: [800.0, 600.0],
            current_time_ms: 0.0,
            word_fade_width: 0.5,
            font_size: 48.0,
            scroll_y: 0.0,
            align_position: 0.35,
            sdf_range: 4.0,
        }
    }
}
