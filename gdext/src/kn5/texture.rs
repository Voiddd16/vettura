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

use godot::classes::{Image, ImageTexture};
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

    // KN5 的导出器经常写出“flags 没声明 PITCH/LINEARSIZE、值却非零”的头，
    // Godot 会直接拒绝（Daytona 赛道 42/274 张贴图中招，看台全黑）。先修头，
    // 后续无论哪条路径都用修正后的字节。
    let fixed = vettura_kn5::fix_orphan_pitch_value(bytes);
    let bytes = fixed.as_deref().unwrap_or(bytes);

    // 1) 原样尝试：完整链的贴图可以直接保留压缩格式与原有 mip。
    if let Some(image) = load_dds(bytes) {
        return wrap(&image);
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

    // 所有修复路径都失败才报错——中间步骤的失败是预期的试错，逐条打 error
    // 会让日志里一屏 ERROR 却其实贴图都救回来了（Daytona 上实测 0 张最终失败）。
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
    wrap(&image)
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
fn wrap(image: &Gd<Image>) -> Option<Gd<ImageTexture>> {
    let texture = ImageTexture::create_from_image(image);
    if texture.is_none() {
        godot_error!("failed to wrap decoded DDS into ImageTexture");
    }
    texture
}
