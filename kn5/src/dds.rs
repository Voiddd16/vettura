//! KN5 内嵌 DDS 贴图的最小头部检查。
//!
//! 这里刻意不解码像素：Godot 层可以把原始 DDS 字节交给
//! `Image::load_dds_from_buffer`，检查工具也只需要尺寸和格式元数据。
//!
//! 另外提供两个“让严格的加载器（Godot）接受”的头部修正工具：
//! [`strip_mip_levels`] 处理**KN5 的 DDS 写到底 2x2 为止**但规范期望完整 1x1
//! 链的情况（否则报 “Expected Image data size … got … instead”）；
//! [`fix_orphan_pitch_value`] 处理 flags 没声明 PITCH/LINEARSIZE 却写了非零
//! 值的自相矛盾头（否则报 “DDS header flags specify that no linear size …”）。

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    pub mipmaps: u32,
    pub fourcc: [u8; 4],
}

impl DdsInfo {
    /// 当头部使用字符串 FourCC（例如 `DXT5`）时返回它。
    pub fn fourcc_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.fourcc).ok()
    }
}

/// DDS 压缩格式（只覆盖 KN5 里实际出现的类型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsFormat {
    /// `DXT1` FourCC。BC1：4:1 压缩，RGB，无 alpha（或 1-bit alpha）。
    Bc1,
    /// `DXT3` FourCC（BC2）。BC2：4:1 压缩，RGBA，alpha 非预乘。
    Bc2,
    /// `DXT5` FourCC（BC3）。BC3：4:1 压缩，RGBA，alpha 插值——KN5 最常用。
    Bc3,
    /// `ATI1` FourCC（BC4）。BC4：2:1 压缩，单通道（法线贴图的 R）。
    Bc4,
    /// `ATI2` FourCC（BC5）。BC5：2:1 压缩，双通道（法线贴图的 RG）。
    Bc5,
    /// DX10 头里的 BC6/BC7（dxgi 71-101）：BC6 为 HDR，BC7 为高质量 RGBA。
    Bc7,
    /// 未压缩或无 FourCC（RGB/RGBA/float…），按 4 字节/像素处理。
    Uncompressed,
}

impl DdsFormat {
    /// 每个 4x4 块的字节数；未压缩返回 `None`。
    pub const fn bytes_per_block(self) -> Option<u32> {
        match self {
            Self::Bc1 | Self::Bc4 => Some(8),
            Self::Bc2 | Self::Bc3 | Self::Bc5 | Self::Bc7 => Some(16),
            Self::Uncompressed => None,
        }
    }
}

/// DDS 文件的完整头部描述：尺寸、mip 级数、payload 偏移与长度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsLayout {
    pub width: u32,
    pub height: u32,
    /// 头部声明的额外 mip 级数（不含基础层）：`dwMipMapCount`。
    pub declared_levels: u32,
    /// payload 在文件中的起始偏移（DX10 头为 148，其它为 128）。
    pub payload_offset: usize,
    /// payload 的字节长度。
    pub payload_len: usize,
    pub format: DdsFormat,
    pub bytes_per_block: Option<u32>,
}

impl DdsLayout {
    /// 头部位每像素字节数（未压缩格式；压缩格式为 `None`）。
    pub const fn bytes_per_pixel(&self) -> Option<u32> {
        match self.format {
            DdsFormat::Bc1
            | DdsFormat::Bc2
            | DdsFormat::Bc3
            | DdsFormat::Bc4
            | DdsFormat::Bc5
            | DdsFormat::Bc7 => None,
            DdsFormat::Uncompressed => Some(4),
        }
    }

    /// 维度上最多可以有多少级 mip（含基础层）。
    pub fn max_levels(&self) -> u32 {
        levels_for(self.width, self.height)
    }

    /// 完整 mip 链（一直缩到 1x1）需要的 payload 字节数。
    pub fn full_chain_size(&self) -> usize {
        (0..self.max_levels())
            .map(|level| self.level_bytes(level))
            .sum()
    }

    /// payload 实际能容纳的 mip 级数（含基础层）。
    pub fn available_levels(&self) -> u32 {
        if self.bytes_per_block.is_none() {
            return 1;
        }

        let mut levels = 0;
        let mut remaining = self.payload_len;
        loop {
            remaining = match remaining.checked_sub(self.level_bytes(levels)) {
                Some(next) => next,
                None => break,
            };
            levels += 1;
            if levels >= self.max_levels() {
                break;
            }
        }
        levels
    }

    /// 第 `level` 级 mip 的字节数（`level` 为 0 表示基础层）。
    pub fn level_bytes(&self, level: u32) -> usize {
        let width = self.width >> level.min(31);
        let height = self.height >> level.min(31);
        match self.bytes_per_block {
            Some(block) => {
                let blocks = width.div_ceil(4) as usize * height.div_ceil(4) as usize;
                blocks * block as usize
            }
            None => width as usize * height as usize * self.bytes_per_pixel().unwrap_or(4) as usize,
        }
    }
}

/// 维度上最多可以有多少级 mip（含基础层）。
fn levels_for(width: u32, height: u32) -> u32 {
    let max_side = width.max(height);
    32 - max_side.leading_zeros()
}

/// 取前 4 个字节组成 FourCC 数组。
fn fourcc_to_array(bytes: &[u8]) -> [u8; 4] {
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(&bytes[..4]);
    fourcc
}

/// 从 DDS 字节推导 [`DdsLayout`]。
pub fn layout(data: &[u8]) -> Result<DdsLayout, DdsError> {
    const HEADER_LEN: usize = 128;
    if data.len() < HEADER_LEN {
        return Err(DdsError::TooShort(data.len()));
    }
    if &data[..4] != b"DDS " {
        return Err(DdsError::BadMagic);
    }

    let u32_at = |offset: usize| {
        data[offset..offset + 4]
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| DdsError::TruncatedField)
    };

    let height = u32_at(12)?;
    let width = u32_at(16)?;
    if width == 0 || height == 0 {
        // 零尺寸会让 `levels_for` / `level_bytes` 算出 0 字节的一级，
        // 也会让 Godot 侧拿到空图，直接拒绝。
        return Err(DdsError::BadDimensions(width, height));
    }
    let declared_levels = u32_at(28)?;
    let fourcc = &data[84..88];

    let (format, payload_offset) = match fourcc {
        b"DXT1" => (DdsFormat::Bc1, HEADER_LEN),
        b"DXT3" => (DdsFormat::Bc2, HEADER_LEN),
        b"DXT5" => (DdsFormat::Bc3, HEADER_LEN),
        b"ATI1" | b"BC4U" => (DdsFormat::Bc4, HEADER_LEN),
        b"ATI2" | b"BC5U" => (DdsFormat::Bc5, HEADER_LEN),
        b"DX10" => {
            // DX10 头只有 20 字节（dxgiFormat + resourceDimension + misc /
            // arraySize / miscFlags），真正格式在第一个 u32。
            let dxgi = u32_at(HEADER_LEN)?;
            let format = match dxgi {
                71 | 72 => DdsFormat::Bc1,
                74 | 75 => DdsFormat::Bc2,
                77 | 78 => DdsFormat::Bc3,
                80 | 81 => DdsFormat::Bc4,
                83..=85 => DdsFormat::Bc5,
                94..=101 => DdsFormat::Bc7,
                _ => DdsFormat::Uncompressed,
            };
            (format, HEADER_LEN + 20)
        }
        _ => (DdsFormat::Uncompressed, HEADER_LEN),
    };

    Ok(DdsLayout {
        width,
        height,
        declared_levels,
        payload_offset,
        payload_len: data.len().saturating_sub(payload_offset),
        format,
        bytes_per_block: format.bytes_per_block(),
    })
}

/// 把 DDS 的 mip 级数砍到“只有基础层”（`dwMipMapCount = 1`）。
///
/// **为什么需要**：Godot 的 `Image::load_dds_from_buffer` 对 mip 链非常严格，
/// 只要头部声明了 mip 就要求完整的 1x1 链；KN5 的导出器经常裁剪到 2x2 就停止
/// （例如 F1 2008 试样里的 `flat_nm.dds`），于是整张贴图被拒绝、返回空图。
/// 实测“只有基础层”是唯一被稳定接受的写法，剩下的 mip 交给 Godot 自己生成。
///
/// 返回的字节序列只在头部改写 mip 级数并清掉相关标志；像素数据原样保留。
/// 本来就没有多级 mip 的文件返回 `None`。
pub fn strip_mip_levels(data: &[u8]) -> Option<Vec<u8>> {
    let layout = layout(data).ok()?;
    if layout.declared_levels <= 1 {
        return None;
    }

    let mut patched = data.to_vec();
    patched[28..32].copy_from_slice(&1u32.to_le_bytes());

    // 与 mip 相关的两个标志位也一起清掉，避免加载器认为“有 mip 但数量为 1”。
    let flags = u32::from_le_bytes(patched[8..12].try_into().ok()?);
    patched[8..12].copy_from_slice(&(flags & !0x0002_0000).to_le_bytes()); // DDSD_MIPMAPCOUNT
    let caps2 = u32::from_le_bytes(patched[112..116].try_into().ok()?);
    patched[112..116].copy_from_slice(&(caps2 & !0x0020_0000).to_le_bytes()); // DDCAPS2_MIPMAPREQUIRED

    Some(patched)
}

/// 修正“flags 没声明 `DDSD_PITCH` / `DDSD_LINEARSIZE`，但 `dwPitchOrLinearSize`
/// 却是非零值”的自相矛盾 DDS 头。
///
/// **为什么需要**：部分导出器写出的头只有 `CAPS|HEIGHT|WIDTH|PIXELFORMAT|
/// MIPMAPCOUNT`（`0x21007`），既没有 PITCH 也没有 LINEARSIZE，却在第 20 字节
/// 留了个值。Godot 的 DDS 加载器对这种不一致直接拒绝——“DDS header flags
/// specify that no linear size will be given … but a non-zero linear size value
/// is present”——整张贴图拿不到（Daytona 赛道 274 张贴图里有 42 张这样，
/// 看台/人群因此全黑）。把字段清零即可让头自洽；实测清零后 Godot 能正常
/// 加载、解压并重建 mip。本来就没有该 flag 或值为 0 的文件返回 `None`。
///
/// 需要改写时返回补丁后的字节（只动头部这 4 个字节，像素原样保留）。
pub fn fix_orphan_pitch_value(data: &[u8]) -> Option<Vec<u8>> {
    const DDSD_PITCH: u32 = 0x8;
    const DDSD_LINEARSIZE: u32 = 0x80000;
    if data.len() < 128 {
        return None;
    }
    let flags = u32::from_le_bytes(data[8..12].try_into().ok()?);
    if flags & (DDSD_PITCH | DDSD_LINEARSIZE) != 0 {
        return None;
    }
    let pitch = u32::from_le_bytes(data[20..24].try_into().ok()?);
    if pitch == 0 {
        return None;
    }

    let mut patched = data.to_vec();
    patched[20..24].copy_from_slice(&0u32.to_le_bytes());
    Some(patched)
}

/// 把 payload 截到“头部声明的 mip 级数”刚好容纳的大小。
///
/// KN5 的导出器常在声明 N 级的同时多写给低层 mip 的尾随字节（`trees_new.dds`：
/// 2048² DXT5 声明 5 级，实际塞了接近 12 级的数据），Godot 对“声明 N 级但
/// 数据超长”的头部整张贴图拒绝（'Expected … got … instead'）。返回只保留
/// `dwMipMapCount` 级数据、像素按字节切分的副本；本来就没有定界差异时返回
/// `None`。
pub fn truncate_to_declared_mips(data: &[u8]) -> Option<Vec<u8>> {
    let layout = layout(data).ok()?;
    let declared = layout.declared_levels.max(1).min(layout.max_levels());
    let wanted: usize = (0..declared).map(|level| layout.level_bytes(level)).sum();
    let end = layout.payload_offset.saturating_add(wanted);
    if end >= data.len() {
        return None;
    }

    Some(data[..end].to_vec())
}

/// 把 payload 截到“只有基础层”的大小（与 [`strip_mip_levels`] 配合使用）。
///
/// Godot 接受“mipmapcount = 1 + 更长 payload”（经验数据：22/30 张 Daytona
/// 的坏贴图这样就能过），但仍有几张只认“长度精确等于基础层”的输入，所以
/// 截断版本也留着作为最后的手段。压缩按 4x4 块算、未压缩按 `w*h*4` 算——
/// Godot 自己就是按这个口径（不认 KN5 里 3 字节/像素的 RGB 写法）。
pub fn truncate_to_base_level(data: &[u8]) -> Option<Vec<u8>> {
    let layout = layout(data).ok()?;
    let wanted = layout.level_bytes(0);
    let end = layout.payload_offset.saturating_add(wanted);
    if end >= data.len() {
        return None;
    }
    Some(data[..end].to_vec())
}

/// DDS 头部检查失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DdsError {
    /// 数据不足 128 字节。
    #[error("DDS data too short: {0} bytes, need at least 128")]
    TooShort(usize),

    /// 前四个字节不是 `DDS `。
    #[error("bad DDS magic")]
    BadMagic,

    /// 头部字段在试图转换定长数组时失败（理论上不会发生，因为在
    /// [`parse`] 中已检查过长度）。
    #[error("truncated DDS header field")]
    TruncatedField,

    /// 宽或高为 0。
    #[error("DDS dimensions {0}x{1} are invalid")]
    BadDimensions(u32, u32),
}

/// 从 DDS 字节中读取尺寸、mip 数量和 FourCC。
///
/// 数据不足 128 字节、magic 不是 `DDS ` 或头部被截断时返回 [`DdsError`]。
pub fn parse(data: &[u8]) -> Result<DdsInfo, DdsError> {
    const HEADER_LEN: usize = 128;
    if data.len() < HEADER_LEN {
        return Err(DdsError::TooShort(data.len()));
    }
    if &data[..4] != b"DDS " {
        return Err(DdsError::BadMagic);
    }

    let height = u32::from_le_bytes(
        data[12..16]
            .try_into()
            .map_err(|_| DdsError::TruncatedField)?,
    );
    let width = u32::from_le_bytes(
        data[16..20]
            .try_into()
            .map_err(|_| DdsError::TruncatedField)?,
    );
    if width == 0 || height == 0 {
        return Err(DdsError::BadDimensions(width, height));
    }
    let mipmaps = u32::from_le_bytes(
        data[28..32]
            .try_into()
            .map_err(|_| DdsError::TruncatedField)?,
    );
    let fourcc = fourcc_to_array(&data[84..88]);

    Ok(DdsInfo {
        width,
        height,
        mipmaps,
        fourcc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个只填了必要字段的 128 字节 DDS 头。
    fn dds_header(width: u32, height: u32, mipmaps: u32, fourcc: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mipmaps.to_le_bytes());
        bytes[84..88].copy_from_slice(fourcc);
        bytes
    }

    #[test]
    fn parses_dimensions_and_fourcc() {
        let bytes = dds_header(2048, 1024, 11, b"DXT5");

        let info = parse(&bytes).expect("合法 DDS 头应当解析成功");
        assert_eq!(info.width, 2048);
        assert_eq!(info.height, 1024);
        assert_eq!(info.mipmaps, 11);
        assert_eq!(info.fourcc, *b"DXT5");
        assert_eq!(info.fourcc_str(), Some("DXT5"));
    }

    #[test]
    fn rejects_short_data_with_len() {
        assert_eq!(parse(b"DDS "), Err(DdsError::TooShort(4)));
        assert_eq!(parse(&[0; 127]), Err(DdsError::TooShort(127)));
    }

    #[test]
    fn rejects_bad_magic_with_error() {
        let mut bytes = dds_header(1, 1, 1, b"DXT1");
        bytes[..4].copy_from_slice(b"NOPE");

        assert_eq!(parse(&bytes), Err(DdsError::BadMagic));
    }

    #[test]
    fn detects_kn5_short_mip_chain() {
        // F1 2008 试样里的 flat_nm.dds：32x32 DXT5，声称 5 级 mip，
        // payload 却少了最后一级 1x1（16 字节）——Godot 会拒绝这种文件。
        let mut bytes = dds_header(32, 32, 5, b"DXT5");
        bytes.resize(128 + 1376, 0);

        let kn5 = layout(&bytes).expect("KN5 贴图应当能解析");
        assert_eq!(kn5.format, DdsFormat::Bc3);
        assert_eq!(kn5.bytes_per_block, Some(16));
        assert_eq!(kn5.declared_levels, 5);
        assert!(kn5.payload_len < kn5.full_chain_size());
    }

    #[test]
    fn full_chain_size_matches_godot() {
        // Godot 对 32x32 DXT5 的完整链期望 1392 字节（1024+256+64+16+16+16）。
        let bytes = dds_header(32, 32, 5, b"DXT5");
        let layout = layout(&bytes).expect("DXT5 头应当解析成功");
        assert_eq!(layout.max_levels(), 6);
        assert_eq!(layout.full_chain_size(), 1392);
    }

    #[test]
    fn strip_mip_levels_rewrites_header_only() {
        let mut bytes = dds_header(32, 32, 5, b"DXT5");
        bytes.resize(128 + 1376, 0);

        let stripped = strip_mip_levels(&bytes).expect("需要砍掉 mip");
        assert_eq!(stripped.len(), bytes.len()); // 长度不变
        assert_eq!(u32::from_le_bytes(stripped[28..32].try_into().unwrap()), 1);
        // 像素数据原样保留（只改头部字段）。
        assert_eq!(stripped[128..].to_vec(), bytes[128..].to_vec());
    }

    #[test]
    fn strip_mip_levels_ignores_files_without_mipmaps() {
        let mut bytes = dds_header(4, 4, 1, b"DXT1");
        bytes.resize(128 + 8 + 8, 0);

        assert!(strip_mip_levels(&bytes).is_none());
    }

    #[test]
    fn layout_reports_dx10_headers() {
        let mut bytes = dds_header(16, 16, 3, b"DX10");
        bytes.resize(128 + 20 + 256 + 64, 0);
        bytes[128..132].copy_from_slice(&99u32.to_le_bytes()); // dxgiFormat = BC7_UNORM

        let header = layout(&bytes).expect("DX10 头应当解析成功");
        assert_eq!(header.format, DdsFormat::Bc7);
        assert_eq!(header.payload_offset, 148);
        assert_eq!(header.bytes_per_block, Some(16));

        let stripped = strip_mip_levels(&bytes).expect("需要砍掉 mip");
        assert_eq!(u32::from_le_bytes(stripped[28..32].try_into().unwrap()), 1);
    }

    #[test]
    fn rejects_zero_dimensions() {
        // 零尺寸会让 levels_for/level_bytes 算出 0 字节的 mip 级，
        // 也会让 Godot 拿到空图，所以 parse/layout 都要拒绝。
        for (width, height) in [(0, 64), (64, 0), (0, 0)] {
            let bytes = dds_header(width, height, 1, b"DXT1");
            assert_eq!(parse(&bytes), Err(DdsError::BadDimensions(width, height)));
            assert_eq!(layout(&bytes), Err(DdsError::BadDimensions(width, height)));
        }
    }

    #[test]
    fn strip_mip_levels_rejects_zero_dimensions() {
        let bytes = dds_header(0, 32, 5, b"DXT5");
        assert!(strip_mip_levels(&bytes).is_none());
    }

    #[test]
    fn fix_orphan_pitch_value_clears_inconsistent_field() {
        // Daytona 的 raceday 贴图：flags 只有 0x21007，值却是 4096。
        let mut bytes = dds_header(2048, 2048, 1, b"DXT1");
        bytes[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&4096u32.to_le_bytes());

        let fixed = fix_orphan_pitch_value(&bytes).expect("自相矛盾的头应当被修正");
        assert_eq!(u32::from_le_bytes(fixed[20..24].try_into().unwrap()), 0);
        // 只动这 4 个字节，其余原样。
        assert_eq!(fixed[..20], bytes[..20]);
        assert_eq!(fixed[24..], bytes[24..]);
        // 修正后不再需要二次修正（幂等）。
        assert!(fix_orphan_pitch_value(&fixed).is_none());
    }

    #[test]
    fn truncate_to_declared_mips_drops_extra_payload() {
        // `trees_new.dds`：2048² DXT5，头部声明 5 级 mip，实际塞了接近 12 级的
        // 数据。Godot 按声明的级数核账，多余字节整张贴图拒收。
        let mut bytes = dds_header(2048, 2048, 5, b"DXT5");
        let declared: usize = (0..5)
            .map(|level| layout(&bytes).unwrap().level_bytes(level))
            .sum();
        bytes.resize(128 + declared + 5488, 0); // 完整 5 级 + 5488 字节尾随垃圾

        let layout = layout(&bytes).expect("合法 DDS");
        assert_eq!(layout.declared_levels, 5);
        // 声明的 5 级 + 尾随垃圾 > Godot 给 5 级的预算。
        assert!(layout.payload_len > declared);

        let trimmed = truncate_to_declared_mips(&bytes).expect("payload 超长应当被截断");
        // 只保留“5 级 mip”刚到容纳得下的字节数，头部一个字节都不动。
        assert_eq!(trimmed.len(), 128 + declared);
        assert_eq!(&trimmed[..128], &bytes[..128]);
    }

    #[test]
    fn truncate_to_declared_mips_keeps_exact_files_intact() {
        let mut bytes = dds_header(64, 64, 7, b"DXT5");
        let layout = layout(&bytes).expect("合法 DDS");
        let wanted: usize = (0..layout.declared_levels)
            .map(|level| layout.level_bytes(level))
            .sum();
        bytes.resize(128 + wanted, 0);

        // 长度刚好等于声明级数所需 → 不动。
        assert!(truncate_to_declared_mips(&bytes).is_none());
    }

    #[test]
    fn truncate_to_base_level_keeps_only_level_zero() {
        let mut bytes = dds_header(512, 1024, 5, b"DXT5");
        bytes.resize(128 + 2_793_472, 0);

        let trimmed = truncate_to_base_level(&bytes).expect("应截到基础层");
        // 512x1024 DXT5 的基础层：128x256 个 4x4 块，每块 16 字节。
        let expected = 128 + 128 * 256 * 16;
        assert_eq!(trimmed.len(), expected);
    }

    #[test]
    fn fix_orphan_pitch_value_leaves_consistent_headers_alone() {
        // 声明了 LINEARSIZE（正常写法）→ 不动。
        let mut linear = dds_header(64, 64, 1, b"DXT1");
        linear[8..12].copy_from_slice(&0x80000u32.to_le_bytes());
        linear[20..24].copy_from_slice(&2048u32.to_le_bytes());
        assert!(fix_orphan_pitch_value(&linear).is_none());

        // 没有 flag 但值本来就是 0 → 与 Godot 不冲突，不动。
        let mut zero = dds_header(64, 64, 1, b"DXT1");
        zero[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        assert!(fix_orphan_pitch_value(&zero).is_none());

        // 太短的输入直接拒绝，不越界。
        assert!(fix_orphan_pitch_value(b"DDS ").is_none());
    }

    #[test]
    fn uncompressed_layout_reports_four_bytes_per_pixel() {
        // 没有 FourCC 的 DDS：按未压缩 RGBA 处理，没有分块尺寸。
        let bytes = dds_header(8, 8, 1, b"\0\0\0\0");
        let layout = layout(&bytes).expect("未压缩头应当解析成功");
        assert_eq!(layout.format, DdsFormat::Uncompressed);
        assert_eq!(layout.bytes_per_block, None);
        assert_eq!(layout.bytes_per_pixel(), Some(4));
        // 8x8 = 2 个 4x4 块，压缩路径不适用；未压缩一级 256 字节。
        assert_eq!(layout.level_bytes(0), 256);
        assert_eq!(layout.available_levels(), 1);
        assert!(strip_mip_levels(&bytes).is_none()); // 只有一层，无需修补
    }
}
