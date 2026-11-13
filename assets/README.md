# vettura-assets

AC 兼容性的"胶水层"：打开车目录 → 选数据源 → 解析参数 → 映射成物理规格 →
找到主模型。

> ⚠️ **这是代码 crate，不是资产目录。** 本地 AC 车模/赛道资产放 `mods/`
> （gitignore + `.gdignore`）。crate 名里的 "assets" 指它负责*解析*资产数据。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `car.rs` | `CarFolder` / `CarData` / `DataSource`：定位数据源（`data.acd` 或已解包的 `data/`）、按名读 INI/LUT、选主模型 `.kn5`（`lods.ini` → 最大文件） |
| `track.rs` | `TrackModule`：`models*.ini` / `surfaces.ini` / `sections.ini` 解析、碰撞 KN5 → 物理帧三角网、`SurfaceMap` / 发车格 / AI 线组装，结果装进 `vettura_world::World` |
| `physics_mapping.rs` | AC 参数 → `vettura_physics::VehicleSpec` 全字段映射（约 1300 行）：`car.ini` / `engine.ini` / `drivetrain.ini` / `suspensions.ini` / `tyres.ini` / `aero.ini` / `brakes.ini`，含 `power.lut` 扭矩曲线、惯量张量合理性校验 |
| `error.rs` | `AssetError`：缺文件、缺 key、数据不合理……带文件与字段定位；赛道侧另有 `TrackError` |

## 关键入口

```rust
let car = vettura_assets::CarFolder::open("mods/<车目录>/assettocorsa/content/cars/<车>")?;
let spec: vettura_physics::VehicleSpec = car.vehicle_spec()?;   // 全量物理参数
let kn5_path: Option<PathBuf> = car.main_model_path();          // 主模型（None = 缺失）

let track = vettura_assets::TrackModule::load_layout(track_dir, "roadcourse")?;
track.add_to_world(&mut world)?;                                // 解析结果回传 world 安装
```

## 边界（不放入）

- 渲染（Godot 资源构造在 `gdext/`）；
- KN5 内部解析（`kn5/`）、INI/ACD 底层解析（`acdata/`）；
- 世界模拟本身（`world/`）：本 crate 只负责把赛道目录**组装**成纯数据交给
  `vettura_world::World`，不实现碰撞/表面/AI/计时语义；
- 单位/坐标转换（走 `vettura_core::coords`，AC 单位在这里的边界处转成 SI）。

## 验证

```bash
cargo test -p vettura-assets                                    # 合成样本
cargo test -p vettura-assets --test real_car -- --ignored       # 真车映射（需 mods/ 在位）
cargo test -p vettura-assets --test track_module -- --ignored   # 真赛道（需 mods/daytona_2017）
```

## 相关文档

- [`docs/architecture.md`](../docs/architecture.md)「数据流 → 加载一辆 AC 车」
- [`docs/physics.md`](../docs/physics.md)：映射的物理语义与取舍
