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

`vettura-assets` 还依赖 `vettura-world` 与 `vettura-core`：它是主程序侧的
**编排层**（车目录 + 赛道目录统一组装），赛道加载 `TrackModule` 住在 assets，
把解析结果（碰撞三角网 / 表面表 / 发车格 / AI 线）交给 world 安装。
`vettura-core`（单位/坐标转换/glam 再导出）在最底层，所有 crate 共用，图里省略。
插件之间（kn5 / acdata / physics / audio / core）零内部依赖。

## 目录与 crate 对应

workspace 根就是 Godot 工程根，每个 crate 一个顶层目录：

| 目录 | crate | 职责 |
| --- | --- | --- |
| `core/` | `vettura-core` | 单位、坐标系转换、共享错误 |
| `kn5/` | `vettura-kn5` | KN5 网格 / 材质 / 贴图 |
| `acdata/` | `vettura-acdata` | `data.acd`、INI、LUT |
| `assets/` | `vettura-assets` | AC 车目录（前三者组合）+ 物理数据映射 + 赛道目录组装（`TrackModule`） |
| `physics/` | `vettura-physics` | 车辆物理后端与 `VehicleSpec` |
| `world/` | `vettura-world` | 多车世界、表面、天气、AI、计时（只吃纯数据，不解析资产） |
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

### `vettura-core`

- 全工作区共享的单位约定（SI）；
- Assetto Corsa / Godot 坐标系转换；
- `glam` 的再导出，避免各 crate 版本漂移。

**不放入**：具体格式解析、物理模型、Godot 类型。

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

### `vettura-world`

基于 Jolt 的世界层：

- `World`：固定步长、多车、静态体、接触/冲击事件；
- `SurfaceMap`：矩形/圆形 patch、水坑、赛道三角形索引（`MeshSurfaceIndex`）、
  默认表面；
- `TrackSection` + `LapTimer`：`sections.ini` 分段语义与圈速计时；
- `AiSpline`：解析 `fast_lane.ai` / `ideal_line.ai`（v7）的采样点与速度/宽度
  元数据，提供起点位姿。

赛道目录的解析（`models*.ini` / `surfaces.ini` / 碰撞 KN5 → 三角网 + 表面 +
发车格 + AI 线）已上移到 `vettura-assets::TrackModule`：world 只接受组装好的
纯数据（`ColliderTriangle` / `SurfaceMap` / `AiSpline`），不再依赖 kn5/acdata。
多布局目录（如 `<track>/roadcourse/ai/fast_lane.ai`）和根目录
（`<track>/ai/fast_lane.ai`）都支持。大三角网会按 800 m 分块后交给 Jolt，
但对外仍是一个逻辑 `TrackObjectId`。

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
  ├── AiDriver（投影 + 前视点纯跟踪 → DriveInput）
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

## 关键约定

- 内部统一 SI：米、千克、秒、牛顿、弧度；
- 物理坐标系与 Godot 一致：`+X` 右、`+Y` 上、`-Z` 前；
- KN5/AC 坐标系：`+X` 左、`+Y` 上、`+Z` 前；
- 转换集中在 `vettura_core::coords::ac_to_godot`；
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
