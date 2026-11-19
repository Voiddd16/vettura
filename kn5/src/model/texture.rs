//! 贴图数据模型：共享文件缓冲区与零拷贝的 DDS 载荷视图。

use std::fmt;
use std::ops::{Deref, Range};
use std::sync::Arc;

use crate::dds::{self, DdsError, DdsInfo};

/// 共享的 KN5 原始字节缓冲区。
///
/// 使用 `Arc<Vec<u8>>` 而不是 `Arc<[u8]>`：从 `Vec<u8>` 构造时不会复制数据，
/// 因此 `Kn5::from_path` 读入文件后可以直接被所有贴图切片共享。
pub type SharedBytes = Arc<Vec<u8>>;

#[derive(Debug, Clone)]
pub struct Texture {
    /// 贴图表中的 `active` 标志。AcTools 把 `1` 视为启用；真实文件中被引用
    /// 的贴图通常都是 `1`。
    pub active: bool,
    pub name: String,
    /// 指向共享文件缓冲区中的 DDS 载荷，不复制像素数据。
    pub data: TextureData,
}

impl Texture {
    /// 尝试读取内嵌 DDS 的尺寸与格式信息。
    ///
    /// 不是 DDS（或数据被截断）时返回 [`DdsError`]。
    pub fn dds_info(&self) -> Result<DdsInfo, DdsError> {
        dds::parse(self.data.as_slice())
    }
}

/// 指向共享 KN5 缓冲区中一段贴图数据的只读视图。
///
/// 克隆 `TextureData` 只会增加 `Arc` 引用计数，不会复制 DDS 字节。`Deref` 到
/// `[u8]`，因此可以继续按切片使用。
#[derive(Clone)]
pub struct TextureData {
    source: SharedBytes,
    range: Range<usize>,
}

impl TextureData {
    /// 创建一段共享视图。范围由解析器保证在源缓冲区之内。
    pub(crate) fn new(source: SharedBytes, range: Range<usize>) -> Self {
        debug_assert!(range.start <= range.end);
        debug_assert!(range.end <= source.len());
        Self { source, range }
    }

    /// 贴图数据切片。
    pub fn as_slice(&self) -> &[u8] {
        &self.source[self.range.clone()]
    }

    /// 在整个 KN5 文件中的字节范围。
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// 底层共享文件缓冲区。
    pub fn source(&self) -> &SharedBytes {
        &self.source
    }

    pub fn len(&self) -> usize {
        self.range.len()
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

impl Deref for TextureData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for TextureData {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl fmt::Debug for TextureData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextureData")
            .field("range", &self.range)
            .field("len", &self.len())
            .finish()
    }
}
