# vettura-godot（gdext/）

**唯一认识 Godot 的 crate**：GDExtension cdylib + 场景桥接层。
格式解析与物理都在别的 crate，这里只做"Rust 数据 ↔ Godot 资源/节点"。

> 目录名 `gdext/`，crate 名 `vettura-godot`；构建入口是
> `cargo build -p vettura-godot`（不在 workspace `default-members` 里）。

## 模块地图

| 模块 | 职责 |
| --- | --- |
| `lib.rs` | 扩展注册（`VetturaNative`，`vettura_native.gdextension`） |
| `ac_car.rs` | `AcCar`（Resource）：打开车目录 → `VehicleSpec` → 实例化主模型；暴露音频发现结果 |
| `kn5/` | KN5 → Godot 管线，按职责分层：`convert`（纯转换）/ `texture`（DDS 解码缓存）/ `material` / `mesh` / `scene`（节点树构建）；`cache`（**按文件路径三级共享缓存**：同一 kn5/DDS/材质只建一次）/ `builder`（`Kn5SceneBuilder` 分帧加载，`start → step(预算) → root`） |
| `kn5_file.rs` | `Kn5File`（Resource）：脚本/编辑器侧查询统计、按需构建独立 `ArrayMesh` |
| `vehicle_body.rs` | `VehicleBody`（Node3D）：独立模式接 `Simulation`；世界模式由 `WorldBody` 驱动；`WHEEL_*` dummy 自动绑定 |
| `world/` | `WorldBody`（Node3D）：拥有 `TrackModule` + `World`——赛道安装、多车调度、固定步长推进、圈速计时、快照同步回场景树。**进入赛车模拟的主入口** |
| `fmod_system.rs` | `FmodSystem`（Node3D）：每帧调 FMOD `update` + 播放/参数/3D 听者 |
| `weather_visuals.gd` / `.tscn` | 天气视觉系统（GDScript 节点）：`game/render/sky.gdshader` 的天空（大气/太阳/月亮/云/星）、雾、雨粒子、太阳阴影 bias；按湿度切换 AC 干/湿共面路面层（`vettura_wet_layer` 标记由 `kn5/mesh.rs` 写入）。Forward+ 下按天气开 SSAO/SSIL/SSR/体积雾，Mobile 保持轻量。预设 `preset_clear/rain/fog/night()`；`WorldBody.get_weather()` / `set_weather()` 与物理天气双向同步 |
| `paths.rs` | `res://` → 文件系统路径解析（`globalize_path` 的统一封装） |
| `logging/` | 全系统日志：环形缓冲 + 控制台 + 滚动文件 subscriber（`user://log.cfg` 驱动，可热重载），panic hook 写 `last-crash.log`，`VetturaLogger` 暴露给 GDScript。业务 crate 只 `use tracing`，安装与落盘只在这里（见 [`docs/logging.md`](../docs/logging.md)） |

## 探针场景（验证 Rust 链路，headless 可跑）

| 场景 | 验证什么 |
| --- | --- |
| `baseline.tscn` | 真实赛道 + 真实车 20 s（需 `mods/`） |
| `kn5_probe.tscn` | 三台真实 mod 车实例化：节点/网格/材质/挂点统计 |
| `fmod_probe.tscn` | FMOD bank 播放（需 `--features fmod`） |
| `ai_lap.tscn` | AI 遥控整圈：赛道 → AI → 物理 → 计时 → 遥测；窗口模式下支持 `chase`/`cockpit`/`hood`/`bumper`/`wheel`/`drone`/`tcam`/`orbit` 八种录制视角，并经 `FmodSystem` 播放车模引擎声（`AI_LAP_AUDIO=0` 可关，无声效依赖降级为 warning） |
| `load_probe.tscn` | 解析 / 冷实例化 / 缓存命中 / 分帧加载分段计时 |
| `desktop_probe.tscn` | 桌面窗口两阶段视觉验收：`game/main.tscn` 四视角截图 + 真实赛道追逐/AI 行驶截图 |
| `upscaler_shots.tscn` | 上采样视觉验收：真车 + 真赛道跑 AI，按 `SHOT_VIEWS`（默认车后/驾驶位）截 PNG，并打印 `DisplayPipeline` 的 requested/active/scale/MSAA；`SHOT_FULLSCREEN=1` 全屏、`SHOT_1TO1=1` 让全屏 3D 按窗口 1:1 渲染（避开 1280×720 stretch 基准的 2.25× 超采样）、`SHOT_UPSCALER` 选 MetalFX/FSR |

> 主游戏场景（大厅/车库/触摸/多视角）在 `game/`，**不要**加到本 crate。

## 边界（不放入）

- 游戏流程/UI/相机/HUD（GDScript，`game/`）；
- 格式解析与物理（`kn5/`、`acdata/`、`physics/`、`world/`）。

## 验证

```bash
cargo build -p vettura-godot                     # 产出 libvettura_native.dylib/so
cargo build -p vettura-godot --features fmod     # + FMOD
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/baseline.tscn
```

## 相关文档

- [`docs/godot.md`](../docs/godot.md)：扩展类 API、冒烟层次、场景探针详解
- [`docs/architecture.md`](../docs/architecture.md)
