//! KN5 内嵌 DDS 贴图的最小头部检查。
//!
//! 这里刻意不解码像素：Godot 层可以把原始 DDS 字节交给
//! `Image::load_dds_from_buffer`，检查工具也只需要尺寸和格式元数据。

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
    let mipmaps = u32::from_le_bytes(
        data[28..32]
            .try_into()
            .map_err(|_| DdsError::TruncatedField)?,
    );
    let fourcc = data[84..88]
        .try_into()
        .map_err(|_| DdsError::TruncatedField)?;

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
}
