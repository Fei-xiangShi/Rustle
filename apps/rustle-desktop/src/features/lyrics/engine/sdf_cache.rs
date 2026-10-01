//! SDF Glyph Cache and Texture Atlas
//!
//! 管理 SDF 字形的生成和 GPU 纹理图集。
//!
//! ## 关键设计
//!
//! - 使用 SDF 生成器在 base_size (64px) 下生成 SDF 纹理
//! - 渲染时根据实际字号进行缩放
//! - 位置使用 cosmic-text 的布局，尺寸使用 SDF 的度量

use crate::features::lyrics::engine::sdf_generator::{SdfBitmap, SdfGenerator};
use cosmic_text::{CacheKey, FontSystem, SubpixelBin, SwashCache};
use iced::wgpu;
use iced::wgpu::{Device, Queue};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

/// 共享字体系统类型
pub type SharedFontSystem = Arc<Mutex<FontSystem>>;

// ---- 全局单例 ----

/// Shared lyrics FontSystem. Iced owns a separate system for ordinary UI text.
static GLOBAL_FONT_SYSTEM: LazyLock<RwLock<Option<SharedFontSystem>>> =
    LazyLock::new(|| RwLock::new(None));

/// 注册全局 FontSystem（在 `init_font_system()` 完成后调用）
pub fn set_global_font_system(fs: SharedFontSystem) {
    *GLOBAL_FONT_SYSTEM.write() = Some(fs);
}

/// 获取全局 FontSystem
pub fn global_font_system() -> SharedFontSystem {
    GLOBAL_FONT_SYSTEM
        .read()
        .as_ref()
        .cloned()
        .expect("Global FontSystem not initialized. Call set_global_font_system first.")
}

/// A completed empty glyph is a cache hit too. Per-key cells prevent a preload
/// and the renderer from rasterizing the same glyph concurrently.
type GlyphBitmap = Option<Arc<SdfBitmap>>;
type GlyphCell = Arc<OnceLock<GlyphBitmap>>;

#[derive(Default)]
struct GlyphBitmapCache {
    entries: HashMap<CacheKey, GlyphCell>,
    insertion_order: VecDeque<CacheKey>,
}

const MAX_CACHED_BITMAPS: usize = 8192;
static GLOBAL_BITMAPS: LazyLock<Mutex<GlyphBitmapCache>> = LazyLock::new(Mutex::default);
static GLOBAL_SWASH: LazyLock<Mutex<SwashCache>> = LazyLock::new(|| Mutex::new(SwashCache::new()));

impl GlyphBitmapCache {
    fn cell(&mut self, key: CacheKey) -> GlyphCell {
        let key = base_sdf_cache_key(key);
        if let Some(cell) = self.entries.get(&key) {
            return cell.clone();
        }
        if self.entries.len() >= MAX_CACHED_BITMAPS {
            // Do not evict a generating or borrowed cell: it owns single-flight.
            let evictable = self.insertion_order.iter().position(|key| {
                self.entries
                    .get(key)
                    .is_some_and(|cell| Arc::strong_count(cell) == 1)
            });
            if let Some(index) = evictable {
                let old = self.insertion_order.remove(index).expect("eviction index");
                self.entries.remove(&old);
            } else {
                // Keep retention bounded even with an exceptional all-in-flight batch.
                return Arc::new(OnceLock::new());
            }
        }
        let cell = Arc::new(OnceLock::new());
        self.entries.insert(key, cell.clone());
        self.insertion_order.push_back(key);
        cell
    }
}

fn base_sdf_cache_key(cache_key: CacheKey) -> CacheKey {
    CacheKey {
        font_size_bits: (SDF_BASE_SIZE as f32).to_bits(),
        // Geometry already retains cosmic-text's floating position and offsets.
        // A single unshifted SDF serves every subpixel position and display size.
        x_bin: SubpixelBin::Zero,
        y_bin: SubpixelBin::Zero,
        ..cache_key
    }
}

fn generate_bitmap(cache_key: CacheKey) -> GlyphBitmap {
    let generator = SdfGenerator::new(SDF_BASE_SIZE, SDF_BUFFER_SIZE);
    let cache_key = CacheKey {
        font_size_bits: (generator.config().base_size as f32).to_bits(),
        ..cache_key
    };
    let image = {
        let mut swash = GLOBAL_SWASH.lock();
        let font_system = global_font_system();
        swash.get_image_uncached(&mut font_system.lock(), cache_key)
    }?;
    generator.generate_from_swash_image(&image).map(Arc::new)
}

fn glyph_bitmap(cache_key: CacheKey) -> GlyphBitmap {
    let key = base_sdf_cache_key(cache_key);
    let cell = GLOBAL_BITMAPS.lock().cell(key);
    cell.get_or_init(|| generate_bitmap(key)).clone()
}

/// 纹理图集大小
/// 4096x4096 可以容纳更多字形，减少清空重建的频率
/// 对于中文歌词，常用汉字约 3000-5000 个，加上标点和英文，4096x4096 足够
const ATLAS_SIZE: u32 = 4096;
const SDF_BASE_SIZE: u32 = 64;
const SDF_BUFFER_SIZE: usize = 12;
/// 字形之间的间距（gutter），防止双线性插值时边缘渗透
/// 4 像素足够防止相邻字形的颜色混合（线性插值需要 1 像素，安全边距 3 像素）
const ATLAS_GUTTER: u32 = 4;

/// 缓存的字形信息
#[derive(Debug, Clone, Copy)]
pub struct SdfGlyphInfo {
    /// 在图集中的 UV 坐标（归一化 0-1）
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// 字形尺寸（像素，来自 cosmic-text Placement）
    pub width: u32,
    pub height: u32,
    /// 相对于基线的偏移（来自 cosmic-text Placement）
    /// left: 字形左边缘相对于笔触原点的偏移
    /// top: 字形顶边缘相对于基线的偏移（正值表示基线以上）
    pub offset_x: i32,
    pub offset_y: i32,
}

/// 字形缓存键
/// 使用 cosmic-text 的 CacheKey 以保持与 TextShaper 的兼容性
pub type SdfGlyphKey = CacheKey;

/// 图集中的行（用于 shelf packing 算法）
struct AtlasRow {
    y: u32,
    height: u32,
    x_cursor: u32,
}

/// SDF 纹理图集
pub struct SdfAtlas {
    /// GPU 纹理（RGB 格式）
    texture: wgpu::Texture,
    /// 缓存的字形信息
    glyphs: HashMap<SdfGlyphKey, SdfGlyphInfo>,
    /// Shelf packing 行
    rows: Vec<AtlasRow>,
    /// 当前 Y 游标
    y_cursor: u32,
    /// 图集尺寸
    width: u32,
    height: u32,
}

impl SdfAtlas {
    /// 创建新的 SDF 图集
    pub fn new(device: &Device) -> Self {
        // 使用 Rgba8Unorm 格式，因为 wgpu 不支持 Rgb8Unorm
        // 我们会在上传时将 RGB 数据转换为 RGBA
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF Glyph Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        Self {
            texture,
            glyphs: HashMap::new(),
            rows: Vec::new(),
            y_cursor: 0,
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
        }
    }

    /// 获取已缓存的字形
    pub fn get(&self, key: &SdfGlyphKey) -> Option<SdfGlyphInfo> {
        self.glyphs.get(key).copied()
    }

    /// 缓存字形
    pub fn cache(
        &mut self,
        queue: &Queue,
        key: SdfGlyphKey,
        bitmap: &SdfBitmap,
    ) -> Option<SdfGlyphInfo> {
        if bitmap.width == 0 || bitmap.height == 0 {
            // 空字形（空格等）
            let info = SdfGlyphInfo {
                uv_min: [0.0, 0.0],
                uv_max: [0.0, 0.0],
                width: 0,
                height: 0,
                offset_x: bitmap.bearing_x,
                offset_y: bitmap.bearing_y,
            };
            self.glyphs.insert(key, info);
            return Some(info);
        }

        // 在图集中分配空间
        let (x, y) = self.allocate(bitmap.width, bitmap.height)?;

        // 将单通道 SDF 数据转换为 RGBA
        let rgba_data = sdf_to_rgba(&bitmap.data);

        // 上传到 GPU
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bitmap.width * 4),
                rows_per_image: Some(bitmap.height),
            },
            wgpu::Extent3d {
                width: bitmap.width,
                height: bitmap.height,
                depth_or_array_layers: 1,
            },
        );

        // 计算 UV 坐标
        let uv_min = [x as f32 / self.width as f32, y as f32 / self.height as f32];
        let uv_max = [
            (x + bitmap.width) as f32 / self.width as f32,
            (y + bitmap.height) as f32 / self.height as f32,
        ];

        let info = SdfGlyphInfo {
            uv_min,
            uv_max,
            width: bitmap.width,
            height: bitmap.height,
            offset_x: bitmap.bearing_x,
            offset_y: bitmap.bearing_y,
        };

        self.glyphs.insert(key, info);
        Some(info)
    }

    /// 使用 shelf packing 算法分配空间
    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let padded_width = width + ATLAS_GUTTER * 2;
        let padded_height = height + ATLAS_GUTTER * 2;

        // 尝试放入现有行
        for row in &mut self.rows {
            if row.height >= padded_height && row.x_cursor + padded_width <= self.width {
                let x = row.x_cursor + ATLAS_GUTTER;
                let y = row.y + ATLAS_GUTTER;
                row.x_cursor += padded_width;
                return Some((x, y));
            }
        }

        // 创建新行
        if self.y_cursor + padded_height <= self.height {
            let row = AtlasRow {
                y: self.y_cursor,
                height: padded_height,
                x_cursor: padded_width,
            };
            let x = ATLAS_GUTTER;
            let y = self.y_cursor + ATLAS_GUTTER;
            self.y_cursor += padded_height;
            self.rows.push(row);
            return Some((x, y));
        }

        None
    }

    /// 清空图集
    pub fn clear(&mut self, queue: &Queue) {
        self.glyphs.clear();
        self.rows.clear();
        self.y_cursor = 0;

        // 清空纹理
        let clear_data = vec![0u8; (self.width * self.height * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &clear_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// 将单通道 SDF 数据转换为 RGBA（R=G=B=SDF, A=255）
fn sdf_to_rgba(sdf: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(sdf.len() * 4);
    for &v in sdf {
        rgba.push(v); // R
        rgba.push(v); // G
        rgba.push(v); // B
        rgba.push(255); // A (完全不透明)
    }
    rgba
}

/// GPU atlas owner. CPU bitmap generation is shared across all consumers.
pub struct SdfCache {
    atlas: Mutex<SdfAtlas>,
    debug_logging: bool,
}

impl SdfCache {
    /// Create with debug logging enabled
    pub fn with_debug(device: &Device, debug_logging: bool) -> Self {
        Self {
            atlas: Mutex::new(SdfAtlas::new(device)),
            debug_logging,
        }
    }

    /// 获取或缓存字形
    ///
    /// Uses shared Swash/SDF results at 64px; geometry scales these metrics.
    ///
    /// 当图集空间不足时，会自动清空图集并重试。
    pub fn get_glyph(&self, queue: &Queue, cache_key: CacheKey) -> Option<SdfGlyphInfo> {
        let atlas_key = base_sdf_cache_key(cache_key);

        // 先检查图集缓存
        {
            let atlas = self.atlas.lock();
            if let Some(info) = atlas.get(&atlas_key) {
                return Some(info);
            }
        }

        // Empty/missing images are retained too, so spaces never rasterize per frame.
        let bitmap = glyph_bitmap(atlas_key)?;

        // 缓存到图集
        let mut atlas = self.atlas.lock();

        // 尝试缓存，如果失败（图集满了），清空后重试
        match atlas.cache(queue, atlas_key, &bitmap) {
            Some(info) => Some(info),
            None => {
                // 图集空间不足，清空后重试
                if self.debug_logging {
                    tracing::info!("[SdfCache] Atlas full, clearing and retrying...");
                }
                atlas.clear(queue);
                atlas.cache(queue, atlas_key, &bitmap)
            }
        }
    }

    /// 获取图集纹理视图
    pub fn atlas_view(&self) -> wgpu::TextureView {
        let atlas = self.atlas.lock();
        atlas
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }
}

/// Warm shared CPU bitmaps before publishing layout. Stop obsolete work between
/// glyphs and publish each result immediately for overlapping song preloads.
pub fn pre_generate_sdf_batch(cache_keys: &[CacheKey], cancelled: &AtomicBool) -> usize {
    let mut seen = HashSet::new();
    let mut generated = 0;
    for &key in cache_keys {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let key = base_sdf_cache_key(key);
        if !seen.insert(key) {
            continue;
        }
        let cell = GLOBAL_BITMAPS.lock().cell(key);
        cell.get_or_init(|| {
            generated += 1;
            generate_bitmap(key)
        });
    }
    generated
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn key(glyph_id: u16) -> CacheKey {
        CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            glyph_id,
            24.0,
            (0.25, 0.75),
            cosmic_text::Weight::NORMAL,
            cosmic_text::CacheKeyFlags::empty(),
        )
        .0
    }

    #[test]
    fn sdf_identity_ignores_display_size_and_phase_but_preserves_weight_and_flags() {
        let original = key(10);
        let changed = CacheKey {
            font_size_bits: 72.0_f32.to_bits(),
            x_bin: SubpixelBin::Three,
            y_bin: SubpixelBin::Zero,
            ..original
        };
        assert_eq!(base_sdf_cache_key(original), base_sdf_cache_key(changed));
        assert_ne!(
            base_sdf_cache_key(original),
            base_sdf_cache_key(CacheKey {
                font_weight: cosmic_text::Weight::BOLD,
                ..original
            })
        );
        assert_ne!(
            base_sdf_cache_key(original),
            base_sdf_cache_key(CacheKey {
                flags: cosmic_text::CacheKeyFlags::FAKE_ITALIC,
                ..original
            })
        );
    }

    #[test]
    fn concurrent_and_repeated_empty_glyphs_generate_once() {
        let cache = Mutex::new(GlyphBitmapCache::default());
        let generated = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..120 {
                        let cell = cache.lock().cell(key(1));
                        assert!(
                            cell.get_or_init(|| {
                                generated.fetch_add(1, Ordering::Relaxed);
                                None
                            })
                            .is_none()
                        );
                    }
                });
            }
        });
        assert_eq!(generated.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn bitmap_retention_is_bounded_and_does_not_evict_an_inflight_cell() {
        let mut cache = GlyphBitmapCache::default();
        let inflight = cache.cell(key(0));
        for glyph in 1..=MAX_CACHED_BITMAPS + 32 {
            cache.cell(key(glyph as u16)).set(None).unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_CACHED_BITMAPS);
        assert!(Arc::ptr_eq(&inflight, &cache.cell(key(0))));
        assert_eq!(cache.insertion_order.len(), MAX_CACHED_BITMAPS);
    }

    #[test]
    fn cancelled_batch_does_not_initialize_fonts_or_rasterize() {
        assert_eq!(pre_generate_sdf_batch(&[key(0)], &AtomicBool::new(true)), 0);
    }
}
