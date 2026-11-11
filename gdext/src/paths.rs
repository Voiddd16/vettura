//! Godot 资源路径 → 文件系统路径。
//!
//! `vettura-world` / `vettura-kn5` 都用 `std::fs` 读盘，拿到的却是 GDScript 习惯
//! 写法的 `res://...`。这里集中做一次 `ProjectSettings::globalize_path`，让 Rust
//! 侧的函数签名保持纯路径字符串，调用方也不需要为每个资源手写一次转换。

use godot::classes::ProjectSettings;
use godot::prelude::*;

/// Godot 能接受但 `std::fs` 不认识的路径前缀。
const VIRTUAL_PREFIXES: [&str; 2] = ["res://", "user://"];

/// 把 `res://` / `user://` 转成文件系统路径；其它路径原样返回。
///
/// 非虚拟路径（绝对路径、相对路径）不做处理——`CarFolder::open` 和
/// `Kn5::from_path` 直接交给操作系统即可，避免掩盖真实错误。
pub(crate) fn native_path(path: &GString) -> String {
    let raw = path.to_string();
    if is_virtual(&raw) {
        ProjectSettings::singleton()
            .globalize_path(path)
            .to_string()
    } else {
        raw
    }
}

/// 是否是 Godot 虚拟路径（`res://` / `user://`）。
fn is_virtual(raw: &str) -> bool {
    VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| raw.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::is_virtual;

    /// 虚拟前缀必须被识别，否则用户会拿到一个 `res://` 开头的字符串，
    /// 之后在 `std::fs` 里报一个非常隐晦的 "No such file or directory"。
    #[test]
    fn recognizes_virtual_prefixes() {
        for prefix in super::VIRTUAL_PREFIXES {
            assert!(is_virtual(&format!("{prefix}foo")));
        }
        assert!(!is_virtual("file:///foo"));
        assert!(!is_virtual("/tmp/foo"));
        assert!(!is_virtual("../vettura/mods/daytona_2017"));
    }
}
