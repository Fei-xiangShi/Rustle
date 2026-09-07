// 歌词合成 Shader
//
// 从模糊金字塔采样并合成最终输出。
// 实现真正的高斯模糊效果，而不是近似。
//
// ## 模糊金字塔级别
//
// - Level 0: 原始 (无模糊)
// - Level 1: ~2px 模糊
// - Level 2: ~4px 模糊
// - Level 3: ~8px 模糊
// - Level 4: ~16px 模糊
// - Level 5: ~32px 模糊
//
// ## 模糊行为
//
// blur_level 直接对应金字塔级别：
// - blur_level 0 → Level 0 (无模糊)
// - blur_level 1-2 → Level 1
// - blur_level 3-4 → Level 2
// - blur_level 5-8 → Level 3
// - blur_level 9-16 → Level 4
// - blur_level 17-32 → Level 5
//
// ## 强调辉光效果 (text-shadow)
//
// 使用 CSS text-shadow 实现强调辉光：
// text-shadow: 0 0 ${blur*0.3}em rgba(255,255,255,${glowLevel})
//
// 我们通过对模糊版本进行叠加来模拟这个效果。

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct CompositeUniforms {
    viewport_size: vec2<f32>,
    current_time_ms: f32,
    font_size: f32,
};

@group(0) @binding(0) var<uniform> uniforms: CompositeUniforms;
@group(0) @binding(1) var blur_level_0: texture_2d<f32>;  // 原始 (无模糊)
@group(0) @binding(2) var blur_level_1: texture_2d<f32>;  // ~2px
@group(0) @binding(3) var blur_level_2: texture_2d<f32>;  // ~4px
@group(0) @binding(4) var blur_level_3: texture_2d<f32>;  // ~8px
@group(0) @binding(5) var blur_level_4: texture_2d<f32>;  // ~16px
@group(0) @binding(6) var blur_level_5: texture_2d<f32>;  // ~32px
@group(0) @binding(7) var tex_sampler: sampler;
// blur_info 纹理: R=blur_level, G=glow_level, B=emphasis, A=alpha
@group(0) @binding(8) var blur_info_texture: texture_2d<f32>;

// 全屏三角形顶点着色器
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    
    // 生成覆盖整个屏幕的三角形
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0)
    );
    
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    
    return out;
}

// 从指定模糊级别采样
fn sample_blur_level(level: i32, uv: vec2<f32>) -> vec4<f32> {
    switch level {
        case 0: { return textureSample(blur_level_0, tex_sampler, uv); }
        case 1: { return textureSample(blur_level_1, tex_sampler, uv); }
        case 2: { return textureSample(blur_level_2, tex_sampler, uv); }
        case 3: { return textureSample(blur_level_3, tex_sampler, uv); }
        case 4: { return textureSample(blur_level_4, tex_sampler, uv); }
        default: { return textureSample(blur_level_5, tex_sampler, uv); }
    }
}

// 根据 的 blur_level (像素值) 映射到金字塔级别
// blur_level: 0-32 像素
// 金字塔级别: 0-5
fn blur_level_to_pyramid_level(blur_level: f32) -> f32 {
    // 的模糊级别是像素值 (0-32)
    // 我们的金字塔级别是 0-5
    // 使用对数映射来获得更自然的过渡
    if blur_level < 0.5 {
        return 0.0;
    }
    // log2(blur_level) 映射:
    // blur 1 → level ~0
    // blur 2 → level ~1
    // blur 4 → level ~2
    // blur 8 → level ~3
    // blur 16 → level ~4
    // blur 32 → level ~5
    let level = log2(blur_level);
    return clamp(level, 0.0, 5.0);
}

// 从模糊金字塔采样，支持级别之间的插值
fn sample_with_blur(uv: vec2<f32>, blur_level: f32) -> vec4<f32> {
    let pyramid_level = blur_level_to_pyramid_level(blur_level);
    
    let floor_level = i32(floor(pyramid_level));
    let ceil_level = min(floor_level + 1, 5);
    let fract_level = fract(pyramid_level);
    
    // 在两个级别之间插值
    let color_a = sample_blur_level(floor_level, uv);
    let color_b = sample_blur_level(ceil_level, uv);
    
    return mix(color_a, color_b, fract_level);
}

// 计算强调辉光效果
// Formula: text-shadow: 0 0 ${blur*0.3}em rgba(255,255,255,${glowLevel})
fn calculate_emphasis_glow(uv: vec2<f32>, glow_level: f32, font_size: f32) -> vec4<f32> {
    if glow_level < 0.01 {
        return vec4<f32>(0.0);
    }
    
    // 辉光模糊半径 = blur * 0.3 * font_size (em 转像素)
    // 使用较高的模糊级别来采样辉光
    let glow_blur = glow_level * 0.3 * font_size;
    let glow_color = sample_with_blur(uv, glow_blur);
    
    // 辉光颜色: 白色，透明度由 glow_level 控制
    return vec4<f32>(1.0, 1.0, 1.0, glow_color.a * glow_level);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 从 blur_info 纹理读取每像素的模糊信息
    let blur_info = textureSample(blur_info_texture, tex_sampler, in.uv);
    let blur_level = blur_info.r;      // 模糊级别 (0-32 像素)
    let glow_level = blur_info.g;      // 辉光级别 (0-1)
    let emphasis = blur_info.b;        // 强调进度 (0-1)
    let pixel_alpha = blur_info.a;     // 像素 alpha
    
    // 如果像素完全透明，直接返回
    if pixel_alpha < 0.01 {
        discard;
    }
    
    // 根据 blur_level 从金字塔采样
    // blur_level < 0.5 表示不需要模糊，直接使用原始纹理
    var color: vec4<f32>;
    if blur_level < 0.5 {
        // 无模糊 - 直接从 level 0 采样
        color = textureSample(blur_level_0, tex_sampler, in.uv);
    } else {
        // 有模糊 - 从对应的金字塔级别采样
        color = sample_with_blur(in.uv, blur_level);
    }
    
    // 添加强调辉光效果 (text-shadow)
    if glow_level > 0.01 {
        let glow = calculate_emphasis_glow(in.uv, glow_level, uniforms.font_size);
        // 辉光叠加 (additive blending)
        let glow_contribution = glow.rgb * glow.a;
        color = vec4<f32>(color.rgb + glow_contribution, color.a);
    }
    
    return color;
}

// 简化版本 - 不使用 blur_info 纹理，直接从 level 0 采样
// 用于调试或不需要模糊效果时
@fragment
fn fs_main_simple(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(blur_level_0, tex_sampler, in.uv);
    return color;
}
