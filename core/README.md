# vettura-core

全工作区共享的地基 crate：**单位约定、坐标系转换、数学库再导出**。
刻意保持轻依赖（几乎只有 glam），所有其它 crate 都依赖它——
这里的每一条约定都是全 workspace 的契约。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `coords.rs` | AC 坐标系（`+X` 左 / `+Y` 上 / `+Z` 前，KN5 实测校准）↔ Vettura/Godot 物理帧（`+X` 右 / `+Y` 上 / `-Z` 前）转换 |
| `units.rs` | SI 单位约定与类型别名（`Meters` / `Seconds` / `Kilograms` / `Newtons` / `NewtonMeters`） |

## 关键入口

```rust
use vettura_core::{coords, units, Vec3, Quat, Mat4};   // glam 统一再导出
```

- `coords::ac_to_godot` 等：坐标系转换**只能**走这里，调用方不要自己转两次；
- `units::*`：SI 别名。原则：**边界转换（assets 层），核心内不转换**——
  AC 的 `%`、`deg`、`km/h` 不允许渗进 physics/world；
- `VERSION`：workspace 版本，供扩展诊断输出。

## 边界（不放入）

- 具体格式解析（`kn5/`、`acdata/` 各自负责）；
- 物理模型（`physics/`、`world/`）；
- Godot 类型（只有 `gdext/` 认识 Godot）；
- glam 之外的数学依赖（防止各 crate 数学版本漂移）。

## 验证

```bash
cargo test -p vettura-core
```

## 相关文档

- [`docs/architecture.md`](../docs/architecture.md)「关键约定」节
- [`AGENTS.md`](../AGENTS.md)
