// Lyrics SDF Shader
//
// 使用 MSDF (Multi-channel Signed Distance Field) 渲染歌词文本。
// 相比位图渲染的优势：
// - 任意缩放保持清晰
// - 单 pass 实现所有特效（模糊、发光、描边）
// - 更低的显存占用
// - 更好的性能
//
// ## 关键技术
//
// 1. MSDF 采样：使用 median(r, g, b) 获取距离值
// 2. 自动抗锯齿：使用 fwidth() 计算屏幕空间梯度
// 3. 外发光：使用 smoothstep 在距离场外部渲染
// 4. 卡拉OK擦除：结合 UV 坐标和时间进度

// === Canvas Expansion Constants (3σ Rule) ===
// 高斯模糊的"可见范围"通常是半径的 2~3 倍（3σ 法则）
// 为了保险起见，使用 3.0 倍，保证光晕能自然淡出到完全透明
const EXPANSION_FACTOR: f32 = 3.0;

// 额外 Padding 防止浮点误差导致的边缘裁剪
const PADDING_SAFE: f32 = 5.0;

// === Vertex Input ===
struct VertexInput {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_max: vec2<f32>,
    @location(4) word_time: vec2<f32>,
    @location(5) glyph_word_pos: vec2<f32>,
    @location(6) line_info: vec2<u32>,
    @location(7) color: u32,
    @location(8) emphasis: f32,
    @location(9) corner: vec2<f32>,
    @location(10) char_info: vec2<f32>,
    @location(11) char_timing: vec2<f32>,
    @location(12) visual_line_info: u32,
    @location(13) pos_in_visual_line: f32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) word_time: vec2<f32>,
    @location(2) glyph_word_pos: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) @interpolate(flat) flags: u32,
    @location(5) emphasis: f32,
    @location(6) @interpolate(flat) line_index: u32,
    @location(7) char_info: vec2<f32>,
    @location(8) local_x: f32,
    @location(9) char_timing: vec2<f32>,
    @location(10) float_progress: f32,
    @location(11) uv_bounds: vec4<f32>,  // 原始 UV 边界 (min_x, min_y, max_x, max_y)
    @location(12) screen_size: vec2<f32>,  // Glyph size in screen pixels (for distance extrapolation)
    @location(13) @interpolate(flat) total_expand: f32,  // 画布扩展量（像素），用于边缘衰减
    @location(14) @interpolate(flat) visual_line_info: u32,  // 视觉行信息 (packed)
    @location(15) pos_in_visual_line: f32,  // 在视觉行中的位置
};

// === Uniforms ===
struct GlobalUniforms {
    viewport_size: vec2<f32>,
    bounds_offset: vec2<f32>,
    bounds_size: vec2<f32>,
    current_time_ms: f32,
    word_fade_width: f32,
    font_size: f32,
    scroll_y: f32,
    align_position: f32,
    sdf_range: f32,  // SDF distance range in pixels (typically 4.0-8.0)
};

struct LineUniforms {
    y_position: f32,
    scale: f32,
    blur: f32,
    opacity: f32,
    glow: f32,
    is_active: u32,
    line_height: f32,
    bright_mask_alpha: f32,
    dark_mask_alpha: f32,
    _padding0: f32,
    _padding1: f32,
    _padding2: f32,
};

@group(0) @binding(0) var<uniform> globals: GlobalUniforms;
@group(0) @binding(1) var<storage, read> lines: array<LineUniforms>;
@group(0) @binding(2) var sdf_atlas: texture_2d<f32>;
@group(0) @binding(3) var sdf_sampler: sampler;

// === Helper Functions ===

fn unpack_color(packed: u32) -> vec4<f32> {
    return vec4<f32>(
        f32((packed >> 24u) & 0xFFu) / 255.0,
        f32((packed >> 16u) & 0xFFu) / 255.0,
        f32((packed >> 8u) & 0xFFu) / 255.0,
        f32(packed & 0xFFu) / 255.0
    );
}

// 解包视觉行信息
// 返回 vec2<u32>(visual_line_index, visual_line_count)
fn unpack_visual_line_info(info: u32) -> vec2<u32> {
    let index = info & 0xFFFFu;
    let count = (info >> 16u) & 0xFFFFu;
    return vec2<u32>(index, count);
}

// 计算全局视觉位置（考虑换行）
// 当一行歌词换行成多个视觉行时，高亮应该按视觉顺序进行
// 例如：2 个视觉行，第一行的字形 global_pos 在 [0, 0.5]，第二行在 [0.5, 1.0]
fn calculate_global_visual_pos(
    visual_line_index: u32,
    visual_line_count: u32,
    pos_in_visual_line: f32
) -> f32 {
    // 单行情况：直接返回行内位置
    if visual_line_count <= 1u {
        return pos_in_visual_line;
    }
    
    // 多行情况：每个视觉行占据 1/visual_line_count 的时间
    let line_fraction = 1.0 / f32(visual_line_count);
    let line_start = f32(visual_line_index) * line_fraction;
    
    return line_start + pos_in_visual_line * line_fraction;
}

fn is_active(flags: u32) -> bool { return (flags & 1u) != 0u; }
fn is_emphasize(flags: u32) -> bool { return (flags & 2u) != 0u; }
fn is_bg(flags: u32) -> bool { return (flags & 4u) != 0u; }
fn is_duet(flags: u32) -> bool { return (flags & 8u) != 0u; }
fn is_translation(flags: u32) -> bool { return (flags & 16u) != 0u; }
fn is_romanized(flags: u32) -> bool { return (flags & 32u) != 0u; }
fn is_last_word(flags: u32) -> bool { return (flags & 64u) != 0u; }
fn is_non_dynamic(flags: u32) -> bool { return (flags & 128u) != 0u; }

// SDF 采样函数（单通道 SDF，R=G=B）
fn median(r: f32, g: f32, b: f32) -> f32 {
    return r;
}

// SDF 距离外推采样（重新设计：使用径向距离消除方框感）
// 
// 核心问题：SDF 纹理边缘的像素形成方形等值线，导致方框感
// 解决方案：在边界外使用径向距离计算衰减，不依赖边缘 SDF 值
// 
// 参数：
// - uv: 可能超出边界的 UV 坐标
// - uv_bounds: 原始 UV 边界 (min_x, min_y, max_x, max_y)
// - screen_size: 字形在屏幕上的尺寸（像素）
// - total_expand: 画布扩展量（像素）
// 
// 返回：(sdf_distance, edge_fade_factor)
fn sample_sdf_with_edge_fade(
    uv: vec2<f32>,
    uv_bounds: vec4<f32>,
    screen_size: vec2<f32>,
    total_expand: f32
) -> vec2<f32> {
    let edge_threshold = 0.5;
    
    // 计算 UV 空间的边界尺寸
    let uv_size = uv_bounds.zw - uv_bounds.xy;
    
    // 将 UV 偏移转换为像素偏移的比例
    var px_per_uv = vec2<f32>(0.0);
    if uv_size.x > 0.0001 {
        px_per_uv.x = screen_size.x / uv_size.x;
    }
    if uv_size.y > 0.0001 {
        px_per_uv.y = screen_size.y / uv_size.y;
    }
    
    // 检查 UV 是否在边界内
    let is_inside = all(uv >= uv_bounds.xy) && all(uv <= uv_bounds.zw);
    
    if is_inside {
        // 在边界内：直接采样，不衰减
        let msdf = textureSample(sdf_atlas, sdf_sampler, uv).rgb;
        let sdf_dist = median(msdf.r, msdf.g, msdf.b) - edge_threshold;
        return vec2<f32>(sdf_dist, 1.0);
    }
    
    // === 在边界外：使用径向距离计算衰减 ===
    
    // 1. 计算 UV 到边界的最近点
    let clamped_uv = clamp(uv, uv_bounds.xy, uv_bounds.zw);
    
    // 2. 计算 UV 偏移量（超出边界的部分）
    let uv_offset = uv - clamped_uv;
    
    // 3. 转换为像素距离
    let pixel_offset = uv_offset * px_per_uv;
    let pixel_distance = length(pixel_offset);
    
    // 4. 采样边缘的 SDF 值（用于确定字形边缘位置）
    let msdf = textureSample(sdf_atlas, sdf_sampler, clamped_uv).rgb;
    let edge_sdf = median(msdf.r, msdf.g, msdf.b) - edge_threshold;
    
    // 5. 计算径向衰减（关键：使用圆形衰减而非方形）
    // 衰减公式：从边界开始，到 total_expand 处完全透明
    var radial_fade = 1.0;
    if total_expand > 0.001 {
        // 使用平滑的径向衰减
        // 从 0 开始衰减，到 total_expand * 0.9 处完全透明
        let fade_range = total_expand * 0.9;
        radial_fade = 1.0 - smoothstep(0.0, fade_range, pixel_distance);
    }
    
    // 6. 外推 SDF 距离
    // 使用径向距离来外推，确保圆形衰减
    var sdf_extrapolation = 0.0;
    if total_expand > 0.001 {
        // 外推速度与衰减范围匹配
        sdf_extrapolation = pixel_distance / (total_expand * 0.5);
    }
    let sdf_dist = edge_sdf - sdf_extrapolation;
    
    return vec2<f32>(sdf_dist, radial_fade);
}

// 自动抗锯齿 SDF 采样
// 使用 fwidth 自动计算屏幕空间梯度，实现完美的 1-2 像素边缘过渡
// edge_threshold: 0.5 = 标准边缘, 更高值 = 更细的笔画
fn sample_sdf_auto_aa(uv: vec2<f32>) -> f32 {
    let msdf = textureSample(sdf_atlas, sdf_sampler, uv).rgb;
    // 使用 0.5 作为标准边缘阈值
    // 如果笔画太粗，可以增加这个值（如 0.52-0.55）
    let edge_threshold = 0.5;
    let sig_dist = median(msdf.r, msdf.g, msdf.b) - edge_threshold;
    let screen_px_dist = sig_dist / fwidth(sig_dist);
    return clamp(screen_px_dist + 0.5, 0.0, 1.0);
}

fn pixel_offset_to_uv(offset_px: vec2<f32>, uv_bounds: vec4<f32>, screen_size: vec2<f32>) -> vec2<f32> {
    let uv_size = uv_bounds.zw - uv_bounds.xy;
    var uv_offset = vec2<f32>(0.0);
    if screen_size.x > 0.001 {
        uv_offset.x = offset_px.x * uv_size.x / screen_size.x;
    }
    if screen_size.y > 0.001 {
        uv_offset.y = offset_px.y * uv_size.y / screen_size.y;
    }
    return uv_offset;
}

fn sample_sdf_coverage_with_edge_fade(
    uv: vec2<f32>,
    uv_bounds: vec4<f32>,
    screen_size: vec2<f32>,
    total_expand: f32
) -> f32 {
    let edge_data = sample_sdf_with_edge_fade(uv, uv_bounds, screen_size, total_expand);
    let sdf_dist = edge_data.x;
    let edge_fade = edge_data.y;
    let sdf_gradient = max(fwidth(sdf_dist), 0.0001);
    let sdf_pixel_dist = sdf_dist / sdf_gradient;
    let coverage = smoothstep(-0.5, 0.5, sdf_pixel_dist);
    return coverage * edge_fade;
}

// 多采样 SDF 模糊采样
// blur_px: 模糊半径（像素），范围 0-32
// uv_bounds: 原始 UV 边界 (min_x, min_y, max_x, max_y)
// screen_size: 字形在屏幕上的尺寸（像素）
// total_expand: 画布扩展量（像素）
//
// 这里对字形 coverage 做多点采样近似高斯卷积。
// 它比简单的 edge softening 更接近真实 blur，同时能借助 edge_fade 抑制矩形边界。
fn sample_sdf_gaussian_blur_v2(uv: vec2<f32>, blur_px: f32, uv_bounds: vec4<f32>, screen_size: vec2<f32>, total_expand: f32) -> f32 {
    if blur_px < 0.5 {
        return sample_sdf_coverage_with_edge_fade(uv, uv_bounds, screen_size, total_expand);
    }

    let radius1 = max(0.75, blur_px * 0.45);
    let radius2 = max(radius1 + 0.75, blur_px * 0.9);
    let diag = radius1 * 0.70710678;

    let uv_x1 = pixel_offset_to_uv(vec2<f32>(radius1, 0.0), uv_bounds, screen_size);
    let uv_y1 = pixel_offset_to_uv(vec2<f32>(0.0, radius1), uv_bounds, screen_size);
    let uv_x2 = pixel_offset_to_uv(vec2<f32>(radius2, 0.0), uv_bounds, screen_size);
    let uv_y2 = pixel_offset_to_uv(vec2<f32>(0.0, radius2), uv_bounds, screen_size);
    let uv_d1 = pixel_offset_to_uv(vec2<f32>(diag, diag), uv_bounds, screen_size);
    let uv_d2 = pixel_offset_to_uv(vec2<f32>(diag, -diag), uv_bounds, screen_size);

    var alpha = 0.0;
    alpha += sample_sdf_coverage_with_edge_fade(uv, uv_bounds, screen_size, total_expand) * 0.20;

    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_x1, uv_bounds, screen_size, total_expand) * 0.14;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_x1, uv_bounds, screen_size, total_expand) * 0.14;
    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_y1, uv_bounds, screen_size, total_expand) * 0.14;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_y1, uv_bounds, screen_size, total_expand) * 0.14;

    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_d1, uv_bounds, screen_size, total_expand) * 0.05;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_d1, uv_bounds, screen_size, total_expand) * 0.05;
    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_d2, uv_bounds, screen_size, total_expand) * 0.05;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_d2, uv_bounds, screen_size, total_expand) * 0.05;

    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_x2, uv_bounds, screen_size, total_expand) * 0.02;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_x2, uv_bounds, screen_size, total_expand) * 0.02;
    alpha += sample_sdf_coverage_with_edge_fade(uv + uv_y2, uv_bounds, screen_size, total_expand) * 0.02;
    alpha += sample_sdf_coverage_with_edge_fade(uv - uv_y2, uv_bounds, screen_size, total_expand) * 0.02;

    return clamp(alpha, 0.0, 1.0);
}

fn gaussian_weight(distance_px: f32, sigma_px: f32) -> f32 {
    let safe_sigma = max(sigma_px, 0.001);
    let normalized = distance_px / safe_sigma;
    return exp(-0.5 * normalized * normalized);
}

// 外发光采样（使用更接近 CSS text-shadow 的高斯核）
// 之前的双环 8 向采样核太稀疏，半径一大就容易出现颗粒感和分层。
// 这里改为 3 个同心环 + 12 个方向的高斯权重近似，离浏览器的文字阴影更近。
fn sample_sdf_glow_v2(uv: vec2<f32>, glow_radius_px: f32, uv_bounds: vec4<f32>, screen_size: vec2<f32>, total_expand: f32) -> f32 {
    if glow_radius_px < 0.5 {
        return 0.0;
    }

    let base_coverage = sample_sdf_coverage_with_edge_fade(uv, uv_bounds, screen_size, total_expand);
    let sigma = max(glow_radius_px * 0.55, 0.75);
    let ring_radii = array<f32, 3>(
        max(0.65, glow_radius_px * 0.28),
        max(1.25, glow_radius_px * 0.58),
        max(2.0, glow_radius_px * 0.95),
    );
    let directions = array<vec2<f32>, 12>(
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.8660254, 0.5),
        vec2<f32>(0.5, 0.8660254),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(-0.5, 0.8660254),
        vec2<f32>(-0.8660254, 0.5),
        vec2<f32>(-1.0, 0.0),
        vec2<f32>(-0.8660254, -0.5),
        vec2<f32>(-0.5, -0.8660254),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(0.5, -0.8660254),
        vec2<f32>(0.8660254, -0.5),
    );

    let center_weight = gaussian_weight(0.0, sigma);
    var blurred = base_coverage * center_weight;
    var total_weight = center_weight;

    for (var ring_i = 0u; ring_i < 3u; ring_i = ring_i + 1u) {
        let radius = ring_radii[ring_i];
        let weight = gaussian_weight(radius, sigma);

        for (var dir_i = 0u; dir_i < 12u; dir_i = dir_i + 1u) {
            let sample_uv = uv + pixel_offset_to_uv(directions[dir_i] * radius, uv_bounds, screen_size);
            blurred += sample_sdf_coverage_with_edge_fade(
                sample_uv,
                uv_bounds,
                screen_size,
                total_expand,
            ) * weight;
            total_weight += weight;
        }
    }

    let blurred_coverage = blurred / max(total_weight, 0.0001);
    let halo = max(blurred_coverage - base_coverage * 0.3, 0.0);
    let halo_mask = 1.0 - smoothstep(0.06, 0.82, base_coverage);

    return clamp(halo * halo_mask * 1.12, 0.0, 1.0);
}


// 渐变遮罩计算
fn calculate_gradient_mask(
    pos_in_word: f32,
    word_start_ms: f32,
    word_end_ms: f32,
    current_time_ms: f32,
    word_fade_width: f32
) -> f32 {
    let word_duration = word_end_ms - word_start_ms;
    
    if current_time_ms < word_start_ms {
        return 0.0;
    }
    if current_time_ms >= word_end_ms {
        return 1.0;
    }
    
    var time_progress: f32;
    if word_duration > 0.0 {
        time_progress = (current_time_ms - word_start_ms) / word_duration;
    } else {
        time_progress = 1.0;
    }
    
    let fade_width = word_fade_width;
    let w = 1.0 + fade_width;
    let mask_pos = -w + time_progress * w;
    let clamped_mask_pos = clamp(mask_pos, -w, 0.0);
    
    let gradient_width = fade_width;
    let total_aspect = 2.0 + gradient_width;
    let width_in_total = gradient_width / total_aspect;
    let left_pos = (1.0 - width_in_total) / 2.0;
    let right_pos = left_pos + width_in_total;
    
    let mask_total_width = total_aspect;
    let char_in_mask = (pos_in_word - clamped_mask_pos) / mask_total_width;
    
    if char_in_mask <= left_pos {
        return 1.0;
    } else if char_in_mask >= right_pos {
        return 0.0;
    } else {
        return 1.0 - (char_in_mask - left_pos) / width_in_total;
    }
}

fn calculate_bright_mask_alpha(scale: f32) -> f32 {
    let normalized = clamp((scale - 0.97) / 0.03, 0.0, 1.0);
    return normalized * 0.8 + 0.2;
}

fn calculate_dark_mask_alpha(scale: f32) -> f32 {
    let normalized = clamp((scale - 0.97) / 0.03, 0.0, 1.0);
    return normalized * 0.2 + 0.2;
}

// Proper CSS cubic-bezier solver (Newton-Raphson)
fn solve_cubic_bezier(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    var t = x;
    for (var i = 0; i < 8; i++) {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        
        let current_x = 3.0 * mt2 * t * x1 + 3.0 * mt * t2 * x2 + t3;
        let error = current_x - x;
        if abs(error) < 0.001 {
            break;
        }
        
        let dx = 3.0 * mt2 * x1 + 6.0 * mt * t * (x2 - x1) + 3.0 * t2 * (1.0 - x2);
        if abs(dx) < 0.001 {
            break;
        }
        
        t = t - error / dx;
        t = clamp(t, 0.0, 1.0);
    }
    
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    return 3.0 * mt2 * t * y1 + 3.0 * mt * t2 * y2 + t3;
}

fn emphasis_easing(x: f32) -> f32 {
    let mid = 0.5;
    if x < mid {
        let normalized_x = x / mid;
        return solve_cubic_bezier(normalized_x, 0.2, 0.4, 0.58, 1.0);
    } else {
        let normalized_x = (x - mid) / (1.0 - mid);
        return 1.0 - solve_cubic_bezier(normalized_x, 0.3, 0.0, 0.58, 1.0);
    }
}

fn calculate_emphasis_amount(word_duration: f32, last_word: bool) -> f32 {
    let du = max(1000.0, word_duration);
    var amount = du / 2000.0;
    amount = select(pow(amount, 3.0), sqrt(amount), amount > 1.0);
    amount = amount * 0.6;
    amount = select(amount, amount * 1.6, last_word);
    return min(1.2, amount);
}

fn calculate_emphasis_blur(word_duration: f32, last_word: bool) -> f32 {
    let du = max(1000.0, word_duration);
    var blur = du / 3000.0;
    blur = select(pow(blur, 3.0), sqrt(blur), blur > 1.0);
    blur = blur * 0.5;
    blur = select(blur, blur * 1.5, last_word);
    return min(0.8, blur);
}

fn calculate_char_emphasis_progress(
    current_time_ms: f32,
    char_delay_ms: f32,
    word_duration_ms: f32
) -> f32 {
    if word_duration_ms <= 0.0 {
        return 0.0;
    }
    let effective_duration = max(1000.0, word_duration_ms);
    let elapsed = current_time_ms - char_delay_ms;
    let progress = elapsed / effective_duration;
    return clamp(progress, 0.0, 1.0);
}

fn calculate_emphasis_float_progress(
    current_time_ms: f32,
    char_delay_ms: f32,
    word_duration_ms: f32
) -> f32 {
    if word_duration_ms <= 0.0 {
        return 0.0;
    }
    let effective_duration = max(1000.0, word_duration_ms);
    let float_duration = effective_duration * 1.4;
    let float_delay = char_delay_ms - 400.0;
    let elapsed = current_time_ms - float_delay;
    let progress = elapsed / float_duration;
    return clamp(progress, 0.0, 1.0);
}

fn calculate_basic_float_progress(
    current_time_ms: f32,
    word_start_ms: f32,
    word_end_ms: f32,
    line_start_ms: f32
) -> f32 {
    let word_duration = word_end_ms - word_start_ms;
    let duration = max(1000.0, word_duration);
    let delay = word_start_ms - line_start_ms;
    let elapsed = current_time_ms - line_start_ms - delay;
    if elapsed < 0.0 {
        return 0.0;
    }
    let progress = clamp(elapsed / duration, 0.0, 1.0);
    // AMLL uses CSS `ease-out`, i.e. cubic-bezier(0, 0, 0.58, 1).
    return solve_cubic_bezier(progress, 0.0, 0.0, 0.58, 1.0);
}

// === Vertex Shader ===
@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    let line_idx = in.line_info.x;
    let line = lines[line_idx];
    let non_dynamic = is_non_dynamic(in.line_info.y);
    let is_sub_line = is_translation(in.line_info.y) || is_romanized(in.line_info.y);
    
    let corner_x = in.corner.x;
    let corner_y = in.corner.y;
    
    var pos = in.pos;
    pos.y += line.y_position;
    
    var scale = line.scale;
    let line_center_y = line.y_position + line.line_height * 0.5;
    
    var emphasis_offset_x = 0.0;
    var emphasis_offset_y = 0.0;
    var emphasis_scale_factor = 1.0;
    
    let char_delay_ms = in.char_timing.x;
    let word_duration_ms = in.char_timing.y;
    let char_emphasis = calculate_char_emphasis_progress(
        globals.current_time_ms,
        char_delay_ms,
        word_duration_ms
    );
    
    let emphasis_float_progress = calculate_emphasis_float_progress(
        globals.current_time_ms,
        char_delay_ms,
        word_duration_ms
    );
    
    let word_start_ms = in.word_time.x;
    let word_end_ms = in.word_time.y;
    let approx_line_start_ms = word_start_ms - char_delay_ms + (word_duration_ms / 2.5 / max(1.0, in.char_info.y)) * in.char_info.x;
    let basic_float_progress = calculate_basic_float_progress(
        globals.current_time_ms,
        word_start_ms,
        word_end_ms,
        approx_line_start_ms
    );
    
    // 基础上浮效果：基于时间进度，不依赖 is_active flag
    // 这样当行从 active 变为非 active 时，上浮效果会保持，
    // 然后通过 Y 位置的 spring 动画平滑过渡到新位置
    if !non_dynamic && !is_sub_line && basic_float_progress > 0.0 {
        let is_bg_line = is_bg(in.line_info.y);
        let float_multiplier = select(1.0, 2.0, is_bg_line);
        let basic_float_y = -basic_float_progress * 0.05 * float_multiplier * globals.font_size;
        emphasis_offset_y += basic_float_y;
    }
    
    if !non_dynamic && !is_sub_line && is_emphasize(in.line_info.y) && char_emphasis > 0.0 {
        let emp = emphasis_easing(char_emphasis);
        let last_word = is_last_word(in.line_info.y);
        let amount = calculate_emphasis_amount(word_duration_ms, last_word);
        
        emphasis_scale_factor = 1.0 + emp * 0.1 * amount;
        
        let char_count = max(1.0, in.char_info.y);
        let char_index = in.char_info.x;
        let pos_from_center = char_count / 2.0 - char_index;
        emphasis_offset_x = -emp * 0.03 * amount * pos_from_center * globals.font_size;
        
        let emphasis_y = -emp * 0.025 * amount * globals.font_size;
        emphasis_offset_y += emphasis_y;
    }
    
    if !non_dynamic && !is_sub_line && is_emphasize(in.line_info.y) && emphasis_float_progress > 0.0 {
        let is_bg_line = is_bg(in.line_info.y);
        let float_multiplier = select(1.0, 2.0, is_bg_line);
        let emphasis_float_y = -sin(emphasis_float_progress * 3.14159) * 0.05 * float_multiplier * globals.font_size;
        emphasis_offset_y += emphasis_float_y;
    }
    
    scale = scale * emphasis_scale_factor;
    pos.y = line_center_y + (pos.y - line_center_y) * scale;
    
    // === 画布扩展：修复模糊裁剪问题 (3σ 法则) ===
    // 行级模糊已经改成真正的离屏 blur，glow 也会单独走离屏 mask + blur + composite。
    // 这里仅为当前 pass 的 line.blur 扩展画布，不再把强调 glow 混进同一个 fragment pass。
    let max_blur_px = line.blur;
    
    // 只有在需要模糊时才扩展画布
    // 如果 max_blur_px < 0.5，不需要扩展（锐利模式）
    var total_expand = 0.0;
    if max_blur_px >= 0.5 {
        // 高斯模糊的"可见范围"通常是半径的 2~3 倍
        // 使用 3.0 倍 + 5.0 像素安全边距，保证光晕能自然淡出到完全透明
        total_expand = (max_blur_px * EXPANSION_FACTOR) + PADDING_SAFE;
    }
    
    // 计算向四周扩展的方向
    // corner.x/y 是 0 或 1，转换为 -1 或 1 的方向向量
    let expand_dir_x = corner_x * 2.0 - 1.0;  // 左侧为-1，右侧为1
    let expand_dir_y = corner_y * 2.0 - 1.0;  // 顶部为-1，底部为1
    let expand_x = expand_dir_x * total_expand;
    let expand_y = expand_dir_y * total_expand;
    
    // 计算屏幕尺寸和 UV 每像素变化量
    let screen_width = in.size.x * scale;
    let screen_height = in.size.y * scale;
    var uv_per_px_x = 0.0;
    if screen_width > 0.001 { 
        uv_per_px_x = (in.uv_max.x - in.uv_min.x) / screen_width; 
    }
    var uv_per_px_y = 0.0;
    if screen_height > 0.001 { 
        uv_per_px_y = (in.uv_max.y - in.uv_min.y) / screen_height; 
    }
    
    // 应用扩展到屏幕位置
    pos.x += corner_x * in.size.x * scale + expand_x;
    pos.y += corner_y * in.size.y * scale + expand_y;
    
    // 计算扩展后的 UV 坐标
    let base_uv = mix(in.uv_min, in.uv_max, vec2<f32>(corner_x, corner_y));
    let expanded_uv = base_uv + vec2<f32>(expand_x * uv_per_px_x, expand_y * uv_per_px_y);
    
    pos.x += emphasis_offset_x;
    pos.y += emphasis_offset_y;
    
    pos.x += globals.bounds_offset.x;
    pos.y += globals.bounds_offset.y;
    
    let clip_x = (pos.x / globals.viewport_size.x) * 2.0 - 1.0;
    let clip_y = 1.0 - (pos.y / globals.viewport_size.y) * 2.0;
    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    
    out.uv = expanded_uv;  // 使用扩展后的 UV
    out.uv_bounds = vec4<f32>(in.uv_min, in.uv_max);  // 传递原始 UV 边界用于钳位
    out.screen_size = vec2<f32>(screen_width, screen_height);  // 传递屏幕尺寸用于距离外推
    out.total_expand = total_expand;  // 传递画布扩展量用于边缘衰减
    out.word_time = in.word_time;
    out.glyph_word_pos = in.glyph_word_pos;
    out.color = unpack_color(in.color);
    out.flags = in.line_info.y;
    out.emphasis = char_emphasis;
    out.line_index = line_idx;
    out.char_info = in.char_info;
    out.local_x = corner_x;
    out.char_timing = in.char_timing;
    out.float_progress = emphasis_float_progress;
    out.visual_line_info = in.visual_line_info;
    out.pos_in_visual_line = in.pos_in_visual_line;
    
    return out;
}


// === Fragment Shader ===
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let line = lines[in.line_index];
    let is_sub_line = is_translation(in.flags) || is_romanized(in.flags);
    
    // 计算像素在单词中的位置
    let pixel_pos_in_word = in.glyph_word_pos.x + in.local_x * in.glyph_word_pos.y;
    
    // === 换行高亮修复：计算全局视觉位置 ===
    // 当一行歌词换行成多个视觉行时，使用全局视觉位置来计算高亮
    // 这样高亮会按视觉顺序（从第一行到第二行）进行，而不是同时高亮所有行
    let visual_info = unpack_visual_line_info(in.visual_line_info);
    let global_visual_pos = calculate_global_visual_pos(
        visual_info.x,  // visual_line_index
        visual_info.y,  // visual_line_count
        pixel_pos_in_word  // 使用单词内位置作为行内位置的近似
    );
    
    var brightness = 1.0;
    var alpha_multiplier = line.opacity;
    if is_sub_line {
        // 子行在视觉上比我们现在更容易随离焦一起变暗。
        // active 行保持 0.3，非 active 行额外乘上整行 opacity，避免翻译行“悬浮发亮”。
        alpha_multiplier = 0.3;
        if line.is_active == 0u {
            alpha_multiplier = alpha_multiplier * line.opacity;
        }
    } else {
        // 主行同样允许梯度扫光。
        // 非动态只是不做逐字上浮/强调，不应该禁掉整行的扫光。
        let highlight = calculate_gradient_mask(
            global_visual_pos,  // 使用全局视觉位置，实现换行按顺序高亮
            in.word_time.x,
            in.word_time.y,
            globals.current_time_ms,
            globals.word_fade_width
        );

        let bright_alpha = line.bright_mask_alpha;
        let dark_alpha = line.dark_mask_alpha;
        brightness = mix(dark_alpha, bright_alpha, highlight);
    }
    
    // 强调效果
    if is_emphasize(in.flags) && in.emphasis > 0.0 {
        let emp = emphasis_easing(in.emphasis);
        let word_duration = in.word_time.y - in.word_time.x;
        let last_word = is_last_word(in.flags);
        let amount = calculate_emphasis_amount(word_duration, last_word);

        brightness = brightness + emp * 0.1 * amount;
    }
    
    // === SDF 采样 ===
    // line.blur 已经是按像素计算的行级模糊半径，直接用于 SDF 采样。
    let blur_px = line.blur;
    
    // 采样主体（使用边缘衰减确保在边缘前归零）
    let shape_alpha = sample_sdf_gaussian_blur_v2(
        in.uv,
        blur_px,
        in.uv_bounds,
        in.screen_size,
        in.total_expand,
    );
    
    // Early discard
    if shape_alpha < 0.01 {
        discard;
    }
    
    // 基础颜色
    let fill_color = in.color.rgb * brightness;
    let fill_alpha = shape_alpha * alpha_multiplier;
    let premul_color = fill_color * fill_alpha;
    let final_alpha = fill_alpha;

    if final_alpha < 0.01 {
        discard;
    }

    return vec4<f32>(premul_color, final_alpha);
}

@fragment
fn fs_glow_mask(in: VertexOutput) -> @location(0) vec4<f32> {
    let line = lines[in.line_index];
    let is_sub_line = is_translation(in.flags) || is_romanized(in.flags);
    if is_sub_line || !is_emphasize(in.flags) || in.emphasis <= 0.0 {
        discard;
    }

    let word_duration = in.word_time.y - in.word_time.x;
    let last_word = is_last_word(in.flags);
    let blur_amount = calculate_emphasis_blur(word_duration, last_word);
    let glow_intensity = emphasis_easing(in.emphasis) * blur_amount;
    if glow_intensity <= 0.01 {
        discard;
    }

    let shape_alpha = sample_sdf_coverage_with_edge_fade(
        in.uv,
        in.uv_bounds,
        in.screen_size,
        in.total_expand,
    );
    if shape_alpha < 0.01 {
        discard;
    }

    let alpha = shape_alpha * glow_intensity * line.opacity;
    if alpha < 0.01 {
        discard;
    }

    return vec4<f32>(vec3<f32>(alpha), alpha);
}
