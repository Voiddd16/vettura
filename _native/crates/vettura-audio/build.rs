use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const PINNED_VERSION: &str = "20222";

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn target_arch() -> String {
    env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default()
}

fn profile() -> String {
    env::var("PROFILE").unwrap_or_else(|_| "debug".to_owned())
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn has_core(dir: &Path) -> bool {
    dir.join("core").exists()
}

fn find_api_in(root: &Path) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    if has_core(root) && root.join("studio").exists() {
        return Some(root.to_path_buf());
    }
    // 解压目录可能是 `<unpacked>/<plat>/api`、`<unpacked>/<plat>/FMOD Programmers API/api`
    // 或者安装器展开后的临时层级，这里做一次有深度限制的搜索。
    if let Some(candidate) = read_dir_recursive(root, 4).into_iter().find(|candidate| {
        candidate.file_name().and_then(|name| name.to_str()) == Some("api")
            && has_core(candidate)
            && candidate.join("studio").exists()
    }) {
        return Some(candidate);
    }
    None
}

fn read_dir_recursive(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut directories = vec![];
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    while let Some((dir, depth)) = pending.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path.clone());
                pending.push((path, depth + 1));
            }
        }
    }
    directories
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut roots = vec![];

    // 1) 显式 SDK 路径，优先级最高
    if let Some(path) = env_path("FMOD_SDK_PATH") {
        roots.push(path);
    }

    // 2) scripts/fetch_fmod.py 输出：runtimes 优先，其次 unpacked
    if let Some(cache) = env_path("FMOD_CACHE_DIR") {
        roots.push(cache.join("runtime"));
    }

    // 3) 当前 crate / 仓库 / 兄弟工程（vettura 主工程）里的约定位置
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        roots.push(ancestor.join(".cache").join("fmod"));
        roots.push(ancestor.join("vettura").join(".cache").join("fmod"));
        roots.push(ancestor.join(".cache"));
    }

    roots
}

fn locate_sdk() -> Option<PathBuf> {
    let host = target_os();

    // 1) 显式 SDK 路径
    if let Some(path) = env_path("FMOD_SDK_PATH")
        && has_core(&path)
    {
        return Some(path);
    }

    // 2) fetch_fmod.py 写的 runtime 目录（无空格、只放动态库，禁止入库）
    if let Some(cache) = env_path("FMOD_CACHE_DIR") {
        let runtime = cache.join("runtime").join(&host);
        if has_core(&runtime) {
            return Some(runtime);
        }
    }

    // 3) JSON 定位文件：{ "runtimes": { "macos": "..." } }
    for root in candidate_paths() {
        let json = root.join("fmod_sdk.json");
        if let Some(path) = parse_runtime_from_json(&json, &host)
            && has_core(&path)
        {
            return Some(path);
        }
    }

    // 4) `scripts/fetch_fmod.py` 解压布局
    for root in candidate_paths() {
        let unpacked = root.join("unpacked");
        for candidate in [
            unpacked.join(&host).join("api"),
            unpacked
                .join(&host)
                .join("FMOD Programmers API")
                .join("api"),
            unpacked.join(&host).join("fmod").join("api"),
        ] {
            if has_core(&candidate) && candidate.join("studio").exists() {
                return Some(candidate);
            }
        }
        if let Some(candidate) = find_api_in(&unpacked)
            && has_core(&candidate)
        {
            return Some(candidate);
        }
    }

    // 5) 各平台默认安装位置
    let defaults: &[&str] = &[
        #[cfg(target_os = "windows")]
        r"C:\Program Files (x86)\FMOD SoundSystem\FMOD Studio API Windows\api",
        #[cfg(target_os = "macos")]
        "/Library/FMOD/api",
        #[cfg(target_os = "linux")]
        "/usr/local/fmod/api",
    ];
    defaults
        .iter()
        .map(PathBuf::from)
        .find(|path| has_core(path))
}

/// 只解析 build 需要的字段，避免引入额外 build-dependency。
fn parse_runtime_from_json(json: &Path, host: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(json).ok()?;
    let marker = format!("\"{host}\"");
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let quote_start = rest.find('"')? + 1;
    let rest = &rest[quote_start..];
    let quote_end = rest.find('"')? + 1;
    Some(PathBuf::from(rest[..quote_end - 1].to_owned()))
}

fn has_runtime_lib(dir: &Path) -> bool {
    ["libfmod.dylib", "libfmod.so", "fmod.dll", "fmod_vc.lib"]
        .iter()
        .any(|name| dir.join(name).exists())
}

/// 只找运行期动态库目录（`scripts/fetch_fmod.py` 的 `.cache/fmod/runtime/<os>`）。
fn locate_runtime() -> Option<PathBuf> {
    let host = target_os();
    let mut roots = vec![];

    if let Some(cache) = env_path("FMOD_CACHE_DIR") {
        roots.push(cache.join("runtime"));
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        roots.push(ancestor.join(".cache").join("fmod").join("runtime"));
        roots.push(
            ancestor
                .join("vettura")
                .join(".cache")
                .join("fmod")
                .join("runtime"),
        );
    }

    roots
        .into_iter()
        .find(|root| has_runtime_lib(&root.join(&host)))
        .map(|root| root.join(host))
}

fn has_lib_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if !path.is_file() {
            return false;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        name.starts_with("fmod") || name.starts_with("libfmod")
    })
}

/// SDK 里动态库可能在 `api/<api>/lib` 或 `api/<api>/lib/<arch>`（Linux/Windows）。
fn lib_dir(sdk: &Path, api: &str, arch: &str) -> PathBuf {
    let direct = sdk.join(api).join("lib");
    if has_lib_files(&direct) {
        return direct;
    }
    let wanted = direct.join(arch);
    if has_lib_files(&wanted) {
        return wanted;
    }
    if let Ok(entries) = fs::read_dir(&direct) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && has_lib_files(&path) {
                return path;
            }
        }
    }
    wanted
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(fmod_sdk_found, fmod_sdk_missing)");
    println!("cargo:rerun-if-env-changed=FMOD_SDK_PATH");
    println!("cargo:rerun-if-env-changed=FMOD_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=FMOD_VERSION");
    println!("cargo:rerun-if-env-changed=PROFILE");

    // 未指定锁定版本时给出警告；本 workspace 的锁定版本是 2.02.22。
    let version = env::var("FMOD_VERSION").unwrap_or_else(|_| PINNED_VERSION.to_owned());
    if version != PINNED_VERSION {
        println!(
            "cargo:warning=vettura-audio 锁定的 FMOD Engine 版本是 {PINNED_VERSION}，当前 FMOD_VERSION={version}"
        );
    }

    let feature_enabled = env::var_os("CARGO_FEATURE_FMOD").is_some();
    let Some(sdk) = locate_sdk() else {
        if feature_enabled {
            println!("cargo:warning=未找到 FMOD SDK，`fmod` feature 将以 stub 模式编译");
            println!("cargo:warning=请先在 Godot 主工程执行：cd scripts && uv run fetch_fmod.py");
            println!("cargo:warning=或在 build.rs 定位位置参见 docs/build.md 的 FMOD 小节");
        }
        println!("cargo:rustc-cfg=fmod_sdk_missing");
        return;
    };

    if !feature_enabled {
        // 没启用 feature 时不链接，也不告诉用户缺 SDK。
        println!("cargo:rustc-cfg=fmod_sdk_missing");
        return;
    }

    println!("cargo:rustc-cfg=fmod_sdk_found");
    println!("cargo:rustc-env=VETTURA_FMOD_SDK_PATH={}", sdk.display());

    if let Some(runtime) = locate_runtime() {
        println!(
            "cargo:rustc-env=VETTURA_FMOD_RUNTIME_PATH={}",
            runtime.display()
        );
    }

    let os = target_os();
    let arch = if os == "windows" {
        match target_arch().as_str() {
            "x86_64" => "x64",
            "x86" => "x86",
            "aarch64" => "arm64",
            _ => "x64",
        }
        .to_owned()
    } else {
        target_arch()
    };

    let debug_build = profile() != "release";
    let core_lib = lib_dir(&sdk, "core", &arch);
    let studio_lib = lib_dir(&sdk, "studio", &arch);
    for dir in [&core_lib, &studio_lib] {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    // 只动态链接；禁止静态打包 FMOD（EULA 与发行规则都不允许）。
    match os.as_str() {
        "windows" => {
            let core = if debug_build { "fmodL_vc" } else { "fmod_vc" };
            let studio = if debug_build {
                "fmodstudioL_vc"
            } else {
                "fmodstudio_vc"
            };
            println!("cargo:rustc-link-lib=dylib={core}");
            println!("cargo:rustc-link-lib=dylib={studio}");
            println!(
                "cargo:warning=Windows 运行期用主工程 scripts/pack_fmod.py 把 fmod.dll / fmodstudio.dll 拷到 exe 同目录"
            );
        }
        "macos" => {
            let core = if debug_build { "fmodL" } else { "fmod" };
            let studio = if debug_build {
                "fmodstudioL"
            } else {
                "fmodstudio"
            };
            println!("cargo:rustc-link-lib=dylib={core}");
            println!("cargo:rustc-link-lib=dylib={studio}");
            // 开发阶段允许从 SDK 缓存加载；发布阶段由 scripts/pack_fmod.py 拷贝。
            for dir in [&core_lib, &studio_lib] {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
            }
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
        }
        "linux" => {
            let core = if debug_build { "fmodL" } else { "fmod" };
            let studio = if debug_build {
                "fmodstudioL"
            } else {
                "fmodstudio"
            };
            println!("cargo:rustc-link-lib=dylib={core}");
            println!("cargo:rustc-link-lib=dylib={studio}");
            for dir in [&core_lib, &studio_lib] {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
            }
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        }
        _ => {
            println!("cargo:warning=未支持的目标平台：{os}");
            println!("cargo:rustc-cfg=fmod_sdk_missing");
        }
    }
}
