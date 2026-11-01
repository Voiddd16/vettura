# Godot 集成

## 版本

- Godot 4.7（工程 `project.godot` 使用 4.7.2）
- `godot` crate 0.5.5，feature `api-4-7`
- `godot` crate 额外启用 `register-docs`，用于把 Rust 文档注释发布到 Godot 编辑器
- 扩展产物：`libvettura_native.dylib`（macOS）

## 构建

```bash
cd vettura-native
cargo build -p vettura-native
```

产物：

```
vettura-native/target/debug/libvettura_native.dylib
```

`godot/vettura_native.gdextension` 已配置（注意 Rust 包名是 `vettura-godot`，
目录名和库名才是 `vettura-native` / `libvettura_native`）：

```ini
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = "4.7"
reloadable = true

[libraries]
macos = "res://vettura-native/target/debug/libvettura_native.dylib"
linux.x86_64 = "res://vettura-native/target/debug/libvettura_native.so"
windows.x86_64 = "res://vettura-native/target/debug/vettura_native.dll"
```

Godot 项目根目录是 `/Users/zhangziyi/Desktop/vettura`，因此
`res://vettura-native/...` 能正确解析。Godot 4.3 起 macOS 平台按架构细分
feature tag（`macos` / `macos.arm64` / `macos.x86_64`），所以几种写法都列了出来。

## 编辑器代码提示与文档

`vettura-native` 已经为 `godot` crate 启用了 `register-docs`：

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
cargo build -p vettura-native
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

### `VehicleBody`（Node3D）

```gdscript
extends Node3D

func _ready():
    $VehicleBody.setup_default()

func _physics_process(_delta):
    $VehicleBody.set_inputs(
        Input.get_action_strength("throttle"),
        Input.get_action_strength("brake"),
        Input.get_axis("steer_right", "steer_left")
    )
```

`VehicleBody` 目前使用 `SimpleBackend`。切换 Chrono 时只需替换
`Simulation::new(Box::new(...))`。

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

## 注意事项

- `register-docs` 需要 Godot ≥ 4.3，当前工程是 4.7，满足要求；
- 文档字符串会嵌入所有 profile（包括 `--release`）；
- 如果不想在发布包里附带说明文本，可以在发布构建中关闭 `register-docs`；
- 只有注册到 Godot 的 `#[func]` / `#[signal]` / `#[var]` 会显示；普通 Rust
  私有函数不会出现在 Godot 补全中。

## 当前类

### `Kn5File`（Resource）

```gdscript
var kn5 := Kn5File.new()
if kn5.load("res://vettura-native/../test/.../car.kn5"):
    print(kn5.node_count())
    print(kn5.mesh_count())
    print(kn5.triangle_count())
    print(kn5.texture_count())
    print(kn5.material_count())
```

目前只解析和统计，不生成 `ArrayMesh`。这是刻意的：先把数据模型稳定下来。

### `AcCar`（Resource）

```gdscript
var car := AcCar.new()
if car.open("res://path/to/cim_2008_ferrari"):
    print(car.display_name())
    print(car.total_mass())
    print(car.wheelbase())
    print(car.data_file_count())
```

### `VehicleBody`（Node3D）

```gdscript
extends Node3D

func _ready():
    $VehicleBody.setup_default()

func _physics_process(_delta):
    $VehicleBody.set_inputs(
        Input.get_action_strength("throttle"),
        Input.get_action_strength("brake"),
        Input.get_axis("steer_right", "steer_left")
    )
```

`VehicleBody` 目前使用 `SimpleBackend`。切换 Chrono 时只需替换
`Simulation::new(Box::new(...))`。

## 下一步：KN5 → 场景

建议按以下顺序实现：

1. **网格转换**
   - `NodeKind::Mesh` → `ArrayMesh`；
   - 顶点布局：position/normal/uv，索引 `u16`；
   - 每个节点生成一个 `MeshInstance3D` 或合并为多 surface mesh；
   - 应用 dummy 的局部/世界变换。

2. **贴图**
   - KN5 内嵌 DDS 字节 → `Image::load_dds_from_buffer`；
   - 创建 `ImageTexture`；
   - 按材质 `txDiffuse` / `txNormal` / `txDetail` 绑定。

3. **材质**
   - 把 `ksPerPixelMultiMap` 等 shader 映射到 Godot `StandardMaterial3D`；
   - 先支持 diffuse/normal/specular，特殊 shader（树、草、玻璃、发光）后续处理。

4. **资源加载器**
   - 实现 `ResourceFormatLoader`，让 `.kn5` 直接拖进场景；
   - 缓存已解析模型，避免重复读取 100MB 文件。

5. **模型节点**
   - 生成 `Node3D` 层级，保留 dummy 名称（`WHEEL_LF`、`SUSP_LF` 等）；
   - 物理层通过名称查找车轮/悬挂挂点。

## 冒烟测试

`godot/kn5_probe.tscn` + `kn5_probe.gd` 会用 `test/` 下的真实 F1 2008 mod 把三台车
实例化进场景，并打印节点/网格/材质/挂点统计：

```bash
cd ..
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . \
    res://vettura-native/godot/kn5_probe.tscn
```

期望输出（示例）：`mesh instances = 125 surfaces = 125`、
`materials = 125 with albedo = 125 with normal = 121`、`WHEEL_LF -> (-0.70, 0.30, -1.53)`。
退出码 0 表示全部通过。

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
- `cargo build -p vettura-native` 失败时先确认 `godot` crate 的 `api-4-7`
  feature；
- 扩展未加载时检查 `.gdextension` 路径和 dylib 是否存在；之前这里踩过一次坑：
   crate 目录从 `vettura-native` 改名后 `.gdextension` 仍指向
  `res://vettura-godot/...`，导致扩展静默不加载；
- 可用 `godot --headless --path .. --editor --quit` 验证扩展注册。
