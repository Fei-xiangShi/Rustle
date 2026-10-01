//! SDF (Signed Distance Field) Generator
//!
//! 使用 ab_glyph + sdf_glyph_renderer 生成高质量 SDF 位图
//! 纯 Rust 实现，无 C++ 依赖
//!
//! ## 使用方式
//!
#[cfg(test)]
use ab_glyph::{Font, FontRef, GlyphId, PxScale};
use cosmic_text::{SwashContent, SwashImage};
use sdf_glyph_renderer::{BitmapGlyph, clamp_to_u8};

/// SDF 生成器配置
#[derive(Debug, Clone, Copy)]
pub struct SdfConfig {
    /// 生成 SDF 时的基准字号（像素）
    pub base_size: u32,
    /// SDF buffer（像素）- 字形周围的边距，用于捕获外部距离
    pub buffer: usize,
    /// SDF radius（像素）- 距离场的有效范围
    pub radius: usize,
    /// clamp_to_u8 的 cutoff 值（0.0-1.0）
    pub cutoff: f64,
}

impl Default for SdfConfig {
    fn default() -> Self {
        Self {
            base_size: 64,
            buffer: 4,
            radius: 8,
            // cutoff 决定 SDF 距离如何映射到 0-255
            // 0.5 表示：距离 = 0（边缘）映射到 128
            // 这与 shader 中的 edge_threshold = 0.5 完美匹配
            cutoff: 0.5,
        }
    }
}

/// SDF 生成结果
#[derive(Debug, Clone)]
pub struct SdfBitmap {
    /// 单通道 SDF 数据（每像素 1 字节）
    pub data: Vec<u8>,
    /// 位图宽度（包含 buffer）
    pub width: u32,
    /// 位图高度（包含 buffer）
    pub height: u32,
    /// 字形相对于原点的水平偏移（考虑 buffer）
    pub bearing_x: i32,
    /// 字形相对于基线的垂直偏移（考虑 buffer）
    pub bearing_y: i32,
}

/// SDF 生成器
pub struct SdfGenerator {
    config: SdfConfig,
}

impl SdfGenerator {
    /// 创建新的 SDF 生成器
    pub fn new(base_size: u32, buffer: usize) -> Self {
        Self {
            config: SdfConfig {
                base_size,
                buffer,
                ..Default::default()
            },
        }
    }

    /// 获取配置
    pub fn config(&self) -> &SdfConfig {
        &self.config
    }

    /// Build an SDF bitmap from a swash-rasterized glyph image.
    ///
    /// `placement_left/top/width/height` must come from the same swash image.
    /// The returned metrics keep our configured SDF buffer around that image.
    pub fn generate_from_swash_image(&self, image: &SwashImage) -> Option<SdfBitmap> {
        let width = image.placement.width as usize;
        let height = image.placement.height as usize;

        if width == 0 || height == 0 {
            return None;
        }

        let alpha = swash_to_alpha(image)?;
        self.generate_from_alpha_bitmap(
            &alpha,
            width,
            height,
            image.placement.left - self.config.buffer as i32,
            image.placement.top + self.config.buffer as i32,
        )
    }

    /// 从已加载的字体生成 SDF
    #[cfg(test)]
    fn generate_from_font(&self, font: &FontRef, glyph_id: GlyphId) -> Option<SdfBitmap> {
        let scale = PxScale::from(self.config.base_size as f32);

        // 获取字形轮廓
        let glyph = glyph_id.with_scale(scale);
        let outlined = font.outline_glyph(glyph)?;

        // 获取边界
        let bounds = outlined.px_bounds();
        let glyph_width = bounds.width().ceil() as usize;
        let glyph_height = bounds.height().ceil() as usize;

        // 空字形检查
        if glyph_width == 0 || glyph_height == 0 {
            return None;
        }

        // 光栅化为 alpha 位图
        let mut alpha = vec![0u8; glyph_width * glyph_height];
        outlined.draw(|x, y, coverage| {
            let idx = y as usize * glyph_width + x as usize;
            if idx < alpha.len() {
                alpha[idx] = (coverage * 255.0) as u8;
            }
        });

        self.generate_from_alpha_bitmap(
            &alpha,
            glyph_width,
            glyph_height,
            bounds.min.x.floor() as i32 - self.config.buffer as i32,
            -bounds.min.y.floor() as i32 + self.config.buffer as i32,
        )
    }

    fn generate_from_alpha_bitmap(
        &self,
        alpha: &[u8],
        glyph_width: usize,
        glyph_height: usize,
        bearing_x: i32,
        bearing_y: i32,
    ) -> Option<SdfBitmap> {
        if glyph_width == 0 || glyph_height == 0 {
            return None;
        }

        let bitmap =
            BitmapGlyph::from_unbuffered(alpha, glyph_width, glyph_height, self.config.buffer)
                .ok()?;
        let sdf_f64 = bitmap.render_sdf(self.config.radius);
        let data = clamp_to_u8(&sdf_f64, self.config.cutoff).ok()?;

        Some(SdfBitmap {
            data,
            width: (glyph_width + self.config.buffer * 2) as u32,
            height: (glyph_height + self.config.buffer * 2) as u32,
            bearing_x,
            bearing_y,
        })
    }
}

fn swash_to_alpha(image: &SwashImage) -> Option<Vec<u8>> {
    let width = image.placement.width as usize;
    let height = image.placement.height as usize;

    match image.content {
        SwashContent::Mask => Some(image.data.clone()),
        SwashContent::Color => {
            if image.data.len() != width * height * 4 {
                return None;
            }
            Some(
                image
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|px| px[3])
                    .collect::<Vec<u8>>(),
            )
        }
        SwashContent::SubpixelMask => {
            if image.data.len() != width * height * 3 {
                return None;
            }
            Some(
                image
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|px| px[0].max(px[1]).max(px[2]))
                    .collect::<Vec<u8>>(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};

    const TEST_FONT: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fonts/RustleSdfTest.ttf"
    ));

    fn load_test_font() -> FontRef<'static> {
        FontRef::try_from_slice(TEST_FONT).expect("valid SDF test font")
    }

    fn test_font_system() -> FontSystem {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(TEST_FONT.to_vec());
        db.set_sans_serif_family("Rustle SDF Test");
        assert_eq!(db.faces().count(), 1);
        FontSystem::new_with_locale_and_db("en-US".to_string(), db)
    }

    #[test]
    fn test_default_config() {
        let config = SdfConfig::default();
        assert_eq!(config.base_size, 64);
        assert_eq!(config.buffer, 4);
        assert_eq!(config.radius, 8);
        assert!((config.cutoff - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_generator_new() {
        let generator = SdfGenerator::new(64, 8);
        assert_eq!(generator.config().base_size, 64);
        assert_eq!(generator.config().buffer, 8);
    }

    #[test]
    fn test_generate_char_a() {
        let font = load_test_font();
        let generator = SdfGenerator::new(64, 4);
        let glyph = font.glyph_id('A');
        assert_ne!(glyph.0, 0, "fixture must contain A");
        let bitmap = generator.generate_from_font(&font, glyph);

        assert!(bitmap.is_some());
        let bitmap = bitmap.unwrap();
        assert!(bitmap.width > 0);
        assert!(bitmap.height > 0);
        assert_eq!(bitmap.data.len(), (bitmap.width * bitmap.height) as usize);
    }

    #[test]
    fn test_space_returns_none() {
        let font = load_test_font();
        let generator = SdfGenerator::new(64, 4);
        let glyph = font.glyph_id(' ');
        assert_ne!(glyph.0, 0, "fixture must contain space");
        let bitmap = generator.generate_from_font(&font, glyph);

        // 空格没有轮廓，应该返回 None
        assert!(bitmap.is_none());
    }

    #[test]
    fn test_generate_from_swash_image_keeps_swash_placement() {
        let mut font_system = test_font_system();

        let metrics = Metrics::new(64.0, 64.0 * 1.4);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_size(Some(1024.0), None);

        let attrs = Attrs::new().family(Family::SansSerif);
        buffer.set_text("A你あ", &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut font_system, false);

        let generator = SdfGenerator::new(64, 12);
        let mut swash_cache = SwashCache::new();

        let mut rendered_glyphs = 0;
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                assert_ne!(glyph.glyph_id, 0, "fixture must cover the shaped character");
                rendered_glyphs += 1;
                let key = glyph.physical((0.0, 0.0), 1.0).cache_key;
                let swash = swash_cache
                    .get_image_uncached(&mut font_system, key)
                    .expect("swash image");

                let bitmap = generator
                    .generate_from_swash_image(&swash)
                    .expect("sdf bitmap");

                assert_eq!(bitmap.bearing_x, swash.placement.left - 12);
                assert_eq!(bitmap.bearing_y, swash.placement.top + 12);
                assert_eq!(bitmap.width, swash.placement.width + 24);
                assert_eq!(bitmap.height, swash.placement.height + 24);
            }
        }
        assert_eq!(rendered_glyphs, 3, "all three scripts must be rasterized");
    }
}
