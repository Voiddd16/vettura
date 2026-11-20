# 架构

> AI 助手请先读仓库根的 [`AGENTS.md`](../AGENTS.md)；本文是设计层文档。

## 总览

Rust workspace 就在本 Godot 工程根目录，最终产物是一个 Godot 4 GDExtension
动态库。核心数据格式和物理求解都与 Godot 解耦，CLI 工具可以直接复用。

```
┌────────────────────────────────────────────┐
│                Godot 4.7                   │
│     scene / render / input / UI            │
└─────────────▲──────────────────────────────┘
              │ GDExtension
┌─────────────┴──────────────────────────────┐
│ gdext/（cdylib，唯一认识 Godot 的 crate）    │
│ Kn5File / AcCar / VehicleBody / WorldBody  │
│ / FmodSystem                               │
└──────┬──────────┬──────────────┬──────┬───┘
       │          │              │      │
┌──────▼─────┐ ┌──▼──────────┐ ┌─▼──────┴──────┐
│ vettura-   │ │ vettura-    │ │ vettura-audio │
│ assets     │ │ world       │ │ GUIDs + bank  │
│ 车目录组织 +│ │ Jolt 世界：  │ │ 定位，FMOD    │
│ spec 映射  │ │ 赛道/AI/表面/│ │ runtime 直连  │
│            │ │ 天气/计时/橡胶│ └───────────────┘
└──┬─────┬───┘ └──┬──────────┘
   │     │        │ Jolt（oxijolt，cross-platform-deterministic）
┌──▼──┐ ┌▼────────┴───────────────┐
│ kn5 │ │ vettura-physics         │
│     │ │ spec/state/controls +   │
│     │ │ VehicleBackend trait +  │
│     │ │ 固定步长 Simulation      │
│     │ │ + Simple / Chrono 后端   │
│     │ └───▲─────────────────────┘
│     │     │ `--features chrono`（可选）
│     │ ┌───┴─────────────────┐
└─────┤ third_party/chrono   │
┌──▼──┴──┐ (C++/CMake)         │
│ acdata │                     │
│ ACD/INI│                     │
│ /LUT   │                     │
└────────┘                     │
```

`vettura-assets` 还依赖 `vettura-track`、`vettura-world`：它是主程序侧的
**编排层**（车目录 + 赛道目录统一组装），赛道加载 `TrackModule` 住在 assets，
把解析结果（碰撞三角网 / 表面表 / 发车格 / AI 线）组装成 `vettura-track` 的纯数据，
再由 world 装进 Jolt。
坐标系与单位约定（原 `vettura-core`）已并入 `vettura-physics::coords` / `units`，
没有独立 crate 也就没有"每个 crate 都绕一圈"的重导出。
插件之间（kn5 / acdata / audio）零内部依赖。

## 目录与 crate 对应

workspace 根就是 Godot 工程根，每个 crate 一个顶层目录：

| 目录 | crate | 职责 |
| --- | --- | --- |
| `kn5/` | `vettura-kn5` | KN5 网格 / 材质 / 贴图 |
| `acdata/` | `vettura-acdata` | `data.acd`、INI、LUT |
| `assets/` | `vettura-assets` | AC 车目录（前三者组合）+ 物理数据映射 + 赛道目录组装（`TrackModule`） |
| `physics/` | `vettura-physics` | 车辆物理后端与 `VehicleSpec` |
| `track/` | `vettura-track` | 赛道/环境纯数据：表面表、橡胶进化、天气、AI 线、计时分段（无 Jolt、不解析资产） |
| `world/` | `vettura-world` | Jolt 多车世界、车辆适配、AI 控制律、圈速计时、遥测（只吃纯数据，不解析资产） |
| `audio/` | `vettura-audio` | `sfx/GUIDs.txt` + FMOD runtime |
| `gdext/` | `vettura-godot` | GDExtension 桥接层与 `.gdextension` |
| `tools/` | `kn5-inspect` / `acd-inspect` | 命令行检查工具 |

**Godot 场景分工**（约定，不要混放）：

| 位置 | 角色 | 场景 |
| --- | --- | --- |
| `gdext/*.tscn` | Rust 扩展验证探针（headless 可跑） | `baseline`（真实赛道+车）、`kn5_probe`、`fmod_probe`、`load_probe` |
| `test_visual/*.tscn` | 游戏性预览页与回归冒烟 | `main`（主场景）、`garage`、`smoke`、`garage_smoke`、`layout_smoke` |

本地 AC 资产（车模/赛道）放 `mods/`（gitignore + `.gdignore`），一律以 `res://mods/...` 引用。

## Crate 职责

### `vettura-physics` 里的共享约定（`coords.rs` / `units.rs`）

- 全工作区共享的单位约定（SI），`GRAVITY = 9.80665` 为全仓唯一来源；
- Assetto Corsa / Godot / Vettura 物理帧坐标系转换；
- 原 `vettura-core` crate 已合并进来（它当时只有一个函数真正被调用）。

**不放入**：具体格式解析、物理模型之外的 Godot 类型。

### `vettura-kn5`

- 解析 `sc6969` 容器；
- 贴图（内嵌 DDS）、材质、节点树、静态网格、蒙皮网格；
- 保留未理解的字节，方便后续补充和回写；
- 提供 `Kn5::parse` / `Kn5::from_path`，错误包含文件偏移。

**不放入**：Godot `ArrayMesh`、纹理上传、渲染材质翻译。

### `vettura-acdata`

- `.acd` 容器解密（key 从车文件夹名推导）；
- INI 解析（section/key、注释、重复 key）；
- LUT 解析与线性插值。

**不放入**：车目录发现、物理语义。

### `vettura-assets`

- 打开车目录，自动选择 `data.acd` 或 `data/`；
- 按名字读取数据文件；
- `CarFolder::vehicle_spec()` 把 `car.ini`、`suspensions.ini`、
  `engine.ini`、`drivetrain.ini`、`tyres.ini`、`aero.ini` 映射到
  `vettura_physics::VehicleSpec`；
- 加载车模 `.kn5`；
- 赛道目录组装 `TrackModule`：读 `models*.ini` / `surfaces.ini` /
  `sections.ini`，扫描物理节点 → 碰撞三角网 + `SurfaceMap` + 发车格 +
  AI 线，装进 `vettura_world::World`（解析在本 crate，世界装配回传 world）。

这一层是 AC 兼容性的“胶水层”，所有 AC 特有语义集中在这里。

### `vettura-physics`

只定义与求解器无关的契约：

- `VehicleSpec`：质量、轴距、车轮、悬架、发动机、传动、轮胎参数；
- `VehicleState`：位姿、速度、车轮状态、转速、挡位；
- `Controls`：油门、刹车、离合、转向、换挡；
- `VehicleBackend`：`step(dt, controls)`；
- `Simulation`：固定步长 + 最大子步数，防止卡顿后的死亡螺旋；
- `SimpleBackend`：占位动力学，用于端到端联调。

**不放入**：Assetto Corsa 字段名、Godot 节点、渲染。

### `vettura-physics` 的 `chrono` feature（原 `vettura-physics-chrono`）

Chrono::Vehicle 的适配器，住在 `vettura-physics` 内的
`src/backends/chrono/`，由 cargo feature `chrono` 门控，**默认关闭**。C++ 侧在
`physics/cpp/`：

- `chrono_bridge.h`：C ABI 契约（POD + 不透明句柄 + status code）；
- `vettura_vehicle_spec.cpp`：spec → Chrono JSON（Chrono 的 wheeled vehicle 是
  JSON 驱动的），纯标准 C++，可脱离 Chrono 单测；
- `bridge.cpp`：`WheeledVehicle` + `FlatTerrain` + 固定步长 + 状态回填；
- `chrono_bridge_stub.cpp`：Chrono 没就位时的替代实现，返回 `NotLinked`。

之所以不再单独拆 crate：适配器本身只是一个后端实现，拆出来 workspace 多一个
几乎为空的成员；真正重的是 Chrono C++ 源码，已统一放到仓库根的
`third_party/chrono/`（见 [third_party/README.md](../third_party/README.md)），
由 `build.rs` 在 feature 打开时构建。`build.rs` 找不到 Chrono 时自动退化成
stub，所以 `cargo build --features chrono` 在任意机器上都能编译。

### `vettura-track`

赛道/环境**纯数据层**（不依赖 Jolt，不解析 AC 文件）：

- `SurfaceMap`：矩形/圆形 patch、水坑、赛道三角形索引（`MeshSurfaceIndex`）、
  默认表面；
- `GrooveMap`：`groove.ini` 位置驱动的橡胶堆积；
- `Weather`：湿滑/积水/温度/风/空气密度；
- `AiSpline`：解析 `fast_lane.ai` / `ideal_line.ai`（v7）的采样点与速度/宽度
  元数据，提供投影、按里程采样、逐点曲率与起点位姿；
- `TrackSection`：`sections.ini` 的分段边界语义。

这层是 `assets`（生产者）与 `world`（消费者）之间的数据契约，拆出来后可被
不跑物理的工具直接使用，`assets` 也不必为了几种数据类型拖进整个 Jolt 世界。

### `vettura-world`

基于 Jolt 的世界层（消费 `vettura-track`，并通过重导出保持旧路径可用）：

- `World`：固定步长、多车、静态体、接触/冲击事件、天气与赛道温度推进；
- `VehicleRuntime`：AC spec → Jolt wheeled vehicle 适配与全部车辆物理子系统；
- `AiDriver`：纯跟踪 + 曲率速度规划 + 车流避让的控制律；
- `LapTimer`：圈速/分段/无效圈计时；
- `TelemetrySink`：JSON 帧 + UDP/TCP 输出。

赛道目录的解析（`models*.ini` / `surfaces.ini` / 碰撞 KN5 → 三角网 + 表面 +
发车格 + AI 线）已上移到 `vettura-assets::TrackModule`：world 只接受组装好的
纯数据（`ColliderTriangle` / `SurfaceMap` / `AiSpline`），不再依赖 kn5/acdata。
多布局目录（如 `<track>/roadcourse/ai/fast_lane.ai`）和根目录
（`<track>/ai/fast_lane.ai`）都支持。大三角网会按 800 m 分块后交给 Jolt，
但对外仍是一个逻辑 `TrackObjectId`。

车辆侧按职责拆成 `world/src/vehicle/` 子模块（原 `vehicle.rs` 单文件 3690 行）：

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | `VehicleRuntime` 生命周期、每步子系统的推进顺序、快照组装 |
| `constraint.rs` | spec → Jolt 尺寸换算（悬架几何/刹车分配/变速箱差速器/惯量）+ 合法性校验 |
| `friction.rs` | 逐轮动态摩擦圆（纵横向共享预算 + 全滑移保底） |
| `grip.rs` | Jolt 轮胎回调的共享输入：per-wheel 状态经 relaxed 原子量发布 |
| `thermal.rs` | 三层胎温 / 刹车盘面与核心温度的纯演化函数 |
| `aids.rs` | ABS / TC 闭环 |

拆分原则：**共享状态仍在 `mod.rs`，纯函数与纯数据下沉**。`grip.rs` 的原子量
只暴露 publish/scale 系列方法，不让其它子系统直接写位模式。

**不放入**：渲染网格/材质（留在 `vettura-godot` 的 KN5 管线）、车辆动力学
（`vettura-physics` / Jolt vehicle constraint）。

### `vettura-audio`

车模声音发现 + FMOD runtime 直达：

- `guids`：解析 AC 车目录 `sfx/GUIDs.txt`，拿到事件名 / 路径 / GUID；
- `manifest`：定位 `sfx/<car_id>.bank`，产出 `SoundManifest`；
- `runtime`：`AudioEngine` 直接把 bank 字节交给 FMOD Studio runtime
  （`Studio::create` → `load_bank_memory` → `get_event` → `create_instance/start` → `update`），
  并用候选参数名适配不同 mod。

**不放入**：bank / FSB 解析、音频解码、混音策略。FMOD 负责这些；crate 只负责
“把 AC 的 GUID 表和 bank 路径送到 FMOD”。

版本/兼容规则见 [`docs/audio.md`](audio.md)：锁定 FMOD Engine **2.02.22**、
`libfmod ~2.222`，动态链接，SDK 不入库；2.02.22 runtime 只保证加载 2.02.x bank。

`fmod` feature 默认关闭。关闭时 `AudioEngine` / `FmodSystem` 保留完整 API，但
`start()` 返回 `NotLinked`，因此没有 FMOD SDK 的机器仍能编译、测试、运行视觉与
物理链路；需要真机声音时才启用。

### `vettura-godot`

Godot GDExtension：

- `Kn5File`（Resource）：加载 KN5，查询统计、按需构建 `ArrayMesh` / 材质；
- `AcCar`（Resource）：打开车目录，映射 `VehicleSpec`，实例化主模型，并暴露
  `soundbank_path()` / `sound_event_names()` 等音频发现结果；
- `VehicleBody`（Node3D）：独立模式接 `Simulation`，世界模式由 `WorldBody` 驱动；
- `WorldBody`（Node3D）：拥有 `vettura_assets::TrackModule` + `World`，负责
  赛道安装、多车调度、固定步长推进、圈速计时，以及把 `VehicleSnapshot`
  同步回场景树。这是主项目调用 Rust 侧赛车模拟的入口；
- `FmodSystem`（Node3D）：每帧调用 FMOD `update`，并提供播放 / 参数 / 3D 听者接口；
- `logging`：全系统日志——`InitStage::Scene` 安装 tracing subscriber，读
  `user://log.cfg`，输出到控制台（stdout）与 `user://logs/` 滚动文件；业务 crate
  只 `use tracing` 发事件，不依赖本模块。详见 [`docs/logging.md`](logging.md)。

**下一步**：`ResourceFormatLoader`；模型缓存与 surface 合并；AI 车 / pit lane 的
节点封装；相机、HUD、输入映射等游戏层。

## 数据流

### 加载一辆 AC 车

```
CarFolder::open(car_dir)
  ├── data.acd 存在 → AcdArchive::from_path → 内存 BTreeMap<文件名, 解密后字节>
  └── 否则 data/ 存在 → 递归读取目录
        │
        ├── car.ini / suspensions.ini / ... → Ini
        ├── power.lut → Lut
        └── vehicle_spec() → VehicleSpec
        │
        └── car.kn5 → Kn5::from_path → Node tree + meshes + textures
```

### 加载一条 AC 赛道

> 解析与组装全部在 `vettura_assets::TrackModule`（主程序侧编排层），
> 结果回传给 `vettura_world::World` 安装。

```
vettura_assets::TrackModule::load_layout(track_dir, "roadcourse")
  ├── models_roadcourse.ini → TrackModelEntry[]（collision.kn5、daytonamesh.kn5…）
  ├── roadcourse/data/surfaces.ini → SurfaceDef[] → SurfaceMap 自定义表面
  ├── 所有模型 → 只保留节点名以 1 开头的物理三角网
  │     ├── AC → 物理帧（ac_to_godot / 180° yaw conjugation）
  │     ├── 节点名 1ROAD/1GRASS/1WALL → SurfaceId
  │     └── MeshSurfaceIndex（10 m 网格宽相位）
  ├── AiSpline::from_path(roadcourse/ai/fast_lane.ai)
  │     └── 逐点水平曲率（三点圆）→ max_curvature_between()
  ├── AiDriver（曲率前馈 + Stanley 反馈 + 刹车包络 → DriveInput）
  │     └── AC AI 难度 0..=110（`AiDriverTuning.skill`，见下）
  ├── pit_lane.ai → 同一 AiSpline/AiDriver 复用
  ├── data/sections.ini → TrackSection[] → LapTimer（圈速/分段/无效圈）
  ├── data/groove.ini → GrooveSettings → GrooveMap（车轮沉积橡胶 → 抓地倍率）
  ├── TrackModule::spawn_poses（AI 线吸附 + 坡度/净空检查）
  └── World::add_static_mesh（800 m 分块 → Jolt static bodies）
```

### 物理一帧

```
Godot _physics_process(delta)              # WorldBody（节点树顺序：父先于子）
  └── 从每个 VehicleBody 收 Controls
        └── World::set_controls(handle, controls)
        └── World::step(delta)             # 固定 1/240 s，最多 max_substeps
  └── World::snapshot(handle) → VehicleBody.sync_snapshot
        ├── Node3D position / quaternion
        ├── WHEEL_* dummy: rest * steer(Y) * spin(X)
        └── HUD 标量（speed/gear/rpm/contacts）
  └── LapTimer::update(car_position, elapsed_s)
```

独立模式（`VehicleBody.setup_from_car` / `setup_default`）仍走
`Simulation::step(delta, Controls)`，不经过 `WorldBody`。

### AI 司机与 AC 难度档

`AiDriver` 是一个纯 Rust 控制器：给它 [`AiSpline`](track/src/spline.rs)、车辆
快照和 `VehicleSpec`，它输出 `DriveInput`。它不认识赛道数据之外的东西——
换任何赛道模组、任何车，都用同一条规则。

**控制律**（同一帧内按顺序）：

1. **投影**：把车投到线上，横向误差沿**路面横向**（水平切线 × 路面法向）
   量，所以倾斜赛道（Daytona oval 的 31° bank）上滑下去 2 m 也看得见；
   水平投影看不出来，那是老 bug。
2. **曲率前馈**：Ackermann `atan(L·κ)` + 轮胎侧偏补偿
   `k·v²·κ·mg/(mg+下压力)`，预瞄 `PREVIEW_S` 秒（难度越高看得越远）。
3. **Stanley 反馈**：航向误差 + 横向误差，权限钳在 35% 锁角内；车已经贴着
   抓地极限时反馈自动衰减（前轮越过峰值侧偏角只会更推头）。
4. **刹车包络**：对每个 4 m 采样点取区间内的**最大水平曲率**（逐点三点圆，
   不会被直道平均掉），折算弯速 `v=√(a_lat·R)`，反向传成
   `允许速度` 与「现在必须维持的最小减速度」；后者按 `plan_decel` 归一
   直接当刹车前馈，所以车贴着包络走而不是等超速才反应。

**弯速只来自车的数据**：`a_lat(v) = clamp(μ·g·share, 0.35g, 0.55g)
+ 0.36·0.5ρ·ClA·v²/m`，倾斜路面再叠加 `g·sin(β_eff)`，规划目标再乘 5% 余量；
`a_plan` 取「刹车扭矩/轮半径/质量」与「μ·g·0.68 + 0.40·下压力/m」中较小的那个，
因此低速按轮胎能力刹、高速按下压力能力刹（实测 F1：25 m/s 1.15 g、50 m/s 2.0 g、
75 m/s 4.0 g）。实测 F1（μ=1.73、ClA=6.64）和一台 1500 kg、μ=1.1 的公路车因此
自动得到不同弯速，不需要按赛道调参。

横向包络用修好辅助弹簧载荷后的真车重测：平地稳态圆 26 m/s 能到 1.02 g、
88 m/s 能到 6 g。但直接按这个包络规划会在真实赛道的路肩/倾斜段飞出去，所以
share/cap/downforce 分别收敛到 0.38 / 0.55 g / 0.36——这是**整圈验证过**的值，
不是平地极限值。侧偏补偿增益也从 0.0085 降到 0.0015：载荷修复后轮胎只需
很小侧偏角就能出同样侧向力，旧值让 AI 在弯里反复摇摆。

**AC 难度档 `AiDriverTuning.skill`（0..=110，默认 100）** 缩放的是司机的
能力，不是车的性能：

| 档位 | 抓地使用率 | 规划减速 | 预瞄 | 方向可用量 | 贴线晃动 | 表现 |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 0.52 | 0.45× | 0.65× | 0.55× | 2.0 m | 慢、晃、晚刹 |
| 50 | 0.76 | 0.70× | 0.83× | 0.78× | 0.4 m | 业余 |
| 100 | 1.00 | 1.00× | 1.00× | 1.00× | 0 | 职业（默认） |
| 110 | 1.03 | 1.05× | 1.15× | 1.05× | 0 | 超人类控制精度 |

100→110 的加成不再放大抓地包络（包络已按物理标定，+12% 抓地会让车冲出赛道），
而是加在预瞄、反馈、前馈和可用锁角上——用同一条包络跑出更小的横向误差。

Godot 侧 `WorldBody.ai_skill()` / `set_ai_skill(skill)` 可运行时切换。
两套完全不同布局的 Daytona 整圈实测（同一台真 F1，600 Hz 物理 / 60 Hz AI /
ABS+TC），用来证明规则不是按某一条赛道调的：

| 档位 | roadcourse（5769 m） | oval（4034 m，线速上限 79 m/s） |
| --- | --- | --- |
| 0 | 196.0 s | — |
| 50 | 165.9 s | — |
| 80 | — | — |
| **100** | **145.9 s（偏线 0.68 m，零腾空）** | **—** |
| 110 | 144.4 s（偏线 0.69 m，零腾空） | 61.8 s |

oval 的 `fast_lane` 是给纳斯卡/GTE 那条抓地写的（全程 79 m/s、最小半径 182 m，
折合 3.4 g）；这台车物理上到不了，于是规划器自己把速度降到 ~69 m/s 并在 31°
bank 上贴线跑完——同一个数字都没有为 oval 改过。

### 轮胎摩擦圆（每轮一个，动态）

`world/src/vehicle/friction.rs` 的 `combined_slip_circle()` 每个解算步、**每个轮子**各自算一份：

```
demand_long = |滑移率| / 1.0      ← 轮子本步自己的纵向滑移
demand_lat  = |侧偏角| / 0.8      ← 轮子本步自己的侧偏角
budget_long = floor + (1−floor)·√(1 − demand_lat²)   ← 横向用掉多少，纵向还剩多少
budget_lat  = floor + (1−floor)·√(1 − demand_long²)
× 松弛长度（力还没建立出来的部分）
峰值 = tyre_peak × 环境(路面干湿 × 赛道温度 × 天气 × 滑水) × 粗糙度损失
        × 橡胶沉积 × 胎窗(温度/压力/磨损) × 载荷敏感(LS_EXPY)
```

地板 `floor` 不再是常数，而是由**轮胎自己的衰减特性**（`FALLOFF_LEVEL`）和
**本轮所压的路面**决定：曲线越平，打滑时剩下的越多；冰面越低（下限 0.5，
保证 bank 起步不会瞬间失去全部侧向力）。磨损/温度/赛道状况通过上面的乘链
缩放摩擦圆的**大小**。所以每个轮子各自拥有一个随状态变化的摩擦圆。

## 关键约定

- 内部统一 SI：米、千克、秒、牛顿、弧度；
- 物理坐标系与 Godot 一致：`+X` 右、`+Y` 上、`-Z` 前；
- KN5/AC 坐标系：`+X` 左、`+Y` 上、`+Z` 前；
- 转换集中在 `vettura_physics::coords::ac_to_godot`；
- AC 输入在 `vettura-assets` 边界完成单位/语义转换，物理层不再处理 AC 的
  `%`、`deg`、`km/h`。

## 为什么不是一个大 crate

- KN5/ACD 可以被 CLI、测试、未来其它引擎直接复用；
- Chrono 的 C++ 构建成本高，必须可选（`vettura-physics` 的 `chrono` feature）；
- Godot API 变动频繁，隔离在最小 cdylib 中；
- 每个 crate 都能单独跑单元测试，不需要启动 Godot。

但也不拆得过细：**只有占位代码、没有独立边界的模块不单独开 crate**，
例如 Chrono 适配器只是 `vettura-physics` 的一个 feature，重量级的 C++ 源码放
`third_party/`。
