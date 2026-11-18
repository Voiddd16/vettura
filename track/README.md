# vettura-track

**赛道/环境纯数据层**：表面表、橡胶进化、天气、AI 线与计时分段。
不依赖 Jolt（`oxijolt`）、不解析任何 AC 文件、不做 IO 之外的事情。

它是 `vettura-assets`（生产者）和 `vettura-world`（消费者）之间的数据契约：
`TrackModule::load_layout` 把赛道目录解析成这里的类型，`World` 再把它们装进
Jolt 世界。拆出来之后，这层数据可以被不跑物理的工具/分析脚本/测试夹具直接
使用，`assets` 也不必为了几种数据类型拖进整个物理世界。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `surface.rs` | `SurfaceMap` / `SurfaceTable` / `MeshSurfaceIndex` / `ColliderTriangle`：矩形/圆 patch（覆盖优先）+ 逐三角形网格索引 → 每轮摩擦/滚阻查询 |
| `groove.rs` | `GrooveMap`：位置驱动橡胶堆积（`groove.ini`）→ 行驶线随时间提升抓地 |
| `weather.rs` | `Weather`：湿滑/积水/温度/风/空气密度对抓地、滚阻、气动的影响 |
| `spline.rs` | `AiSpline`（`fast_lane.ai` / `pit_lane.ai` / `ideal_line.ai` v7）解析、投影、按里程采样、逐点曲率 |
| `section.rs` | `TrackSection`：`sections.ini` 的分段边界语义 |
| `lib.rs` | `TrackError` / `TrackResult` 与公开重导出 |

## 边界（不放入）

- **AI 驾驶策略**（`AiDriver` / `AiDriverTuning` / `AiTraffic`）留在
  `vettura-world`：它吃 `VehicleSnapshot`，属于控制律，不是赛道数据；
- **圈速计时器**（`LapTimer`）留在 `vettura-world`；
- **AC 文件解析**（`models.ini` / `surfaces.ini` / KN5 / ACD）在
  `vettura-assets` / `vettura-kn5` / `vettura-acdata`；
- **Jolt 世界与车辆物理**在 `vettura-world` / `vettura-physics`。

## 关键入口

```rust
use vettura_track::{AiSpline, SurfaceKind, SurfaceMap, Weather};

let map = SurfaceMap::all(SurfaceKind::Asphalt);
let line = AiSpline::from_path("mods/daytona_2017/roadcourse/ai/fast_lane.ai")?;
let wet = Weather::wet(0.6);
# let _ = (map, line, wet);
# Ok::<(), vettura_track::TrackError>(())
```

坐标系：解析时已经把 AC 授权帧转成 Vettura 物理帧
（`+X` 右/`+Y` 上/`-Z` 前，转换走 `vettura_physics::coords`），调用方不要二次转换。

## 验证

```bash
cargo test -p vettura-track
```

## 相关文档

- [`docs/physics.md`](../docs/physics.md)：表面/天气/橡胶模型细节
- [`docs/architecture.md`](../docs/architecture.md)：crate 图与数据流
