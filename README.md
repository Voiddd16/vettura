# vettura-native

面向 Assetto Corsa 内容兼容的 Rust 原生核心 + Godot 4 GDExtension 工作区。

目标不是重写一个完整游戏引擎，而是把两件最难替换的事情做好：

1. **AC 内容管线**：读取 `.kn5`、`data.acd`、INI/LUT，把 mod 车完整加载进来；
2. **车辆物理胶水层**：定义稳定的 `VehicleBackend` 接口，优先接 Chrono::Vehicle，
   也可以替换成其它成熟车辆动力学实现。

渲染、场景、输入、UI 继续由 Godot 负责。

## 当前状态

| 模块 | 状态 |
| --- | --- |
| KN5 解析（v5/v6、节点树、网格、材质、DDS 贴图） | 可用 |
| `data.acd` 解密与解包 | 可用 |
| AC INI / LUT 读取 | 可用 |
| AC 车数据 → `VehicleSpec` 映射 | 初版 |
| 物理后端接口 + 固定步长调度 | 可用 |
| SimpleBackend（占位动力学） | 可用，仅用于联调 |
| Chrono::Vehicle FFI | 可用：vehicle 平地工况（`--features chrono`，需先拉 Chrono 源码） |
| `vettura-world`：Jolt 固定步长世界、表面/天气、车-车/车-墙碰撞 | 可用 |
| AC 赛道：`models*.ini` / `surfaces.ini` / 物理节点扫描 / `fast_lane.ai` / `pit_lane.ai` / 安全发车格 / AI 纯跟踪 + 车流避让 / 分段计时 | 可用 |
| 车辆物理进阶：气动下压力（前后轴）/ 差速器限滑 / 刹车热衰减 / 燃油载荷 / 变速箱离合器 / DRS | 可用 |
| 赛道演进：`groove.ini` → 位置驱动橡胶堆积，赛线随行驶提升抓地 | 可用 |
| 轮胎热/胎压/磨损：滑动生热 + 工作窗口 + 理想胎压 + grain/blister | 可用 |
| 轮胎平点/抱死振动：抱死功耗 → 平点损伤 + 每转一次垂向冲击 + 抓地折减与恢复 | 可用 |
| ERS/混动：电池部署/回收、每圈额度、制动能量回收 | 可用 |
| 发动机热/功率保护：水温/油温窗口、冷启动与过热掉动力 | 可用 |
| 水膜/aquaplaning：轮胎动态水膜（驻留时间模型）、与路面水深叠加、水膜冷却胎热 | 可用 |
| 气动侧向力/横摆稳定性 + 轮胎联合滑移（摩擦圆）+ 路面起伏（`SIN_HEIGHT`/`SIN_LENGTH`） | 可用 |
| 悬架非线性：渐进弹簧 / bump & rebound stop / heave 第三弹簧 / 主动 & 半主动执行器 | 可用 |
| 发动机：完整 `power.lut` 扭矩曲线 / 涡轮增压滞后与泄压阀 / 收油制动（`COAST_REF`） / INI 行内注释 | 可用 |
| 气动 LUT：逐翼 `LUT_AOA_CL` × `LUT_GH_CL`（攻角曲线 + 离地曲线 = 地效/失速）+ `car.ini INERTIA` 惯性张量 | 可用 |
| 刹车：转子核心热浸（`CORE_TRANSFER_*`，松油门后刹车片继续被煎）+ 产热只按实际刹车输入 | 可用 |
| 轮胎：载荷敏感（`FZ0`/`LS_EXP*`）+ 松弛长度（`RELAXATION_LENGTH`）+ Magic Formula 滑移曲线（移植自 Chrono） | 可用 |
| 轮胎垂直刚度：`RATE`/`PRESSURE_SPRING_GAIN` 与悬挂串联（热胎更硬）+ 三层胎温 + `PERFORMANCE_CURVE` 温度-抓地表 | 可用 |
| 冬季工况：零下环境温度/胎压/路面抓地/温度外推，全部允许负温度（无 0 °C 夹断） | 可用 |
| 回正力矩（`pneumatic_trail`）+ 气动尾流 dirty air + AC 滚动阻力（`ROLLING_RESISTANCE_*`）+ 倒挡/断油/手刹扭矩 | 可用 |
| 转向锁（`STEER_LOCK/STEER_RATIO`，修掉 2× 偏差）+ 侧倾中心修正（roll centre → 侧倾力矩） | 可用 |
| anti-dive/anti-squat（制动/加速载荷转移的连杆修正）+ 阳光加热赛道（路面温度随时间演化，链路：阳光→路温→胎温→抓地） | 可用 |
| 碰撞碎片：世界持有刚体碎片（盒/球/圆柱），二次碰撞写入 `ImpactEvent` | 可用 |
| ABS/牵引力控制：按实测轮滑移闭环调节刹车与油门（`Controls.abs` / `Controls.tc`） | 可用 |
| 悬架运动学：动态外倾角 + 外倾推力（指向倾斜侧，静态左右抵消）+ 外倾抓地窗口 | 可用 |
| 悬架阻尼：两级阻尼（bleeder 阈值）+ 压缩/回弹非对称（`DAMP_FAST_*`） | 可用 |
| KN5 → Godot 场景（节点树 / ArrayMesh / 贴图 / 材质） | 可用 |
| Godot 扩展类（Kn5File / AcCar / VehicleBody / WorldBody） | 可用：模型 + 单机 Jolt + 赛道世界，见 `godot/baseline.tscn` |
| AC 车模声音发现 + FMOD runtime（`vettura-audio` / `FmodSystem`） | 可用：读 `sfx/GUIDs.txt` → 直接调 FMOD 播放 `.bank`；`--features fmod` |

## 快速开始

```bash
cd vettura-native

cargo fmt
cargo check --workspace
cargo test --workspace
```

赛道模块的真实资产测试默认忽略（Daytona 的 25 MB collider 会构建 Jolt
static mesh）。本地有 `../vettura/test/daytona_2017/` 时可运行：

```bash
cargo test -p vettura-world --test track_module -- --ignored
```

`vettura-godot`（crates/vettura-godot）的单元测试只覆盖不依赖引擎的纯转换，
不需要启动 Godot；端到端验证见下面的冒烟测试。

用测试 mod 检查解析器：

```bash
cargo run -p kn5-inspect -- \
  "../test/F1 2008 by Cimmerian_Iter/alternate_kn5/cim_2008_ferrari_2.kn5" \
  --textures --materials --nodes --depth 2

cargo run -p acd-inspect -- \
  "../test/F1 2008 by Cimmerian_Iter/ponz_physics/cim_2008_ferrari/data.acd" \
  --print car.ini
```

构建 Godot 扩展：

```bash
cargo build -p vettura-godot
cargo test -p vettura-godot   # kn5::convert 的纯转换单测
```

`godot/vettura_native.gdextension` 已把库路径指向
`res://vettura-native/target/debug/libvettura_native.dylib`（macOS）。
所以 **Godot 工程根目录下必须能看到 `vettura-native` 目录**：默认约定工程在本仓库
上一级的 `vettura/`，用 `./scripts/link_into_godot.sh` 建立符号链接即可；其它位置
把路径作为第一个参数传进去。

用真实 mod 车做端到端冒烟测试（无头 Godot）：

```bash
./scripts/link_into_godot.sh      # 或 ./scripts/link_into_godot.sh /path/to/godot/project
cargo build -p vettura-godot
godot --headless --path /path/to/godot/project \
    res://vettura-native/godot/baseline.tscn
```

接 FMOD 真机声音时：

```bash
FMOD_SYS_FMOD_DIRECTORY=/path/to/FMODEngine \
  cargo build -p vettura-godot --features fmod
```

然后在 GDScript 里用 `FmodSystem.start()` / `load_car()` / `play("engine_ext")`；
详见 [`docs/build.md`](docs/build.md) 与 [`docs/godot.md`](docs/godot.md)。

## 目录结构

```
vettura-native/
├── Cargo.toml                 # workspace 与共享依赖
├── crates/
│   ├── vettura-core/          # 单位、坐标约定、glam 再导出
│   ├── vettura-kn5/           # KN5 二进制解析（不依赖 Godot）
│   ├── vettura-acdata/        # data.acd / INI / LUT
│   ├── vettura-assets/        # 车目录解析 + AC 数据 → 物理 spec
│   ├── vettura-physics/       # 物理后端接口、状态、固定步长调度、SimpleBackend
│   │   └── src/backends/      #   simple / chrono（后者由 `chrono` feature 门控）
│   ├── vettura-world/         # Jolt 世界、SurfaceMap/天气、赛道 collider、AI line
│   ├── vettura-audio/         # AC sfx/GUIDs.txt + FMOD runtime 直达（`fmod` feature）
│   └── vettura-godot/         # Godot 4 GDExtension (cdylib + rlib)
│       └── src/kn5/           # KN5 → Godot 桥接（convert/texture/material/mesh/scene）
├── third_party/               # 远端拉取的第三方源码（vendored）
│   └── chrono/                # Project Chrono C++（Chrono::Vehicle），按需拉取，见 third_party/README.md
├── tools/
│   ├── kn5-inspect/           # 查看/导出 KN5
│   └── acd-inspect/           # 解密/解包 data.acd
├── godot/
│   ├── vettura_native.gdextension
│   ├── kn5_probe.tscn/.gd     # 无头模型冒烟测试
│   └── baseline.tscn/.gd      # 主项目接入冒烟（真实赛道 + 真实车）
├── scripts/
│   └── link_into_godot.sh     # 让 res://vettura-native 在 Godot 工程里可解析
└── docs/
    ├── architecture.md
    ├── kn5-format.md
    ├── ac-data.md
    ├── physics.md
    ├── godot.md
    └── roadmap.md
```

## 依赖方向

```
vettura-godot (Godot)
   ├── vettura-assets ──> vettura-kn5
   │        │            vettura-acdata
   │        └──> vettura-physics
   ├── vettura-core
   ├── vettura-physics
   └── vettura-world ──> vettura-physics / vettura-kn5 / vettura-acdata

third_party/chrono ──(build.rs, 仅 `--features chrono`)──> vettura-physics
```

规则：

- `vettura-kn5`、`vettura-acdata`、`vettura-physics`、`vettura-world` **不依赖 Godot**，可单独测试；
- AC 格式知识只放在 `vettura-kn5` / `vettura-acdata` / `vettura-assets` / `vettura-world`；
- `vettura-physics` 不知道 Assetto Corsa 的存在；赛道语义在 `vettura-world` 边界转换；
- Godot 层只做类型转换和生命周期管理；其中 `kn5/convert.rs` 刻意写成纯 Rust，
  让坐标/命名/材质参数映射可以在没有引擎的环境下单元测试。

## 前提

- Rust stable（workspace 要求 1.94+）
- macOS 编译需要 Xcode Command Line Tools：`xcode-select --install`
- 运行扩展需要 Godot 4.7（当前工程使用 4.7.2）

## 进一步阅读

- [编译指南](docs/build.md)
- [架构说明](docs/architecture.md)
- [KN5 格式笔记](docs/kn5-format.md)
- [AC 数据格式笔记](docs/ac-data.md)
- [物理后端设计](docs/physics.md)
- [Godot 集成](docs/godot.md)
- [路线图](docs/roadmap.md)
