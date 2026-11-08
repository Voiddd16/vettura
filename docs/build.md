# 编译指南

本文档列出编译本项目不同产物所需的命令。所有命令默认在
`vettura-native/` 目录下执行。

## 1. 环境准备

| 依赖 | 要求 | 检查命令 |
| --- | --- | --- |
| Rust | stable，且 ≥ 1.94（workspace `rust-version`） | `rustc --version` |
| Cargo | 随 Rust 安装 | `cargo --version` |
| rustfmt / clippy | 由 `rust-toolchain.toml` 声明 | `cargo fmt --version` / `cargo clippy --version` |
| Xcode Command Line Tools（macOS） | 编译 dylib 与链接需要 | `xcode-select -p` |
| Godot | 4.7（当前工程 4.7.2），仅运行扩展时需要 | `godot --version` |

首次拉取仓库后，建议先确认工具链：

```bash
rustup show
```

如果 `rustc` 版本低于 1.94，先升级：

```bash
rustup update stable
```

## 2. 产物与命令速查

| 想得到什么 | 命令 | 产物位置 |
| --- | --- | --- |
| 日常检查（不编译 Godot 扩展） | `cargo check --workspace --exclude vettura-godot` | 无 |
| 默认成员调试构建 | `cargo build` | `target/debug/` |
| 默认成员发布构建 | `cargo build --release` | `target/release/` |
| 单个库 crate | `cargo build -p vettura-kn5` | `target/debug/libvettura_kn5.rlib` |
| Godot 扩展（调试） | `cargo build -p vettura-godot` | `target/debug/libvettura_native.dylib` |
| Godot 扩展（发布） | `cargo build -p vettura-godot --release` | `target/release/libvettura_native.dylib` |
| 整个 workspace（含 Godot 扩展） | `cargo build --workspace` | `target/debug/` |
| KN5 检查工具 | `cargo build --release -p kn5-inspect` | `target/release/kn5-inspect` |
| ACD 检查工具 | `cargo build --release -p acd-inspect` | `target/release/acd-inspect` |

> **关键区别**：`Cargo.toml` 的 `default-members` 刻意排除了
> `vettura-godot`。因此 `cargo build` **不会**编译 Godot 扩展；
> 需要扩展时必须显式 `cargo build -p vettura-godot`，或使用
> `cargo build --workspace`。

## 3. 纯 Rust 部分

### 3.1 检查与编译

```bash
# 只做类型检查，速度最快，不产生最终二进制
cargo check --workspace --exclude vettura-godot

# 调试构建所有默认成员（core / kn5 / acdata / assets / physics / tools）
cargo build

# 发布构建所有默认成员
cargo build --release
```

默认成员包括：

```
vettura-core
vettura-kn5
vettura-acdata
vettura-assets
vettura-physics
kn5-inspect
acd-inspect
```

### 3.2 只编译某一个 crate

```bash
cargo build -p vettura-core
cargo build -p vettura-kn5
cargo build -p vettura-acdata
cargo build -p vettura-assets
cargo build -p vettura-physics
# Chrono 后端（需要 CMake / C++ 工具链，见第 8 节）
cargo build -p vettura-physics --features chrono
```

库 crate 默认只产出 `rlib`。如果只想确认它能通过编译，用 `cargo check -p <crate>` 更快。

### 3.3 清理

```bash
# 清理整个 workspace 的构建缓存
cargo clean

# 只清理某个 crate 的产物
cargo clean -p vettura-godot
```

## 4. Godot GDExtension

### 4.1 调试版（开发默认）

```bash
cargo build -p vettura-godot
```

产物（macOS）：

```
target/debug/libvettura_native.dylib
```

`godot/vettura_native.gdextension` 已把 `macos.debug` 指向该路径：

```ini
macos.debug = "res://vettura-native/target/debug/libvettura_native.dylib"
```

因此编译完成后，Godot 工程根目录下必须有 `vettura-native` 这个链接名（默认约定
工程在 `../vettura`，用 `./scripts/link_into_godot.sh` 建立）。路径搞定时打开该
`project.godot` 即可加载扩展。

### 4.2 发布版

```bash
cargo build -p vettura-godot --release
```

产物：

```
target/release/libvettura_native.dylib
```

`.gdextension` 中已有对应的 `macos.release` 配置：

```ini
macos.release = "res://vettura-native/target/release/libvettura_native.dylib"
```

### 4.3 其他平台产物名

| 平台 | 产物 |
| --- | --- |
| macOS | `libvettura_native.dylib` |
| Linux | `libvettura_native.so` |
| Windows | `vettura_native.dll` |

`.gdextension` 已包含三个平台的 debug/release 路径。跨平台编译时还需要
`rustup target add <target>` 以及对应的链接器（例如 Windows GNU 需要 MinGW）。

### 4.4 验证扩展能否加载

```bash
# 在 vettura-native/ 下执行；--path 指向 Godot 工程根目录
godot --headless --path /path/to/godot/project --editor --quit
```

如果扩展加载失败，优先检查：

1. `target/debug/libvettura_native.dylib` 是否真的存在；
2. `.gdextension` 中的路径是否与 `target` 目录一致；
3. `godot` crate 的 feature 是否为 `api-4-7`（在根 `Cargo.toml` 中）；
4. Godot 版本是否为 4.7。

> 不要随意设置 `CARGO_TARGET_DIR`。`.gdextension` 写死了
> `res://vettura-native/target/...`，改变 target 目录会导致 Godot 找不到库。
> 如确需修改，请同步修改 `.gdextension`。

## 5. 命令行工具

两个工具都支持 `cargo run` 直接运行，也支持单独编译后执行。

### 5.1 `kn5-inspect`

```bash
# 直接运行（调试版）
cargo run -p kn5-inspect -- <path/to/car.kn5> \
  --textures --materials --nodes --depth 2

# 导出内嵌 DDS 贴图
cargo run -p kn5-inspect -- <path/to/car.kn5> \
  --extract-textures ./out-textures

# 编译发布版二进制
cargo build --release -p kn5-inspect
./target/release/kn5-inspect <path/to/car.kn5>
```

常用参数：

| 参数 | 作用 |
| --- | --- |
| `--textures` | 列出内嵌贴图 |
| `--materials` | 列出材质与贴图槽 |
| `--nodes` | 打印节点树 |
| `--depth N` | 节点树最大深度，默认 3 |
| `--extract-textures DIR` | 把内嵌 DDS 写到目录 |

### 5.2 `acd-inspect`

```bash
# 列出 data.acd 内所有文件（key 从父目录名推导）
cargo run -p acd-inspect -- <path/to/data.acd>

# 打印其中一个文件
cargo run -p acd-inspect -- <path/to/data.acd> --print car.ini

# 全部解包到目录
cargo run -p acd-inspect -- <path/to/data.acd> --extract ./data-unpacked

# 手动指定 key
cargo run -p acd-inspect -- <path/to/data.acd> --key <folder-name>

# 编译发布版二进制
cargo build --release -p acd-inspect
./target/release/acd-inspect <path/to/data.acd>
```

常用参数：

| 参数 | 作用 |
| --- | --- |
| `--key <KEY>` | 覆盖从父目录名推导的 key |
| `--print <NAME>` | 输出指定文件内容 |
| `--extract <DIR>` | 解包全部文件 |
| `--quiet` | 不打印逐文件列表 |

## 6. 测试、格式与静态检查

```bash
# 代码格式检查 / 自动格式化
cargo fmt --all -- --check
cargo fmt --all

# Clippy（排除 Godot 扩展，保持 headless 快速）
cargo clippy --workspace --exclude vettura-godot --all-targets -- -D warnings

# 单元测试 + 集成测试 + 文档测试
cargo test --workspace --exclude vettura-godot
```

`vettura-assets` 包含集成测试 `crates/vettura-assets/tests/car_folder.rs`，
它会读取测试车目录；没有测试数据时相关用例会跳过。

`vettura-kn5` 有一个可选的真实样例测试。设置环境变量后会额外解析该文件：

```bash
VETTURA_KN5_SAMPLE="/path/to/car.kn5" \
  cargo test -p vettura-kn5
```

### 性能测试（bench）

`vettura-kn5` 自带一个不依赖 criterion 的 bench，使用 `std::time::Instant`
测量解析延迟、吞吐和节点遍历速度：

```bash
# 使用内置合成模型（128 网格 × 1024 顶点，32 张贴图）
cargo bench -p vettura-kn5

# 测试真实 KN5 文件（也可用 VETTURA_KN5_SAMPLE 指定）
cargo bench -p vettura-kn5 -- "/path/to/car.kn5"
```

可用环境变量调整规模：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `VETTURA_BENCH_ITERS` | 20 | 正式迭代次数 |
| `VETTURA_BENCH_WARMUP` | 3 | 预热次数 |
| `VETTURA_BENCH_MESHES` | 128 | 合成模型网格数 |
| `VETTURA_BENCH_VERTS` | 1024 | 每个网格顶点数 |
| `VETTURA_BENCH_TEXTURES` | 32 | 合成模型贴图数 |
| `VETTURA_BENCH_TEXTURE_KB` | 64 | 每张贴图大小（KiB） |

输出包含最快、中位、平均、p95、最慢耗时，以及 `MiB/s` 和
`M 三角形/s`。当前 bench 会分别测量：

- `解析（零拷贝共享输入）`：`parse_shared`
- `解析（含输入复制）`：`parse(&[u8])`
- `遍历节点树`
- `解析（含文件读取）`：`from_path`

> `cargo bench` 使用 `bench` profile，默认带优化，不需要手动加 `--release`。

## 7. 文档

```bash
# 生成 Rust API 文档（不编译 Godot 扩展）
cargo doc --workspace --exclude vettura-godot --no-deps

# 生成后打开
cargo doc --workspace --exclude vettura-godot --no-deps --open
```

## 8. Chrono::Vehicle（`vettura-physics` 的 `chrono` feature）

Chrono 适配器**不再是独立 crate**，而是 `crates/vettura-physics/src/backends/chrono/`，
由 cargo feature `chrono` 门控，默认关闭；重量级的 C++ 源码统一放在仓库根的
`third_party/chrono/`（见 [third_party/README.md](../third_party/README.md)）。

只有在能找到 Chrono 时才会真正链接它：`build.rs` 按
`CHRONO_ROOT` > 仓库根 `third_party/chrono` 的顺序定位，两者都没有时改链
`cpp/chrono_bridge_stub.cpp`（同一个 ABI 的空实现，返回 `NotLinked`），
因此命令始终可跑：

```bash
cargo build -p vettura-physics --features chrono
cargo test -p vettura-physics --features chrono
```

Chrono 源码就位（`third_party/chrono/CMakeLists.txt` 存在，或 `CHRONO_ROOT` 指向
CMake 源码树 / 安装目录）后第一次构建会跑 CMake，需要 **CMake ≥ 3.15 + C++17
工具链**；之后就是普通的增量编译。

### 8.1 胶水层选型：C ABI（不是 cxx）

桥接层是 `crates/vettura-physics/cpp/chrono_bridge.h` 声明的 `extern "C"` 接口，
Rust 侧手写 `unsafe extern "C"` 声明。选 C ABI 的原因：

- 要跨边界的东西全是 POD（`vettura_chrono_spec/pose/controls/state`）加一个
  不透明句柄，用不到 cxx 的类型安全；
- C ABI 是所有编译器/平台唯一共识的契约，Chrono 可以用任意 C++ 工具链编出来；
- 契约是一个可 review 的头文件，不依赖代码生成，也没有 cxx 与 Rust 版本的耦合。

### 8.2 让不同编译器结果一致的编译选项

这些 flag 不是为了性能，是为了**逐位一致的浮点结果**（clang/gcc 默认
`-ffp-contract=fast` 会融合 FMA，MSVC 不融合，这是最常见的 divergence 来源）：

| 编译器 | C++ flag |
| --- | --- |
| clang / gcc | `-ffp-contract=off -fno-fast-math -fno-unsafe-math-optimizations -fno-reciprocal-math -fno-finite-math-only -fno-math-errno` |
| x86 额外 | `-mno-fma -mno-avx2 -mno-avx512f`（不同机器的向量化宽度会改变 Eigen 分块顺序） |
| MSVC | `/fp:strict`（不要 `/fp:fast`；MSVC 路径 build.rs 里还是 TODO） |
| 通用 | 不用 `-Ofast`；`CMAKE_BUILD_TYPE=Release`；`ENABLE_OPENMP=OFF` |

Chrono 侧 CMake 实际选项（`build.rs` 写死）：

- `-DENABLE_OPENMP=OFF`（多线程归约顺序会让结果随核数变化）；
- `-DCH_ENABLE_MODULE_VEHICLE=ON`，其余模块全部 OFF；
- `-DBUILD_DEMOS=OFF -DBUILD_TESTING=OFF -DBUILD_BENCHMARKING=OFF -DBUILD_SAMPLES=OFF`
  （Chrono 10 的开关名是 `BUILD_*`，旧的 `ENABLE_DEMOS` 之类会被静默忽略）；
- `-DCMAKE_POSITION_INDEPENDENT_CODE=ON`（要链接进 cdylib）。

> **保证范围**：Chrono 自己的 `cmake/FindSIMD.cmake` 会在 clang≥15 / gcc 上追加
> `-march=native`，且排在 `CMAKE_CXX_FLAGS` 之后，没有 CMake 选项能关掉。所以
> “逐位一致”的准确说法是**同一台机器、不同编译器**（同一个 `-march=native`）。
> 跨机器（M1 vs M3、不同 x86）还需要锁基线 ISA：临时办法是
> `-DCMAKE_CXX_COMPILER_TARGET` 或给 FindSIMD 打补丁，长期方案见 §8.5。

验证分两层：

1. **同进程确定性**：`identical_inputs_produce_bit_identical_states` 已随
   `chrono` feature 跑，守住「后端自身不引入不确定性」。
2. **跨编译器对拍**：`golden_replay_hash_is_stable` 把固定场景（怠速 → 半油门 →
   全油门左转 → 刹车，共 2400 步）的 `VehicleState` 位模式压成 FNV-1a 哈希：

   ```bash
   # 编译器 A：拿到哈希
   cargo test -p vettura-physics --features chrono golden_replay -- --nocapture
   # 编译器 B：比对（不一致会打印 vettura_chrono_build_id()）
   VETTURA_GOLDEN_HASH=<hash> cargo test -p vettura-physics --features chrono golden_replay
   ```

   哈希依赖 Chrono 版本与机器 ISA，所以不写死在仓库里；它验证的是
   「同一台机器 + 不同编译器」这条承诺。

### 8.3 build.rs 的行为

`crates/vettura-physics/build.rs`（feature 打开时执行，不开时直接返回）：

| 步骤 | 内容 |
| --- | --- |
| 定位 Chrono | `CHRONO_ROOT` → 仓库根 `third_party/chrono`；找不到就转 stub |
| CMake | §8.2 的选项 + `CMAKE_POSITION_INDEPENDENT_CODE=ON`；`CMAKE_CXX_FLAGS` 就是 §8.2 那张表；x86 额外 `-DSIMD_USE_AVX2=OFF -DSIMD_USE_AVX=OFF` |
| 编译胶水 | `cpp/bridge.cpp` + `cpp/vettura_vehicle_spec.cpp` → `.o` → `ar` 成 `libvettura_chrono_bridge.a`（不引入 `cc`/`cmake` crate，直接调本机 `clang++`/`g++`） |
| 链接 | 扫 CMake 产物（`<build>/` 和 `<build>/lib/`）里的 `libChrono_*`：`.a` 走 `static=`、`.dylib` 走 `dylib=`，依赖方在前、`Chrono_core` 兜底最后；再加 `c++`/`stdc++` |
| JSON 冒烟 | 编译并运行 `cpp/bridge_dump.cpp`，断言顶层车辆 JSON 含 `Chassis/Axles/Steering Subsystems/Wheelbase/Driveline/Powertrain` 且各子系统文件落盘——Chrono 换个版本改了 schema，这里立刻失败而不是运行时 |

环境避坑：本机 CLT 的 `usr/include/c++/v1` 可能是坏的（只剩 3 个文件），
build.rs 会先编译探测一次 `#include <string>`，失败就自动
`-nostdinc++` + `-isysroot` 指向 SDK 里完整的 libc++，并提示你
重新安装 CLT。Chrono 首次构建约 10 分钟（`NUM_JOBS` 控制并行度）。

### 8.4 诊断探针（`cpp/chrono_probe.cpp`）

不参与 `cargo` 构建的独立程序：生成默认 spec 的 JSON、手搭
`FlatTerrain + WheeledVehicle`，按阶段（静置 / 加速 / 左转 / 刹车）打印
车身位姿、四轮轮心位置、轮胎全局力、滑移率，用来定位「接地 / 轮位 / 转向」
一类只有真跑 Chrono 才暴露的问题。

```bash
BUILD=$(ls -d target/debug/build/vettura-physics-*/out/chrono-build | head -1)
SDK=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk   # 本机 CLT 头损坏时的兜底
c++ -std=c++17 -O2 -w -nostdinc++ -isysroot "$SDK" -I"$SDK/usr/include/c++/v1" \
  -Icrates/vettura-physics/cpp -Ithird_party/chrono/src -I"$BUILD" \
  -isystem /opt/homebrew/include/eigen3 \
  crates/vettura-physics/cpp/chrono_probe.cpp \
  crates/vettura-physics/cpp/vettura_vehicle_spec.cpp \
  -L"$BUILD/lib" -lChrono_vehicle -lChrono_core -o /tmp/vettura-probe
DYLD_LIBRARY_PATH="$BUILD/lib" /tmp/vettura-probe /tmp/vettura-probe-json
```

### 8.5 待办

- MSVC `/fp:strict` 路径（build.rs 目前只实现 clang/gcc）；
- 跨机器基线 ISA：绕开 Chrono `FindSIMD` 的 `-march=native`；
- golden replay：固定 spec + 输入序列 → 比对 `VehicleState` 位模式，clang vs gcc。


## 9. 按场景组合命令

FMOD 默认**不链接**；要真正播放 AC 车模声浪时，先用主工程 Python 脚本拉 SDK，
再启用 `fmod` feature：

```bash
cd /path/to/vettura/scripts
uv run fetch_fmod.py                 # 下载 FMOD Engine 2.02.22 并解压/编译插件

cd /path/to/vettura-native
cargo build -p vettura-godot --features fmod
```

`build.rs` 会定位 `.cache/fmod/runtime/<os>` 或 `FMOD_SDK_PATH`；找不到就以 stub
编译。启用后 Godot 侧出现 `FmodSystem`；不启用时 `start()` 返回 `false`，
`last_error()` 会说明原因。版本 / 兼容规则 / 限制见 [audio.md](audio.md)。

### 日常开发（最快反馈）

```bash
cargo fmt --all
cargo check --workspace --exclude vettura-godot
cargo test --workspace --exclude vettura-godot
```

### 改完 Rust 核心，准备进 Godot 验证

```bash
cargo build -p vettura-godot
godot --headless --path .. --editor --quit
```

### 出发布版扩展

```bash
cargo build -p vettura-godot --release
```

### 只导出 AC 资源做离线分析

```bash
cargo build --release -p kn5-inspect -p acd-inspect
./target/release/kn5-inspect <car.kn5> --textures --materials --nodes
./target/release/acd-inspect <data.acd> --extract ./data-unpacked
```

### 完整验证（CI 风格）

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude vettura-godot --all-targets -- -D warnings
cargo test --workspace --exclude vettura-godot
cargo build -p vettura-godot
# 可选：FMOD 真机声音联调
cd /path/to/vettura/scripts && uv run fetch_fmod.py
cd /path/to/vettura-native && cargo build -p vettura-godot --features fmod
godot --headless --path /path/to/vettura \
    res://vettura-native/godot/fmod_probe.tscn
```

### 发行打包

导出包要自带 FMOD runtime（只动态链接，SDK 不入库）：

```bash
cargo build -p vettura-godot --features fmod --release
godot --headless --path /path/to/vettura --export-release "macOS" ../dist
cd /path/to/vettura/scripts
uv run pack_fmod.py --platform macos --dest ../dist/Vettura.app   # .app 自动落到 Contents/MacOS
```

`vettura_native.gdextension` 已区分 debug / release 条目：编辑器用 `target/debug`，
导出包用 `target/release`。细节与平台差异见 [audio.md](audio.md) 的「打包 / 发行」。

## 10. 常见问题

| 现象 | 原因与处理 |
| --- | --- |
| `cargo build` 后 `target/debug` 里没有 dylib | 正常，默认成员不含 `vettura-native`；执行 `cargo build -p vettura-godot` |
| Godot 报找不到 GDExtension | 检查 dylib 是否存在、`.gdextension` 路径、Godot 是否为 4.7 |
| 链接错误或 `cc` 找不到 | macOS 安装 Xcode CLT：`xcode-select --install` |
| `rust-version` 不满足 | 升级 Rust：`rustup update stable` |
| 调试构建太慢 / 运行卡顿 | 根 `Cargo.toml` 已设 `opt-level = 1`（依赖为 3），不要改成 0 |
| 修改了 `CARGO_TARGET_DIR` 后扩展失效 | `.gdextension` 使用固定 `target/` 路径，需同步修改 |
| 测试试图编译 Godot 扩展 | 使用 `--exclude vettura-godot` |
