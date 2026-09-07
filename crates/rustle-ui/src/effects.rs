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
