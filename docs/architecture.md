# 架构

## 总览

`vettura-native` 是一个 Rust workspace，最终产物是一个 Godot 4 GDExtension
动态库。核心数据格式和物理求解都与 Godot 解耦，CLI 工具可以直接复用。

```
┌──────────────────────────────┐
│        Godot 4.7             │
│  scene / render / input / UI │
└──────────────▲───────────────┘
               │ GDExtension
┌──────────────┴───────────────┐
│     vettura-native (cdylib)  │  Kn5File / AcCar / VehicleBody
└───────┬──────────────┬───────┘
        │              │
┌───────▼──────┐  ┌────▼──────────────┐
│ vettura-     │  │ vettura-physics   │
│ assets       │  │  spec/state/trait │
└───┬──────┬───┘  └────┬──────────────┘
    │      │           │
┌───▼───┐ ┌▼─────────┐ │
│ kn5   │ │ acdata   │ │
└───────┘ └──────────┘ │
                       │  `--features chrono`（可选）
              ┌────────▼──────────┐
              │ third_party/      │
              │ chrono (C++/CMake)│
              └───────────────────┘
```

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
- 加载车模 `.kn5`。

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
`crates/vettura-physics/cpp/`：

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

### `vettura-godot`

Godot GDExtension：

- `Kn5File`（Resource）：加载 KN5，暴露统计信息；
- `AcCar`（Resource）：打开车目录，暴露质量/轴距/显示名；
- `VehicleBody`（Node3D）：固定步长驱动 `Simulation`，同步位姿到节点。

**下一步**：把 `Kn5` 转成 `ArrayMesh` + `ImageTexture`，并实现 Godot
`ResourceFormatLoader`，让 `.kn5` 出现在资源浏览器中。

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

### 物理一帧

```
Godot _physics_process(delta)
  └── VehicleBody
        └── Simulation::step(delta, Controls)
              ├── accumulator += delta
              └── while accumulator >= fixed_dt:
                    backend.step(fixed_dt, controls)
        └── 读取 VehicleState → Node3D position/quaternion
```

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
