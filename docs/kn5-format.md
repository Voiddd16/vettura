# KN5 格式笔记

本文记录 `vettura-kn5` 实际解析的布局。字段顺序以 AcTools 的
`AcTools/Kn5File/Kn5Basic.cs`（`Kn5Reader`）、`Kn5Node.cs`、
`Kn5Material.cs` 为准，并用 `mods/` 下的真实车模交叉验证。

所有整数/浮点均为小端。

## 文件头

| 偏移 | 类型 | 说明 |
| --- | --- | --- |
| 0 | `u8[6]` | magic，固定 `sc6969` |
| 6 | `u32` | version，当前为 5；代码接受 5/6 |
| 10 | `u32` | 仅当 `version > 5` 时出现的额外字段，含义未知，保存在 `Kn5::header_extra` |

随后依次是：贴图表 → 材质表 → 节点树。

## 贴图表

| 类型 | 说明 |
| --- | --- |
| `u32` | 贴图数量 |
| 每项：`i32` | active 标志，AcTools 以 `1` 表示启用 |
| 每项：`u32` | 文件名长度 |
| 每项：`u8[len]` | 文件名，例如 `t-cam_diffuse.dds`，无 NUL 结尾 |
| 每项：`u32` | DDS 数据长度 |
| 每项：`u8[len]` | 原始 DDS 数据 |

贴图名通常是 `.dds`，但本质是原始 DDS 载荷，可以交给 Godot 的
`Image::load_dds_from_buffer`。

AcTools 会丢弃长度为 0 的贴图；`vettura-kn5` 保留所有条目，方便离线检查
和将来回写。

## 材质表

| 类型 | 说明 |
| --- | --- |
| `u32` | 材质数量 |
| 每项：`u32 + bytes` | 材质名 |
| 每项：`u32 + bytes` | shader 名，例如 `ksPerPixelMultiMap` |
| 每项：`u8` | blend mode |
| 每项：`u8` | alpha tested（布尔） |
| 每项：`i32` | depth mode |
| 每项：`u32` | 属性数量 |
| 属性：`u32 + bytes` | 属性名，例如 `ksAmbient` |
| 属性：`f32` | `ValueA` |
| 属性：`f32[2]` | `ValueB` |
| 属性：`f32[3]` | `ValueC` |
| 属性：`f32[4]` | `ValueD` |
| 每项：`u32` | 贴图槽数量 |
| 槽：`u32 + bytes` | 槽名，例如 `txDiffuse` |
| 槽：`i32` | 槽索引 |
| 槽：`u32 + bytes` | 引用的贴图名 |

每个 shader 属性固定 40 字节（`4 + 4 + 8 + 12 + 16`）。多数属性只使用
`ValueA`，但法线、颜色等属性会用到其余分量，因此 Rust 模型完整保留
`value_a`/`value_b`/`value_c`/`value_d`。

blend mode：

| 值 | 含义 |
| --- | --- |
| 0 | Opaque |
| 1 | Alpha blend |
| 2 | Alpha to coverage |
| 其它 | 未知值，保留为 `BlendMode::Unknown(u8)` |

depth mode：

| 值 | 含义 |
| --- | --- |
| 0 | Normal |
| 1 | Read-only（depth write 关闭） |
| 2 | Off |
| 其它 | 未知值，保留为 `DepthMode::Unknown(i32)` |

已确认的常见属性：`ksAmbient`、`ksDiffuse`、`ksSpecular`、
`ksSpecularEXP`、`diffuseMult`、`normalMult`、`useDetail`、
`detailUVMultiplier`、`ksEmissive`、`ksAlphaRef`。

已确认的常见槽：`txDiffuse`、`txNormal`、`txDetail`、`txDetailR/G/B/A`、
`txDetailNM`、`txMask`。

## 节点树

文件只存一个隐式根节点，根节点通过 `children_count` 递归拥有整棵树。
节点通用头：

| 类型 | 说明 |
| --- | --- |
| `i32` | 节点类型：1=dummy，2=mesh，3=skinned mesh |
| `u32 + bytes` | 节点名 |
| `i32` | 子节点数量 |
| `u8` | active 标志（AC 的 bool） |

三个 mesh 标志在文件里是三个连续字节：

| 字节 | 对应字段 |
| --- | --- |
| 0 | `cast_shadows` |
| 1 | `is_visible` |
| 2 | `is_transparent` |

### type 1：Dummy

| 类型 | 说明 |
| --- | --- |
| `f32[16]` | 4x4 局部变换，行主序 |

车轮、悬挂枢轴、相机点等通常是 dummy。样本中：

- `WHEEL_LF` = `(+0.711, 0.301, +1.526)`
- `WHEEL_RF` = `(-0.711, 0.301, +1.526)`
- `WHEEL_RR` = `(-0.696, 0.301, -1.646)`

由此确定 AC 坐标系为：`+X` 左、`+Y` 上、`+Z` 前。

`vettura_physics::coords::ac_to_godot` 负责转换；`vettura_kn5::transform_to_mat4`
可把行主序的 `Transform` 转成 `glam::Mat4`。

### type 2：Mesh

| 类型 | 说明 |
| --- | --- |
| `u8[3]` | castShadows / isVisible / isTransparent |
| `u32` | 顶点数 |
| 每顶点 `f32[3]` | position |
| 每顶点 `f32[3]` | normal |
| 每顶点 `f32[2]` | uv |
| 每顶点 `f32[3]` | tangent |
| `u32` | 索引数 |
| `u16[index_count]` | 索引 |
| `u32` | 材质索引；部分导出器用 `0xFFFFFFFF` 表示无材质 |
| `u32` | layer |
| `f32` | LOD in |
| `f32` | LOD out |
| `f32[3]` | 包围球中心 |
| `f32` | 包围球半径 |
| `u8` | isRenderable（布尔） |

每个顶点固定 44 字节。

### type 3：Skinned Mesh

| 类型 | 说明 |
| --- | --- |
| `u8[3]` | castShadows / isVisible / isTransparent |
| `u32` | 骨骼数 |
| 每骨骼：`u32+bytes` | 骨骼名 |
| 每骨骼：`f32[16]` | 绑定变换，行主序 |
| `u32` | 顶点数 |
| 每顶点 | position/normal/uv/tangent（44 字节） |
| 每顶点 | weights `f32[4]` |
| 每顶点 | bone indices `f32[4]` |
| `u32` | 索引数 |
| `u16[index_count]` | 索引 |
| `u32` | 材质索引 |
| `u32` | layer |
| `f32` | LOD in |
| `f32` | LOD out |

每个顶点固定 76 字节。骨骼索引在文件里是 **float**，不是整数；`-1.0`
表示未绑定。`SkinnedVertex::bones()` 会过滤未绑定项并把索引转成 `u32`。

注意：AcTools 的“只读层级”路径把结尾 16 字节标记为 mystery bytes，而完整
解析路径把它们读成 `lodIn`/`lodOut`。本实现采用后者。

## 与 AcTools 的差异

保持兼容，但在损坏输入上更严格：

- 未知节点类型、负数子节点数、截断文件 → 返回带偏移的错误，而不是猜测；
- 未知 blend/depth mode 保留原始值，不静默改成默认值；
- 所有贴图条目都保留（包括 0 长度）；
- 材质索引使用 `u32` 原值，`Mesh::material_index()` 负责处理
  `0xFFFFFFFF`；
- 不提供 AcTools 的“只读层级 / 跳过网格”流式模式；当前一次性解析完整模型。

## 零拷贝与批量读取

`vettura-kn5` 解析完成后不会为贴图另外复制一份 DDS 字节：

- `Kn5::from_path` 读入文件后得到 `SharedBytes = Arc<Vec<u8>>`；
- `Texture::data` 是 `TextureData`，只保存 `SharedBytes + Range<usize>`；
- `TextureData` 可以 `Deref<Target=[u8]>`、`AsRef<[u8]>`，因此按普通切片使用；
- 克隆 `TextureData` 或整个 `Kn5` 只会增加引用计数，不会复制贴图。

相关 API：

```rust
pub type SharedBytes = Arc<Vec<u8>>;

Kn5::from_path(path)?;              // 文件 → 共享缓冲，无二次复制
Kn5::parse_owned(vec)?;             // Vec<u8> → 共享缓冲，无二次复制
Kn5::parse_shared(Arc::clone(&b))?; // 直接复用已有共享缓冲
Kn5::parse(&[u8])?;                 // 便捷入口；内部会复制整份输入
```

顶点、蒙皮顶点和材质属性改为批量读取后再按固定步长解析：

- 静态网格一次性读入 `vertex_count * 44` 字节，再用 `as_chunks::<44>` 遍历；
- 蒙皮网格一次性读入 `vertex_count * 76` 字节；
- 材质属性一次性读入 40 字节，拆成 `ValueA/B/C/D`。

这样减少了逐字段的边界检查，也让编译器和 CPU 更接近连续内存访问。

## 已实现的防御

- 所有读取都做越界检查，错误包含文件偏移；
- 数量字段先按“每项最小字节数”校验，避免损坏文件触发超大分配；
- 顶点/蒙皮顶点的批量长度使用 `checked_mul`，避免 32 位平台溢出；
- 节点递归限制 256 层；
- 文件尾允许全零填充，其它尾随数据报错；
- 字符串按 UTF-8 lossy 解码，兼容非标准编码的 mod；
- `u16` 索引读取使用带检查的乘法，避免 32 位平台溢出。

## 待办

- 验证 version 6 的额外头部字段；
- 确认 layer、LOD 的实际渲染语义；
- 支持写入/回写（当前只读）；
- 贴图零拷贝已实现；后续可继续减少材质/节点的小 `Vec` 分配。

## KN5 DDS 的 mip 链问题

F1 2008 试样里 50 张贴图中，`DXT5` 35 张、无 FourCC 的未压缩格式 13 张、
`DXT1` 1 张、DX10 头（BC7）1 张。**压缩贴图普遍少写最小的几级 mip**：例如
`flat_nm.dds`（32x32 DXT5）声明 `dwMipMapCount = 5`，payload 却只有 5 级
（少了最后一级 1x1 的 16 字节）；而 `f2008_top_2_diffuse.dds`（1024x1024）的
链是完整的。

后果：严格加载器（例如 Godot 的 `Image::load_dds_from_buffer`）只要头部声明了
mip 就要求完整链，否则整张贴图作废、返回空图（报 “Expected Image data size …
got … instead”）。`vettura-kn5::strip_mip_levels()` 负责把头部改成“只有基础层”，
Rust 侧（`kn5/`）的解码路径随后解压缩并重新生成 mip；详见 `docs/godot.md`。
