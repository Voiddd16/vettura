# Vettura

Vettura 是一款以 **Godot 4** 为运行时的赛车游戏主工程，目标是导入 Assetto Corsa 的车模与物理数据（KN5 / ACD），并在桌面 Forward+ / 移动端 Mobile 双渲染管线下运行。Rust 原生部分（KN5/ACD 解析、车辆物理、FMOD 声音）作为 workspace 直接放在本工程里。

## 环境要求

| 项目 | 版本 |
| --- | --- |
| Godot | 4.7（桌面 Forward+ / 移动端 Mobile） |
| Rust | stable ≥ 1.94（`rust-toolchain.toml` 锁定） |
| 物理引擎 | Jolt Physics（`project.godot` 中已启用） |
| 目标平台 | 移动端为主，桌面端可运行 |

## 目录结构

```
vettura/
├── project.godot              # Godot 工程配置（应用名、渲染、物理、主场景）
├── Cargo.toml                 # Rust workspace 根（就是本工程根）
├── kn5/                       # vettura-kn5：KN5 网格与材质解析
├── acdata/                    # vettura-acdata：data.acd 解密与 INI/LUT
├── assets/                    # vettura-assets：AC 车目录组织（上面三者的组合）
├── physics/                   # vettura-physics：车辆物理后端（Jolt / Simple / Chrono FFI）
├── track/                     # vettura-track：赛道/环境纯数据（表面、橡胶、天气、AI 线、分段，无 Jolt）
├── world/                     # vettura-world：Jolt 多车世界、车辆适配、AI 控制、计时、遥测
├── audio/                     # vettura-audio：AC sfx 发现 + FMOD runtime 直达
├── gdext/                     # vettura-godot：GDExtension 桥接层（Kn5File/AcCar/VehicleBody/WorldBody/FmodSystem）
│   ├── vettura_native.gdextension
│   └── baseline.tscn / fmod_probe.tscn / kn5_probe.tscn / load_probe.tscn  # Rust 验证探针
├── tools/                     # kn5-inspect / acd-inspect 命令行工具
├── engine/                    # 魔改引擎落点：Godot 补丁序列、显示能力契约、扩展 manifest（不是 crate）
├── scripts/                   # Python 标准库工程脚本：构建 / Android 导出 / FMOD 下载与打包
├── docs/                      # 架构、构建、Godot 集成、音频、物理文档 + plans/（方案文档）
├── game/                      # 主游戏场景：大厅 / 车库 / 车辆预览 / 设置
│   ├── render/                # 渲染基线：天空 shader / 后端档位 / 屏幕后处理
│   └── fx/                    # 赛道/车辆级 FX：雨、草、灯光、粒子
├── stylized/                  # 风格化渲染子项目：玻璃/HUD/颜色/形状/光照/滤镜 shader 框架
├── addons/
│   └── godot_mcp/             # Godot MCP 编辑器插件，供 AI/CLI 操作场景与脚本
├── mods/                      # 游戏 mod 根目录（递归扫描自动分类，不纳入版本库）
└── target/                    # cargo 构建产物（.gdignore，Godot 不导入）
```

`.godot/` 为编辑器导入缓存，`mods/` 为本地大体积资源，二者均被 `.gitignore` 排除。
每个 crate 目录与 `game/` 内都有自己的 `README.md`（模块地图 / 关键入口 / 边界 / 验证命令）；
AI 助手的仓库导航见根目录 [`AGENTS.md`](AGENTS.md)。

## Rust / native 部分

合并前 native 是独立仓库（`github.com/Voiddd16/vettura-native`，已归档），现在就是本工程的
workspace：每个 crate 一个顶层目录，`cargo` 命令在工程根执行。

| 模块 | 状态 |
| --- | --- |
| KN5 解析（v5/v6、节点树、网格、材质、DDS 贴图） | 可用 |
| `data.acd` 解密与解包 / AC INI / LUT 读取 | 可用 |
| AC 车数据 → `VehicleSpec` 映射 | 初版 |
| 物理后端接口 + 固定步长调度；SimpleBackend 占位动力学 | 可用 |
| Chrono::Vehicle FFI（vehicle 平地工况，`--features chrono`） | 可用，需先拉 Chrono 源码 |
| `world/`：Jolt 固定步长世界、表面/天气、车-车/车-墙碰撞 | 可用 |
| AC 赛道：`models*.ini` / `surfaces.ini` / 物理节点扫描 / `fast_lane.ai` / `pit_lane.ai` / 发车格 / AI 纯跟踪 + 车流避让 / 分段计时 | 可用 |
| 车辆物理进阶：气动下压力 / 差速器限滑 / 刹车热衰减 / 燃油载荷 / 变速箱离合器 / DRS | 可用 |
| 赛道演进：`groove.ini` → 橡胶堆积，赛线随行驶提升抓地 | 可用 |
| 轮胎：热/胎压/磨损、平点/抱死振动、载荷敏感、松弛长度、Magic Formula 滑移曲线 | 可用 |
| ERS/混动、发动机热/功率保护、水膜/aquaplaning、气动侧向力/横摆稳定性、悬架非线性与运动学 | 可用 |
| 传动/环境补全：`power.lut` 全扭矩曲线、涡轮增压滞后、`COAST_REF` 收油制动、软限速器、换挡断油、海拔功率与空气密度 | 可用 |
| KN5 → Godot 场景（节点树 / ArrayMesh / 贴图 / 材质） | 可用 |
| Godot 扩展类（`Kn5File` / `AcCar` / `VehicleBody` / `WorldBody` / `FmodSystem`） | 可用，见 `gdext/baseline.tscn` |
| AC 车模声音 + FMOD runtime | 可用：读 `sfx/GUIDs.txt` → 直接调 FMOD 播放 `.bank`（`--features fmod`） |

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo build -p vettura-godot                       # 产出 target/debug/libvettura_native.dylib
cargo build -p vettura-godot --features fmod       # 接真机 FMOD 声音
```

## 物理系统（`world/` + `physics/`）

车辆物理直接读 AC 的 `data.acd`（`car.ini` / `engine.ini` / `drivetrain.ini` /
`suspensions.ini` / `tyres.ini` / `aero.ini` / `brakes.ini` / 赛道 `groove.ini`
/ `surfaces.ini`），落到 Jolt 的固定步长世界里。写法是**同一套原则**：Jolt
线性提供底座（线性弹簧、阻尼、轮胎曲线、传动），其余按"平行力/乘性倍率/输入
修正"叠加——所以每个子系统的默认参数都不改变 Jolt 原有行为，只有 AC 数据
（或显式 authored 值）才打开对应物理。

| 子系统 | 覆盖内容 |
| --- | --- |
| 轮胎 | `DY0/DX0` 峰值摩擦、`FZ0`/`LS_EXP*` 载荷敏感、`RELAXATION_LENGTH` 松弛长度、`FRICTION_LIMIT_ANGLE`/`FALLOFF_LEVEL` → Magic Formula 滑移曲线（移植自 Project Chrono 的 Pac89 公式）、胎温/胎压/磨损/grain-blister、平点与抱死振动、联合滑移摩擦圆、动态水膜 |
| 悬架 | 渐进弹簧 / bump & rebound stop / heave 第三弹簧 / 两级阻尼（bleeder）/ 压缩-回弹非对称、静态与动态外倾、外倾推力、侧倾中心与 anti-dive/anti-squat 的力臂修正、caster/KPI 转向轴几何、半主动/全主动执行器 |
| 动力总成 | `power.lut` 全扭矩曲线、涡轮增压（`MAX_BOOST`/`WASTEGATE`/`REFERENCE_RPM`/`GAMMA`/`LAG_*`）、`COAST_REF` 收油制动、换挡/离合器时间、差速器限滑、软限速器（`LIMITER_HZ`）、换挡断油（`AUTO_CUTOFF_TIME`）、降挡自动补油（rev-match，`physics::ShiftBlip`）、倒挡齿比（`GEAR_R`）、手刹扭矩 |
| 空气动力学 | 逐翼 `LUT_AOA_CL`/`LUT_GH_CL` 攻角 × 离地表（地效与失速）、前后轴分配、侧向力与回正横摆力矩、DRS、脏空气尾流（`dirty_air`）、海拔稀薄空气 |
| 热与环境 | 刹车双质量（摩擦面 + 转子核心热浸）、发动机水温/油温窗口、三层胎温与 `PERFORMANCE_CURVE`、负温度（冬季工况）、阳光加热路面的时间演化（阳光→路温→胎温→抓地） |
| 车身与控制 | `car.ini INERTIA` 惯量张量、燃油载荷与油耗、ERS 部署/回收与油耗、ABS/TC 闭环、`STEER_LOCK`/`STEER_RATIO` 转向锁 |
| 世界与内容 | AC 赛道加载与物理三角、`groove.ini` 橡胶堆积、`fast_lane.ai`/`pit_lane.ai`、AI 纯跟踪 + 车流避让、分段计时、碰撞碎片刚体（二次碰撞）、固定步长确定性回放 |

已知边界（写进 `docs/physics.md`，不是 TODO）：

- **轮胎力与轮毂转动由 Jolt 同一套冲量驱动**。要接管轮胎力（逐六分力、逐轮
  刹车/ABS）就必须同时接管轮毂，而 oxijolt 公开 API 不给这个入口，所以这条
  留给"换后端/fork"的架构阶段；
- `Chrono::Vehicle` 后端（`--features chrono`）只覆盖整车平地工况，与 Jolt
  世界并存而不是混合；共享的是同一份 `VehicleSpec`；
- AC 的 `WBCAR_*`/`WBTYRE_*` 悬架硬点与 `TRACK` 不在同一可信坐标系（实测瞬心
  落到车外 5.9 m），因此侧倾中心 / caster / anti-dive 用**典型默认值 + 显式
  覆盖**，没有从错数据反推。

设计取舍与 Chrono 移植边界见 `docs/physics.md` 与 `third_party/README.md`。

跑真实资产验证（Daytona 赛道 + F1 2008 车；需本地 `mods/` 资源在位）：

```bash
cargo test -p vettura-assets --test track_module -- --ignored   # 赛道/AI/计时/发车格
cargo test -p vettura-assets --test real_car -- --ignored       # 真实车数据映射
```

单元侧覆盖每一条物理模型（`world/tests/world_physics.rs` 60 例 + spec/映射层
单测），新增模型时按"可观测行为"写测试：例如侧倾中心降低 → 侧倾变小、
anti-dive 增大 → 制动俯仰变小、脏空气 → 跟车下压力变小。

`gdext/vettura_native.gdextension` 把库路径指向 `res://target/debug/libvettura_native.dylib`
（macOS），与工程同根，无需软链接。端到端冒烟：

```bash
godot --headless --path . res://gdext/baseline.tscn      # 真实赛道 + 真实车 20 秒
godot --headless --path . res://gdext/fmod_probe.tscn    # 需要 --features fmod 与本地测试车
godot --headless --path . res://gdext/kn5_probe.tscn
```

细节见 [`docs/build.md`](docs/build.md)、[`docs/godot.md`](docs/godot.md)、
[`docs/architecture.md`](docs/architecture.md)、[`docs/audio.md`](docs/audio.md)、
[`docs/rendering.md`](docs/rendering.md)。

## 加载性能探针

车模加载慢的定位与回归（合成本地车模，不需要 `mods/` 真车资产）：

```bash
python3 scripts/load_probe.py            # 生成 66 MiB 合成车模并跑无头探针
python3 scripts/load_probe.py --meshes 64 --vertices 1024   # 小模型快速迭代
```

输出解析 / 冷实例化 / 缓存命中 / 分帧加载四段耗时；重复实例化（同一
`Kn5File`）走缓存通常快 3–4 倍。

## 运行

1. 用 Godot 4.7 打开本目录下的 `project.godot`。
2. 首次打开时编辑器会重新生成 `.godot/` 导入缓存。
3. 按 `F5` 运行主场景：`res://game/lobby.tscn`（游戏大厅）。
4. 也可以用无头模式验证交互链路：

```bash
godot --headless --path . res://game/i18n_smoke.tscn
godot --headless --path . res://game/mod_library_smoke.tscn
godot --headless --path . res://game/lobby_smoke.tscn
godot --headless --path . res://game/drive_smoke.tscn
godot --headless --path . res://game/smoke.tscn
# 期望最后一行 [i18n-smoke] PASS / [mod-library-smoke] PASS / [lobby-smoke] PASS / [drive-smoke] PASS / [visual-smoke] PASS，退出码 0
```

`game/lobby.tscn` 这一版做的是：

- 用可替换的 AC 车库（track）模组做 3D 背景，默认 `mods/ltk_csr_garage`；
- 把当前选择的车辆停在车库的 `AC_PIT_0` / `AC_START_0` 或室内地坪上；
- 慢速环绕镜头，可拖动旋转、滚轮缩放；
- 「车辆库」「赛道」「车库模组」「设置」入口，选择与设置写入 `user://settings.cfg`；
- 界面支持 **简体中文 / English**，设置 →「界面」切换，默认跟随系统；
- 「开始驾驶」进入 `game/drive.tscn`：真实赛道 + 选中车辆、圈速 HUD、天气/FX、
  键盘/手柄/触屏/体感驾驶；
- 「展厅预览」进入 `game/main.tscn` 只看车模。

`game/main.tscn` 这一版做的是：

- 默认 F1 2008 Ferrari 车模浮空预览（不接赛道物理）；
- 左半屏触摸 = 刹车，右半屏触摸 = 油门；
- 右上角按钮循环切换视角：车后 / 驾驶位 / 绕车 / 前机盖；
- HUD 显示当前油门/刹车与车轮角速度。

## godot_mcp 插件

`addons/godot_mcp` 提供 MCP 服务，让外部工具（AI 助手、CLI）可以直接查询节点树、读写脚本、修改场景。插件默认在编辑器启动时加载，配置项位于 编辑器 → 项目设置 → `godot_mcp`。使用说明见 `addons/godot_mcp/README.zh.md`。

## 游戏 mods 目录（`mods/`）

`mods/` 是游戏统一的 mod 存放目录，游戏启动时由 `ModLibrary` 递归扫描并自动分类：

- **车辆**：`data.acd` / `data/car.ini` + `.kn5`；
- **赛道**：`models*.ini`、`surfaces.ini`、`ai/fast_lane.ai` 或 `ui_track.json`；
- **车库**：有 `.kn5` 但不是车/赛道的展示场景。

扫描顺序是 `res://mods` → `user://mods`（自动创建、Android 可用）→ 桌面导出版
可执行文件旁的 `mods/`。包目录（如 `某车包/assettocorsa/content/cars/xxx`）会
继续向下递归，识别到 mod 根后停止。车模库、车库模组选择器和大厅都直接复用
这份索引；通过「导入」加入的车也会写进 `user://mods/imported_cars/`。

`mods/` 体积可达数 GB（含多个 100 MB+ 的 `.kn5` / `.psd`），因此 **不提交到版本库**。本地按下列结构放置即可：

```
mods/
├── F1 2008 by Cimmerian_Iter/   # 车模包（KN5 + 纹理 + data.acd）
├── daytona_2017/                # 赛道（models*.ini / surfaces.ini / AI 线）
├── ltk_csr_garage/              # 车库展示场景
└── actools/                     # KN5/ACD 解析参考实现（第三方仓库克隆，扫描时跳过）
```

资源仅用于本地开发验证，请勿将其提交或再分发。

## 脚本

工程脚本放在 `scripts/`，只用 Python 标准库（`python3` 直接跑，无需 uv/venv）；目前提供：

- `scripts/fetch_fmod.py`：拉取 FMOD Engine SDK 2.02.22 并编译 mod bank 所需 DSP 插件
- `scripts/pack_fmod.py`：把 FMOD 运行期动态库拷进 Godot 导出目录（发行包必需）

用法见 [`scripts/README.md`](scripts/README.md)。
