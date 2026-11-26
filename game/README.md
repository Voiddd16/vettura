# `game/` — 主游戏场景

GDScript 游戏流程、UI 与场景编排。只认识 Godot 节点和 `gdext/` 暴露的类
（`AcCar` / `Kn5File` / `Kn5SceneBuilder` / `WorldBody` / `VehicleBody`）；
KN5/ACD 解析、赛道装配与物理都在 Rust crate 里。

## 场景

| 场景 | 作用 |
| --- | --- |
| `lobby.tscn` | **`project.godot` 主场景**：可替换的车库（track）模组 + 当前车辆 + 慢速环绕镜头 |
| `main.tscn` | 车辆浮空预览/驾驶页：触摸油门刹车、四视角、HUD、车模导入 |
| `garage.tscn` | 车模库：列出 `mods/` 与 `user://imported_cars/`，导入/选择/重命名/删除/搜索 |
| `garage_mod_picker.tscn` | 车库模组选择：列出 `ModLibrary` 自动发现的赛道/车库，也可浏览任意目录 |
| `settings_popup.tscn` | 游戏内设置弹窗：音频/画面/输入/车库镜头，大厅与 `main` 共用 |

## 共享脚本

| 脚本 | 作用 |
| --- | --- |
| `game_settings.gd` | autoload `GameSettings`：读写 `user://settings.cfg`，应用音量/窗口模式（窗口/全屏/无边框）/VSync/3D 分辨率，发 `changed` 信号 |
| `input/game_input.gd` | autoload `GameInput`：运行时注册 `game_*` 动作、键鼠/手柄/触屏/陀螺仪、可重绑键位（`user://controls.cfg`）、返回栈 `push_nav_handler()` |
| `mod_library.gd` | autoload `ModLibrary`：递归扫描 mods 目录，自动分类为车辆 / 赛道 / 车库，提供列表、计数与预览图 |
| `garage_mod.gd` | 车库目录校验、单 KN5 / `models*.ini` 加载计划、展示名与预览图路径、AC→Godot 放置帧转换 |
| `safe_area.gd` | 把刘海/挖孔/系统栏安全区换算成 `MarginContainer` 边距 |

## 输入与导航

- 动作在 `GameInput` 里运行时注册，不把键位写死进场景：油门/刹车/转向/视角/
  车辆库/设置/返回/全屏；
- 键盘：W/S 油门刹车、A/D 转向、C 视角、G 车辆库、Esc 设置、F11 全屏；
- 手柄：RT/LT 油门刹车、左摇杆转向、Y 视角、LB 车辆库、Start 设置、B 返回；
- 触屏：左右半屏踏板、底部转向滑杆；支持的设备上用重力方向做倾斜转向；
- 体感：手机陀螺仪或手柄体感（DualSense 等）可开关、调灵敏度、反转、校准；
- 返回：弹窗先消费（`push_nav_handler`），否则驾驶页回大厅、大厅弹退出确认；
  Android 系统返回手势也走同一条链路；
- 全屏：设置里窗口/全屏/无边框三档，F11 / Alt+Enter / 手柄 Start 之外也可
  直接点大厅与驾驶页的「全屏」按钮；
- 全屏自动分辨率：3D 在 35%~100% 之间按帧率调整，macOS 用 MetalFX Spatial、
  其他 Forward+ 平台用 FSR1 上采样；UI/后处理保持原生分辨率。设置 → 画面
  可关闭自动分辨率，改用「3D 分辨率」滑杆固定比例。

## mods 目录

`ModLibrary` 按顺序扫描：

1. `res://mods`：工程/开发期目录；
2. `user://mods`：游戏可写目录（启动时自动创建，Android 也适用）；
3. 桌面导出版可执行文件旁的 `mods/`；
4. macOS `.app` 同级的 `mods/`（包内放 AC mod 的 `extension/*.lua` 会破坏
   代码签名，所以导出包保持 app 与 mods 同级）。

扫描是递归的：包目录（例如 `某车包/assettocorsa/content/cars/xxx`）会继续向下找，
识别到 mod 根后停止下钻。分类规则：

- `car`：有 `data.acd` / `data/car.ini` 且有 `.kn5`；
- `track`：有 `models*.ini`、`data/surfaces.ini`、`ai/fast_lane.ai` 或 `ui_track.json`；
- `garage`：有 `.kn5` 但不是车/赛道（适合大厅展示）。

大厅与车模库都直接使用这份索引；导入的档案写到 `user://mods/imported_cars/`。

## 常用命令

```bash
# 主场景（大厅）
/Applications/Godot.app/Contents/MacOS/Godot --path . res://game/lobby.tscn

# 无头回归
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/mod_library_smoke.tscn
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/lobby_smoke.tscn
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/smoke.tscn
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/garage_smoke.tscn
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/layout_smoke.tscn
```

## 边界

- 不写 KN5/ACD/INI 解析——用 `gdext/` 的 `Kn5File`、`AcCar`、`Kn5SceneBuilder`；
- 不做赛道碰撞与车辆物理——用 `WorldBody` / `VehicleBody`；
- 本地 AC 资产只从 `res://mods/...` 或 `user://` 读，不把大文件放进本目录；
- Rust 侧探针放 `gdext/`，不要为了测 Rust 往 `game/` 加场景。

## 相关文档

- [`docs/godot.md`](../docs/godot.md)：Godot 集成、场景与冒烟
- [`docs/architecture.md`](../docs/architecture.md)：crate 边界与数据流
- [`README.md`](../README.md)：运行方式与本地 `mods/` 结构
