# vettura-world

**Jolt 世界层**：把 `vettura-track` 的纯数据装成能跑的物理场地——多车调度、
车-车/车-墙碰撞、车辆物理适配、AI 控制律、计时与遥测。
Jolt 5.6 经 `oxijolt` 安全封装接入（`cross-platform-deterministic`）。

赛道/环境数据（表面、橡胶、天气、AI 线、分段）在 `vettura-track`；本 crate
只消费它们，并通过 `lib.rs` 重导出保持 `vettura_world::SurfaceMap` 这类旧路径
可用。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `world.rs` | `World`：固定步长、静态体 800 m 分块、冲击/接触事件、多车调度、天气与赛道温度推进、橡胶沉积调度 |
| `vehicle/mod.rs` | `VehicleRuntime`：车身/约束装配、每步子系统推进（气动、动力、刹车、热、ERS、燃油、辅助）、快照组装；约 2600 行 |
| `vehicle/constraint.rs` | AC `VehicleSpec` → Jolt `wheeled vehicle` 尺寸换算：悬架几何、刹车力矩分配、变速箱/差速器、整车惯量与合法性校验 |
| `vehicle/friction.rs` | 每轮动态摩擦圆：纵向/横向共享抓地预算 + 随路面与轮胎衰减特性变的全滑移保底份额 |
| `vehicle/grip.rs` | Jolt 轮胎回调的共享输入：表面/水膜/橡胶/轮胎窗/松弛长度按轮发布成 relaxed 原子量，回调只做原子读 + 查表 + 纯函数运算 |
| `vehicle/thermal.rs` | 三层轮胎温度（胎面/胎体/核心）与刹车盘面/核心温度的纯演化函数，以及油液节拍常量 |
| `vehicle/aids.rs` | ABS 与牵引力控制：按上一步快照的真实滑移闭环调节刹车压力与油门 |
| `ai.rs` | `AiDriver` 纯跟踪 + 曲率速度规划 + `AiTraffic` 车流避让 + AC 难度档 |
| `timing.rs` | `LapTimer`：圈速/最佳圈/分段/无效圈与逆行判定（`TrackSection` 在 `vettura-track`） |
| `debris.rs` | 碰撞碎片刚体（车身件/轮胎/锥桶），与赛道/墙/车二次碰撞 |
| `telemetry.rs` | `telemetry.cfg` 配置解析 + JSON 帧 + UDP/TCP 输出（headless） |
| `backend.rs` | `JoltBackend`：`VehicleBackend` 的 Jolt 实现 |
| `frame.rs` | Vettura 物理帧 ↔ Jolt 帧的向量/四元数转换 |

## 关键入口

```rust
use vettura_assets::TrackModule;                               // 赛道组装在 assets
let track = TrackModule::load_layout(track_root, "roadcourse")?; // 目录 → 纯数据
track.add_to_world(&mut world)?;                                // 碰撞网分块进 Jolt
let snap = world.snapshot(handle);                              // 同步回渲染
```

坐标系：AC 授权帧 `+X` 左/`+Y` 上/`+Z` 前，本 crate 输出**一律已是**
Vettura 物理帧（`+X` 右/`+Y` 上/`-Z` 前，转换走 `vettura_physics::coords`），调用方不要二次转换。

## 边界（不放入）

- 渲染网格/材质（`gdext/` 的 KN5 管线）；
- 车辆动力学契约本身（`physics/` + Jolt vehicle constraint；本 crate 只做世界与适配）；
- **赛道/环境纯数据**（`vettura-track`）：表面表、橡胶、天气、AI 线、分段；
- AC 资产解析（`assets/`）：车目录与赛道目录的文件/INI/KN5 解析都在 assets，
  本 crate 只吃组装好的纯数据。

## 验证

```bash
cargo test -p vettura-world                                  # 单元 + 集成（60+ 例）
cargo test -p vettura-assets --test track_module -- --ignored # 真赛道（需 mods/daytona_2017）
```

## 相关文档

- [`docs/physics.md`](../docs/physics.md)：世界与表面模型细节
- [`docs/architecture.md`](../docs/architecture.md)「数据流 → 加载一条 AC 赛道」
