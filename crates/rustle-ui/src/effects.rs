//! WGPU Shader system for visual effects
//!
//! Provides custom shader widgets for rendering:
//! - Animated mesh gradient backgrounds
//! - Gaussian blur effects
//! - Album artwork with flow animations
//! - Vignette and noise/dithering effects
//! - Bicubic Hermite Patch mesh gradients
//! - Image preprocessing (blur, contrast, saturation)

pub mod background;
pub mod image_processing;
pub mod mesh;
pub mod textured_background;

/// Assemble a color-aware shader for the actual render attachment.
fn color_shader(source: &str, format: iced::wgpu::TextureFormat) -> String {
    format!("{}\n{source}", include_str!("effects/color.wgsl")).replace(
        "OUTPUT_IS_SRGB",
        if format.is_srgb() { "true" } else { "false" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_shaders_validate_for_both_attachment_encodings() {
        use iced::wgpu::{TextureFormat, naga};
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
            for source in [
                background::BACKGROUND_SHADER,
                textured_background::MESH_SHADER,
            ] {
                let assembled = color_shader(source, format);
                let module = naga::front::wgsl::parse_str(&assembled).expect("valid color shader");
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .expect("valid shader bindings and color operations");
            }
        }
    }
}
