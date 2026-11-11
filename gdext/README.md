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
| `paths.rs` | `res://` → 文件系统路径解析（`globalize_path` 的统一封装） |

## 探针场景（验证 Rust 链路，headless 可跑）

| 场景 | 验证什么 |
| --- | --- |
| `baseline.tscn` | 真实赛道 + 真实车 20 s（需 `mods/`） |
| `kn5_probe.tscn` | 三台真实 mod 车实例化：节点/网格/材质/挂点统计 |
| `fmod_probe.tscn` | FMOD bank 播放（需 `--features fmod`） |
| `load_probe.tscn` | 解析 / 冷实例化 / 缓存命中 / 分帧加载分段计时 |

> 游戏性预览页（车库/触摸/多视角）在 `test_visual/`，**不要**加到本 crate。

## 边界（不放入）

- 游戏流程/UI/相机/HUD（GDScript，`test_visual/`）；
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
