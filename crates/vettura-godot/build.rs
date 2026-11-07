use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const PINNED_VERSION: &str = "20222";

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn has_core(dir: &Path) -> bool {
    dir.join("libfmod.dylib").exists()
        || dir.join("libfmodstudio.dylib").exists()
        || dir.join("libfmod.so").exists()
        || dir.join("libfmodstudio.so").exists()
        || dir.join("fmod.dll").exists()
        || dir.join("fmodstudio.dll").exists()
        || dir.join("fmod_vc.lib").exists()
        || dir.join("fmod_vc.dll").exists()
}

/// 只找最终 cdylib 链接需要的动态库目录。
///
/// `vettura-audio/build.rs` 负责编译期定位与库符号；这里负责让 Godot 进程真正
/// dlopen 得到 runtime lib：把 SDK 缓存目录和 @loader_path 写进最终二进制 rpath。
fn locate_runtime() -> Option<PathBuf> {
    if let Some(path) = env::var_os("FMOD_SDK_PATH") {
        let path = PathBuf::from(path);
        if has_core(&path) {
            return Some(path);
        }
    }

    let host = target_os();
    let mut candidates = vec![];
    if let Some(cache) = env::var_os("FMOD_CACHE_DIR") {
        let cache = PathBuf::from(cache);
        candidates.push(cache.join("runtime").join(&host));
        candidates.push(cache.join("fmod").join("runtime").join(&host));
    }

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        candidates.push(
            ancestor
                .join(".cache")
                .join("fmod")
                .join("runtime")
                .join(&host),
        );
        candidates.push(
            ancestor
                .join("vettura")
                .join(".cache")
                .join("fmod")
                .join("runtime")
                .join(&host),
        );
    }

    if let Some(found) = candidates.iter().find(|path| has_core(path)) {
        return Some(found.clone());
    }
    None
}

fn main() {
    println!("cargo:rerun-if-env-changed=FMOD_SDK_PATH");
    println!("cargo:rerun-if-env-changed=FMOD_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=FMOD_VERSION");

    if env::var("FMOD_VERSION").unwrap_or_else(|_| PINNED_VERSION.to_owned()) != PINNED_VERSION {
        println!(
            "cargo:warning=vettura-godot 期望 FMOD Engine {PINNED_VERSION}；当前 FMOD_VERSION 不匹配"
        );
    }

    if env::var_os("CARGO_FEATURE_FMOD").is_none() {
        return;
    }

    let Some(runtime) = locate_runtime() else {
        println!(
            "cargo:warning=启用 fmod 但未找到 runtime 库；请先运行主工程 scripts/fetch_fmod.py"
        );
        return;
    };

    println!("cargo:rustc-link-search=native={}", runtime.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", runtime.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");

    // 让 cargo 在 runtime 目录变化时重跑。
    println!("cargo:rerun-if-changed={}", runtime.display());
    let _ = fs::read(runtime.join("fmod_sdk.json"));
}
