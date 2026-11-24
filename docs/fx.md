# FX 子系统架构

本文描述 `vettura-render::fx` 的设计：Weather / Rain / Grass / Extra / Lighting /
Particles / Brake Disc 如何以模块方式接入，以及后续新增 FX 的扩展流程。

> 前提：**不运行 CSP `.fx` / Lua，不做运行时 shader 注入。** 参数可以热加载，
> 行为必须进源码；未知项进入兼容报告并降级。

## 1. 分层

```
ExtConfigResolver
  └─ ext_config.ini → FxConfigPatch ─┐
                                    ▼
FxDefinition ──create(config)──► FxInstance
  │                               │
  ├─ FxMeta（id/scope/requires/   ├─ update(ctx)
  │   capabilities/min_quality）  ├─ snapshot()
  └─ 默认 FxConfig                └─ shutdown()
                                    │
FxManager                           ▼
  ├─ 依赖拓扑排序                FxReport
  ├─ 启用/质量/预算协商           ├─ enabled/reason
  ├─ 生命周期                    ├─ snapshot
  └─ 兼容报告                    └─ warnings

gdext/src/fx（后续）
  └─ 把 FxReport 的状态接成 Godot 材质/灯光/粒子/SubViewport
```

## 2. 核心类型

| 类型 | 作用 |
| --- | --- |
| `FxMeta` | 模块 ID、scope、依赖、能力、最低质量、成本 |
| `FxDefinition` | 静态定义：`meta()` / `default_config()` / `create()` |
| `FxInstance` | 运行时实例：`update(&mut FxContext)` / `snapshot()` |
| `FxRegistration` | 后续渲染桥的能力登记（Render/Light/Particle/Material） |
| `FxManager` | 注册、依赖排序、构建、每帧更新、报告 |
| `FxProfile` | 质量、启用/禁用、每模块配置（TOML） |
| `FxContext` | 每帧只读快照：时间/天气/相机/车辆/预算 |
| `FxBudget` | draw call / 粒子 / 灯光 / post pass / ms 上限 |
| `FxReport` | 每个模块的启用状态、原因、快照、警告 |

### scope

- `Global`：跨场景，例如全局粒子服务；
- `Scene`：进入赛道时创建，例如 Weather / Lighting / Rain；
- `Track`：赛道级，例如 Grass；
- `Car`：每辆车一个实例，例如 Brake Disc；
- `Camera`：相机级，例如 DoF / Motion Blur。

v1 在 `FxManager` 里为每个定义创建一个实例；`Car` scope 的多实例在渲染桥接入时再按车辆展开。

## 3. 内置模块

| 模块 ID | scope | 能力 | 依赖 | 说明 |
| --- | --- | --- | --- | --- |
| `vettura.fx.weather` | Scene | Environment | — | 时间、雨量、湿滑、风、雾 |
| `vettura.fx.particles` | Scene | Particle | — | 统一粒子预算/模板入口 |
| `vettura.fx.rain` | Scene | Render, Particle | weather + particles | 雨丝/挡风/水花/积水状态 |
| `vettura.fx.grass` | Track | Render, Material | — | 草密度、距离、风 |
| `vettura.fx.lighting` | Scene | Light | — | 车灯/路灯/额外灯预算 |
| `vettura.fx.brake_disc` | Car | Material, Render | — | 刹车温度→发光强度 |
| `vettura.fx.tyres` | Car | Render, Material | — | 轮胎 FX：滑动/胎面/侧壁/模糊贴图状态 |
| `vettura.fx.extra` | Scene | Render | — | Extra FX 伞形配置（具体 pass 仍是独立模块） |
| `vettura.fx.generic` | Scene | Render | — | 未识别 FX section 的兜底记录 |
| `vettura.fx.extra.bloom` | Scene | Render | — | post pass |
| `vettura.fx.extra.ssao` | Scene | Render | — | 高画质 |
| `vettura.fx.extra.ssr` | Scene | Render | — | 高画质 |
| `vettura.fx.extra.dof` | Camera | Render | — | 高画质 |
| `vettura.fx.extra.motion_blur` | Camera | Render | — | 高画质 |
| `vettura.fx.extra.sharpen` | Scene | Render | — | 中画质 |

`vettura-render` 负责状态/预算/报告；真实画面在 `game/fx/`：

| 模块 | 状态 | 真实效果 |
| --- | --- | --- |
| Weather | 启用 | 复用 `weather_visuals.gd`（天空/雾/太阳/雨丝） |
| Rain | 启用 | 湿路面材质、相机局部雨丝、贴地积水、挡风玻璃雨滴、车轮水花 |
| Grass | 启用 | `SurfaceMap` 采样 + MultiMesh 草 |
| Lighting | 启用 | LIGHT_SERIES/EMISSIVE_LIGHT → Omni/SpotLight3D |
| Particles | 启用 | SMOKE/PARTICLES → GPUParticles3D |
| Brake Disc | 启用 | 刹车盘温度 → emission |
| Tyres | 启用 | TYRES_FX 贴图（含 `extension/textures` zip）替换 |
| Extra | 启用 | Bloom/tonemap/adjustment，高画质 SSAO/SSIL |
| Generic | 记录 | 只进报告，不执行 |

入口是 `game/fx/fx_runtime.tscn`；它自动发现 `WorldBody` / `VehicleBody`，
并把 `AcVisualCompat` 的 `ext_config.records` 分发给各模块。

## 4. 生命周期

```
启动
  └─ FxManager::new() 注册内置定义
进入赛道
  ├─ 读取 FxProfile / ext_config
  ├─ build(profile)
  │    ├─ 依赖拓扑排序
  │    ├─ 检查 enabled / quality / requires
  │    ├─ create(config) → FxInstance
  │    └─ 生成 FxReport
每帧
  ├─ 填充 FxContext（时间/天气/相机/车辆/预算）
  └─ FxManager::update()
       └─ 按依赖顺序 update 每个实例
退出
  └─ shutdown()
```

## 5. 质量与预算

质量档：

```text
Off < Low < Medium < High < Ultra
```

- 每个模块声明 `min_quality`；低于门槛自动禁用并写入报告；
- `FxBudget` 是后续渲染桥的硬上限；
- 超预算降级顺序建议：
  1. 关闭最贵的 post pass；
  2. 降低粒子/草密度；
  3. 减少雨层/水花；
  4. 减少动态灯/阴影；
  5. 关闭整个模块。

移动端不要硬跑桌面效果：模块应声明能力/成本，渲染桥按平台裁剪。

## 6. ext_config 映射

`ExtConfigResolver` 把声明式 section 变成 FX 参数或材质/贴图覆盖：

| section | 处理 |
| --- | --- |
| `[SHADER_REPLACEMENT_...]` | `MaterialOverride`（shader + properties） |
| `[TEXTURE_OVERRIDE_...]` | `TextureOverride` |
| `[MATERIAL_...]` / `[EMISSIVE_...]` | `MaterialOverride` |
| `[LIGHTING]` / `[LIGHT_SERIES_...]` / `[LIGHT_POLLUTION]` / `[EMISSIVE_LIGHT_...]` | `FxConfigPatch` + `FxConfigRecord` → `vettura.fx.lighting` |
| `[EXTRA_LIGHT_...]` | `LightDefinition` + record |
| `[PARTICLES_...]` / `[SMOKE_...]` | `ParticleDefinition` + record |
| `[RAIN_FX]` | patch + record → `vettura.fx.rain` |
| `[WIND]` / `[WEATHER_FX]` | patch + record → `vettura.fx.weather` |
| `[GRASS_FX_...]` | patch + record → `vettura.fx.grass` |
| `[TYRES_FX_...]` | record → `vettura.fx.tyres` |
| `[BRAKE_DISC_FX]` | patch + record → `vettura.fx.brake_disc` |
| `[EXTRA_FX]` / `[EXTRA_FX_SSAO]` / `[EXTRA_FX_SSR]` / `[EXTRA_FX_BLOOM]` / `[EXTRA_FX_DOF]` / `[EXTRA_FX_MOTION_BLUR]` / `[EXTRA_FX_SHARPEN]` | 按名称映射到对应 extra 模块 |
| 其他 `*_FX*` section | 兜底 `vettura.fx.generic` record |
| `[ANIMATION_...]` | 解析保留，标记未接入 |
| `[CONDITION_...]` | 简单条件求值，复杂条件降级 |
| `[MODEL_REPLACEMENT_...]` / `[DRIVER3D_...]` / `[SCRIPTABLE_DISPLAY_...]` / `[DASH_...]` / `[CAMERA_...]` | 识别并报告“模块未接入” |
| `[INCLUDE]` | 递归加载，防环/限深 |
| `[PHYSICS]` / `[AUDIO]` / `[GUI]` | 识别，视觉兼容范围外 |
| 未知 | 报告，忽略 |

**不处理**：`[SCRIPT_...]`、任意 `.fx`、运行时 shader 替换、版本谎报。

## 7. 添加一个新 FX

以 `Tire FX` 为例：

1. 在 `render/src/fx/modules.rs` 加 `definition(...)`；
2. 声明 `FxMeta`：id、scope、capabilities、requires、min_quality、cost；
3. 在 `ConfigInstance::update` 里维护状态；
4. 在 `builtin_definitions()` 注册；
5. 加测试：启用、依赖、质量门槛、快照；
6. 如果需要 ext_config，在 `ExtConfigKind` / resolver 增加映射；
7. 在渲染桥（`gdext/src/fx`）实现对应 pass/材质/粒子；
8. 在 `docs/fx.md` 增加模块说明。

新增模块不应修改核心渲染代码，也不应绕过 `FxManager` 的预算/质量协商。

## 8. 后续渲染桥

`gdext/src/fx` 计划提供：

- `FxNode`：场景节点，持有 `FxManager`；
- `FxRenderBridge`：把 `FxReport` 状态映射到 Godot 材质/灯光/粒子；
- `FxSubViewport`：Rain/Grass/Extra 的离屏 pass；
- `FxLab`：`game/fx_lab.tscn`，开关/调参/调试视图。

在桥接完成前，`render-inspect` 的 FX 报告已经可以用于兼容性评估。

## 9. 边界

- 不执行任意 `.fx`/Lua；
- 不复制 CSP/模组资产；
- 不在每帧扫描 INI/文件；
- 未知项必须报告；
- 社区扩展走源码 PR/fork，不走动态库 ABI。
