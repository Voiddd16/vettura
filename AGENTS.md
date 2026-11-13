# AGENTS.md — AI 助手仓库导航

给 AI 助手（OpenCode / Claude / 其他 agent）的项目地图与工作规则。
人类读者请看 [`README.md`](README.md)；深入设计看 [`docs/architecture.md`](docs/architecture.md)。

## 项目一句话

拟真赛车游戏主工程：**Godot 4.7（GDExtension）+ Rust workspace**。
Rust 负责 AC 格式解析（KN5/ACD/INI）、车辆物理（Jolt 底座 + Chrono 可选后端）、
FMOD 音频直连；GDScript 只做 UI/流程。加载时直读 AC 原格式，不做资产转换。

## 仓库地图（每个目录是什么 / 不是什么）

> 每个 crate 目录内都有 `README.md`（模块地图、关键入口、边界、验证命令）；
> 下表只给取舍概览，进入 crate 先读它的 README。

| 目录 | 是什么 | 不是什么 |
| --- | --- | --- |
| `core/` | `vettura-core`：SI 单位约定、AC↔Godot 坐标转换、glam 再导出 | 不含格式解析、物理、Godot 类型 |
| `kn5/` | `vettura-kn5`：KN5 二进制解析（节点树/网格/材质/内嵌 DDS 解码/合成车模） | 不做 Godot 资源转换 |
| `acdata/` | `vettura-acdata`：`.acd` 解密 + INI/LUT 解析 | 不管车目录结构 |
| `assets/` | **代码 crate** `vettura-assets`：AC 车目录组织 + AC 参数→`VehicleSpec` 映射 | **不是资产文件目录！** 资产在 `mods/` |
| `physics/` | `vettura-physics`：`VehicleSpec/State/Controls` + `VehicleBackend` trait + 固定步长 `Simulation`（默认 600 Hz）+ Simple/Chrono 后端 | 不含赛道碰撞（那是 world/） |
| `world/` | `vettura-world`：Jolt 世界——赛道三角网、表面摩擦、橡胶进化、AI、天气、计时、碎片 | 不做渲染 |
| `audio/` | `vettura-audio`：`sfx/GUIDs.txt` 事件表 + bank 定位 + **FMOD Studio runtime 直连** | 不解析 bank/FSB（FMOD 负责） |
| `gdext/` | `vettura-godot`：GDExtension 桥（cdylib）+ 4 个 Rust 验证探针场景 | 不是游戏 UI |
| `test_visual/` | 游戏性预览页与冒烟（GDScript）：车库/触摸/多视角/布局 | 不是 Rust 冒烟 |
| `stylized/` | 风格化渲染子项目：玻璃/HUD/颜色/形状/光照/滤镜 shader 框架（纯 shader+GDScript，见其 README） | 不放游戏逻辑/物理/Rust |
| `tools/` | `kn5-inspect` / `acd-inspect` CLI（格式逆向排查用） | — |
| `scripts/` | Python(uv) 构建脚本：`build.py`（host/android/apk/check/test）、`android.py`、`clean.py`、`fetch_fmod.py`、`load_probe.py` | 不是游戏代码 |
| `docs/` | 设计文档 + `plans/`（方案文档，如主项目方案评审稿） | — |
| `mods/` | **本地 AC 资产**（车模/赛道/actools，数 GB，gitignore + `.gdignore`） | **不是测试代码！** |
| `addons/` | `godot_mcp` 编辑器插件（AI 操作 Godot 的 MCP 服务） | 导出 APK 时会被临时摘除（见下） |
| `third_party/` | Chrono C++ 源码（按需拉取，gitignore，只跟踪 README） | — |
| `android/` `build/` `target/` `.godot/` `.cache/` | 构建产物 / 缓存 | 全部 gitignore，别提交 |
| `.claude/` `.agents/` `.codegraph/` | AI 技能 / 索引 | gitignore |

## Godot 场景分工约定

- **`gdext/*.tscn` = Rust 扩展验证探针**：`baseline`（真实赛道+车 20s）、`kn5_probe`、
  `fmod_probe`、`load_probe`。headless 可跑，验证的是 Rust/GDExtension 链路。
- **`test_visual/*.tscn` = 游戏性预览页**：`main`（默认浮空车+触摸+多视角+HUD，也是
  `project.godot` 主场景）、`garage`（车模库）、`smoke` / `garage_smoke` / `layout_smoke`
  （回归冒烟）。
- **`stylized/showcase.tscn` = 风格化效果展示页**：六大类 shader 各一个展台（纯合成几何，
  不依赖 `mods/`），桌面窗口运行看效果。
- 新增页面/控件实验放 `test_visual/`；shader 效果放 `stylized/`；不要往 `gdext/` 里加游戏 UI。

## 常用命令

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace   # 日常三连
cargo build -p vettura-godot                        # 构建 GDExtension（gdext 不在 default-members）
cargo build -p vettura-godot --features fmod        # 接真 FMOD 声音（需先 scripts/fetch_fmod.py）

python3 scripts/build.py check|test|host|android|apk|all   # 一站式构建（详见 scripts/README.md）
python3 scripts/build.py apk                        # 导出 build/android/Vettura.apk

# 冒烟（--quit-after N 是帧数不是秒！）
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . --quit-after 1500 res://test_visual/garage_smoke.tscn
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/baseline.tscn   # 需 mods/ 真车

uv run scripts/load_probe.py                        # 合成车模加载性能探针（不依赖 mods/）
```

真机资产测试（需本地 `mods/` 在位）：

```bash
cargo test -p vettura-world  --test track_module -- --ignored   # 赛道/AI/计时
cargo test -p vettura-assets --test real_car     -- --ignored   # 真车数据映射
```

## 代码约定

- **语言**：注释与文档用中文（现状一致）；代码标识符英文。
- **位置**：GDScript 场景→`test_visual/`；Python 脚本→`scripts/`；新 crate→顶层新目录
  （workspace 根就是 Godot 工程根，`Cargo.toml` 在 `project.godot` 旁边）。
- **crate 边界**：格式解析/物理不依赖 godot-rust；只有 `gdext/` 认识 Godot。
  宁可加 feature，不开新空 crate（Chrono 适配器只是 physics 的 feature，前车之鉴）。
- **提交**：中文 commit message，一事一提交，写清动机；推送方式
  `git push origin release/0.0.1:main`。
- **错误处理**：面向用户的降级给 warning（如默认车缺失），硬错误不静默吞。

## 已知坑（都踩过，别再踩）

1. **`mods/` 是本地资产不是测试**：车模/赛道放这里，几 GB，gitignore + `.gdignore`
   （Godot 不导入）。新代码引用路径一律 `res://mods/...`。
2. **导出 APK 与 godot_mcp**：`addons/*` 被 exclude_filter 排除，但插件注册的
   `MCPRuntimeProbe` autoload 指向 addons，真机启动报 Unrecognized UID。
   `scripts/android.py` 会在导出期临时摘除 autoload + `[editor_plugins]` 条目再恢复，
   不要手工改 `project.godot` 来"修"它。
3. **Android 链接**：`.cargo/config.toml` 里三个 android target 的
   `-C link-arg=-lc++_shared` 不能删——否则 dlopen 报 `__cxa_pure_virtual`，
   整个 GDExtension 加载失败。
4. **FMOD 不进 APK**：SDK 凭证在 `.fmod.env`，`fetch_fmod.py` 拉取，`pack_fmod.py`
   打包进导出目录；APK 默认无声音是预期行为。
5. **`--quit-after N` 是帧数**，不是秒；headless 编辑器会加载 EditorPlugin，
   插件日志走 `log_message` 信号不进 stdout。
6. **物理引擎**：`project.godot` 用 Jolt（`3d/physics_engine="Jolt Physics"`），
   车辆动力学走 `world/`(Jolt) 与 `physics/`(Chrono 可选) 两条链，共享 `VehicleSpec`。
   **现阶段不自研完整轮胎模型**（roadmap 非目标；与 `docs/plans/主项目方案.md`
   的自研 sim-* 方向是当前最大待决策分歧，评审记录见该目录）。
7. **MCP**：godot_mcp 配置在编辑器 `user://mcp_settings.cfg`（auto_start, port 9080）；
   会话里的 `godot` MCP 工具可用于场景/节点操作。

## 文档索引

| 文档 | 内容 |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | **先读这个**：crate 图、职责与"不放入"、数据流 |
| [`docs/physics.md`](docs/physics.md) | 物理子系统全覆盖清单、设计取舍、Chrono 映射边界 |
| [`docs/godot.md`](docs/godot.md) | Godot 集成：扩展类、场景探针、冒烟层次 |
| [`docs/build.md`](docs/build.md) | 构建、环境、导出细节 |
| [`docs/audio.md`](docs/audio.md) | FMOD 版本/兼容规则、GUID 事件表 |
| [`docs/telemetry.md`](docs/telemetry.md) | 实时遥测：`telemetry.cfg` 配置全表、JSON 帧格式、UDP/TCP 输出 |
| [`docs/kn5-format.md`](docs/kn5-format.md) / [`docs/ac-data.md`](docs/ac-data.md) | 格式逆向记录 |
| [`docs/roadmap.md`](docs/roadmap.md) | 阶段进度（Phase 0–8）与非目标 |
| [`docs/plans/`](docs/plans/) | 方案文档（主项目方案 v1.0 等） |
| [`CHANGELOG.md`](CHANGELOG.md) | 版本变更与真机验证记录 |
