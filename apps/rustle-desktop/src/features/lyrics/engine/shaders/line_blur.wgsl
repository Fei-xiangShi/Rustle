struct BlurUniforms {
    texture_size_and_direction: vec4<f32>,
    radius_and_padding: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> blur: BlurUniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );

    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );

    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texture_size = blur.texture_size_and_direction.xy;
    let direction = blur.texture_size_and_direction.zw;
    let radius = blur.radius_and_padding.x;
    let base = textureSample(source_texture, source_sampler, in.uv);
    if radius < 0.5 {
        return base;
    }

    // Offscreen line textures are stored with premultiplied alpha because the
    // text pass blends them over a transparent target first. Keep the blur pass
    // in premultiplied space to avoid dark fringes on glyph edges.
    //
    let sigma = max(radius, 0.5);
    let kernel_extent_px = max(1.0, sigma * 3.0);
    // 之前固定 15 tap，在大半径时采样点会被拉得很稀，容易出现网格和分层。
    // 改成 41 tap，并尽量把采样间距压到 1px 附近，超大半径时再逐步放大。
    let sample_step_px = max(1.0, kernel_extent_px / 20.0);
    let uv_step = direction / texture_size * sample_step_px;

    var premul = vec3<f32>(0.0);
    var alpha = 0.0;
    var total_weight = 0.0;

    for (var i = -20; i <= 20; i = i + 1) {
        let pixel_offset = f32(i) * sample_step_px;
        if abs(pixel_offset) > kernel_extent_px {
            continue;
        }
        let weight = exp(-(pixel_offset * pixel_offset) / (2.0 * sigma * sigma));
        let sample_color =
            textureSample(source_texture, source_sampler, in.uv + uv_step * f32(i));

        premul += sample_color.rgb * weight;
        alpha += sample_color.a * weight;
        total_weight += weight;
    }

    if total_weight > 0.0 {
        premul = premul / total_weight;
        alpha = alpha / total_weight;
    }

    if alpha < 0.001 {
        return vec4<f32>(0.0);
    }

    return vec4<f32>(premul, alpha);
}
