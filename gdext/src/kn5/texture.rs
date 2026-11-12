//! KN5 内嵌 DDS 贴图的解码。
//!
//! KN5 的贴图是原始 DDS 字节（DXT1/DXT3/DXT5、ATI1/ATI2、DX10 头里的 BC7、
//! 未压缩 RGB 都可能出现）。Godot 的 `Image::load_dds_from_buffer` 本身能处理
//! 这些格式，但对 **mip 链的完整性很严格**：只要头部声明了 mip，就必须提供完整
//! 的 1x1 链；KN5 的导出器经常裁剪到 2x2 就停止（例如试样里的 `flat_nm.dds`），
//! 于是 Godot 报 “Expected Image data size … got … instead” 并返回一张空图。
//!
//! 这里分两步走：
//!
//! 0. 先做头部自洽性修正（[`vettura_kn5::fix_orphan_pitch_value`]）：部分导出器
//!    写的 flags 没声明 PITCH/LINEARSIZE、值却非零，Godot 会直接拒绝整张贴图；
//! 1. 再按原样加载——完整链的贴图（尤其是大尺寸 diffuse）可以直接保留压缩格式
//!    与原有 mip，省显存；
//! 2. 失败（或拿到空图）时把头部改成“只有基础层”（[`strip_mip_levels`]），再
//!    解压缩并让 Godot 重新生成 mip；法线类贴图会开启 `renormalize`，避免逐级
//!    降采样后法线长度衰减。
//!
//! 解码结果按贴图名缓存（[`super::cache::FileResources`]），重复引用不会重复
//! 解码。

pub use vettura_kn5::strip_mip_levels;

use godot::classes::{Image, ImageTexture};
use godot::global::Error;
use godot::prelude::*;
use vettura_kn5::dds_layout;

/// 贴图用途；决定 mip 重新生成的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    Diffuse,
    Normal,
    Mask,
    Detail,
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
    // 后续无论原样加载还是“砍 mip 再加载”都用修正后的字节。
    let fixed = vettura_kn5::fix_orphan_pitch_value(bytes);
    let bytes = fixed.as_deref().unwrap_or(bytes);

    // KN5 的压缩贴图经常少了最小的几级 mip，Godot 会整张贴图拒绝。这里先看头部
    // 判断，直接走“只有基础层”的路径，省掉一次注定失败的调用（它还会往 stderr
    // 报错，虽然结果一样，但很吵）。
    if needs_mip_repair(bytes) {
        return decode_regenerated(name, &strip_mip_levels(bytes)?, kind);
    }

    // 1) 原样尝试：完整链的贴图可以直接保留压缩格式与原有 mip。
    if let Some(image) = load_dds(bytes) {
        return wrap(&image);
    }

    // 2) 其它失败输入（未知格式、非标准 pitch…）：同样退回单层 + 重新生成 mip。
    match strip_mip_levels(bytes) {
        Some(stripped) => decode_regenerated(name, &stripped, kind),
        None => None,
    }
}

/// 压缩格式、声明了多级 mip、但 payload 装不下完整链 → 需要走退化路径。
fn needs_mip_repair(bytes: &[u8]) -> bool {
    match dds_layout(bytes) {
        Ok(layout) => {
            layout.bytes_per_block.is_some()
                && layout.declared_levels >= 2
                && layout.payload_len < layout.full_chain_size()
        }
        Err(_) => false,
    }
}

/// “只有基础层”的 DDS → 加载 + 解压缩 + 重新生成 mip。
fn decode_regenerated(name: &str, stripped: &[u8], kind: TextureKind) -> Option<Gd<ImageTexture>> {
    let mut image = load_dds(stripped)?;
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

/// 交给 Godot 加载。Godot 在部分损坏输入上会返回 `OK` 但图像为空，所以这里
/// 额外检查 `is_empty`。
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
