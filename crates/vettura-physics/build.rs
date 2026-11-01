//! `vettura-physics` 的构建脚本（只在 `chrono` feature 打开时做实事）。
//!
//! # 它做什么
//!
//! 1. **定位 Chrono**：`CHRONO_ROOT` 环境变量 > 仓库根的 `third_party/chrono`。
//! 2. 找到 → 用 CMake 按 docs/build.md §8.2 的严格浮点选项构建
//!    `Chrono::Core` + `Chrono::Vehicle`，再把 `cpp/bridge.cpp` 一起编进去。
//! 3. 没找到 → 链 `cpp/chrono_bridge_stub.cpp`（同一 ABI 的占位实现，返回
//!    `VETTURA_CHRONO_ERR_NOT_LINKED`）并打印 warning。这样 workspace 始终能
//!    编译/测试/链接，调用方回退到 `SimpleBackend`。
//!
//! # 为什么不引入 cmake/cc crate
//!
//! 直接命令行调用本机 `cmake` 和 C++ 编译器，`[build-dependencies]` 保持为空。
//! 代价是要自己拼命令行（以及 MSVC 路径尚未实现，见文末 TODO）。
//!
//! # 跨编译器一致性
//!
//! flag 必须和文档一致：`-ffp-contract=off` + 禁 fast-math + 禁
//! `-march=native`/`-mfma`，并强制 `ENABLE_OPENMP=OFF`（线程数不能改变归约
//! 顺序）。见 docs/build.md §8.2。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cpp/");
    println!("cargo:rerun-if-env-changed=CHRONO_ROOT");
    println!("cargo:rerun-if-env-changed=VETTURA_CHRONO_CXX");

    // feature 没开：这个 crate 只有纯 Rust，不碰 C++。
    if env::var_os("CARGO_FEATURE_CHRONO").is_none() {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let build_dir = out_dir.join("chrono-build");

    match locate_chrono(&manifest_dir) {
        Some(root) => build_with_chrono(&manifest_dir, &root, &build_dir),
        None => build_stub(&manifest_dir, &out_dir),
    }
}

/// `CARGO_MANIFEST_DIR` = `<workspace>/crates/vettura-physics`。
fn workspace_root(manifest_dir: &Path) -> Option<&Path> {
    manifest_dir.ancestors().nth(2)
}

fn locate_chrono(manifest_dir: &Path) -> Option<PathBuf> {
    if let Some(root) = env::var_os("CHRONO_ROOT") {
        let root = PathBuf::from(root);
        if root.join("CMakeLists.txt").is_file() {
            return Some(root);
        }
        println!(
            "cargo:warning=CHRONO_ROOT={} does not look like a Chrono checkout (no \
             CMakeLists.txt); falling back to third_party/chrono",
            root.display()
        );
    }

    let vendored = workspace_root(manifest_dir)?.join("third_party/chrono");
    if vendored.join("CMakeLists.txt").is_file() {
        println!(
            "cargo:warning=vendoring Chrono from {}; the first build takes a while",
            vendored.display()
        );
        return Some(vendored);
    }
    None
}

/// 我们自己的胶水代码（和 stub 是互斥的两个实现，同一个 ABI）。
fn bridge_sources(manifest_dir: &Path) -> Vec<PathBuf> {
    vec![
        manifest_dir.join("cpp/bridge.cpp"),
        manifest_dir.join("cpp/vettura_vehicle_spec.cpp"),
    ]
}

fn detect_cxx() -> String {
    if let Some(cxx) = env::var_os("VETTURA_CHRONO_CXX") {
        return cxx.to_string_lossy().into_owned();
    }
    for candidate in ["c++", "g++", "clang++"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return candidate.to_string();
        }
    }
    "c++".to_string()
}

/// docs/build.md §8.2 的严格浮点选项（clang/gcc）。
fn strict_cxx_flags() -> Vec<String> {
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env.contains("msvc") {
        // TODO(msvc): MSVC 路径还没接。接上来时这里是 /fp:strict /EHsc。
        return Vec::new();
    }
    let mut flags = vec![
        // 禁止 FMA 收缩：clang/gcc 默认 -ffp-contract=fast，MSVC 不收缩，
        // 这是 clang 与 MSVC 结果分叉的头号来源。
        "-ffp-contract=off".to_string(),
        // fast-math 一律禁用（clang/gcc 各家的 fast-math 集合还不一样）。
        "-fno-fast-math".to_string(),
        "-fno-unsafe-math-optimizations".to_string(),
        "-fno-reciprocal-math".to_string(),
        "-fno-finite-math-only".to_string(),
        "-fno-math-errno".to_string(),
    ];
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "x86" || arch == "x86_64" {
        // x86 专属：不用 FMA，不超基线 ISA（不同机器向量化宽度不同，Eigen 的
        // 分块顺序也会变）。arm64 没有这些开关，平台基线 ISA 本身就是固定的。
        flags.push("-mno-fma".to_string());
        flags.push("-mno-avx2".to_string());
        flags.push("-mno-avx512f".to_string());
    }
    flags
}

/// 探测本机 C++ 工具链能否 `#include <string>`；不能就加上兜底 include 路径。
///
/// 背景：有些 macOS 机器上 `/Library/Developer/CommandLineTools/usr/include/c++/v1`
/// 只剩 3 个文件（CLT 安装不完整/版本错位），而 SDK 里的 `c++/v1` 是完整的。
/// clang 优先搜前者，于是连 `<string>` 都找不到。这里做一次编译探测，坏了就
/// `-nostdinc++` 并显式指向 SDK 里的 libc++ 头。
fn workaround_broken_libcxx(compiler: &str) -> Vec<String> {
    let probe = r#"#include <string>
int main() { std::string s = "ok"; return s.empty() ? 1 : 0; }
"#;
    let dir = env::temp_dir();
    let src = dir.join("vettura_cxx_probe.cpp");
    if std::fs::write(&src, probe).is_err() {
        return Vec::new();
    }
    let ok = Command::new(compiler)
        .arg("-std=c++17")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("vettura_cxx_probe.bin"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        return Vec::new();
    }

    // 收集带完整 c++/v1 的 SDK，取版本最高的一个（排序后最后一个）。
    let mut flags = vec!["-nostdinc++".to_string()];
    let mut found = false;
    for root in [
        PathBuf::from("/Library/Developer/CommandLineTools/SDKs"),
        PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs",
        ),
    ] {
        if let Ok(entries) = fs::read_dir(&root) {
            let mut sdks: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            sdks.sort();
            for sdk in sdks {
                if sdk.join("usr/include/c++/v1/string").is_file() {
                    flags = vec![
                        "-nostdinc++".to_string(),
                        "-isysroot".to_string(),
                        sdk.display().to_string(),
                        "-I".to_string(),
                        sdk.join("usr/include/c++/v1").display().to_string(),
                    ];
                    found = true;
                }
            }
        }
    }
    if !found {
        println!(
            "cargo:warning=the C++ toolchain cannot find <string> and no SDK with a complete \
             libc++ was found. Run `xcode-select --install`, or reinstall Xcode CLT, then retry"
        );
        return Vec::new();
    }
    flags
}

/// 把我们自己的胶水代码编成静态库；`extra_flags` 是额外的编译选项
/// （Chrono 的 include 路径等，见 `chrono_compile_flags`）。
///
/// `-o <archive>` 对 clang/gcc 不成立（那不是 ar 归档），所以这里走
/// 「编译成 .o → ar 成 .a」两步。MSVC 路径 TODO（build.rs 顶部）。
fn compile_bridge(out_dir: &Path, sources: &[PathBuf], extra_flags: &[String]) -> PathBuf {
    let compiler = detect_cxx();
    let toolchain_fixups = workaround_broken_libcxx(&compiler);

    let mut objects = Vec::new();
    for src in sources {
        let object = out_dir.join(
            src.file_name()
                .expect("bridge source file name")
                .to_string_lossy()
                .replace('.', "_")
                + ".o",
        );
        let mut cmd = Command::new(&compiler);
        cmd.arg("-std=c++17")
            .arg("-O2")
            .arg("-fPIC")
            .arg("-fvisibility=hidden")
            .arg("-c")
            .arg(src)
            .arg("-o")
            .arg(&object)
            .arg("-Wall")
            .arg("-Wextra");
        for flag in strict_cxx_flags().iter().chain(toolchain_fixups.iter()) {
            cmd.arg(flag);
        }
        for flag in extra_flags {
            cmd.arg(flag);
        }
        let status = cmd.status().unwrap_or_else(|e| {
            panic!(
                "failed to run C++ compiler {compiler:?}: {e}\n\
                 install a C++17 toolchain (clang or g++), or point VETTURA_CHRONO_CXX at one"
            )
        });
        assert!(
            status.success(),
            "compiling {} failed (status {status})",
            src.display()
        );
        objects.push(object);
    }

    let archive = out_dir.join("libvettura_chrono_bridge.a");
    let status = Command::new("ar")
        .arg("crs")
        .arg(&archive)
        .args(&objects)
        .status()
        .expect("run `ar`");
    assert!(status.success(), "`ar` failed (status {status})");
    archive
}

/// 从 CMake 生成的 `compile_commands.json` 里抓 Chrono 自己的编译选项
/// （`-I` / `-isystem` / `-D`），用于编译我们的胶水代码。
///
/// 为什么不自己拼：Chrono 的 include 集合并不可预测（Eigen 的位置、生成
/// 头的目录随版本和平台变），跟着 CMake 走最稳。
fn chrono_compile_flags(build_dir: &Path, chrono_root: &Path) -> Vec<String> {
    let mut flags = vec![
        format!("-I{}", chrono_root.join("src").display()),
        format!("-I{}", build_dir.display()),
    ];
    if let Some(install_include) = env::var_os("CHRONO_ROOT").and_then(|r| {
        let r = PathBuf::from(r);
        r.join("include").is_dir().then_some(r.join("include"))
    }) {
        flags.push(format!("-I{}", install_include.display()));
    }

    let path = build_dir.join("compile_commands.json");
    let Ok(text) = fs::read_to_string(&path) else {
        println!(
            "cargo:warning={} not found; falling back to -Isrc only \
             (add Eigen's include dir with CHRONO_CXXFLAGS if compiling fails)",
            path.display()
        );
        return flags;
    };

    // 逐条取 command，只保留 include / define 类选项。
    let mut seen: Vec<String> = Vec::new();
    for command in extract_command_strings(&text) {
        let tokens: Vec<&str> = command.split(' ').collect();
        for (index, token) in tokens.iter().enumerate() {
            // Eigen 之类常写成 "-isystem <path>"（分立两个 token）。
            if *token == "-isystem" || *token == "-I" {
                let next = tokens.get(index + 1).copied().unwrap_or("");
                if !next.is_empty() {
                    let flag = format!("{token}{next}");
                    if !flags.contains(&flag) && !seen.contains(&flag) {
                        seen.push(flag);
                    }
                }
                continue;
            }
            let flag = if let Some(rest) = token.strip_prefix("-I") {
                Some(format!("-I{rest}"))
            } else if let Some(rest) = token.strip_prefix("-isystem") {
                Some(format!("-isystem{rest}"))
            } else if token.starts_with("-D") {
                Some((*token).to_string())
            } else {
                None
            };
            if let Some(flag) = flag
                && !flags.contains(&flag)
                && !seen.contains(&flag)
            {
                seen.push(flag);
            }
        }
    }
    flags.extend(seen);
    flags
}

/// 从 compile_commands.json 里粗抽出每条 "command" 字符串。
///
/// 不做完整 JSON 解析：只要 command 字段的值。逃义的引号按字面处理
/// （include 路径里通常没有）。
fn extract_command_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let key = "\"command\": \"";
    let mut offset = 0;
    while let Some(start) = text[offset..].find(key) {
        let begin = offset + start + key.len();
        // command 值以 未转义的 "\n" 结尾：找最后出现的 `\"` 后紧跟 `,` 或 `}`。
        let mut cursor = begin;
        let mut end = None;
        while cursor < text.len() {
            if text.as_bytes()[cursor] == b'\\' {
                cursor += 2;
                continue;
            }
            if text.as_bytes()[cursor] == b'"' {
                end = Some(cursor);
                break;
            }
            cursor += 1;
        }
        if let Some(end) = end {
            out.push(text[begin..end].replace("\\\"", "\""));
            offset = end + 1;
        } else {
            break;
        }
    }
    out
}

/// 我们的 C++ 用了 libc++ (`std::string`)，Rust 侧链接时必须带上 C++ 标准库。
fn link_cxx_stdlib() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env_kind = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "macos" || os == "ios" {
        // macOS 的 `c++` 是 libc++ 的 dylib 存根。
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if env_kind.contains("gnu") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    } else if env_kind.contains("msvc") {
        // MSVC: C++ 运行时由 cl 编译单元自带；这里不需要额外库。
    }
}

fn build_stub(manifest_dir: &Path, out_dir: &Path) {
    println!(
        "cargo:warning=Chrono not found: put the sources in third_party/chrono (see \
         third_party/README.md) or set CHRONO_ROOT. Linking the placeholder bridge — \
         `ChronoBackend::try_new` returns `ChronoError::NotLinked` so callers fall back \
         to `SimpleBackend`."
    );
    compile_bridge(
        out_dir,
        &[
            manifest_dir.join("cpp/chrono_bridge_stub.cpp"),
            manifest_dir.join("cpp/vettura_vehicle_spec.cpp"),
        ],
        &[format!("-I{}", manifest_dir.join("cpp").display())],
    );
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=vettura_chrono_bridge");
    link_cxx_stdlib();

    // 同样的冒烟检查：stub 路径上也要保证 JSON 生成层没坏。
    smoke_check_vehicle_json(
        manifest_dir,
        out_dir,
        &[format!("-I{}", manifest_dir.join("cpp").display())],
    );
}

fn build_with_chrono(manifest_dir: &Path, chrono_root: &Path, build_dir: &Path) {
    if let Some(root) = workspace_root(manifest_dir) {
        // Chrono 被拉取/改动时重新配置。
        println!(
            "cargo:rerun-if-changed={}",
            root.join("third_party/chrono/CMakeLists.txt").display()
        );
    }

    // ---- 1. CMake 配置 + 构建：只 Core + Vehicle ----
    fs::create_dir_all(build_dir)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", build_dir.display()));

    let mut cmake = Command::new("cmake");
    cmake
        .arg("-S")
        .arg(chrono_root)
        .arg("-B")
        .arg(build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON")
        .arg("-DENABLE_OPENMP=OFF")
        .arg("-DCH_ENABLE_MODULE_VEHICLE=ON")
        .arg("-DENABLE_MODULE_IRRLICHT=OFF")
        .arg("-DENABLE_MODULE_VSG=OFF")
        .arg("-DENABLE_MODULE_SENSOR=OFF")
        .arg("-DENABLE_MODULE_FSI=OFF")
        .arg("-DENABLE_MODULE_PARSERS=OFF")
        .arg("-DENABLE_MODULE_MODAL=OFF")
        .arg("-DENABLE_MODULE_POSTPROCESS=OFF")
        .arg("-DENABLE_MODULE_DISTRIBUTED=OFF")
        .arg("-DENABLE_MODULE_CASCADE=OFF")
        .arg("-DENABLE_MODULE_MUMPS=OFF")
        .arg("-DENABLE_MODULE_PARDISO_MKL=OFF")
        .arg("-DENABLE_MODULE_MATLAB=OFF")
        .arg("-DENABLE_MODULE_COSIMULATION=OFF")
        // Chrono 10 的开关名是 BUILD_*，不是 ENABLE_*（后者会被忽略）。
        .arg("-DBUILD_TESTING=OFF")
        .arg("-DBUILD_BENCHMARKING=OFF")
        .arg("-DBUILD_DEMOS=OFF")
        .arg("-DBUILD_SAMPLES=OFF")
        // Chrono 的 FindSIMD 在 clang≥15 / gcc 上会追加 `-march=native`（arm64 是
        // 具体 CPU 型号，x86 是 AVX2/FMA 等），而且它排在 CMAKE_CXX_FLAGS 之后，
        // 关不掉。所以逐位一致性的承诺是「**同一台机器**上的不同编译器」；跨机器
        // 需要锁基线 ISA（TODO：用 -DCMAKE_CXX_COMPILER_TARGET / 打补丁跳过
        // FindSIMD）。x86 上再显式关掉 AVX2/AVX，尽量把差距收回到基线。
        .arg(format!(
            "-DCMAKE_CXX_FLAGS={}",
            strict_cxx_flags()
                .iter()
                .chain(workaround_broken_libcxx(&detect_cxx()).iter())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        ));
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "x86_64" || arch == "x86" {
        cmake.arg("-DSIMD_USE_AVX2=OFF").arg("-DSIMD_USE_AVX=OFF");
    }

    let status = cmake.status().unwrap_or_else(|e| {
        panic!(
            "cannot run `cmake` (needed to build Chrono): {e}\n\
             install CMake ≥ 3.15, or set CHRONO_ROOT to a prebuilt Chrono install"
        )
    });
    assert!(
        status.success(),
        "CMake configuration of Chrono failed (status {status})"
    );

    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(build_dir)
        .arg("--parallel")
        .arg(env::var("NUM_JOBS").unwrap_or_else(|_| "1".to_string()));
    let status = build.status().expect("cmake --build");
    assert!(status.success(), "building Chrono failed (status {status})");

    // ---- 2. 编译我们自己的胶水 ----
    // include 路径跟着 CMake 的 compile_commands.json 走（Eigen 等）。
    let chrono_flags = chrono_compile_flags(build_dir, chrono_root);
    let archive = compile_bridge(build_dir, &bridge_sources(manifest_dir), &chrono_flags);
    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=vettura_chrono_bridge");
    let _ = archive;

    // ---- 2b. spec → JSON 冒烟检查（Chrono 就位时） ----
    smoke_check_vehicle_json(manifest_dir, build_dir, &chrono_flags);

    // ---- 3. 链接 Chrono ----
    // Chrono 10 默认产出 **dylib**（libChrono_*.dylib），也可能被开关切成 .a；
    // 位置分别在 <build>/ 和 <build>/lib。这里扫两处，按「依赖方在前、
    // 被依赖方在后」的逆字母序链接，Chrono_core 兜底排最后。
    let lib_dirs = [build_dir.to_path_buf(), build_dir.join("lib")];
    let mut names: Vec<String> = Vec::new();
    for dir in &lib_dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = name
                    .strip_prefix("lib")
                    .and_then(|s| s.strip_suffix(".a").or_else(|| s.strip_suffix(".dylib")));
                if let Some(stem) = stem
                    && stem.starts_with("Chrono")
                    && !names.iter().any(|n| n == stem)
                {
                    names.push(stem.to_string());
                }
            }
        }
    }
    if names.is_empty() {
        println!(
            "cargo:warning=no libChrono* found in {} (or its lib/ subdir); linking will likely \
             fail. Inspect the CMake output above and adjust the target names in build.rs.",
            build_dir.display()
        );
    } else {
        names.sort();
        // 反向排序：Z→A，依赖方在前（Chrono_* 命名大体上字母序 ≈ 依赖序）。
        names.reverse();
        // Chrono_core 是公共底座，排到最后。
        if let Some(pos) = names.iter().position(|l| l == "Chrono_core") {
            let core = names.remove(pos);
            names.push(core);
        }
        for dir in &lib_dirs {
            if dir.is_dir() {
                println!("cargo:rustc-link-search=native={}", dir.display());
            }
        }
        for lib in &names {
            // dylib 优先（Chrono 默认）；cargo 对 static=/dylib= 的处理不同，
            // 这里按后缀选：.a → static，.dylib → dylib。
            let path = lib_dirs
                .iter()
                .find(|d| d.join(format!("lib{lib}.a")).is_file())
                .map(|d| d.join(format!("lib{lib}.a")))
                .unwrap_or_else(|| lib_dirs[0].join(format!("lib{lib}.dylib")));
            if path.extension().is_some_and(|e| e == "a") {
                println!("cargo:rustc-link-lib=static={lib}");
            } else {
                println!("cargo:rustc-link-lib=dylib={lib}");
            }
        }
    }
    link_cxx_stdlib();
}

/// 编译 `cpp/bridge_dump.cpp` 并跑一遍，确认「spec → Chrono JSON」这层没坏：
/// 顶层 schema 的必需 key 都在，子系统文件也都落盘了。
///
/// 这一步只用标准 C++，不依赖 Chrono，所以 stub / 真链接两条路径都能跑。
fn smoke_check_vehicle_json(manifest_dir: &Path, out_dir: &Path, extra_flags: &[String]) {
    let compiler = detect_cxx();
    let dump_bin = out_dir.join("vettura_bridge_dump");
    let mut cmd = Command::new(&compiler);
    cmd.arg("-std=c++17")
        .arg("-O1")
        .arg("-fPIC")
        .arg("-c")
        .arg(manifest_dir.join("cpp/bridge_dump.cpp"))
        .arg("-o")
        .arg(out_dir.join("bridge_dump.o"))
        .arg("-I")
        .arg(manifest_dir.join("cpp"));
    for flag in strict_cxx_flags()
        .iter()
        .chain(workaround_broken_libcxx(&compiler).iter())
    {
        cmd.arg(flag);
    }
    for flag in extra_flags {
        cmd.arg(flag);
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to run {compiler:?}: {e}"));
    assert!(
        status.success(),
        "compiling bridge_dump.cpp failed (status {status})"
    );

    // 链接 dump 工具（把同一份 vettura_vehicle_spec.o 一起链进去）。
    let json_object = out_dir.join("vettura_vehicle_spec_cpp.o");
    let link = Command::new(&compiler)
        .arg(&json_object)
        .arg(out_dir.join("bridge_dump.o"))
        .arg("-o")
        .arg(&dump_bin)
        .status()
        .unwrap_or_else(|e| panic!("failed to link bridge_dump: {e}"));
    assert!(link.success(), "linking bridge_dump failed");

    let out_dir_json = out_dir.join("smoke-json");
    let _ = fs::remove_dir_all(&out_dir_json);
    let run = Command::new(&dump_bin)
        .arg(&out_dir_json)
        .output()
        .unwrap_or_else(|e| panic!("failed to run bridge_dump: {e}"));
    assert!(run.status.success(), "bridge_dump failed");

    let vehicle_json = out_dir_json.join("vehicle/vettura_vehicle.json");
    let text = fs::read_to_string(&vehicle_json)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", vehicle_json.display()));
    for key in [
        "\"Template\": \"WheeledVehicle\"",
        "\"Chassis\"",
        "\"Axles\"",
        "\"Steering Subsystems\"",
        "\"Wheelbase\"",
        "\"Driveline\"",
        "\"Powertrain\"",
        "\"Tire Input File\"",
    ] {
        assert!(
            text.contains(key),
            "generated vehicle JSON is missing {key}; the Chrono schema may have drifted"
        );
    }
    for subsystem in [
        "vettura/chassis/chassis.json",
        "vettura/suspension/front.json",
        "vettura/wheel/wheel.json",
        "vettura/brake/front.json",
        "vettura/steering/steering.json",
        "vettura/driveline/driveline.json",
        "vettura/powertrain/engine.json",
        "vettura/powertrain/transmission.json",
        "vettura/tire/tire.json",
    ] {
        assert!(
            out_dir_json.join(subsystem).is_file(),
            "generated subsystem JSON is missing: {subsystem}"
        );
    }
    let _ = fs::remove_dir_all(&out_dir_json);
}
