# Godot 集成

## 版本

- Godot 4.7（工程 `project.godot` 使用 4.7.2）
- `godot` crate 0.5.5，feature `api-4-7`
- `godot` crate 额外启用 `register-docs`，用于把 Rust 文档注释发布到 Godot 编辑器
- 扩展产物：`libvettura_native.dylib`（macOS）

## 构建

Rust workspace 就在本 Godot 工程根目录（每个 crate 一个顶层目录，`gdext/` 是
GDExtension 桥接层）：

```bash
cargo build -p vettura-godot            # 产物 target/debug/libvettura_native.dylib
cargo build -p vettura-godot --features fmod   # 接真机 FMOD 声音
```

产物：

```
target/debug/libvettura_native.dylib
```

`gdext/vettura_native.gdextension` 已配置（注意 Rust 包名是 `vettura-godot`，
目录名是 `gdext/`，库名是 `libvettura_native`）：

```ini
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = "4.7"
reloadable = true

[libraries]
macos = "res://target/debug/libvettura_native.dylib"
linux.x86_64 = "res://target/debug/libvettura_native.so"
windows.x86_64 = "res://target/debug/vettura_native.dll"
```

`.gdextension` 里的路径相对本工程根目录解析，workspace 根就是工程根，所以
`res://target/...` 直接可用；Godot 4.3 起 macOS 平台按架构细分
feature tag（`macos` / `macos.arm64` / `macos.x86_64`），所以几种写法都列了出来。
`target/` 里有 `.gdignore`，Godot 的资源系统会整目录跳过，不会去导入构建产物。

## 主项目接入（已就位）

Rust 代码已经在工程里，不需要额外链接步骤：

```bash
cargo build -p vettura-godot
godot --headless --path . res://gdext/baseline.tscn
```

`baseline.tscn` 是主项目接入冒烟：加载 Daytona `roadcourse`，从
`cim_2008_ferrari` 生成真实规格车辆，全油门 20 秒，输出
`[baseline] ... PASS` 并以退出码 `0` 结束。失败会 `push_error` + `quit(1)`。

> `.godot/extension_list.cfg` 记录已加载的扩展，内容应至少包含：
> `res://gdext/vettura_native.gdextension`。改了 `.gdextension` 位置后在编辑器里
> 重新打开一次工程即可。

## 编辑器代码提示与文档

本 workspace 已经为 `godot` crate 启用了 `register-docs`：

```toml
# 根 Cargo.toml
godot = { version = "0.5.5", features = ["api-4-7", "register-docs"] }
```

这样 Rust 的 `///` 文档注释会被编译进扩展，并注册进 Godot 的文档系统。在 Godot
编辑器里写 GDScript 时：

- `Kn5File`、`AcCar`、`VehicleBody` 都会出现在自动补全中；
- 方法、函数参数、返回类型来自 `#[func]` 签名；
- 悬停说明、F1 搜索文档、信号/成员面板会显示 Rust 文档注释内容。

### 写法约定

- 类级文档写在 `#[derive(GodotClass)]` 结构体上；
- 方法文档写在 `#[func]` 上方的 `///`；
- 第一段尽量简短，Godot 会用第一段作为 brief；
- 使用 Markdown 列表/代码块即可，转换由 `godot` crate 完成；
- 未加文档注释的 `#[func]` 仍会出现在补全里，只是没有描述。

示例：

```rust
/// KN5 模型资源。
///
/// 把 Assetto Corsa 的 `.kn5` 容器解析后暴露给 Godot。
#[godot_api]
impl Kn5File {
    /// 加载一个 `.kn5` 文件；成功返回 `true`。
    #[func]
    pub fn load(&mut self, path: GString) -> bool {
        // ...
    }
}
```

### 更新提示内容

修改 Rust 文档注释后必须重新编译扩展，然后让 Godot 重新加载：

```bash
cargo build -p vettura-godot
```

如果编辑器没有立即刷新脚本补全，可以重启 Godot，或用：

```bash
godot --headless --path .. --editor --quit
```

### 注意事项

- `register-docs` 需要 Godot ≥ 4.3，当前工程是 4.7，满足要求；
- 文档字符串会嵌入所有 profile（包括 `--release`）；
- 如果不想在发布包里附带说明文本，可以在发布构建中关闭 `register-docs`；
- 只有注册到 Godot 的 `#[func]` / `#[signal]` / `#[var]` 会显示；普通 Rust
  私有函数不会出现在 Godot 补全中。

## 当前类

### `Kn5File`（Resource）

```gdscript
var kn5 := Kn5File.new()
if kn5.load("res://cars/my_car.kn5"):
    print(kn5.node_count(), " nodes, ", kn5.mesh_count(), " meshes")
    print(kn5.triangle_count(), " tris, ", kn5.texture_count(), " textures")

    var root: Node3D = kn5.instantiate(self)
    print(root.get_node("WHEEL_LF").position)

    # 只要一张网格 / 一个材质时，可以单独生成
    var mesh: ArrayMesh = kn5.build_array_mesh("F2008_BODY_SF")
    var material: Material = kn5.build_material(0)
    print(kn5.dummy_position("SUSP_LF"))
```

`instantiate(parent)` 会把整棵 KN5 树搬到 `parent` 下：

- dummy 节点 → `Node3D`，保留原名字（`WHEEL_LF`、`SUSP_LF`…）；
- 网格节点 → `MeshInstance3D` + `ArrayMesh` + `StandardMaterial3D`；
- KN5 里标记为不可见的节点及其子树默认跳过；`instantiate_with_options()` 可以
  保留它们（`visible = false`），Physics 层仍可用 `find_child()` 找挂点；
- 兄弟重名时追加 `_2`、`_3` 后缀，避免 `get_node()` 歧义。

### `AcCar`（Resource）

```gdscript
var car := AcCar.new()
if car.open("res://cars/my_car"):
    print(car.display_name(), " ", car.total_mass(), " kg")
    print(car.data_file_count(), " data files")
    print(car.model_path())          # lods.ini → 最大 .kn5
    var root: Node3D = car.instantiate(self)
```

`instantiate()` 会先解析主模型 `.kn5`（规则见 `docs/ac-data.md`）再走
`Kn5File` 的同一条实例化路径。

### `FmodSystem`（Node3D）

```gdscript
var audio := FmodSystem.new()
add_child(audio)
if not audio.start(64):
    push_warning("FMOD not linked: ", audio.last_error())
else:
    var car := AcCar.new()
    car.open("res://cars/my_car")
    audio.load_car(car)
    audio.play("engine_ext")       # 循环声：引擎、轮胎、风
    audio.play_once("gear_grind")  # 一次性声：碰撞、换挡、回火
    audio.set_vehicle_parameters("engine_ext", 6000.0, 0.5, 0.25, 30.0, 0.0)
```

- `start(max_channels)` 启动 FMOD runtime，并把坐标系设为 Godot 的右手系；
- `load_car(car)` / `load_car_path(path)` 只把 `.bank` 字节交给 FMOD，**不解析 bank**；
- `event_count()` / `event_name(i)` / `event_guid_by_name(name)` 来自
  `sfx/GUIDs.txt`；
- `parameter_names(event)` / `set_vehicle_parameters(...)` 用于不同 mod 的参数名适配；
- `set_event_transform()` / `set_listener()` 用 Vettura 物理帧
  （`+X` 右、`+Y` 上、`-Z` 前）；`process()` 每帧自动调用 FMOD `update`。

未启用 `fmod` feature 时这些接口仍存在，但 `start()` 返回 `false` 并在
`last_error()` 里说明原因；SDK 下载与版本锁定见 [audio.md](audio.md)。

### `VehicleBody`（Node3D）

```gdscript
extends Node3D

func _ready():
    var car := AcCar.new()
    car.open("res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari")
    $VehicleBody.setup_from_car(car)

func _physics_process(_delta):
    $VehicleBody.set_controls(
        Input.get_action_strength("throttle"),
        Input.get_action_strength("brake"),
        Input.get_axis("steer_right", "steer_left"),
        Input.is_action_pressed("handbrake"),
        Input.is_action_pressed("drs"),
        Input.is_action_pressed("abs"),
        Input.is_action_pressed("tc")
    )
    print($VehicleBody.speed_kmh(), $VehicleBody.gear())
```

- `setup_default()` 仍是 `SimpleBackend`，只用于验证固定步长链路；
- `setup_from_car(car)` / `setup_from_car_folder(path)` 用 Jolt 真实车辆（平地）；
- `set_controls(throttle, brake, steering, handbrake, drs, abs, tc)`；换挡默认自动；
- 读取：`speed_mps` / `speed_kmh` / `forward_speed_mps` / `gear` / `engine_rpm` /
  `wheel_contact(index)` / `wheel_spin(index)` / `wheel_load(index)`；
- `bind_wheels(root)` 按 `WHEEL_LF/RF/LR/RR` 绑 dummy，车轮转向与滚动由快照驱动。

### `WorldBody`（Node3D）

```gdscript
var world := WorldBody.new()
add_child(world)
world.load_track("res://test/daytona_2017", "roadcourse")
var car: VehicleBody = world.spawn_vehicle_from_path(
    "res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
)
```

`WorldBody` 拥有 Jolt `World` 和 `TrackModule`，在自己的 `_physics_process` 里统一
步进，然后把 `VehicleSnapshot` 推回每个 `VehicleBody`；子节点因此**不要**再自带
`Simulation`。赛道视觉默认用碰撞体三角网生成一张低模 `MeshInstance3D`，可用
`world.set_track_mesh_visible(false)` 关掉。圈速：`lap_time_s()` / `last_lap_s()` /
`best_lap_s()` / `laps()` / `sector_count()` / `current_sector()` /
`sector_times_s()` / `reset_lap_timer()`。

## KN5 → 场景的实现要点

1. **网格转换**
   - `NodeKind::Mesh` → 单 surface `ArrayMesh`（`VERTEX/NORMAL/TANGENT/TEX_UV/INDEX`）；
   - `NodeKind::SkinnedMesh` → 额外写入 `BONES/WEIGHTS`，并创建 `Skeleton3D` +
     `Skin`（bind 矩阵同时作为骨骼 rest pose 与 Skin bind pose）；
   - 切线写成 Godot 约定的 4 float（`xyz` + 副切线符号，取 `+1.0`）；
   - 索引 `u16` → `PackedInt32Array`。

2. **坐标**
   - 顶点数据保持 KN5 原样；坐标转换只发生在**节点变换**上
     （`kn5::convert::kn5_transform_to_godot`）；
   - AC 帧 → Godot 帧等价于绕 Y 转 180°（行列式 `+1`），所以三角形绕序、法线
     方向都不用额外翻转——这与 Content Manager 的做法不同（它只镜像 X，必须用
     `ToIndicesFixX()` 倒转绕序）。

3. **贴图**
   - 内嵌 DDS 原样交给 `Image::load_dds_from_buffer`；
   - **mip 链不完整的 KN5 DDS** 会被 Godot 拒绝（导出器常裁剪到 2x2），此时改成
     “只有基础层”再解压缩、重新生成 mip，法线类贴图开启 `renormalize`；
   - 解码结果按贴图名缓存。

4. **材质**：`ksDiffuse/ksAmbient/ksSpecular/ksSpecularEXP/ksEmissive/ksAlphaRef`、
   `txDiffuse/txNormal/txMaps/txDetail/txNormalDetail` 与
   `BlendMode/DepthMode/isAdditive` 映射到 `StandardMaterial3D`，语义参考
   Content Manager 的 `Kn5MaterialDark*`。

5. **模型节点**：见上面 `instantiate()`；dummy 名保留，物理层可以按名字挂载。

### 待接

- `ResourceFormatLoader`：让 `.kn5` 能像普通场景资源那样拖进编辑器；
- 模型缓存（同一 `.kn5` 反复加载时复用解析结果）；
- 按材质合并 surface，降低 draw call；
- 贴图内存策略（现在退化路径会解压成未压缩 RGBA8）；
- AC 专有 shader（树、草、玻璃、发光贴花）与 `layer` 语义。

## 冒烟测试

有两个层次：

1. **模型管线**：`godot/kn5_probe.tscn` + `kn5_probe.gd` 用 `test/` 下的真实
   F1 2008 mod 把三台车实例化进场景，打印节点/网格/材质/挂点统计：

   ```bash
   godot --headless --path /path/to/godot/project \
       res://gdext/kn5_probe.tscn
   ```

   期望输出（示例）：`mesh instances = 125 surfaces = 125`、
   `materials = 125 with albedo = 125 with normal = 121`、
   `WHEEL_LF -> (-0.70, 0.30, -1.53)`。

2. **主项目接入**：`gdext/baseline.tscn` 加载真实赛道 + 真实车，全油门 20 秒，
   打印 `[baseline] ... PASS` 并以退出码 `0` 结束。见到 `[baseline] PASS`
   就说明 `.gdextension` 的 `res://target/...` 路径、`WorldBody` / `VehicleBody`、
   `AcCar → VehicleSpec`、Jolt step、轮子绑定这条链路都是通的。

## 注意事项

- Godot 是 `+X` 右、`+Y` 上、`-Z` 前；KN5 是 `+X` 左、`+Y` 上、`+Z` 前；
- 统一使用 `vettura_core::coords::ac_to_godot`（`kn5::convert` 里的实现与它等价）；
- KN5 顶点可能上百万，转换时避免逐顶点 `Variant` 调用，使用
  `PackedVector3Array` 等批量类型；
- DDS 贴图可能 12MB+，注意显存和加载时间（退化路径会解压成 RGBA8）；
- Godot 4.7 的 GDExtension API 与 `godot` crate 版本必须匹配，升级时同步改
  `api-4-x` feature；
- 节点名里的 `:` 和 `.` 会被 Godot 自己换成 `_`，`sanitize_node_name()` 提前做掉，
  保证脚本侧拿到的名字与预期一致。

## 调试

- Godot 控制台会打印 `godot_error!`；
- `cargo build -p vettura-godot` 失败时先确认 `godot` crate 的 `api-4-7`
  feature；
- 扩展未加载时检查 `.gdextension` 路径和 dylib 是否存在；之前这里踩过一次坑：
  crate 目录改名后 `.gdextension` 仍指向旧路径，导致扩展静默不加载；
- 可用 `godot --headless --path .. --editor --quit` 验证扩展注册。
