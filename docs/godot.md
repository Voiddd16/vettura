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
    car.open("res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari")
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
world.load_track("res://mods/daytona_2017", "roadcourse")
var car: VehicleBody = world.spawn_vehicle_from_path(
    "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
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
   - 轴镜像 `S`（`(x,y,z) → (-x,y,-z)`）分两半做：节点局部变换做 `S·M·S`
     （`kn5::convert::kn5_transform_to_godot`），顶点分量（位置/法线/切线）
     乘 `S`（`mesh.rs` 的 `vertex_vec3`）；链式复合后整体是 `S·T·v`，网格与
     物理 dummy 挂点走同一条转换，位置严格一致；
   - `S` 行列式 `+1`（绕 Y 转 180°），轴镜像不影响三角形绕序与手性。KN5 原始
     绕序与 Godot 正面相反（D3D 顺时针 vs GL 逆时针），在 `set_indices` 里统一
     翻转；这与 Content Manager 的做法不同（它只镜像 X，用 `ToIndicesFixX()`
     倒转绕序）。

3. **贴图**
   - 内嵌 DDS 原样交给 `Image::load_dds_from_buffer`；解码结果按贴图名缓存；
   - **KN5 导出器的 DDS 头千奇百怪**，Godot 会整张贴图拒绝。修复链按顺序试
     （`gdext/src/kn5/texture.rs`，每一级都拿 Daytona/F1 2008 真实贴图验过）：
     0. 修自相矛盾的头（flags 没声明 PITCH/LINEARSIZE、值却非零 —— Daytona
        274 张里 42 张，看台/人群因此全黑）；
     1. 原样加载（完整链的保留压缩格式与原有 mip，省显存）；
     2. 失败则把头改成“只有基础层”（`strip_mip_levels`，payload 原样留着：
        Godot 对 mipcount=1 的输入只按基础层算账）；
     3. 再失败则把 payload 截到基础层大小；
     4. 再失败则截到“头部声明级数”刚好容纳的大小（救声明 5 级却塞了接近 12 级
        数据的那类贴图）；
   - 退化路径会解压成 RGBA8 并让 Godot 重新生成 mip，法线类贴图开启
     `renormalize`；贴图名写进 Godot 材质名，出问题一眼定位。

4. **材质**：`ksDiffuse/ksAmbient/ksSpecular/ksSpecularEXP/ksEmissive/ksAlphaRef`、
   `txDiffuse/txNormal/txMaps/txDetail/txNormalDetail` 与
   `BlendMode/DepthMode/isAdditive` 映射到 `StandardMaterial3D`，语义参考
   Content Manager 的 `Kn5MaterialDark*`。踩过的坑都在 `kn5/material.rs` 的注释里，
   要点：
   - **车漆的红色来自 `txDetail`**：AC 的 `txDiffuse` 对车漆材质常常是一张灰色
     底图（`f2008_top_2_diffuse.dds`），涂装色在 `txDetail`
     （`f2008_top_2.dds`，均值 (0.45, 0.096, 0.094)），shader 按
     `uv * detailUVMultiplier` 乘回去。映射为 Godot 的 detail(MULTIPLY) +
     `uv2_scale` + 网格补一份 `TEX_UV2`（只在真有 detail 材质时生成）；
   - **`BlendMode::AlphaBlend` 不等于“表面要半透明”**：KN5 的 diffuse 贴图
     alpha 常常是别的东西（Daytona 看台那栋楼 alpha 均值 0.37，接上 ALPHA 混合
     会变成一层黑纱）。代表“整体透明”的是 `ksOpacity` / `alpha`；
   - **`isAdditive == 1` 不是“整面叠加”**：AC 只在镜面通道做叠加，映射成
     `BlendMode::ADD` 会让沥青看台、碳纤维这类深色 albedo 变黑并互相闪烁；
     `isAdditive == 2`（且不是 `ksTyres`）才是车漆 → 清漆层，但清漆强度要压到
     0.35，否则环境反射把红车漆刷成白车；
   - `ksPerPixelReflection`（护栏玻璃、看台墙面、水面、车手玻璃）在 AC 里靠
     环境反射发亮，映射成 metal + 低 roughness。

5. **模型节点**：见上面 `instantiate()`；dummy 名保留，物理层可以按名字挂载。

6. **环境光**：`project.godot` 用 `environment/defaults/default_environment`
   指向 `game_environment.tres`（天空 + 恒定色环境光）。没有它，`main.tscn`
   （只有一盏日光）太阳照不到的地方全是死黑——看台内壁、车身底面、马路阴影。
   注意：运行时用代码建的 `AMBIENT_SOURCE_SKY` 环境实测不产生任何环境光，只有
   显式颜色照得亮；这是 Godot 4.7 的行为，写在这里省得再踩。

### 待接

- `ResourceFormatLoader`：让 `.kn5` 能像普通场景资源那样拖进编辑器；
- 按材质合并 surface，降低 draw call；
- 贴图内存策略（现在退化路径会解压成未压缩 RGBA8）；
- AC 专有 shader（树、草、玻璃、发光贴花）与 `layer` 语义；
- 皮肤（skin）系统：AC 的涂装是按 skin 换 `txDiffuse` 的贴图，现在只用 KN5
  内嵌的那一套。

## 冒烟测试

有两个层次：

1. **模型管线**：`gdext/kn5_probe.tscn` + `kn5_probe.gd` 用 `mods/` 下的真实
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

3. **日志链路**：`test_visual/log_smoke.tscn` 验证 `VetturaLogger` GDScript API
   （ring/recent/dump/reload/set_level/dump_to_file），打印 `[log-smoke] PASS`；
   `LOG_SMOKE_PANIC=1` 前缀触发一次真实 Rust panic，之后 `last-crash.log`
   应包含 panic 消息 + backtrace + 环形缓冲转储（命令见
   [`logging.md`](logging.md) §10）。

4. **AI 整圈**：`gdext/ai_lap.tscn` 让 `AiDriver` 遥控真车沿 `fast_lane.ai`
   跑完整一圈，逐秒打印遥测，结束打印 `[ai-lap] PASS`。见
   [`architecture.md`](architecture.md) 的「AI 司机与 AC 难度档」。

## 录一整圈 AI 驾驶视频

`scripts/record_ai_lap.py`：Godot 抓帧 + ffmpeg 编码，不需要录屏权限。

```bash
uv run scripts/record_ai_lap.py                # 完整一圈，720p30 / H.264 CRF 24
uv run scripts/record_ai_lap.py --every 3      # 抓帧抽稀（20fps 视频）
uv run scripts/record_ai_lap.py --skill 60     # 换 AI 难度档（0..=110）
uv run scripts/record_ai_lap.py --keep-frames  # 保留 JPEG 帧目录
```

- 帧落在 `user://ai_lap_frames`（macOS 在
  `~/Library/Application Support/Godot/app_userdata/Vettura/ai_lap_frames`），
  编码后默认清理；
- 物理走 `--fixed-fps 60`，所以视频时长 = 圈速，与机器快慢无关；
- 环境变量 `AI_LAP_TRACK` / `AI_LAP_LAYOUT` / `AI_LAP_CAR` 可换赛道与车；
- 产物落在 `build/ai_lap_<track>_<layout>_<时间戳>.mp4`。Daytona roadcourse
  175 秒一圈实测约 48 MB。

## 注意事项

- Godot 是 `+X` 右、`+Y` 上、`-Z` 前；KN5 是 `+X` 左、`+Y` 上、`+Z` 前；
- 统一使用 `vettura_physics::coords::ac_to_godot`（`kn5::convert` 里的实现与它等价）；
- KN5 顶点可能上百万，转换时避免逐顶点 `Variant` 调用，使用
  `PackedVector3Array` 等批量类型；
- DDS 贴图可能 12MB+，注意显存和加载时间（退化路径会解压成 RGBA8）；
- Godot 4.7 的 GDExtension API 与 `godot` crate 版本必须匹配，升级时同步改
  `api-4-x` feature；
- 节点名里的 `:` 和 `.` 会被 Godot 自己换成 `_`，`sanitize_node_name()` 提前做掉，
  保证脚本侧拿到的名字与预期一致。

## 调试

- Godot 控制台会打印 `godot_error!`；
- **Rust 侧全系统日志**：运行时同时写控制台、`user://logs/vettura.<日期>.log`
  与内存环形缓冲；级别/target 可用 `VETTURA_LOG`（或 `user://log.cfg`）调，
  GDScript 侧用 `VetturaLogger`（运行期调级别 / 读最近日志 / 导出转储）；崩溃时
  panic hook 写 `user://logs/last-crash.log`；详见 [`logging.md`](logging.md)；
- `cargo build -p vettura-godot` 失败时先确认 `godot` crate 的 `api-4-7`
  feature；
- 扩展未加载时检查 `.gdextension` 路径和 dylib 是否存在；之前这里踩过一次坑：
  crate 目录改名后 `.gdextension` 仍指向旧路径，导致扩展静默不加载；
- 可用 `godot --headless --path .. --editor --quit` 验证扩展注册。

## 加载缓存与分帧加载

车模加载的耗时集中在**重复解码**：同一台车被 `AcCar`、`Kn5File` 或多个实例引用
时，早期实现会重新解析整份 `.kn5`、重新解压每张贴图、给每个网格节点各建一份
材质。现在由 `gdext/src/kn5/cache.rs` 统一处理：

| 缓存 | 键 | 失效时机 |
| --- | --- | --- |
| 解析结果 `Arc<Kn5>` | 规范化文件路径 | 没有任何对象再引用该文件 |
| 贴图 `Gd<ImageTexture>` | KN5 贴图下标 | 同上（随条目一起释放） |
| 材质 `Gd<StandardMaterial3D>` | KN5 材质下标 | 同上 |

注册表只持 `Weak` 引用：不把贴图永久钉在显存里，也不会无限增长。线程模型是
“主线程访问”（GDExtension 方法只会在主线程被调用），跨线程误触会被 gdext 的
safeguards 当场报错。

需要把加载摊到多帧（例如加载页进度条）时用 `Kn5SceneBuilder`：

```gdscript
var builder := Kn5SceneBuilder.new()
if builder.start("res://cars/cim_2008_ferrari/cim_2008_ferrari.kn5"):
    while not builder.step(Kn5SceneBuilder.STEP_BUDGET, self):
        progress_bar.value = builder.progress()
        await get_tree().process_frame
    var car: Node3D = builder.root()
```

`start()` 只做“展开任务列表”（读文件 + 解析，通常几十毫秒），几何与贴图在
`step()` 里按需生成；`scene::instantiate()` 内部的 `Build` 与 `Kn5SceneBuilder`
是同一条代码路径，只是预算设为“一次做完”。测量脚本见
`scripts/load_probe.py`。
