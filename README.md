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
| Chrono::Vehicle FFI | 占位，待接 |
| KN5 → Godot ArrayMesh/贴图 | 下一步 |
| Godot 扩展类（Kn5File / AcCar / VehicleBody） | 骨架可用 |

## 快速开始

```bash
cd vettura-native

cargo fmt
cargo check --workspace --exclude vettura-godot
cargo test --workspace --exclude vettura-godot
```

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
```

`godot/vettura_native.gdextension` 已把库路径指向
`res://vettura-godot/target/debug/libvettura_godot.dylib`（macOS）。
Godot 项目根目录就是本目录的上一级，所以打开现有 `project.godot` 即可识别扩展。

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
│   ├── vettura-physics-chrono/# Chrono::Vehicle 适配器（占位）
│   └── vettura-godot/        # Godot 4 GDExtension (cdylib)
├── tools/
│   ├── kn5-inspect/           # 查看/导出 KN5
│   └── acd-inspect/           # 解密/解包 data.acd
├── godot/
│   └── vettura_native.gdextension
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
   └── vettura-physics

vettura-physics-chrono ──> vettura-physics
```

规则：

- `vettura-kn5`、`vettura-acdata`、`vettura-physics` **不依赖 Godot**，可单独测试；
- AC 格式知识只放在 `vettura-kn5` / `vettura-acdata` / `vettura-assets`；
- `vettura-physics` 不知道 Assetto Corsa 的存在；
- Godot 层只做类型转换和生命周期管理。

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
