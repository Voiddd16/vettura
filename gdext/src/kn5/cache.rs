//! 按文件共享的 KN5 解码结果缓存。
//!
//! 加载慢的主要来源是**重复解码**：`Kn5File` / `AcCar` / 同一个车目录的第二次
//! `instantiate()` 都会重新解析同一份 `.kn5`，并把每一张内嵌 DDS 重新解压、
//! 重新生成 mip、给每个网格节点各建一份 `StandardMaterial3D`。对一台 30 MB、
//! 32 张贴图、16 个材质、130 个网格节点的车模来说，这些工作绝大部分是重复的。
//!
//! 这里做三级共享，都按文件路径归属：
//!
//! 1. 解析结果 `Arc<Kn5>`：同一文件只解析一次；
//! 2. 贴图 `Vec<Option<Gd<ImageTexture>>>`：按 KN5 贴图下标惰性解码；
//! 3. 材质 `Vec<Option<Gd<StandardMaterial3D>>>`：按 KN5 材质下标惰性创建。
//!
//! 线程模型：Godot 在主线程调用扩展方法，缓存读写只发生在主线程（`Gd` 本身
//! 不允许跨线程，误用会被 gdext 的 safeguards 当场报错而不是静默竞态），所以
//! 注册表用 `thread_local` + `Weak` 持有：没有任何对象引用某个条目时，注册表
//! 只留一个弱引用——既不把贴图永久钉在显存里，也不会无限增长条目。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use godot::classes::{ImageTexture, StandardMaterial3D};
use godot::prelude::*;
use vettura_kn5::Kn5;

use super::texture::TextureKind;

/// 按文件路径共享的缓存。
pub struct FileResources {
    kn5: Arc<Kn5>,
    /// 与 `kn5.textures` 等长；`None` = 尚未解码或解码失败。
    textures: RefCell<Vec<Option<Gd<ImageTexture>>>>,
    /// 与 `kn5.materials` 等长；`None` = 尚未创建或创建失败。
    materials: RefCell<Vec<Option<Gd<StandardMaterial3D>>>>,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<PathBuf, Weak<FileResources>>> = RefCell::new(HashMap::new());
    /// 跨文件贴图去重表：内容哈希 → 已解码的 `ImageTexture`。
    ///
    /// AC 赛道模组会把同一张贴图分别嵌进多个 KN5 里（Nürburgring 的
    /// `curb2.dds`/`global_logos.dds`/`walls_new.dds` 等同时出现在
    /// `fn_nurburgring.kn5` 和 `gp.kn5`，按名统计能省 197 MB 显存）。
    /// Godot 的 `Gd` 没有 `Weak`，这里持强引用，并在 `WorldBody.load_track`
    /// 开头调用 [`clear_texture_cache`]——切换赛道时旧纹理自然释放。
    static TEXTURE_CACHE: RefCell<HashMap<u64, Gd<ImageTexture>>> =
        RefCell::new(HashMap::new());
    /// 去重命中/未命中计数（诊断日志用）。
    static TEXTURE_CACHE_STATS: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
}

/// 清空跨文件贴图去重表（切换赛道前调用，避免旧赛道的纹理常驻显存）。
pub(crate) fn clear_texture_cache() {
    TEXTURE_CACHE.with(|cache| cache.borrow_mut().clear());
    TEXTURE_CACHE_STATS.with(|stats| *stats.borrow_mut() = (0, 0));
}

/// 取去重命中的贴图（`None` = 首次见到这个内容）。
fn cached_texture(hash: u64) -> Option<Gd<ImageTexture>> {
    TEXTURE_CACHE.with(|cache| {
        let hit = cache.borrow().get(&hash).cloned();
        TEXTURE_CACHE_STATS.with(|stats| {
            let mut stats = stats.borrow_mut();
            if hit.is_some() {
                stats.0 += 1;
            } else {
                stats.1 += 1;
            }
        });
        hit
    })
}

fn remember_texture(hash: u64, texture: &Gd<ImageTexture>) {
    TEXTURE_CACHE.with(|cache| {
        cache.borrow_mut().insert(hash, texture.clone());
    });
}

/// 内容哈希：8 字节块混合，速度足够（贴图解码+压缩本身更慢），用途只是
/// 判断“两个 KN5 里同名/同字节的贴图能否共享同一份 GPU 纹理”。
fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0x9e37_79b9_7f4a_7c15u64 ^ bytes.len() as u64;
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        hash ^= u64::from_le_bytes(word).wrapping_mul(0x1000_0000_01b3);
        hash = hash.rotate_left(27).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    hash
}

fn texture_cache_stats() -> (usize, usize) {
    TEXTURE_CACHE_STATS.with(|stats| *stats.borrow())
}

/// 打印跨文件贴图去重的统计（命中数 / 新建数）——模块级函数，不依赖
/// `FileResources` 实例（赛道加载完成后实例可能已被释放）。
pub(crate) fn log_texture_dedup_stats() {
    let (hits, misses) = texture_cache_stats();
    godot_print!(
        "KN5 texture dedup: {} hit(s) shared across files, {} unique decoded",
        hits,
        misses
    );
}

/// 取（或新建）某个文件的共享解码缓存。
///
/// `parse` 只在缓存未命中时调用，因此同一文件不会重复解析；`path` 作为缓存键
/// （先规范化，于是 `res://foo` 与绝对路径撞到同一个条目）。
pub(crate) fn acquire(
    path: &Path,
    parse: impl FnOnce() -> Option<Arc<Kn5>>,
) -> Option<Rc<FileResources>> {
    let key = cache_key(path);
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if let Some(existing) = registry.get(&key).and_then(Weak::upgrade) {
            return Some(existing);
        }

        let kn5 = parse()?;
        let resources = Rc::new(FileResources::new(kn5));
        registry.insert(key, Rc::downgrade(&resources));
        Some(resources)
    })
}

/// 规范化缓存键；无法规范化的路径（尚不存在）退化成原路径。
fn cache_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl FileResources {
    fn new(kn5: Arc<Kn5>) -> Self {
        let textures = vec![None; kn5.textures.len()];
        let materials = vec![None; kn5.materials.len()];
        Self {
            kn5,
            textures: RefCell::new(textures),
            materials: RefCell::new(materials),
        }
    }

    /// 解析后的模型（已被整个桥接层共享，调用方只读）。
    pub fn kn5(&self) -> &Arc<Kn5> {
        &self.kn5
    }

    /// 取贴图；`index` 是 KN5 贴图下标，`kind` 决定 mip 修补方式。
    ///
    /// 首次访问会解码并写入缓存；失败的贴图返回 `None`，材质退化为无贴图。
    pub fn texture(&self, index: usize, kind: TextureKind) -> Option<Gd<ImageTexture>> {
        let texture = self.kn5.textures.get(index)?;
        let slot = &mut self.textures.borrow_mut()[index];
        if let Some(cached) = slot {
            return Some(cached.clone());
        }

        if !texture.active {
            godot_warn!("KN5 texture '{}' is marked inactive", texture.name);
            return None;
        }

        // 跨文件去重：同一张贴图被多个 KN5 各嵌一份时只解一次、只占一份显存。
        let hash = content_hash(texture.data.as_slice());
        if let Some(existing) = cached_texture(hash) {
            *slot = Some(existing.clone());
            return Some(existing);
        }

        let decoded = super::texture::decode(&texture.name, texture.data.as_slice(), kind);
        *slot = decoded.clone();
        if let Some(texture) = decoded.as_ref() {
            remember_texture(hash, texture);
        }
        decoded
    }

    /// 按贴图名取贴图（大小写不敏感，与 KN5 材质槽的写法一致）。
    pub fn texture_by_name(&self, name: &str, kind: TextureKind) -> Option<Gd<ImageTexture>> {
        let lower = name.to_ascii_lowercase();
        let index = self
            .kn5
            .textures
            .iter()
            .position(|texture| texture.name.to_ascii_lowercase() == lower)?;
        self.texture(index, kind)
    }

    /// 取材质；首次访问时创建并写入缓存。
    ///
    /// 同一份模型里的所有网格节点因此共享一份 `StandardMaterial3D`——这正是
    /// Godot 的常规用法（导入资源本来就是共享的），既省内存也省掉重复的属性写入。
    pub fn material(&self, index: usize) -> Option<Gd<StandardMaterial3D>> {
        let material = self.kn5.materials.get(index)?;
        let slot = &mut self.materials.borrow_mut()[index];
        if let Some(cached) = slot {
            return Some(cached.clone());
        }

        let created = super::material::create(material, self);
        *slot = Some(created.clone());
        Some(created)
    }

    /// 当前已解码的贴图数量（探针与诊断用）。
    pub fn decoded_texture_count(&self) -> usize {
        self.textures.borrow().iter().flatten().count()
    }

    /// 模型里是否有材质真的会用 `txDetail` / `txNormalDetail`。
    ///
    /// Godot 的 detail 层采样第二套 UV（`UV2`）；只有确实要生成细节层的模型才
    /// 额外写一份 `TEX_UV2`，否则白费每顶点 8 字节（Daytona 那种百万三角的
    /// 赛道就是几 MB）。判断只依赖 `useDetail` 与槽位，和材质是否创建无关，
    /// 所以在网格构建时就能问。
    pub fn needs_uv2(&self) -> bool {
        const DETAIL_SLOTS: [&str; 2] = ["txDetail", "txNormalDetail"];
        self.kn5.materials.iter().any(|material| {
            material.property("useDetail").unwrap_or(1.0) > 0.5
                && material.samples.iter().any(|sample| {
                    DETAIL_SLOTS
                        .iter()
                        .any(|slot| sample.slot_name.eq_ignore_ascii_case(slot))
                })
        })
    }
}
