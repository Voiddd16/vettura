//! KN5 内嵌 DDS 贴图的解码。
//!
//! KN5 的贴图是原始 DDS 字节（DXT1/DXT3/DXT5、ATI1/ATI2、DX10 头里的 BC7、
//! 未压缩 RGB 都可能出现）。Godot 的 `Image::load_dds_from_buffer` 本身能处理
//! 这些格式，但对 **payload 与头部声明的 mip 级数是否精确吻合**非常严格：多一
//! 个字节少一个字节都整张贴图拒绝。KN5 导出器的实际出线法五花八门：
//!
//! - 链裁剪到 2x2 就停（`flat_nm.dds`），声明 5 级只给 2 级数据；
//! - 声明 5 级却塞了接近 12 级的尾随字节（`trees_new.dds`）；
//! - flags 没声明 PITCH/LINEARSIZE、`dwPitchOrLinearSize` 却非零（Daytona
//!   42/274 张贴图中招，看台全黑）；
//! - 未压缩（`fourcc = 0`）且 mip 链被裁剪（`water_NM.dds`）——旧版只修压缩
//!   格式，这类整张贴图拿不到。
//!
//! 修复顺序（每一级都实测过，见 `tex_audit2.csv` 的统计）：
//!
//! 0. 头部自洽性修正（[`vettura_kn5::fix_orphan_pitch_value`]）；
//! 1. 原样加载——完整链的贴图直接保留压缩格式与原有 mip，省显存；
//! 2. 失败则“只有基础层”的头（[`strip_mip_levels`]，payload 不动）——Godot
//!    对 mipcount=1 的输入只按基础层算账，能救回链被裁剪的贴图；
//! 3. 再失败则连 payload 一起截到基础层（[`truncate_to_base_level`]）——少数
//!    只认“长度精确等于基础层”的输入；
//! 4. 再失败则截到“头部声明级数”刚好容纳的大小（[`truncate_to_declared_mips`]）
//!    ——救的是声明 5 级却塞了 12 级数据的那批。
//!
//! 所有退化路径都会解压缩 + 让 Godot 重新生成 mip；法线类贴图开启
//! `renormalize`，避免逐级降采样后法线长度衰减。解码结果按贴图名缓存
//! （[`super::cache::FileResources`]），重复引用不会重复解码。

pub use vettura_kn5::strip_mip_levels;

use godot::classes::{DisplayServer, Image, ImageTexture};
use godot::global::Error;
use godot::prelude::*;

/// 贴图用途；决定 mip 重新生成的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// 漫反射/基础贴图：直接加载，完整 mip 链原样保留。
    Diffuse,
    /// 法线贴图：重建 mip 时重新归一化。
    Normal,
    /// `txMaps`（AC 的 AO/mask 打包贴图）：按单个通道使用。
    Mask,
    /// `txDetail`：AC 的细节/涂装层，乘进漫反射（车漆的红色就来自这里）。
    Detail,
    /// `txNormalDetail`：细节法线。
    DetailNormal,
}

impl TextureKind {
    /// 法线类贴图在生成 mip 时需要重新归一化。
    const fn needs_renormalize(self) -> bool {
        matches!(self, Self::Normal | Self::DetailNormal)
    }
}

/// DDS 字节 → `ImageTexture`。失败时打印告警并返回 `None`（材质退化为无贴图）。
pub(crate) fn decode(name: &str, bytes: &[u8], kind: TextureKind) -> Option<Gd<ImageTexture>> {
    if bytes.is_empty() {
        godot_warn!("embedded KN5 texture has no data");
        return None;
    }

    // 现代 mod（Nürburgring 等）的 KN5 里会直接嵌 PNG / JPEG；以前的导出器
    // 只嵌 DDS。按 magic 分派，别把 PNG 喂给 DDS 修复链（白费 5 次试错）。
    if bytes.starts_with(b"\x89PNG") {
        let mut image = Image::new_gd();
        if image.load_png_from_buffer(&PackedByteArray::from(bytes)) == Error::OK
            && !image.is_empty()
        {
            return wrap(&image, kind);
        }
        godot_warn!("embedded PNG '{}' failed to load", name);
        return None;
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut image = Image::new_gd();
        if image.load_jpg_from_buffer(&PackedByteArray::from(bytes)) == Error::OK
            && !image.is_empty()
        {
            return wrap(&image, kind);
        }
        godot_warn!("embedded JPEG '{}' failed to load", name);
        return None;
    }

    // KN5 的导出器经常写出“flags 没声明 PITCH/LINEARSIZE、值却非零”的头，
    // Godot 会直接拒绝（Daytona 赛道 42/274 张贴图中招，看台全黑）。先修头，
    // 后续无论哪条路径都用修正后的字节。
    let fixed = vettura_kn5::fix_orphan_pitch_value(bytes);
    let bytes = fixed.as_deref().unwrap_or(bytes);

    // 1) 先按头部与实际 payload 对齐再交给 Godot。Godot 只要看到 mip 声明，
    //    就要求“一直缩到 1x1 的完整链 + 精确长度”；KN5 常裁到 2x2 或带尾随
    //    字节，于是打印 "Expected Image data size ... got ... instead"（有时
    //    还返回半张图）。非完整链直接走“只有基础层”的已知可用路径，既避免
    //    日志噪音，也保证拿到完整的基础层。
    let mut pre_handled = false;
    if let Ok(layout) = vettura_kn5::dds_layout(bytes) {
        if layout.declared_levels > 1 {
            // 逐级扣减，算出 payload 实际装得下的级数（压缩/未压缩都适用）；
            // 有尾随字节（remaining > 0）也视为“不精确”，Godot 同样会报错。
            let mut remaining = layout.payload_len;
            let mut levels = 0u32;
            for level in 0..layout.max_levels() {
                let need = layout.level_bytes(level);
                if need > remaining {
                    break;
                }
                remaining -= need;
                levels += 1;
            }
            let exact = levels == layout.max_levels()
                && remaining == 0
                && layout.declared_levels == layout.max_levels();
            if !exact {
                pre_handled = true;
                if let Some(stripped) = strip_mip_levels(bytes) {
                    if let Some(image) = load_dds(&stripped) {
                        return decompress_and_wrap(name, image, kind);
                    }
                }
                if let Some(trimmed) = vettura_kn5::truncate_to_base_level(bytes) {
                    if let Some(image) = load_dds(&trimmed) {
                        return decompress_and_wrap(name, image, kind);
                    }
                }
            }
        }
    }
    if !pre_handled {
        // 完整链（或无法解析头）：原样加载，保留压缩格式与原有 mip。
        if let Some(image) = load_dds(bytes) {
            return wrap(&image, kind);
        }
    }

    // 2) “只有基础层”的头，payload 原样保留：Godot 对 mipcount=1 的输入只按
    //    基础层算账，链被裁剪（声明多级、payload 却装不下）的贴图靠这个救回。
    if let Some(stripped) = strip_mip_levels(bytes) {
        if let Some(image) = load_dds(&stripped) {
            return decompress_and_wrap(name, image, kind);
        }

        // 3) payload 截到基础层大小：只认“长度精确等于基础层”的少输入。
        if let Some(trimmed) = vettura_kn5::truncate_to_base_level(&stripped) {
            if let Some(image) = load_dds(&trimmed) {
                return decompress_and_wrap(name, image, kind);
            }
        }
    } else if let Some(trimmed) = vettura_kn5::truncate_to_base_level(bytes) {
        // 头部本来就声明 1 级（strip 不下手）但 payload 明显超长。
        if let Some(image) = load_dds(&trimmed) {
            return decompress_and_wrap(name, image, kind);
        }
    }

    // 4) 截到“声明的 mip 级数”刚好容纳的大小：救的是声明 5 级却塞了接近 12 级
    //    数据的那类贴图（payload 超长，Godot 按声明的级数核账）。
    if let Some(trimmed) = vettura_kn5::truncate_to_declared_mips(bytes) {
        if let Some(image) = load_dds(&trimmed) {
            return decompress_and_wrap(name, image, kind);
        }
    }

    // 所有修复路径都失败 → 最后的兜底：Rust 侧直接把 base level 的 BC 块解成
    // RGBA8（`decode_base_rgba`），绕过 Godot 挑剔的 DDS 加载器。Nürburgring
    // 的 `newPanels.dds`（2127x591 非 4 倍数宽）无论怎么修头都被 Godot 拒绝，
    // 只有这条路径能救回——正是“覆盖更多赛道 mod”的关键。
    if let Some(decoded) = vettura_kn5::decode_base_rgba(bytes) {
        let image = Image::create_from_data(
            decoded.width as i32,
            decoded.height as i32,
            false,
            godot::classes::image::Format::RGBA8,
            &PackedByteArray::from(decoded.pixels.as_slice()),
        );
        let Some(image) = image else {
            godot_warn!("embedded KN5 texture '{}' decoded but Image is null", name);
            return None;
        };
        if image.is_empty() {
            godot_warn!("embedded KN5 texture '{}' decoded but Image is empty", name);
            return None;
        }
        return decompress_and_wrap(name, image, kind);
    }

    // 真救不回来才报错——中间步骤的失败是预期的试错，逐条打 error 会让日志里
    // 一屏 ERROR 却其实贴图都救回来了（Daytona 上实测 0 张最终失败）。
    let dims = match vettura_kn5::dds_layout(bytes) {
        Ok(layout) => format!(
            "{}x{} mip_declared={} payload={} full_chain={}",
            layout.width,
            layout.height,
            layout.declared_levels,
            layout.payload_len,
            layout.full_chain_size()
        ),
        Err(err) => format!("header invalid: {err}"),
    };
    godot_error!(
        "embedded KN5 texture '{}' unusable after all repair paths: {} bytes, {}",
        name,
        bytes.len(),
        dims
    );
    None
}

fn decompress_and_wrap(
    name: &str,
    mut image: Gd<Image>,
    kind: TextureKind,
) -> Option<Gd<ImageTexture>> {
    if image.is_compressed() && image.decompress() != Error::OK {
        godot_warn!("failed to decompress embedded DDS '{}'", name);
        return None;
    }
    if image
        .generate_mipmaps_ex()
        .renormalize(kind.needs_renormalize())
        .done()
        != Error::OK
    {
        godot_warn!("failed to regenerate mipmaps for '{}'", name);
    }

    // 显存优化：退化路径的贴图现在是未压缩 RGBA8（4 B/px），Daytona 那种
    // 几百张大贴图会吃掉数百 MB 显存。按平台重新压回 GPU 友好的格式：
    // 桌面 S3TC（BC1/BC3，0.5~1 B/px），移动 ETC2（Godot 的 compress 只支持
    // 这两类）。压缩失败就保留 RGBA8，不影响显示。原样加载成功的贴图
    // （走 `wrap` 而非这里）保留 DDS 原格式，这条路径只处理“修过头”的贴图。
    //
    // **法线贴图例外**：桌面上保留 RGBA8 不重压。BC3 的 RGB 只有 8 位端点
    // 插值，法线方向会被量化到明显失真的程度（高光/凹凸细节发脏）；法线贴图
    // 数量远少于漫反射，占的显存可以接受。移动端仍压 ETC2（显存优先）。
    let is_normal = kind.needs_renormalize();
    let desktop = !matches!(
        godot::classes::Os::singleton()
            .get_name()
            .to_string()
            .as_str(),
        "Android" | "iOS"
    );
    if !(is_normal && desktop) {
        if let Some(mode) = platform_compress_mode() {
            if image.compress(mode) != Error::OK {
                godot_warn!(
                    "failed to re-compress repaired texture '{}' to GPU format; keeping RGBA8",
                    name
                );
            }
        }
    }
    wrap(&image, kind)
}

/// 按运行平台选择运行时纹理压缩模式。
///
/// - 桌面（macOS / Windows / Linux）：S3TC（BC1/BC3）——所有桌面 GPU 都支持；
/// - Android / iOS：ETC2——移动 GPU 的通用压缩格式（Godot 的 ASTC 需要
///   `Image.compress` 的 ASTC 支持与硬件匹配，ETC2 更稳）。
///
/// headless / 无渲染设备时返回 `None`，避免无意义地压缩（测试环境）。
fn platform_compress_mode() -> Option<godot::classes::image::CompressMode> {
    use godot::classes::image::CompressMode;
    if DisplayServer::singleton().get_name() == "headless" {
        return None;
    }
    let os = godot::classes::Os::singleton().get_name();
    let os = os.to_string();
    match os.as_str() {
        "Android" | "iOS" => Some(CompressMode::ETC2),
        _ => Some(CompressMode::S3TC),
    }
}

/// 交给 Godot 加载（静默失败——调用方是“逐条试错”的修复链，失败本身不打日志，
/// 只有 [`decode`] 所有路径都失败时才统一报一条）。Godot 在部分损坏输入上会
/// 返回 `OK` 但图像为空，所以这里额外检查 `is_empty`。
fn load_dds(bytes: &[u8]) -> Option<Gd<Image>> {
    let mut image = Image::new_gd();
    if image.load_dds_from_buffer(&PackedByteArray::from(bytes)) != Error::OK || image.is_empty() {
        return None;
    }
    Some(image)
}

/// `Image` → `ImageTexture`（可直接赋给材质）。
fn wrap(image: &Gd<Image>, kind: TextureKind) -> Option<Gd<ImageTexture>> {
    // 尺寸上限（移动端 1024 / `VETTURA_MAX_TEXTURE` 覆盖）：Nordschleife 这类
    // 超大赛道光贴图就要 1.7 GB（桌面可以承受，移动端会 OOM）。
    let image = maybe_downscale(image, kind);
    let texture = ImageTexture::create_from_image(&image);
    if texture.is_none() {
        godot_error!("failed to wrap decoded DDS into ImageTexture");
    }
    texture
}

/// 贴图尺寸上限（按平台 + 可选环境变量 `VETTURA_MAX_TEXTURE` 覆盖）。
///
/// - 移动端默认 1024（显存/带宽优先）；
/// - 桌面默认 0 = 不限（精度优先，4096 贴图保留原尺寸）；
/// - 显存紧张的用户可以设 `VETTURA_MAX_TEXTURE=2048` 全局降采样。
fn texture_size_limit() -> i32 {
    let override_limit = std::env::var("VETTURA_MAX_TEXTURE")
        .ok()
        .and_then(|raw| raw.trim().parse::<i32>().ok())
        .filter(|value| *value > 0);
    if let Some(limit) = override_limit {
        return limit;
    }
    let os = godot::classes::Os::singleton().get_name().to_string();
    if os == "Android" || os == "iOS" {
        1024
    } else {
        0
    }
}

/// 按平台/配置把超限贴图等比降采样；不需要降采样时直接克隆（零拷贝）。
///
/// 注意：Godot 的 `Image.resize` **对压缩图像无效**，而原样加载成功的 DDS
/// 正是压缩格式——必须先解压再缩放，缩放后按 kind/平台决定是否重压回去。
fn maybe_downscale(image: &Gd<Image>, kind: TextureKind) -> Gd<Image> {
    let max_side = texture_size_limit();
    let (width, height) = (image.get_width(), image.get_height());
    if max_side <= 0 || (width <= max_side && height <= max_side) {
        return image.clone();
    }
    let mut scaled = image.clone();
    let was_compressed = scaled.is_compressed();
    if was_compressed && scaled.decompress() != Error::OK {
        return image.clone();
    }
    let scale = max_side as f32 / width.max(height) as f32;
    let target_w = ((width as f32 * scale).round() as i32).max(1);
    let target_h = ((height as f32 * scale).round() as i32).max(1);
    scaled
        .resize_ex(target_w, target_h)
        .interpolation(godot::classes::image::Interpolation::LANCZOS)
        .done();
    if was_compressed {
        // 原本是压缩格式的，缩放后按同样规则重压（法线贴图桌面保留 RGBA8）。
        let desktop = !matches!(
            godot::classes::Os::singleton()
                .get_name()
                .to_string()
                .as_str(),
            "Android" | "iOS"
        );
        if !(kind.needs_renormalize() && desktop) {
            if let Some(mode) = platform_compress_mode() {
                let _ = scaled.compress(mode);
            }
        }
    }
    scaled
}
