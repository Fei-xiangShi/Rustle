// Substituted once when constructing a pipeline for its actual target format.
const OUTPUT_SRGB: bool = OUTPUT_IS_SRGB;

fn srgb_to_linear(rgb: vec3f) -> vec3f {
    let c = clamp(rgb, vec3f(0.0), vec3f(1.0));
    return select(pow((c + vec3f(0.055)) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}
fn linear_to_srgb(rgb: vec3f) -> vec3f {
    let c = clamp(rgb, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(c, vec3f(1.0 / 2.4)) - vec3f(0.055), c * 12.92, c <= vec3f(0.0031308));
}
fn oklab_to_linear(lab: vec3f) -> vec3f {
    let l = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let lms = vec3f(l*l*l, m*m*m, s*s*s);
    return vec3f(
        dot(vec3f(4.0767417, -3.3077116, 0.23096994), lms),
        dot(vec3f(-1.268438, 2.6097574, -0.34131938), lms),
        dot(vec3f(-0.0041960863, -0.7034186, 1.7076147), lms)
    );
}
fn gamut_mapped_lab(lab: vec3f) -> vec3f {
    let rgb = oklab_to_linear(lab);
    if all(rgb >= vec3f(0.0)) && all(rgb <= vec3f(1.0)) { return rgb; }
    var low = 0.0;
    var high = 1.0;
    for (var i = 0; i < 20; i++) {
        let factor = (low + high) * 0.5;
        let candidate = oklab_to_linear(vec3f(lab.x, lab.yz * factor));
        if all(candidate >= vec3f(0.0)) && all(candidate <= vec3f(1.0)) { low = factor; } else { high = factor; }
    }
    return clamp(oklab_to_linear(vec3f(lab.x, lab.yz * low)), vec3f(0.0), vec3f(1.0));
}
// Straight alpha: the pipeline blend state applies alpha exactly once.
// Dither is applied in encoded output units even for an sRGB attachment.
fn color_output(linear: vec3f, alpha: f32, dither: f32) -> vec4f {
    let encoded = clamp(linear_to_srgb(linear) + vec3f(dither), vec3f(0.0), vec3f(1.0));
    if OUTPUT_SRGB { return vec4f(srgb_to_linear(encoded), alpha); }
    return vec4f(encoded, alpha);
}
