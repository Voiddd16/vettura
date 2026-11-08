# third_party

**使用指定的 release 版本而不是 master**

放置**从远端拉取的第三方源码**（vendored，不交给 Cargo 管理）。这些内容不参与
workspace 编译，只被某个 crate 在需要时通过 `build.rs` / CMake 引用。

当前内容：

| 目录 | 内容 | 谁在用 |
| --- | --- | --- |
| `chrono/` | [Project Chrono 10.0.0](https://github.com/projectchrono/chrono.git)（C++，含 `Chrono::Vehicle`） | `physics` 的 `chrono` feature（见 [docs/build.md](../docs/build.md) §8） |

> `third_party/*` 默认被 `.gitignore` 忽略（源码体积大，约 1.8 GB 含 `.git`），
> 只跟踪本文件。新克隆仓库后需要按下面的命令自己拉一次。

## Project Chrono 10.0.0

```sh
# 仓库根目录下执行（浅克隆，只取 tag，约 1.8 GB）
git clone --depth 1 --branch 10.0.0 \
  https://github.com/projectchrono/chrono.git third_party/chrono
```

- tag：`10.0.0`
- commit：`9faf13dd8f1128dd75ed233a9627027b0422c3f7`
- 验证：`git -C third_party/chrono describe --tags` 应为 `10.0.0`

`physics/build.rs` 按以下顺序定位源码（见 `find_chrono_root`）：

1. 环境变量 `VETTURA_CHRONO_ROOT`（指向含 `CMakeLists.txt` 的目录）；
2. 仓库根 `third_party/chrono/`；
3. 都没有时链 `chrono_bridge_stub.cpp`，Rust 侧得到
   `ChronoError::NotLinked`，workspace 仍可正常编译与测试。

Chrono 的 CMake 构建产物落在 Cargo 的 `OUT_DIR`（`target/.../out/chrono-build/`），
不会污染源码目录；直接用 CMake 构建 `third_party/chrono` 时才会在源码里留下
`build/`、`lib/` 等，均已被忽略。

## 我们从 Chrono 移植了什么，以及为什么不直接用

`Chrono::Vehicle` 是一套完整、成熟、有文献支撑的车辆建模栈，但它的求解器是
Chrono 自己（约束/有限元）的；我们默认走 Jolt。因此引用方式分三类：

| Chrono 里的内容 | 我们的做法 | 说明 |
| --- | --- | --- |
| `ChPac89Tire` 的 Magic Formula 曲线核心 | **公式移植**到 `physics/src/tyre_model.rs`（BSD-3-Clause，已注明出处） | 就一行解析式，移植后 Jolt 路径不引入第二套物理引擎 |
| `ChPac02Tire` 的松弛长度公式（`CalcSigmaK/CalcSigmaA`） | 不移植 | 需要 MF-Swift 参数（`PTX1/PTY1/...`），而 AC 直接给 `RELAXATION_LENGTH`，用 AC 的数据更诚实 |
| `ChFialaTire`/`ChTMeasyTire` 等完整轮胎模型 | 不移植 | 需要接管轮胎力的注入与轮毂转动，而 Jolt 的轮胎冲量同时驱动两者，绕开它会破坏传动耦合 |
| 悬架模板（双叉臂/麦弗逊…） | 不移植 | Jolt 的 `WheelSettings` 建好后不可变，真实运动学没有落点（见 [docs/physics.md](../docs/physics.md) 的外倾一节） |
| 传动/发动机/地形 | 不移植 | 已有自己的实现，且直接吃 AC 数据 |

移植原则：**只移植与求解器无关的公式/标定数据**，凡是需要替换求解器的一律不碰。

## 规则

- 每个第三方库一个子目录，目录名小写；在**本文件**记录来源仓库、tag/commit、拉取命令；
- 拉取方式不限：`git clone`、`git submodule`、`git subtree` 都可以；
- Rust 侧依赖一律走 `Cargo.toml` + `crates.io`，不放这里——这里只放 Cargo 管不了的代码。
