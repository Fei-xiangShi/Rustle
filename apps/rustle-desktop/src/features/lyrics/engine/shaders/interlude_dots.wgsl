// Interlude Dots Shader
//
// Renders three animated dots during instrumental interludes.
// Features:
// - Breathing scale animation
// - Sequential dot lighting
// - Smooth fade in/out
// - Configurable dot size and spacing

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) dot_index: u32,
};

struct DotsUniforms {
    // Position in physical pixels (relative to widget)
    position: vec2<f32>,
    // Overall scale (0.0 - 1.0, includes breathing animation)
    scale: f32,
    // Dot size in pixels
    dot_size: f32,
    // Dot spacing in pixels
    dot_spacing: f32,
    // Individual dot opacities (0.0 - 1.0)
    dot0_opacity: f32,
    dot1_opacity: f32,
    dot2_opacity: f32,
    // Whether dots are enabled
    enabled: f32,
    // Padding to align viewport_size
    _pad1: f32,
    // Viewport info
    viewport_size: vec2<f32>,
    bounds_offset: vec2<f32>,
    // Padding to align _padding to 16 bytes
    _pad2: vec2<f32>,
    // Final padding (vec4<f32>)
    _padding: vec4<f32>,
};

@group(0) @binding(0) var<uniform> dots: DotsUniforms;

// Generate vertices for a single dot quad
// vertex_index: 0-3 for each corner of the quad
// dot_index: 0-2 for which dot
@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32
) -> VertexOutput {
    var out: VertexOutput;
    
    // Skip if not enabled
    if dots.enabled < 0.5 {
        out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0); // Behind clip plane
        out.uv = vec2<f32>(0.0, 0.0);
        out.dot_index = 0u;
        return out;
    }
    
    let dot_index = instance_index;
    
    // position is the padded inner origin of the dot row.
    let dot_x =
        dots.position.x +
        (dots.dot_size * 0.5 + f32(dot_index) * (dots.dot_size + dots.dot_spacing)) * dots.scale;
    let dot_y = dots.position.y + dots.dot_size * 0.5 * dots.scale;
    
    // Calculate corner offset
    let corner_x = f32(vertex_index & 1u);
    let corner_y = f32((vertex_index >> 1u) & 1u);
    
    // Apply scale to dot size
    let scaled_size = dots.dot_size * dots.scale;
    
    // Calculate vertex position
    var pos_x = dot_x + (corner_x - 0.5) * scaled_size;
    var pos_y = dot_y + (corner_y - 0.5) * scaled_size;
    
    // Add bounds offset
    pos_x += dots.bounds_offset.x;
    pos_y += dots.bounds_offset.y;
    
    // Convert to clip space
    let clip_x = (pos_x / dots.viewport_size.x) * 2.0 - 1.0;
    let clip_y = 1.0 - (pos_y / dots.viewport_size.y) * 2.0;
    
    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    out.uv = vec2<f32>(corner_x, corner_y);
    out.dot_index = dot_index;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let centered = (in.uv - vec2<f32>(0.5, 0.5)) * 2.0;
    let dist = length(centered);
    let aa = max(fwidth(dist), 0.01);
    let circle_alpha = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, dist);
    
    // Get dot opacity based on index
    var dot_opacity: f32;
    switch in.dot_index {
        case 0u: { dot_opacity = dots.dot0_opacity; }
        case 1u: { dot_opacity = dots.dot1_opacity; }
        case 2u: { dot_opacity = dots.dot2_opacity; }
        default: { dot_opacity = 0.0; }
    }
    
    // Final alpha
    let alpha = circle_alpha * dot_opacity;
    
    if alpha < 0.01 {
        discard;
    }
    
    let color = vec3<f32>(1.0, 1.0, 1.0) * alpha;
    return vec4<f32>(color, alpha);
}
