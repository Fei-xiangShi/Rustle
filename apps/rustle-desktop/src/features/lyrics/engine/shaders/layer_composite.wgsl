struct CompositeUniforms {
    target_size: vec2<f32>,
    dest_origin: vec2<f32>,
    dest_size: vec2<f32>,
    src_uv_min: vec2<f32>,
    src_uv_max: vec2<f32>,
    _padding: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: CompositeUniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = corners[vertex_index];
    let pixel_pos = uniforms.dest_origin + corner * uniforms.dest_size;
    let clip_x = (pixel_pos.x / uniforms.target_size.x) * 2.0 - 1.0;
    let clip_y = 1.0 - (pixel_pos.y / uniforms.target_size.y) * 2.0;

    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    out.uv = mix(uniforms.src_uv_min, uniforms.src_uv_max, corner);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(layer_texture, layer_sampler, in.uv);
    if color.a < 0.001 {
        discard;
    }
    return color;
}
