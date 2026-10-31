//! `data.acd` container support.
//!
//! # Container layout
//!
//! Decrypted `data.acd` is a flat sequence of records:
//!
//! ```text
//! i32 name_length          (negative values are followed by an extra u32)
//! u8  name[name_length]
//! u32 content_length       (plaintext bytes)
//! u8  encoded[content_length * 4]
//! ```
//!
//! Each plaintext byte occupies four bytes on disk. Zero bytes are dropped,
//! and every remaining byte is rotated by a key byte:
//!
//! ```text
//! plain = encoded.wrapping_sub(key[i % key.len()])
//! ```
//!
//! # Key
//!
//! The key is derived from the **car folder name** (the parent directory of
//! `data.acd`), formatted as eight decimal numbers joined by `-`. The ASCII
//! bytes of that string are the actual cipher key. This is why an extracted
//! `data.acd` becomes unreadable after being moved to another folder.

use std::path::Path;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcdEntry {
    pub name: String,
    /// Decrypted file content.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcdArchive {
    pub entries: Vec<AcdEntry>,
}

impl AcdArchive {
    /// Open an ACD using the parent folder name as the key.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                Error::InvalidAcd(format!(
                    "cannot derive key: '{}' has no UTF-8 parent folder",
                    path.display()
                ))
            })?;
        let key = derive_key(folder);
        Self::from_path_with_key(path, &key)
    }

    pub fn from_path_with_key(path: impl AsRef<Path>, key: &str) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes, key)
    }

    pub fn from_bytes(bytes: &[u8], key: &str) -> Result<Self> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let key = key.as_bytes();

        let mut entries = Vec::new();
        let mut pos = 0usize;

        while pos + 4 <= bytes.len() {
            // A trailing zero word means padding, not a zero-length filename.
            let mut name_len = read_i32(bytes, pos)
                .ok_or_else(|| Error::InvalidAcd(format!("truncated name length at {pos}")))?;
            pos += 4;

            if name_len == 0 {
                break;
            }

            // Some writers prefix the real length with a negative sentinel.
            if name_len < 0 {
                let value = read_u32(bytes, pos).ok_or_else(|| {
                    Error::InvalidAcd(format!("truncated filename length at {pos}"))
                })?;
                pos += 4;
                name_len = value as i32;
            }

            let name_len = usize::try_from(name_len)
                .map_err(|_| Error::InvalidAcd(format!("negative filename length at {pos}")))?;
            let name_bytes = bytes.get(pos..pos + name_len).ok_or_else(|| {
                Error::InvalidAcd(format!(
                    "filename of {name_len} bytes exceeds file at {pos}"
                ))
            })?;
            pos += name_len;
            let name = String::from_utf8_lossy(name_bytes)
                .trim_end_matches('\0')
                .to_owned();

            let content_len = read_u32(bytes, pos)
                .ok_or_else(|| Error::InvalidAcd(format!("truncated content length at {pos}")))?
                as usize;
            pos += 4;

            let encoded_len = content_len.checked_mul(4).ok_or_else(|| {
                Error::InvalidAcd(format!("content length {content_len} overflows at {pos}"))
            })?;
            let encoded = bytes.get(pos..pos + encoded_len).ok_or_else(|| {
                Error::InvalidAcd(format!(
                    "content of {encoded_len} bytes exceeds file at {pos}"
                ))
            })?;
            pos += encoded_len;

            entries.push(AcdEntry {
                name,
                data: decrypt(encoded, key),
            });
        }

        Ok(Self { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .map(|entry| entry.data.as_slice())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }
}

/// Derive the ACD key string from a car folder name.
pub fn derive_key(folder_name: &str) -> String {
    let chars: Vec<i64> = folder_name.chars().map(|c| c as i64).collect();
    let n = chars.len() as i64;

    let mut part1 = 0i64;
    for &c in &chars {
        part1 = part1.wrapping_add(c);
    }

    let mut part2 = 0i64;
    let mut i = 0i64;
    while i < n - 1 {
        part2 = part2.wrapping_mul(chars[i as usize]);
        i += 1;
        part2 = part2.wrapping_sub(chars[i as usize]);
        i += 1;
    }

    let mut part3 = 0i64;
    let mut i = 1i64;
    while i < n - 3 {
        part3 = part3.wrapping_mul(chars[i as usize]);
        i += 1;
        let divisor = chars[i as usize] + 0x1b;
        part3 = if part3 < 0 {
            -(part3.abs() / divisor)
        } else {
            part3 / divisor
        };
        i -= 2;
        part3 = part3.wrapping_add(-0x1b - chars[i as usize]);
        i += 4;
    }

    let mut part4 = 0x1683i64;
    for &c in chars.iter().skip(1) {
        part4 = part4.wrapping_sub(c);
    }

    let mut part5 = 0x42i64;
    let mut i = 1i64;
    while i < n - 4 {
        let a = part5.wrapping_mul(chars[i as usize] + 0xf);
        i -= 1;
        let b = chars[i as usize];
        i += 1;
        part5 = b.wrapping_add(0xf).wrapping_mul(a).wrapping_add(0x16);
        i += 4;
    }

    let mut part6 = 0x65i64;
    let mut i = 0i64;
    while i < n - 2 {
        part6 = part6.wrapping_sub(chars[i as usize]);
        i += 2;
    }

    let mut part7 = 0xabi64;
    let mut i = 0i64;
    while i < n - 2 {
        let divisor = chars[i as usize];
        if divisor != 0 {
            part7 %= divisor;
        }
        i += 2;
    }

    let mut part8 = 0xabi64;
    let mut i = 0i64;
    while i < n - 1 {
        let divisor = chars[i as usize];
        if divisor != 0 {
            part8 /= divisor;
        }
        i += 1;
        part8 = part8.wrapping_add(chars[i as usize]);
    }

    format!(
        "{}-{}-{}-{}-{}-{}-{}-{}",
        part1 & 0xff,
        part2 & 0xff,
        part3 & 0xff,
        part4 & 0xff,
        part5 & 0xff,
        part6 & 0xff,
        part7 & 0xff,
        part8 & 0xff,
    )
}

fn decrypt(encoded: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded.len() / 4);
    let mut key_index = 0usize;

    for group in encoded.as_chunks::<4>().0 {
        for &byte in group {
            if byte == 0 {
                continue;
            }
            let key_byte = key[key_index % key.len()];
            out.push(byte.wrapping_sub(key_byte));
            key_index += 1;
        }
    }

    out
}

fn read_u32(bytes: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(pos..pos + 4)?.try_into().ok()?,
    ))
}

fn read_i32(bytes: &[u8], pos: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(pos..pos + 4)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_known_f1_2008_key() {
        assert_eq!(derive_key("cim_2008_ferrari"), "172-5-6-58-214-253-22-106");
    }

    #[test]
    fn decrypts_one_record() {
        let key = derive_key("cim_2008_ferrari");
        let key_bytes = key.as_bytes();
        let plaintext = b"-10|1\r\n10|1\r\n";
        let mut encoded = Vec::new();
        for (i, &byte) in plaintext.iter().enumerate() {
            let value = byte.wrapping_add(key_bytes[i % key_bytes.len()]);
            encoded.extend_from_slice(&(value as u32).to_le_bytes());
        }

        let mut container = Vec::new();
        container.extend_from_slice(&5i32.to_le_bytes());
        container.extend_from_slice(b"1.lut");
        container.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
        container.extend_from_slice(&encoded);

        let archive = AcdArchive::from_bytes(&container, &key).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.get("1.lut"), Some(plaintext.as_slice()));
    }
}
