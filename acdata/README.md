# vettura-acdata

Assetto Corsa 数据格式三件套：**`.acd` 解密容器 + 宽容 INI + LUT 查表**。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `acd.rs` | `AcdArchive`：解密并解包 `data.acd`；key 从**车目录名**推导 |
| `ini.rs` | `Ini`：宽容 INI 读取（section/key、`;` 与 `//` 注释、重复 key 策略），供 `car.ini` / `engine.ini` / `surfaces.ini` 等共用 |
| `lut.rs` | `Lut` / `LutPoint`：`x|y` 查表（`power.lut`、扭矩曲线…）与线性插值 |

## 关键入口

```rust
let acd = vettura_acdata::AcdArchive::from_path("cars/xxx/data.acd")?;
let ini = vettura_acdata::Ini::parse(text);     // 宽容解析，不 panic、不静默丢 section
let lut = vettura_acdata::Lut::parse(text);     // 精细控制用 parse_with_report
```

**注意**：ACD key 由车目录名推导，**车目录改名后 `data.acd` 就解不开了**
（这是 AC 的规则，不是本 crate 的限制）。`from_path` 自动处理推导。

## 边界（不放入）

- 车目录结构发现与"读哪个文件"的决策（`assets/` 负责）；
- 物理语义（字段含义映射在 `assets/physics_mapping.rs`）；
- 赛道侧专用解析（`models*.ini` / `surfaces.ini` 的语义解析在 `world/track.rs`，
  本 crate 只提供通用 INI 读取）。

## 验证

```bash
cargo test -p vettura-acdata
cargo run -p acd-inspect -- mods/<车目录>/data.acd --extract /tmp/out   # 解包排查
```

## 相关文档

- [`docs/ac-data.md`](../docs/ac-data.md)：acd 容器格式与 key 推导规则
- [`docs/architecture.md`](../docs/architecture.md)
