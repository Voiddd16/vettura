# third_party

**使用指定的 release 版本而不是 master**

放置**从远端拉取的第三方源码**（vendored，不交给 Cargo 管理）。这些内容不参与
workspace 编译，只被某个 crate 在需要时通过 `build.rs` / CMake 引用。

当前内容：

| 目录 | 内容 | 谁在用 |
| --- | --- | --- |
| `chrono/` | [Project Chrono 10.0.0](https://github.com/projectchrono/chrono.git)（C++，含 `Chrono::Vehicle`） | `crates/vettura-physics` 的 `chrono` feature（见 [docs/build.md](../docs/build.md) §8） |

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

`crates/vettura-physics/build.rs` 按以下顺序定位源码（见 `find_chrono_root`）：

1. 环境变量 `VETTURA_CHRONO_ROOT`（指向含 `CMakeLists.txt` 的目录）；
2. 仓库根 `third_party/chrono/`；
3. 都没有时链 `chrono_bridge_stub.cpp`，Rust 侧得到
   `ChronoError::NotLinked`，workspace 仍可正常编译与测试。

Chrono 的 CMake 构建产物落在 Cargo 的 `OUT_DIR`（`target/.../out/chrono-build/`），
不会污染源码目录；直接用 CMake 构建 `third_party/chrono` 时才会在源码里留下
`build/`、`lib/` 等，均已被忽略。

## 规则

- 每个第三方库一个子目录，目录名小写；在**本文件**记录来源仓库、tag/commit、拉取命令；
- 拉取方式不限：`git clone`、`git submodule`、`git subtree` 都可以；
- Rust 侧依赖一律走 `Cargo.toml` + `crates.io`，不放这里——这里只放 Cargo 管不了的代码。
