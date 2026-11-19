# vettura-kn5

Assetto Corsa `.kn5` 模型容器读取器。二进制布局参考社区广泛使用的
`AcTools.Kn5File`（Content Manager 背后的解析器），字段顺序与容错策略保持一致。

解析策略：**对结构刻意严格（损坏文件明确报错并给出文件偏移），对文本编码、
未知枚举值、文件尾补零保持宽容。**

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `lib.rs` | 入口与公开重导出；`MAGIC` / 节点深度上限常量 |
| `parse.rs` | `Kn5::from_path` / `parse` / `parse_shared`（共享字节、零拷贝）与各区块解析（贴图 / 材质 / 节点树） |
| `model.rs` + `model/` | 数据模型：`Kn5` 容器 + 节点树（dummy / mesh / skinned）、材质、贴图、顶点结构（按子领域拆成 `model/{texture,material,node,mesh}.rs`） |
| `tests.rs` + `tests/parse.rs` | 解析行为单元测试：字节流构造辅助与用例 |
| `reader.rs` | 带边界检查的小端字节游标；每次读取要么成功推进、要么返回带偏移的 `Error::Eof` |
| `dds.rs` | 内嵌 DDS 解码（DXT 系 → RGBA）；KN5 mip 链不完整时兜底补齐 |
| `synth.rs` | 合成车模生成器（合法 DXT1 DDS + 截断 mip 链）：bench / 测试 / `kn5-inspect --synth` 共用，不依赖真车资产 |
| `error.rs` | `Error`：magic 不符、越界、未知变体……全部带文件偏移与上下文 |

## 关键入口

```rust
let kn5 = vettura_kn5::Kn5::from_path("car.kn5")?;
println!("{} nodes, {} textures", kn5.node_count(), kn5.textures.len());
```

- `Kn5` / `Node` / `NodeKind`：保留完整节点层级（挂点、轮子、可动部件），
  `WHEEL_LF` 等命名约定由上层（`gdext`）解释；
- 材质：shader 名、参数 KV、贴图槽位原样保留，Godot 侧翻译在 `gdext/kn5/material.rs`。

## 边界（不放入）

- Godot 资源构造（`ArrayMesh` / `Image` / 材质翻译在 `gdext/kn5/`）；
- 车目录组织与参数语义（`assets/`）；
- INI / ACD / LUT（`acdata/`）。

## 验证

```bash
cargo test -p vettura-kn5                                     # 合成样本，无需真车
cargo run -p kn5-inspect -- car.kn5 --nodes --materials       # 真车排查
cargo run -p kn5-inspect -- --synth                           # 生成合成车模
```

## 相关文档

- [`docs/kn5-format.md`](../docs/kn5-format.md)：字段级格式逆向记录
- [`docs/architecture.md`](../docs/architecture.md)
