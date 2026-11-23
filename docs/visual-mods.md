# 视觉模组兼容指南

本文面向两类人：

- **玩家**：把 AC 视觉模组放进 `mods/`，想知道哪些能直接用、哪些会降级；
- **视觉模组作者**：想为 Vettura 贡献 shader / 材质 / FX。

> 设计原则：**素材和参数可热加载；shader 与渲染行为进源码。**
> 不运行 CSP `.fx`，不执行任意 Lua，不做运行时替换。

## 1. 兼容分级

| 档位 | 用户做什么 | 是否需要改源码 |
| --- | --- | --- |
| **T0 素材** | 皮肤、贴图、PP LUT、天气预设 | 否 |
| **T1 声明式** | `ext_config.ini`、`visual.toml` 参数覆盖 | 否 |
| **T2 源码扩展** | 新 shader / 新 FX 模块 / RenderFeature | 是（PR 或 fork） |
| **T3 Fork** | 深度改渲染管线 | 是 |

不做 T4：运行时 shader / 代码注入。

## 2. 现在能自动处理什么

### 2.1 材质

`render/data/recipes.toml` 覆盖常见 AC shader：

- `ksPerPixel` / `ksPerPixelNM` / `ksPerPixelMultiMap`
- `ksPerPixelReflection` / `ksMultilayer_fresnel_nm`
- `ksWindscreen` / `ksGlass`
- `ksEmissive`
- `ksBrakeDisc`
- `ksTyres`
- `ksTree` / `ksGrass` / `ksFlags`
- `ksSkidMarks` / `ksDecal`

未知 shader → fallback + 报告，不阻断加载。

### 2.2 `ext_config.ini`

已解析/映射：

- `[SHADER_REPLACEMENT_...]`
- `[TEXTURE_OVERRIDE_...]`
- `[MATERIAL_...]` / `[EMISSIVE_...]`
- `[LIGHTING]`
- `[EXTRA_LIGHT_...]`
- `[PARTICLES_...]`
- `[CONDITION_...]`（简单条件）
- `[INCLUDE]`（递归、防环）

已识别但暂未接入渲染：

- `[ANIMATION_...]`
- `[MODEL_REPLACEMENT_...]`
- `[DRIVER3D_...]`
- `[SCRIPTABLE_DISPLAY_...]`
- `[TYRES_FX_...]`
- `[DASH_...]`
- `[CAMERA_...]`

视觉范围外：

- `[PHYSICS]` / `[TYRES]`
- `[AUDIO]` / `[SOUND]`
- `[GUI]` / `[UI]`
- `[SCRIPT_...]` / 任意 Lua

未知 section 会进入报告，不会静默吞掉。

### 2.3 FX 模块

| 模块 | 当前状态 |
| --- | --- |
| Weather | 状态/参数（时间、雨、湿滑、风、雾） |
| Rain | 状态（雨丝/挡风/水花/积水开关） |
| Grass | 状态（密度、距离、风） |
| Lighting | 灯光预算/数量 |
| Particles | 粒子预算/开关 |
| Brake Disc | 刹车温度→发光强度 |
| Extra | Bloom/SSAO/SSR/DoF/Motion Blur/Sharpen 的质量门槛与启用状态 |

v1 的状态由 `FxManager` 维护；Godot 渲染桥接入后才会真正出画面。
详见 [`docs/fx.md`](fx.md)。

## 3. 用户：怎么检查一个 mod

```bash
cargo run -p vettura-render-inspect -- "mods/某车包"
cargo run -p vettura-render-inspect -- "mods/某车包" --json
```

报告示例：

```text
kind: car
kn5:   1/70 parsed
materials: 19/23 mapped (82.6%), 4 unknown
  unknown shaders:
    ksPerPixelMultiMap_NMDetail: 3
ext_config: 335 sections, 24/335 supported
  material overrides: 2, texture overrides: 0, fx patches: 0
fx: 8 enabled, 4 disabled (quality medium)
warnings: ...
```

看到未知 shader 时：

1. 先确认它是否影响主要视觉（车身/玻璃/路面）；
2. 在 `render/data/recipes.toml` 加配对；
3. 如果是复杂自定义 shader，提交 T2 PR 或在报告里标记为降级。

游戏内也可以通过 `AcVisualCompat` 调：

```gdscript
var compat := AcVisualCompat.new()
print(compat.inspect_text("res://mods/某车包"))
```

## 4. 作者：三条贡献路径

### 路径 A：只做皮肤/贴图/PP/天气

- 按 AC 习惯放 `skins/`、贴图、PP LUT、天气配置；
- 确保引用路径在 mod 目录内；
- 跑 `render-inspect` 检查覆盖率；
- 不需要改源码。

### 路径 B：做材质/视觉配置

- 用 `ext_config.ini` 的声明式 section，或提供 `visual.toml`；
- 不要依赖 `[SCRIPT_...]` / 任意 `.fx`；
- 参数必须能映射到已注册材质家族/FX 配置；
- 未知项会在报告里列出。

### 路径 C：做新 shader / 新 FX（源码）

1. 材质：在 `render/data/recipes.toml` 加 recipe；shader 实现放 `render/shaders/`（后续）并接到 `gdext` 材质创建；
2. FX：在 `render/src/fx/modules.rs` 加 `FxDefinition`，声明 scope/依赖/质量/成本；
3. 在 `gdext/src/fx` 实现渲染桥（材质/灯光/粒子/SubViewport）；
4. 加测试：启用、依赖、质量门槛、快照；
5. 更新 `docs/fx.md`。

## 5. 为什么不支持 `.fx`

`.fx` 是 HLSL/Effect 风格 shader，依赖 CSP 注入的：

- `PS_IN`（`Tex`/`PosH`）
- `g*` uniform（`gTint`/`gTexel`/`gColor`…）
- `tx*` 纹理槽
- `sam*` 采样器
- render state / technique / pass

跨平台没有成熟的 `.fx` Effects 运行时；运行任意 shader 也有安全/性能/许可问题。
所以：

- 简单 `.fx` 可以人工/离线对应到 `.gdshader`；
- 复杂 `.fx`（车模预览、后处理）用原生 SubViewport/WorldEnvironment 重新实现；
- 未知 `.fx` → 降级 + 报告，不执行。

## 6. 许可

- 不要提交别人的车模/贴图/声音/shader 到仓库；
- 只接受原创或宽松许可的贡献；
- CSP 是闭源商业软件，不能复制其代码/资源；
- GT7 HUD 等 Strict 许可模组不能分发衍生作品；
- 游戏运行时读取用户本地 `mods/`，不随包分发内容。
