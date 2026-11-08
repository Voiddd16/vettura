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

            // Reject absurd sizes before touching the slice, so a corrupted
            // header cannot accidentally cause an enormous allocation or a
            // pathological memory access pattern.
            const MAX_ENCODED_LEN: usize = 512 * 1024 * 1024; // 512 MiB
            if encoded_len > MAX_ENCODED_LEN {
                return Err(Error::InvalidAcd(format!(
                    "content of {encoded_len} bytes at {pos} exceeds the {MAX_ENCODED_LEN} byte limit"
                )));
            }
            if encoded_len > bytes.len().saturating_sub(pos) {
                return Err(Error::InvalidAcd(format!(
                    "content of {encoded_len} bytes exceeds file at {pos}"
                )));
            }
            let encoded = &bytes[pos..pos + encoded_len];
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

/// The ASCII bytes of the derived key are the actual cipher key.
///
/// The function implements the classic Assetto Corsa "seven parts + checksum"
/// key derivation: eight decimal numbers, each masked to one byte (`& 0xff`)
/// and joined by hyphens. Each part uses a distinct reduction over the folder
/// name's character codes; the loops are deliberately kept close to the
/// original Pascal so the resulting bytes stay bit-for-bit compatible.
///
/// # Arguments
///
/// * `folder_name` — the car folder name (e.g. `"cim_2008_ferrari"`).
///   The bytes of the returned string are used directly as the cipher key.
pub fn derive_key(folder_name: &str) -> String {
    let chars: Vec<i64> = folder_name.chars().map(|c| c as i64).collect();
    let n = chars.len() as i64;

    // Each part is masked to a single byte (0-255) before formatting.
    let parts = [
        derive_part_sum_all(&chars, n),
        derive_part_alternating_product(&chars, n),
        derive_part_scaled_difference(&chars, n),
        derive_part_subtract_from_constant(&chars, n),
        derive_part_scaled_accumulator(&chars, n),
        derive_part_subtract_every_other(&chars, n),
        derive_part_modulo_every_other(&chars, n),
        derive_part_dividing_accumulator(&chars, n),
    ];

    parts
        .iter()
        .map(|p| (p & 0xff).to_string())
        .collect::<Vec<_>>()
        .join("-")
}

/// Part 1: sum of all character codes.
fn derive_part_sum_all(chars: &[i64], _n: i64) -> i64 {
    chars.iter().fold(0i64, |acc, &c| acc.wrapping_add(c))
}

/// Part 2: alternating multiply–subtract over consecutive characters.
fn derive_part_alternating_product(chars: &[i64], n: i64) -> i64 {
    let mut part = 0i64;
    let mut i = 0i64;
    while i < n - 1 {
        part = part.wrapping_mul(chars[i as usize]);
        i += 1;
        part = part.wrapping_sub(chars[i as usize]);
        i += 1;
    }
    part
}

/// Part 3: divide by an offset divisor then subtract the divisor again.
fn derive_part_scaled_difference(chars: &[i64], n: i64) -> i64 {
    let mut part = 0i64;
    let mut i = 1i64;
    while i < n - 3 {
        part = part.wrapping_mul(chars[i as usize]);
        i += 1;
        let divisor = chars[i as usize] + 0x1b;
        part = if part < 0 {
            -(part.abs() / divisor)
        } else {
            part / divisor
        };
        i -= 2;
        part = part.wrapping_add(-0x1b - chars[i as usize]);
        i += 4;
    }
    part
}

/// Part 4: start at a fixed seed and subtract every character after the first.
fn derive_part_subtract_from_constant(chars: &[i64], _n: i64) -> i64 {
    chars
        .iter()
        .skip(1)
        .fold(0x1683i64, |acc, &c| acc.wrapping_sub(c))
}

/// Part 5: multiply-accumulate with bounded stride of 4.
fn derive_part_scaled_accumulator(chars: &[i64], n: i64) -> i64 {
    let mut part = 0x42i64;
    let mut i = 1i64;
    while i < n - 4 {
        let a = part.wrapping_mul(chars[i as usize] + 0xf);
        i -= 1;
        let b = chars[i as usize];
        i += 1;
        part = b.wrapping_add(0xf).wrapping_mul(a).wrapping_add(0x16);
        i += 4;
    }
    part
}

/// Part 6: subtract every other character starting from index 0.
fn derive_part_subtract_every_other(chars: &[i64], n: i64) -> i64 {
    let mut part = 0x65i64;
    let mut i = 0i64;
    while i < n - 2 {
        part = part.wrapping_sub(chars[i as usize]);
        i += 2;
    }
    part
}

/// Part 7: modulo every other character (guarding against zero).
fn derive_part_modulo_every_other(chars: &[i64], n: i64) -> i64 {
    let mut part = 0xabi64;
    let mut i = 0i64;
    while i < n - 2 {
        let divisor = chars[i as usize];
        if divisor != 0 {
            part %= divisor;
        }
        i += 2;
    }
    part
}

/// Part 8: divide by successive characters and accumulate the next one.
fn derive_part_dividing_accumulator(chars: &[i64], n: i64) -> i64 {
    let mut part = 0xabi64;
    let mut i = 0i64;
    while i < n - 1 {
        let divisor = chars[i as usize];
        if divisor != 0 {
            part /= divisor;
        }
        i += 1;
        part = part.wrapping_add(chars[i as usize]);
    }
    part
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

    /// The one folder name we have a real, captured key for. Any other folder
    /// name MUST be added together with its key captured from a real AC
    /// installation, not guessed.
    #[test]
    fn derives_known_keys_from_real_installations() {
        let cases = [("cim_2008_ferrari", "172-5-6-58-214-253-22-106")];
        for (folder, expected) in cases {
            assert_eq!(derive_key(folder), expected, "key for {folder} mismatched");
        }
    }

    /// Short folder names exercise loop bounds that the multi-character tests
    /// cannot reach.
    #[test]
    fn derives_keys_for_short_folder_names() {
        for folder in ["", "a", "ab", "abc", "abcd", "abcde"] {
            let key = derive_key(folder);
            // 8 decimal parts joined by hyphens.
            assert_eq!(key.split('-').count(), 8);
            for part in key.split('-') {
                let value: u64 = part.parse().expect("parts are decimal digits");
                assert!(value <= 255, "part {part} escaped one byte in {key}");
            }
        }
    }

    /// The helper must not panic on non-ASCII folder names; `chars()` returns
    /// `char` code points, not bytes.
    #[test]
    fn derive_key_handles_unicode_folder_names() {
        let key = derive_key("フェラーリ");
        assert_eq!(key.split('-').count(), 8);
    }

    /// 单个空 key 必须被拒绝，否则 `decrypt` 会对 key 取模而 panic。
    #[test]
    fn rejects_empty_key() {
        assert!(matches!(
            AcdArchive::from_bytes(b"", "").unwrap_err(),
            Error::EmptyKey
        ));
    }

    #[test]
    fn rejects_oversized_content_length_instead_of_reading() {
        // name "x", then a content_len of u32::MAX => encoded_len overflows
        // the 512 MiB limit before we ever index into the buffer.
        let mut container = Vec::new();
        container.extend_from_slice(&1i32.to_le_bytes());
        container.extend_from_slice(b"x");
        container.extend_from_slice(&u32::MAX.to_le_bytes());
        // A handful of trailing bytes that look like the start of a payload.
        container.extend_from_slice(&[0u8; 32]);

        let err = AcdArchive::from_bytes(&container, "key").unwrap_err();
        assert!(
            matches!(err, Error::InvalidAcd(_)),
            "expected InvalidAcd, got {err:?}"
        );
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
