//! Bounded, deterministic artwork color extraction; callers own loading/identity.
use super::Oklcha;
use image::DynamicImage;

pub const FALLBACK: [Oklcha; 3] = [
    Oklcha::new(0.32, 0.04, 185.0, 1.0),
    Oklcha::new(0.25, 0.03, 210.0, 1.0),
    Oklcha::new(0.18, 0.02, 230.0, 1.0),
];

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

/// Oklab centroids weighted by alpha, ordered by represented population.
pub fn dominant(image: &DynamicImage) -> [Oklcha; 3] {
    let small = image
        .resize_exact(32, 32, image::imageops::FilterType::Nearest)
        .to_rgba8();
    let samples: Vec<_> = small
        .pixels()
        .filter(|p| p[3] > 0)
        .map(|p| {
            let rgb = iced::Color::from_rgb8(p[0], p[1], p[2]);
            (Oklcha::from_color(rgb).lab(), f32::from(p[3]) / 255.0)
        })
        .collect();
    if samples.is_empty() {
        return FALLBACK;
    }
    let mut centers = [samples[0].0; 3];
    for i in 1..3 {
        centers[i] = samples
            .iter()
            .max_by(|a, b| {
                let score = |sample: &([f32; 3], f32)| {
                    centers[..i]
                        .iter()
                        .map(|c| distance(sample.0, *c))
                        .fold(f32::MAX, f32::min)
                        * sample.1
                };
                score(a).total_cmp(&score(b))
            })
            .expect("nonempty samples")
            .0;
    }
    let mut weights = [0.0_f32; 3];
    for _ in 0..12 {
        let mut sums = [[0.0; 3]; 3];
        weights = [0.0; 3];
        for &(sample, weight) in &samples {
            let index = (0..3)
                .min_by(|&a, &b| {
                    distance(sample, centers[a]).total_cmp(&distance(sample, centers[b]))
                })
                .expect("three centers");
            weights[index] += weight;
            for (sum, channel) in sums[index].iter_mut().zip(sample) {
                *sum += channel * weight;
            }
        }
        for i in 0..3 {
            if weights[i] > 0.0 {
                centers[i] = sums[i].map(|s| s / weights[i]);
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]).then(a.cmp(&b)));
    order.map(|i| Oklcha::from_lab(centers[i], 1.0))
}

/// Dark artwork surfaces keep lyrics readable; original images remain unchanged.
pub fn background(source: Oklcha, rank: usize) -> Oklcha {
    let l = [0.34, 0.27, 0.20][rank] + 0.10 * source.lightness();
    // Keep the character of colorful artwork. The old 0.10 cap made most
    // covers converge toward gray after the darkening step.
    let chroma = if source.chroma() < 0.01 {
        source.chroma()
    } else {
        (source.chroma() * 1.12).clamp(0.035, 0.18)
    };
    source.with_lightness(l).with_chroma(chroma)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    #[test]
    fn uniform_neutral_transparent_and_population() {
        let neutral =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([128, 128, 128, 255])));
        assert_eq!(dominant(&neutral), dominant(&neutral));
        assert!(dominant(&neutral).iter().all(|c| c.chroma() < 0.0001));
        let transparent =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([255, 0, 0, 0])));
        assert_eq!(dominant(&transparent), FALLBACK);
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(32, 32, |x, _| {
            if x < 24 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        }));
        assert!((dominant(&image)[0].hue() - 29.2339).abs() < 0.05);
        for rgba in [[0, 0, 0, 255], [255, 255, 255, 255], [0, 255, 0, 255]] {
            let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba(rgba)));
            for (i, c) in dominant(&image).into_iter().enumerate() {
                let color = background(c, i).to_color();
                assert!(iced::Color::WHITE.relative_contrast(color) >= 4.5);
            }
        }
    }
}
