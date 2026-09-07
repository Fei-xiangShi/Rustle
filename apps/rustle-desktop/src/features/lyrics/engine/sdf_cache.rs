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
use cosmic_text::{CacheKey, FontSystem, SwashCache};
use iced::wgpu;
use iced::wgpu::{Device, Queue};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

/// 共享字体系统类型
pub type SharedFontSystem = Arc<Mutex<FontSystem>>;

// ---- 全局单例 ----

/// 全局 FontSystem — 整个应用只有一个实例, 字形光栅化由 cosmic-text 内部处理
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

/// 全局预生成缓存
/// 用于在后台线程生成 SDF 位图后，在主线程导入到 SdfCache
static GLOBAL_PRE_GENERATED: LazyLock<Mutex<HashMap<CacheKey, SdfBitmap>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 导入预生成的位图到全局缓存
pub fn import_to_global_cache(bitmaps: HashMap<CacheKey, SdfBitmap>) {
    let mut cache = GLOBAL_PRE_GENERATED.lock();
    for (key, bitmap) in bitmaps {
        cache.insert(base_sdf_cache_key(key), bitmap);
    }
}

/// 从全局缓存中获取预生成的位图。
///
/// The global cache is intentionally reusable: shaping can be requested more
/// than once for the same viewport/song window, and SDF generation is much more
/// expensive than cloning the already generated bitmap.
pub fn get_from_global_cache(key: &CacheKey) -> Option<SdfBitmap> {
    GLOBAL_PRE_GENERATED
        .lock()
        .get(&base_sdf_cache_key(*key))
        .cloned()
}

fn base_sdf_cache_key(cache_key: CacheKey) -> CacheKey {
    CacheKey {
        font_size_bits: (SDF_BASE_SIZE as f32).to_bits(),
        ..cache_key
    }
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

/// 预生成的 SDF 位图（用于异步生成）
#[derive(Clone)]
pub struct PreGeneratedSdf {
    pub bitmap: SdfBitmap,
}

/// 线程安全的 SDF 缓存管理器
pub struct SdfCache {
    /// SDF 生成器
    generator: SdfGenerator,
    /// 纹理图集
    atlas: Mutex<SdfAtlas>,
    /// Enable debug logging
    debug_logging: bool,
    /// 预生成的 SDF 位图缓存（用于异步生成后在主线程上传）
    pre_generated: Mutex<HashMap<CacheKey, PreGeneratedSdf>>,
}

impl SdfCache {
    fn sdf_cache_key(cache_key: CacheKey, base_size: u32) -> CacheKey {
        CacheKey {
            font_size_bits: (base_size as f32).to_bits(),
            ..cache_key
        }
    }

    fn generate_bitmap_from_swash(
        &self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
    ) -> Option<SdfBitmap> {
        let base_key = Self::sdf_cache_key(cache_key, self.generator.config().base_size);
        let mut swash_cache = SwashCache::new();
        let image = swash_cache.get_image_uncached(font_system, base_key)?;
        self.generator.generate_from_swash_image(&image)
    }

    /// Create with debug logging enabled
    pub fn with_debug(device: &Device, debug_logging: bool) -> Self {
        Self {
            // base_size = 64px, buffer = 12px
            // 64px 是速度和质量的平衡点：
            // - 比 96px 快约 2 倍
            // - 质量足够好，适合大多数显示器
            // - 更大的 buffer 能给 glow/blur 留出真实采样空间，减少矩形外推带来的颗粒感
            generator: SdfGenerator::new(SDF_BASE_SIZE, SDF_BUFFER_SIZE),
            atlas: Mutex::new(SdfAtlas::new(device)),
            debug_logging,
            pre_generated: Mutex::new(HashMap::new()),
        }
    }

    /// 获取或缓存字形
    ///
    /// 使用 msdfgen 生成 SDF 纹理和度量。
    /// 度量是在 base_size (48px) 下计算的，需要在渲染时缩放。
    ///
    /// 当图集空间不足时，会自动清空图集并重试。
    pub fn get_glyph(&self, queue: &Queue, cache_key: CacheKey) -> Option<SdfGlyphInfo> {
        let atlas_key = Self::sdf_cache_key(cache_key, self.generator.config().base_size);

        // 先检查图集缓存
        {
            let atlas = self.atlas.lock();
            if let Some(info) = atlas.get(&atlas_key) {
                return Some(info);
            }
        }

        // 检查本地预生成缓存
        let pre_gen_bitmap = {
            let mut pre_gen = self.pre_generated.lock();
            pre_gen.remove(&atlas_key)
        };

        // 如果本地缓存没有，检查全局预生成缓存
        let pre_gen_bitmap = pre_gen_bitmap
            .or_else(|| get_from_global_cache(&atlas_key).map(|bitmap| PreGeneratedSdf { bitmap }));

        let bitmap = if let Some(pre_gen) = pre_gen_bitmap {
            // 使用预生成的位图（快速路径）
            pre_gen.bitmap
        } else {
            // 需要同步生成（慢速路径）
            let font_system = global_font_system();
            let mut font_system = font_system.lock();

            // Check if font_id exists in the font system
            let font_info = font_system.db().face(cache_key.font_id);
            if font_info.is_none() && self.debug_logging {
                tracing::warn!(
                    "[SdfCache] Font mismatch: font_id {:?} not found in font system",
                    cache_key.font_id
                );
            }

            // 生成 SDF（字形光栅化由 cosmic-text 的 SwashCache 内部处理）
            self.generate_bitmap_from_swash(&mut font_system, cache_key)?
        };

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

/// 批量预生成 SDF 位图（无 GPU，可在后台线程使用）
pub fn pre_generate_sdf_batch(cache_keys: &[CacheKey]) -> HashMap<CacheKey, SdfBitmap> {
    let generator = SdfGenerator::new(SDF_BASE_SIZE, SDF_BUFFER_SIZE);
    let mut bitmaps = HashMap::new();
    let mut seen_keys = HashSet::new();
    let cached_keys: HashSet<CacheKey> = GLOBAL_PRE_GENERATED.lock().keys().copied().collect();
    let font_system = global_font_system();
    let mut swash_cache = SwashCache::new();

    for &cache_key in cache_keys {
        let sdf_key = base_sdf_cache_key(cache_key);

        if !seen_keys.insert(sdf_key) || cached_keys.contains(&sdf_key) {
            continue;
        }

        let image = {
            let mut font_system = font_system.lock();
            if font_system.db().face(sdf_key.font_id).is_none() {
                continue;
            }
            swash_cache.get_image_uncached(&mut font_system, sdf_key)
        };
        let Some(image) = image else {
            continue;
        };

        let Some(bitmap) = generator.generate_from_swash_image(&image) else {
            continue;
        };
        bitmaps.insert(sdf_key, bitmap);
    }

    bitmaps
}
