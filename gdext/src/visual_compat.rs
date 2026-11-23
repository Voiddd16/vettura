//! `AcVisualCompat`：把 `vettura-render` 的兼容报告暴露给 GDScript / 编辑器。
//!
//! 这个类只读：扫描一个 mod 目录，返回 JSON 文本。它不修改渲染状态，适合
//! 导入时的兼容性提示、CI 截图前的预检、以及开发者定位未知 shader。

use godot::prelude::*;
use vettura_render::{InspectOptions, inspect_path};

use crate::paths;

/// 视觉模组兼容性检查器。
///
/// ```gdscript
/// var compat := AcVisualCompat.new()
/// print(compat.inspect_json("res://mods/F1 2008 by Cimmerian_Iter"))
/// ```
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct AcVisualCompat {
    base: Base<RefCounted>,
}

#[godot_api]
impl AcVisualCompat {
    /// 扫描目录并返回 JSON 报告。
    ///
    /// `max_kn5` <= 0 时使用默认值（8）。路径支持 `res://` / `user://` 或绝对路径。
    #[func]
    fn inspect_json(&self, path: GString, max_kn5: i64) -> GString {
        let native = paths::native_path(&path);
        let options = InspectOptions {
            max_kn5: if max_kn5 > 0 { max_kn5 as usize } else { 8 },
            ..InspectOptions::default()
        };
        match inspect_path(&native, options) {
            Ok(report) => {
                let text = report.to_json();
                GString::from(&text)
            }
            Err(error) => {
                let text = serde_json::json!({
                    "error": error.to_string(),
                    "path": native,
                })
                .to_string();
                GString::from(&text)
            }
        }
    }

    /// 扫描目录并返回人类可读文本报告。
    #[func]
    fn inspect_text(&self, path: GString, max_kn5: i64) -> GString {
        let native = paths::native_path(&path);
        let options = InspectOptions {
            max_kn5: if max_kn5 > 0 { max_kn5 as usize } else { 8 },
            ..InspectOptions::default()
        };
        match inspect_path(&native, options) {
            Ok(report) => {
                let text = report.to_text();
                GString::from(&text)
            }
            Err(error) => {
                let text = format!("inspect failed: {error}");
                GString::from(&text)
            }
        }
    }
}
