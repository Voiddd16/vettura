# vettura-world

**Jolt 世界层**：把 AC 赛道目录变成能跑的物理场地——三角网碰撞、表面摩擦、
橡胶进化、AI、天气、计时——并负责多车调度与车-车/车-墙碰撞。
Jolt 5.6 经 `oxijolt` 安全封装接入（`cross-platform-deterministic`）。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `surface.rs` | `SurfaceMap` / `SurfaceTable` / `MeshSurfaceIndex` / `ColliderTriangle`：矩形/圆 patch（覆盖优先）+ 逐三角形网格索引 → 每轮摩擦/滚阻查询 |
| `groove.rs` | `GrooveMap`：位置驱动橡胶堆积（`groove.ini`）→ 行驶线随时间提升抓地 |
| `ai.rs` | `AiSpline`（`fast_lane.ai` / `pit_lane.ai` / `ideal_line.ai` v7）+ `AiDriver` 纯跟踪 + `AiTraffic` 车流感知与避让 |
| `timing.rs` | `LapTimer` + `TrackSection`：圈速/最佳圈/分段/无效圈与逆行判定 |
| `weather.rs` | `Weather`：对表面抓地/环境的影响 |
| `debris.rs` | 碰撞碎片刚体（车身件/轮胎/锥桶），与赛道/墙/车二次碰撞 |
| `world.rs` | `World`：固定步长、静态体 800 m 分块、冲击/接触事件、碎片调度 |
| `vehicle.rs` | `VehicleHandle` / `VehicleSnapshot` / `DriveInput` / `VehicleTuning` |
| `backend.rs` | `JoltBackend`：`VehicleBackend` 的 Jolt 实现 |
| `frame.rs` | 帧级调度辅助 |

## 关键入口

```rust
use vettura_assets::TrackModule;                               // 赛道组装在 assets
let track = TrackModule::load_layout(track_root, "roadcourse")?; // 目录 → 纯数据
track.add_to_world(&mut world)?;                                // 碰撞网分块进 Jolt
let snap = world.snapshot(handle);                              // 同步回渲染
```

坐标系：AC 授权帧 `+X` 左/`+Y` 上/`+Z` 前，本 crate 输出**一律已是**
Vettura 物理帧（`+X` 右/`+Y` 上/`-Z` 前，转换走 `vettura_core::coords`），调用方不要二次转换。

## 边界（不放入）

- 渲染网格/材质（`gdext/` 的 KN5 管线）；
- 车辆动力学本身（`physics/` 契约 + Jolt vehicle constraint；本 crate 只做世界与接触）；
- AC 资产解析（`assets/`）：车目录与赛道目录的文件/INI/KN5 解析都在 assets，
  本 crate 只吃组装好的纯数据（`ColliderTriangle` / `SurfaceMap` / `AiSpline`），
  对 kn5 / acdata 零依赖。

## 验证

```bash
cargo test -p vettura-world                                  # 单元：每条物理模型一例（60+ 例）
cargo test -p vettura-assets --test track_module -- --ignored # 真赛道（需 mods/daytona_2017，已随 TrackModule 搬到 assets）
```

## 相关文档

- [`docs/physics.md`](../docs/physics.md)：世界与表面模型细节
- [`docs/architecture.md`](../docs/architecture.md)「数据流 → 加载一条 AC 赛道」
