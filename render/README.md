# vettura-render

Assetto Corsa **视觉兼容层**（纯 Rust，不依赖 Godot）。

职责：

- KN5 材质 → 原生材质家族 / 配方，统计覆盖率；
- CSP `ext_config.ini` 的声明式子集解析 → 可应用的 `BindingPlan`；
- Weather / Rain / Grass / Extra / Lighting / Particles / Brake Disc 的模块化 FX
  框架：注册、依赖排序、质量档位、预算与兼容报告。

> 这个 crate 不做 `.fx` 运行时、不执行任意 Lua、不替换原材质。未知项一律
> 进入报告并降级，而不是静默失败。

## 模块地图

| 模块 | 职责 |
| --- | --- |
| `src/material.rs` + `data/recipes.toml` | 材质配方：shader/材质/贴图槽 → 材质家族；覆盖率统计 |
| `src/ext_config.rs` | `ext_config.ini` 解析、include、section 分类 → `BindingPlan` |
| `src/fx/definition.rs` | `FxDefinition` / `FxInstance` / `FxMeta` / 能力/阶段/质量 |
| `src/fx/manager.rs` | 注册表、依赖拓扑排序、生命周期、报告 |
| `src/fx/modules.rs` | 内置 Weather / Rain / Grass / Extra / Lighting / Particles / Brake Disc |
| `src/fx/profile.rs` | 启用/禁用、质量、每模块 TOML 配置 |
| `src/report.rs` | 扫描一个 mod 目录，输出文本/JSON 兼容报告 |
| `src/warning.rs` | 统一的 `CompatWarning`（source/section/severity） |

## 快速使用

```bash
# 单元测试（不需要 mods/）
cargo test -p vettura-render

# 扫描真实 mod
cargo run -p vettura-render-inspect -- "mods/F1 2008 by Cimmerian_Iter"
cargo run -p vettura-render-inspect -- "mods/trr_porsche_992_gt3r" --json

# 只解析少量 KN5（大包更快）
cargo run -p vettura-render-inspect -- "mods/fn_nurburgring" --max-kn5 2
```

报告包含：

- KN5 数量/解析数量；
- 材质映射率、未知 shader、材质家族分布；
- `ext_config` section 统计、材质/贴图覆盖、灯光/粒子/条件；
- FX 模块启用/禁用原因、快照；
- 所有 warning。

## 添加材质配方

编辑 `data/recipes.toml`：

```toml
[[recipe]]
id = "vettura.core.my_material"
priority = 150
family = "pbr"

[recipe.match]
shader = ["ksMyShader"]
has_texture = ["txDiffuse"]

[recipe.textures]
albedo = "txDiffuse"

[recipe.parameters]
roughness = 0.4
```

- `match` 内字段是 AND，同一字段的多个值是 OR；
- 没有命中的材质落到 `vettura.core.fallback`，只报告，不阻断加载；
- 修改后跑 `cargo test -p vettura-render` 和 `render-inspect`。

## 添加 FX 模块

1. 在 `src/fx/modules.rs` 里加一个 `definition(...)`；
2. 声明 `scope` / `capabilities` / `min_quality` / `requires`；
3. 在 `ConfigInstance::update` 里维护状态（v1 不直接画）；
4. 加测试：启用、质量门槛、依赖、快照；
5. 如果需要 `ext_config`，在 `ExtConfigKind` / resolver 里加映射；
6. 文档补到 [`docs/fx.md`](../docs/fx.md)。

## 边界

- 不依赖 `godot`；Godot 适配在 `gdext/src/visual_compat.rs`；
- 不做 `.fx` 解析/执行、不做任意 Lua、不做运行时 shader 注入；
- 不复制 CSP / AC / 模组资产；只读取用户本地文件；
- 大文件扫描是诊断工具，不在每帧运行。

## 相关文档

- [`docs/fx.md`](../docs/fx.md)：FX 子系统架构与扩展流程
- [`docs/visual-mods.md`](../docs/visual-mods.md)：视觉模组作者/用户指南
- [`docs/architecture.md`](../docs/architecture.md)：crate 边界
