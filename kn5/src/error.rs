use thiserror::Error;

/// KN5 解析结果类型。
pub type Result<T> = std::result::Result<T, Error>;

/// KN5 解析过程中可能出现的错误。
///
/// 所有与文件结构有关的错误都携带字节偏移，方便定位损坏位置。
#[derive(Debug, Error)]
pub enum Error {
    /// 读取文件失败。
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// 读取越过文件末尾或发生偏移溢出。
    #[error("unexpected end of file at offset {offset}: need {needed} bytes, {remaining} left")]
    Eof {
        offset: usize,
        needed: usize,
        remaining: usize,
    },

    /// 文件头 magic 不是 `sc6969`。
    #[error("invalid KN5 magic: {0:?}")]
    InvalidMagic([u8; 6]),

    /// 文件版本不是 5 或 6。
    #[error("unsupported KN5 version {0} (supported: 5 and 6)")]
    UnsupportedVersion(u32),

    /// 节点类型不是 1、2 或 3。
    #[error("invalid node type {0} at offset {1}")]
    InvalidNodeType(i32, usize),

    /// 数量字段与剩余字节数明显不匹配。
    #[error("invalid {what} count {count} at offset {offset} ({remaining} bytes remaining)")]
    InvalidCount {
        what: &'static str,
        count: usize,
        offset: usize,
        remaining: usize,
    },

    /// 节点递归超过 [`crate::MAX_NODE_DEPTH`] 层。
    #[error("node tree is nested deeper than {0} levels")]
    DepthLimit(usize),

    /// 根节点之后存在非零尾随数据。
    #[error("trailing data after the root node at offset {0}")]
    TrailingData(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eof_error_reports_offset_and_sizes() {
        let message = Error::Eof {
            offset: 12,
            needed: 44,
            remaining: 8,
        }
        .to_string();

        assert!(message.contains("offset 12"));
        assert!(message.contains("44"));
        assert!(message.contains("8"));
    }

    #[test]
    fn invalid_count_reports_what_and_count() {
        let message = Error::InvalidCount {
            what: "mesh vertex",
            count: 999,
            offset: 64,
            remaining: 3,
        }
        .to_string();

        assert!(message.contains("mesh vertex"));
        assert!(message.contains("999"));
        assert!(message.contains("64"));
    }
}
