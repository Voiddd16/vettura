use crate::error::{Error, Result};
use crate::model::Transform;

/// 带边界检查的小端字节游标。
///
/// 每次读取要么成功并推进游标，要么返回 [`Error::Eof`] 且不改变游标位置，
/// 因此调用方可以安全地为错误补充上下文。
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// 当前游标相对文件开头的偏移。
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    /// 剩余可读字节数。
    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// 借用接下来的 `len` 个字节；失败时不消费任何数据。
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or(Error::Eof {
            offset: self.pos,
            needed: len,
            remaining: self.remaining(),
        })?;

        if end > self.data.len() {
            return Err(Error::Eof {
                offset: self.pos,
                needed: len,
                remaining: self.remaining(),
            });
        }

        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    /// 借用 `count * item_size` 个字节，作为一批固定大小的记录。
    ///
    /// 批量读取一次完成边界检查，随后调用方可以用 `chunks_exact` 顺序解析，
    /// 避免每个字段都单独检查游标。
    pub(crate) fn take_array(&mut self, count: usize, item_size: usize) -> Result<&'a [u8]> {
        let len = count.checked_mul(item_size).ok_or(Error::Eof {
            offset: self.pos,
            needed: usize::MAX,
            remaining: self.remaining(),
        })?;
        self.take(len)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// KN5 的布尔值占一个字节。任何非零值都视为 `true`，与 AcTools 的
    /// `ReadBoolean` 行为一致。
    pub(crate) fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    /// 读取 `N` 个连续的 `f32`。
    pub(crate) fn f32_array<const N: usize>(&mut self) -> Result<[f32; N]> {
        let bytes = self.take(N * 4)?;
        let mut out = [0.0; N];
        for (slot, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *slot = f32::from_le_bytes(*chunk);
        }
        Ok(out)
    }

    /// 读取一个按行主序存储的 4x4 矩阵（16 个 `f32`）。
    pub(crate) fn matrix(&mut self) -> Result<Transform> {
        let values = self.f32_array::<16>()?;
        let mut transform = [[0.0; 4]; 4];
        for (row, chunk) in transform.iter_mut().zip(values.as_chunks::<4>().0) {
            row.copy_from_slice(chunk);
        }
        Ok(transform)
    }

    /// 读取带长度前缀的字符串。
    ///
    /// KN5 字符串是显式记录字节长度的原始字节序列，不带终止符。部分 mod 使用
    /// Windows-1252 编码，因此这里用 lossy 方式解码，而不是拒绝非法 UTF-8。
    pub(crate) fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        let text = String::from_utf8_lossy(bytes);
        Ok(text.trim_end_matches('\0').to_owned())
    }

    /// 读取 `count` 个小端 `u16`。
    pub(crate) fn u16_vec(&mut self, count: usize) -> Result<Vec<u16>> {
        let bytes = self.take(count.checked_mul(2).ok_or(Error::Eof {
            offset: self.pos,
            needed: usize::MAX,
            remaining: self.remaining(),
        })?)?;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes(*chunk))
            .collect())
    }

    /// 在分配或循环之前拒绝明显不合理的数量。
    ///
    /// `min_bytes_per_item` 是单个元素最小的编码长度；如果
    /// `count * min_bytes_per_item` 超过剩余字节数，这个数量就不可能是真的。
    pub(crate) fn check_count(
        &self,
        what: &'static str,
        count: usize,
        min_bytes_per_item: usize,
    ) -> Result<()> {
        let needed = count.saturating_mul(min_bytes_per_item);
        if needed > self.remaining() {
            return Err(Error::InvalidCount {
                what,
                count,
                offset: self.pos,
                remaining: self.remaining(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_scalars() {
        let mut reader = Reader::new(&[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(reader.u32().unwrap(), 0x1234_5678);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn eof_does_not_advance() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert!(reader.take(4).is_err());
        assert_eq!(reader.position(), 0);
        assert_eq!(reader.remaining(), 3);
    }

    #[test]
    fn reads_bools_as_non_zero() {
        let mut reader = Reader::new(&[0, 1, 2, 255]);
        assert!(!reader.bool().unwrap());
        assert!(reader.bool().unwrap());
        assert!(reader.bool().unwrap());
        assert!(reader.bool().unwrap());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn reads_length_prefixed_string_without_terminator() {
        let mut reader = Reader::new(b"\x05\0\0\0hello");
        assert_eq!(reader.string().unwrap(), "hello");
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn trims_trailing_nul_from_string() {
        let mut reader = Reader::new(b"\x05\0\0\0hi\0\0\0");
        assert_eq!(reader.string().unwrap(), "hi");
    }

    #[test]
    fn reads_row_major_matrix() {
        let values: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 3.0, 4.0, 5.0, 1.0,
        ];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        let mut reader = Reader::new(&bytes);
        let matrix = reader.matrix().unwrap();
        assert_eq!(matrix[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(matrix[3], [3.0, 4.0, 5.0, 1.0]);
    }

    #[test]
    fn reads_u16_indices() {
        let mut reader = Reader::new(&[0x00, 0x00, 0x01, 0x00, 0x02, 0x00]);
        assert_eq!(reader.u16_vec(3).unwrap(), [0, 1, 2]);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn rejects_count_larger_than_input() {
        let reader = Reader::new(&[0; 4]);
        let err = reader.check_count("test", 2, 8).unwrap_err();
        assert!(matches!(err, Error::InvalidCount { .. }));
    }

    #[test]
    fn take_array_borrows_contiguous_records() {
        let mut reader = Reader::new(&[1, 2, 3, 4, 5, 6]);
        let records = reader.take_array(2, 3).unwrap();
        assert_eq!(records, [1, 2, 3, 4, 5, 6]);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn take_array_rejects_short_input() {
        let mut reader = Reader::new(&[0; 5]);
        assert!(reader.take_array(2, 3).is_err());
        assert_eq!(reader.position(), 0);
    }
}
