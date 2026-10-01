//! Bounded artwork preprocessing: OKLCH artistic adjustment, linear premultiplied
//! filtering, and a single final sRGB byte encoding. Original artwork is untouched.
use crate::color::Oklcha;
use image::{DynamicImage, RgbaImage};

pub const PLAYLIST_FOOTER_WIDTH: u32 = 160;
pub const PLAYLIST_FOOTER_HEIGHT: u32 = 56;

pub struct ProcessedImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}
impl ProcessedImage {
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }
    pub fn as_rgba(&self) -> &[u8] {
        &self.data
    }
}

fn process(rgba: RgbaImage, footer: bool) -> ProcessedImage {
    let (width, height) = rgba.dimensions();
    let mut pixels: Vec<[f32; 4]> = rgba
        .pixels()
        .map(|p| {
            let source = Oklcha::from_color(iced::Color::from_rgba8(
                p[0],
                p[1],
                p[2],
                f32::from(p[3]) / 255.0,
            ));
            // Bound L even for a white cover so the artwork foreground remains readable.
            let l = 0.08 + source.lightness() * if footer { 0.40 } else { 0.42 };
            let c = (source.chroma() * if footer { 1.05 } else { 1.45 }).min(0.12);
            let [r, g, b, a] = source
                .with_lightness(l)
                .with_chroma(c)
                .to_color()
                .into_linear();
            [r * a, g * a, b * a, a]
        })
        .collect();
    blur(
        &mut pixels,
        width as usize,
        height as usize,
        if footer { 9 } else { 2 },
        if footer { 3 } else { 4 },
    );
    let data = pixels
        .into_iter()
        .flat_map(|[r, g, b, a]| {
            if a <= 0.000001 {
                [0, 0, 0, 0]
            } else {
                iced::Color::from_linear_rgba(
                    (r / a).clamp(0.0, 1.0),
                    (g / a).clamp(0.0, 1.0),
                    (b / a).clamp(0.0, 1.0),
                    a.clamp(0.0, 1.0),
                )
                .into_rgba8()
            }
        })
        .collect();
    ProcessedImage::from_rgba(width, height, data)
}

/// Separable box filter on linear premultiplied samples, with replicated edges.
fn blur(pixels: &mut Vec<[f32; 4]>, width: usize, height: usize, radius: usize, passes: usize) {
    let mut scratch = vec![[0.0; 4]; pixels.len()];
    for _ in 0..passes {
        for horizontal in [true, false] {
            for y in 0..height {
                for x in 0..width {
                    let mut sum = [0.0; 4];
                    for delta in -(radius as isize)..=radius as isize {
                        let sx = if horizontal {
                            (x as isize + delta).clamp(0, width as isize - 1) as usize
                        } else {
                            x
                        };
                        let sy = if horizontal {
                            y
                        } else {
                            (y as isize + delta).clamp(0, height as isize - 1) as usize
                        };
                        for (s, v) in sum.iter_mut().zip(pixels[sy * width + sx]) {
                            *s += v;
                        }
                    }
                    scratch[y * width + x] = sum.map(|v| v / (2 * radius + 1) as f32);
                }
            }
            std::mem::swap(pixels, &mut scratch);
        }
    }
}

pub fn process_image_for_background(image: &DynamicImage, target_size: u32) -> ProcessedImage {
    let size = target_size.clamp(1, 128);
    process(
        image
            .resize_exact(size, size, image::imageops::FilterType::Nearest)
            .to_rgba8(),
        false,
    )
}

pub fn process_image_for_playlist_footer(image: &DynamicImage) -> ProcessedImage {
    let rgba = image
        .flipv()
        .resize_to_fill(
            PLAYLIST_FOOTER_WIDTH,
            PLAYLIST_FOOTER_HEIGHT,
            image::imageops::FilterType::Nearest,
        )
        .to_rgba8();
    process(rgba, true)
}

#[cfg(test)]
mod playlist_footer_tests {
    use super::{PLAYLIST_FOOTER_HEIGHT, PLAYLIST_FOOTER_WIDTH, process_image_for_playlist_footer};
    use image::{DynamicImage, Rgba, RgbaImage};

    #[test]
    fn transparent_rgb_cannot_contaminate_filtered_artwork() {
        let source = |hidden| {
            DynamicImage::ImageRgba8(RgbaImage::from_fn(32, 32, |x, _| {
                if x < 16 {
                    Rgba([20, 180, 150, 255])
                } else {
                    Rgba(hidden)
                }
            }))
        };
        let black = super::process_image_for_background(&source([0, 0, 0, 0]), 32);
        let red = super::process_image_for_background(&source([255, 0, 0, 0]), 32);
        assert_eq!(black.data, red.data);
        let opaque = super::process_image_for_background(
            &DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([20, 180, 150, 255]))),
            32,
        );
        for pixel in black.data.as_chunks::<4>().0.iter().filter(|p| p[3] > 0) {
            for (channel, value) in pixel.iter().enumerate().take(3) {
                assert!(value.abs_diff(opaque.data[channel]) <= 1);
            }
        }
    }

    #[test]
    fn playlist_footer_has_expected_dimensions_and_blended_pixels() {
        let mut source = RgbaImage::new(PLAYLIST_FOOTER_WIDTH, PLAYLIST_FOOTER_HEIGHT);
        for (_x, y, pixel) in source.enumerate_pixels_mut() {
            *pixel = if y < PLAYLIST_FOOTER_HEIGHT / 2 {
                Rgba([255, 32, 32, 255])
            } else {
                Rgba([32, 32, 255, 255])
            };
        }

        let processed = process_image_for_playlist_footer(&DynamicImage::ImageRgba8(source));

        assert_eq!(processed.width, PLAYLIST_FOOTER_WIDTH);
        assert_eq!(processed.height, PLAYLIST_FOOTER_HEIGHT);
        let center = ((PLAYLIST_FOOTER_HEIGHT / 2 * PLAYLIST_FOOTER_WIDTH
            + PLAYLIST_FOOTER_WIDTH / 2)
            * 4) as usize;
        assert!(processed.data[center] > 20);
        assert!(processed.data[center + 2] > 20);
        assert_eq!(processed.data[center + 3], 255);

        let top = ((10 * PLAYLIST_FOOTER_WIDTH + PLAYLIST_FOOTER_WIDTH / 2) * 4) as usize;
        let bottom = (((PLAYLIST_FOOTER_HEIGHT - 10) * PLAYLIST_FOOTER_WIDTH
            + PLAYLIST_FOOTER_WIDTH / 2)
            * 4) as usize;
        assert!(processed.data[top + 2] > processed.data[top]);
        assert!(processed.data[bottom] > processed.data[bottom + 2]);
    }
}
