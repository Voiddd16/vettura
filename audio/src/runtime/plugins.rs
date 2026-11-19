//! bank 插件（FMOD Distance Filter）的搜索与注册。
//!
//! 整个模块只在 build.rs 找到本地 SDK（`fmod_sdk_found`）时编译。

use std::env;
use std::path::PathBuf;

use libfmod::Studio;

use super::engine::fmod_error;
use crate::error::Result;

/// 注册本地 runtime 目录里的 bank 插件（当前是 FMOD Distance Filter）。
///
/// SDK 里只给插件源码，不发布二进制；`scripts/fetch_fmod.py` 会把它编到
/// `.cache/fmod/runtime/<os>/`，`scripts/pack_fmod.py` 再拷进发行目录。
/// 找不到插件时不报错：老 bank 需要它，普通 bank 不需要。
pub(super) fn load_bank_plugins(studio: &Studio) -> Result<Vec<u32>> {
    let dirs = plugin_dirs();
    if dirs.is_empty() {
        return Ok(vec![]);
    }
    let core = studio.get_core_system().map_err(fmod_error)?;
    let mut handles = vec![];
    let mut loaded: Vec<String> = vec![];

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut candidates: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            let name = name.to_ascii_lowercase();
                            name.contains("fmod") && name.contains("distance")
                        })
            })
            .collect();
        candidates.sort();

        for plugin in candidates {
            let Some(stem) = plugin.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if loaded.iter().any(|name| name == stem) {
                continue; // 前面的目录已经加载过同名插件
            }
            let path = plugin.to_string_lossy().to_string();
            match core.load_plugin(&path, None) {
                Ok(handle) => {
                    loaded.push(stem.to_owned());
                    handles.push(handle);
                }
                Err(_) => {
                    // 某些环境可能已注册同名插件；这里不阻断 bank 加载。
                }
            }
        }
    }

    Ok(handles)
}

/// 插件搜索顺序：发行包优先，开发缓存兜底。
///
/// - `VETTURA_FMOD_PLUGIN_DIR`：显式覆盖（CI / 手动验证用）；
/// - 可执行文件同目录、`Frameworks`、`fmod` 子目录：Godot 导出的 exe 或
///   `.app/Contents/MacOS` 就在那里，`pack_fmod.py` 也是拷到这些位置；
/// - 当前工作目录下的 `fmod/`：编辑器里跑主工程时的兜底；
/// - build.rs 烘焙的 `.cache/fmod/runtime/<os>`：本地开发。
fn plugin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![];

    if let Some(dir) = env::var_os("VETTURA_FMOD_PLUGIN_DIR").filter(|value| !value.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("Frameworks"));
        dirs.push(dir.join("fmod"));
        if let Some(parent) = dir.parent() {
            dirs.push(parent.join("Frameworks"));
            dirs.push(parent.join("fmod"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("fmod"));
    }

    if let Some(dir) = option_env!("VETTURA_FMOD_RUNTIME_PATH") {
        dirs.push(PathBuf::from(dir));
    }

    let mut seen: Vec<PathBuf> = vec![];
    dirs.retain(|dir| {
        let keep = dir.is_dir() && !seen.contains(dir);
        if keep {
            seen.push(dir.clone());
        }
        keep
    });
    dirs
}
