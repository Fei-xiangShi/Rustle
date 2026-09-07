//! 图像预处理模块 - 基于 img.ts 实现
//!
//! 提供以下图像处理功能:
//! - Box blur (模糊)
//! - Saturation (饱和度)
//! - Brightness (亮度)
//! - Contrast (对比度)

use image::DynamicImage;

pub const PLAYLIST_FOOTER_WIDTH: u32 = 160;
pub const PLAYLIST_FOOTER_HEIGHT: u32 = 56;

/// 图像预处理结果
pub struct ProcessedImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl ProcessedImage {
    /// 从 RGBA 数据创建
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// 获取 RGBA 像素数据
    pub fn as_rgba(&self) -> &[u8] {
        &self.data
    }
}

/// Box blur 实现 (参考 img.ts)
///
/// 使用多遍 box blur 近似高斯模糊效果
pub fn blur_image(data: &mut [u8], width: usize, height: usize, radius: usize, quality: usize) {
    if radius == 0 || quality == 0 {
        return;
    }

    let wm = width.saturating_sub(1);
    let hm = height.saturating_sub(1);
    let rad1x = radius + 1;
    let divx = radius + rad1x;
    let rad1y = radius + 1;
    let divy = radius + rad1y;
    let div2 = 1.0 / ((divx * divy) as f32);

    let pixel_count = width * height;
    let mut r = vec![0i32; pixel_count];
    let mut g = vec![0i32; pixel_count];
    let mut b = vec![0i32; pixel_count];
    let mut a = vec![0i32; pixel_count];

    let mut vmin = vec![0usize; width.max(height)];
    let mut vmax = vec![0usize; width.max(height)];

    for _ in 0..quality {
        let mut yw: usize = 0;
        let mut yi: usize = 0;

        // 水平方向
        for y in 0..height {
            let mut rsum = data[yw * 4] as i32 * rad1x as i32;
            let mut gsum = data[yw * 4 + 1] as i32 * rad1x as i32;
            let mut bsum = data[yw * 4 + 2] as i32 * rad1x as i32;
            let mut asum = data[yw * 4 + 3] as i32 * rad1x as i32;

            for i in 1..=radius {
                let p = (yw + i.min(wm)) * 4;
                rsum += data[p] as i32;
                gsum += data[p + 1] as i32;
                bsum += data[p + 2] as i32;
                asum += data[p + 3] as i32;
            }

            for x in 0..width {
                r[yi] = rsum;
                g[yi] = gsum;
                b[yi] = bsum;
                a[yi] = asum;

                if y == 0 {
                    vmin[x] = (x + rad1x).min(wm);
                    vmax[x] = x.saturating_sub(radius);
                }

                let p1 = (yw + vmin[x]) * 4;
                let p2 = (yw + vmax[x]) * 4;

                rsum += data[p1] as i32 - data[p2] as i32;
                gsum += data[p1 + 1] as i32 - data[p2 + 1] as i32;
                bsum += data[p1 + 2] as i32 - data[p2 + 2] as i32;
                asum += data[p1 + 3] as i32 - data[p2 + 3] as i32;

                yi += 1;
            }
            yw += width;
        }

        // 垂直方向
        for x in 0..width {
            let mut yp = x;
            let mut rsum = r[yp] * rad1y as i32;
            let mut gsum = g[yp] * rad1y as i32;
            let mut bsum = b[yp] * rad1y as i32;
            let mut asum = a[yp] * rad1y as i32;

            for i in 1..=radius {
                if i <= hm {
                    yp += width;
                }
                rsum += r[yp];
                gsum += g[yp];
                bsum += b[yp];
                asum += a[yp];
            }

            yi = x;
            for y in 0..height {
                data[yi * 4] = ((rsum as f32 * div2 + 0.5) as i32).clamp(0, 255) as u8;
                data[yi * 4 + 1] = ((gsum as f32 * div2 + 0.5) as i32).clamp(0, 255) as u8;
                data[yi * 4 + 2] = ((bsum as f32 * div2 + 0.5) as i32).clamp(0, 255) as u8;
                data[yi * 4 + 3] = ((asum as f32 * div2 + 0.5) as i32).clamp(0, 255) as u8;

                if x == 0 {
                    vmin[y] = (y + rad1y).min(hm) * width;
                    vmax[y] = y.saturating_sub(radius) * width;
                }

                let p1 = x + vmin[y];
                let p2 = x + vmax[y];

                rsum += r[p1] - r[p2];
                gsum += g[p1] - g[p2];
                bsum += b[p1] - b[p2];
                asum += a[p1] - a[p2];

                yi += width;
            }
        }
    }
}

/// 调整饱和度 (参考 img.ts)
///
/// saturation: 0.0 = 灰度, 1.0 = 原始, >1.0 = 增强饱和度
pub fn saturate_image(data: &mut [u8], saturation: f32) {
    for i in (0..data.len()).step_by(4) {
        let r = data[i] as f32;
        let g = data[i + 1] as f32;
        let b = data[i + 2] as f32;

        // 灰度值 (使用标准权重)
        let gray = r * 0.3 + g * 0.59 + b * 0.11;

        data[i] = (gray * (1.0 - saturation) + r * saturation).clamp(0.0, 255.0) as u8;
        data[i + 1] = (gray * (1.0 - saturation) + g * saturation).clamp(0.0, 255.0) as u8;
        data[i + 2] = (gray * (1.0 - saturation) + b * saturation).clamp(0.0, 255.0) as u8;
    }
}

/// 调整亮度 (参考 img.ts)
///
/// brightness: 0.0 = 黑色, 1.0 = 原始, >1.0 = 更亮
pub fn brightness_image(data: &mut [u8], brightness: f32) {
    for i in (0..data.len()).step_by(4) {
        data[i] = (data[i] as f32 * brightness).clamp(0.0, 255.0) as u8;
        data[i + 1] = (data[i + 1] as f32 * brightness).clamp(0.0, 255.0) as u8;
        data[i + 2] = (data[i + 2] as f32 * brightness).clamp(0.0, 255.0) as u8;
    }
}

/// 调整对比度 (参考 img.ts)
///
/// contrast: 0.0 = 灰色, 1.0 = 原始, >1.0 = 增强对比度
pub fn contrast_image(data: &mut [u8], contrast: f32) {
    for i in (0..data.len()).step_by(4) {
        data[i] = ((data[i] as f32 - 128.0) * contrast + 128.0).clamp(0.0, 255.0) as u8;
        data[i + 1] = ((data[i + 1] as f32 - 128.0) * contrast + 128.0).clamp(0.0, 255.0) as u8;
        data[i + 2] = ((data[i + 2] as f32 - 128.0) * contrast + 128.0).clamp(0.0, 255.0) as u8;
    }
}

/// 处理图像用于背景渲染
///
/// 按 的方式处理图像 (来自 MeshGradientRenderer.setAlbum):
/// 1. 缩小到指定尺寸 (32x32)
/// 2. 对比度 0.4 (降低对比度)
/// 3. 饱和度 3.0 (大幅增加饱和度)
/// 4. 对比度 1.7 (增加对比度)
/// 5. 亮度 0.75 (降低亮度)
/// 6. 模糊 (radius=2, quality=4)
///
/// 注意：的模糊效果主要来自于 Bicubic Hermite Patch 插值，
/// 而不是图像本身的模糊。32x32 的小图像在 mesh 上被平滑插值放大，
/// 自然产生了柔和的渐变效果。
pub fn process_image_for_background(image: &DynamicImage, target_size: u32) -> ProcessedImage {
    // 缩小图像以提高处理速度 (使用 32x32)
    let resized = image.resize_exact(
        target_size,
        target_size,
        image::imageops::FilterType::Triangle,
    );

    let rgba = resized.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data: Vec<u8> = rgba.into_raw();

    // 按 的精确顺序和参数处理
    // 来自: MeshGradientRenderer.setAlbum() in index.ts
    contrast_image(&mut data, 0.4); // 先降低对比度
    saturate_image(&mut data, 3.0); // 大幅增加饱和度
    contrast_image(&mut data, 1.7); // 再增加对比度
    brightness_image(&mut data, 0.75); // 降低亮度
    blur_image(&mut data, width as usize, height as usize, 2, 4); // 原始参数

    ProcessedImage::from_rgba(width, height, data)
}

/// Build the small, darkened blur used behind playlist-card metadata.
///
/// Processing a bounded footer-sized buffer keeps the work cheap while the
/// repeated box blur produces a Gaussian-like result suitable for text.
pub fn process_image_for_playlist_footer(image: &DynamicImage) -> ProcessedImage {
    let flipped = image.flipv();
    let resized = flipped.resize_to_fill(
        PLAYLIST_FOOTER_WIDTH,
        PLAYLIST_FOOTER_HEIGHT,
        image::imageops::FilterType::Triangle,
    );
    let rgba = resized.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data = rgba.into_raw();

    blur_image(&mut data, width as usize, height as usize, 9, 3);
    saturate_image(&mut data, 1.15);
    brightness_image(&mut data, 0.58);
    contrast_image(&mut data, 0.92);

    ProcessedImage::from_rgba(width, height, data)
}

#[cfg(test)]
mod playlist_footer_tests {
    use super::{PLAYLIST_FOOTER_HEIGHT, PLAYLIST_FOOTER_WIDTH, process_image_for_playlist_footer};
    use image::{DynamicImage, Rgba, RgbaImage};

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
