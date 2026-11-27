# 渲染层

本文描述 Vettura 的画面渲染层：渲染后端怎么选、天空/环境/后处理怎么叠、
AC 材质如何映射，以及哪些效果只在桌面 Forward+ 可用。

> 参考素材：`mods/imitate/acc-weatherfx-base`（CSP 天气/后处理 shader）与
> `mods/imitate/acc-shaders`（AC 原生 shader 参数语义）。Vettura 不运行
> CSP `.fx` / Lua，也不做运行时 shader 注入；这里是**同语义的原生近似**。

## 1. 后端：桌面 Forward+，移动端 Mobile

`project.godot`：

```ini
[rendering]
renderer/rendering_method="forward_plus"
renderer/rendering_method.mobile="mobile"
```

- **桌面（macOS / Windows / Linux）**：Forward+，完整屏幕空间效果；
- **Android / iOS**：Mobile，不把 SSAO/SSIL/SSR/体积雾带进 APK；
- `config/features` 标记为 `Forward Plus`；移动端由 `.mobile` 覆盖，导出脚本
  不需要改。

`game/render/render_quality.gd`（autoload `RenderQuality`）在启动时按后端给
`game_environment.tres` 的共享实例打开/关闭屏幕空间特性。**不要**把
`ssao_enabled` / `ssr_enabled` / `volumetric_fog_enabled` 直接写进 `.tres`：
Mobile 后端会对每个属性打印“仅 Forward+ 可用”的警告。

## 2. 天空

`game/render/sky.gdshader` 是程序化天空着色器：

- 大气渐变（天顶/地平线/地面）+ 太阳侧暖色地平线；
- 太阳盘 + 光晕（`LIGHT0_*`），夜间月亮；
- 云层从预生成的 `NoiseTexture2D`（FastNoiseLite fBm，256 seamless）
  采样，不再在每像素里跑 5 层 value noise；
- 桌面 4 次纹理采样（主云 + 细节 + 朝太阳采样 + 高层卷云），移动端只采 1 次；
- 夜间星空（三层哈希点阵）+ 低权重银河噪声；
- `WeatherRig`（`game/render/weather_rig.gd`）按 10 Hz 推进 `cloud_offset`，
  云连续缓慢移动（约 0.012/s，一个噪声周期约 5 分钟；以前 0.35/s 会让云
  几分钟跑完几十个周期，看起来像跟着车飞）；没有天气系统的
  大厅/车库由 `RenderQuality` 推进默认环境同一参数。10 Hz 的步进在噪声
  纹理上不到 1 像素，肉眼无差别，但天空 uniform 不会每帧触发 radiance
  cubemap 重算；
- 天空的 `process_mode = INCREMENTAL` 把辐射 cubemap 更新分摊到多帧，配合
  上面的 10 Hz 写入，辐射图有时间收敛，不再出现“每 0.5 s 跳一次”的卡顿。

天空资源在 `game_environment.tres` 里以 `ShaderMaterial` 形式挂到 `Sky`，
所有场景共享；`WeatherRig`（主游戏/大厅）duplicate 后按 wetness 改 uniform；
`WeatherVisuals`（ai_lap 探针的旧路径）仍保留同一套 uniform 接口。

## 3. 环境与后处理

`game_environment.tres` 的基线：

| 项 | 值 | 说明 |
| --- | --- | --- |
| tonemap | ACES（基线）/ AgX（WeatherRig 场景） | `game_environment.tres` 是 ACES；WeatherRig 在运行时切到 AgX 并插值白点/对比度 |
| exposure | 1.15 | ACES 下略提亮，避免画面发灰 |
| glow | screen blend / threshold 1.5 | 车灯、太阳盘、湿路高光 |
| adjustment | contrast 1.04 / saturation 1.0 | 很轻的色彩校正 |
| ambient | COLOR 2.0 | 4.7 的 `AMBIENT_SOURCE_SKY` 实测不亮，只能显式颜色 |
| fog | 默认关，天气开 | depth fog + height fog + aerial perspective |
| SSAO/SSIL/SSR | 桌面开 | 由 `RenderQuality` 打开 |

`game/render/post_fx.gd`（autoload `RenderFX`）在 3D 之后、HUD 之前挂
`CanvasLayer(-100)`：

- 桌面：`post_fx.gdshader`，暗角（0.22）/ 径向色散（0.0006）/ 胶片颗粒
  （0.012）/ 轻锐化（0.14）/ 可选桶形畸变；
- 移动端：`post_fx_vignette.gdshader`，只做一层**不读屏幕纹理**的径向暗角，
  避免 tile GPU 的全屏 framebuffer 读写。

因为层号低于 UI，按钮/文字不会被暗角或色散影响。可用
`VETTURA_POST_FX=0` 临时关闭做对比。预设：`subtle`（默认）/ `cinematic` /
`off`，通过 `RenderFX.set_preset()` 切换。

### 3.1 移动端档位

| 项 | 桌面 | 移动端 |
| --- | --- | --- |
| 后处理 | 完整 ppfilter | 仅暗角（无 screen texture） |
| SSAO/SSIL/SSR/体积雾 | 开 | 关 |
| 天空云层 | 4 次噪声纹理采样 / 256 cubemap | 1 次采样 / 128 cubemap |
| 阴影 | 4 级 CSM / 120 m / 0.5° 软阴影 | 2 级 CSM / 64 m / 0.35° |
| MSAA | 窗口 4x + debanding；全屏自动 2x | 关（雨系统按档位可开 2x）+ FXAA |
| 草 | medium 6000 / high 14000，半径 78 m | medium 3200 / high 7200，半径 58 m，更早距离淡出 |
| 湿路面 | SSR + 0.30 粗糙度 + 0.10 介电高光 | 无 SSR；0.22~0.24 粗糙度 + 0.20 介电高光，靠天空 cubemap 反射 |

### 3.2 全屏动态分辨率（桌面）

`canvas_items` 拉伸在 Retina/全屏下会把根视口放大到窗口的若干倍；直接按原生
全屏渲染会让 M1 这类 GPU 掉到 15 fps。`DisplayPipeline` 因此对**全屏**做 3D
动态分辨率：

- 3D 内部目标约为屏幕宽度的 90%（上限 2880），再按帧率在 35%~100% 之间
  缓慢调整；UI 与后处理仍在原生分辨率，不是整屏 720p；
- 上采样器可选手动/自动，默认 auto 在 macOS 用 **MetalFX Spatial**、
  其他 Forward+ 用 **FSR1**；设置里可显式切 FSR2、MetalFX Temporal，
  或（引擎补丁后）FSR3.1；
- 全屏 MSAA 固定 2x（窗口仍是 4x）；
- 全屏后处理切到性能模式：保留暗角/锐化，去掉几乎看不见的径向色散与桶形
  畸变，颗粒改为细粒度；
- 设置 → 画面可关闭「全屏自动分辨率」，改用「3D 分辨率」滑杆固定比例；
- 窗口尺寸被全屏切换写坏时（接近整屏/非法值）启动会自动回到 1280×720，
  避免下一次以超大窗口渲染导致帧率暴跌。

### 3.3 显示管线：上采样 / 动态分辨率 / VRS / XR

autoload `DisplayPipeline`（`game/render/display_pipeline.gd`）是所有这些
Viewport 显示状态的唯一入口。`RenderQuality` 只负责环境/天空/后处理档位，
不再直接写 `scaling_3d_mode` / `scaling_3d_scale` / `vrs_*`。

| 上采样器 | 可用条件 | Viewport 模式 |
| --- | --- | --- |
| `native` | 所有平台 | BILINEAR |
| `fsr1` | 非 headless、非 Compatibility | FSR |
| `fsr2` | 桌面 Forward+ | FSR2（时域） |
| `metalfx_spatial` | macOS/iOS + Metal + `RenderingDevice.SUPPORTS_METALFX_SPATIAL` | METALFX_SPATIAL |
| `metalfx_temporal` | macOS/iOS + Metal + `SUPPORTS_METALFX_TEMPORAL` | METALFX_TEMPORAL（时域） |
| `fsr3_1` | **魔改引擎**：Viewport 增加 `SCALING_3D_MODE_FSR3_1`，或 manifest 桥接 | 补丁枚举 / 引擎桥接 |

`auto` 策略（保守、优先稳定帧率）：

1. 有引擎补丁的 FSR3.1 就用 FSR3.1；
2. macOS/iOS 用 MetalFX Spatial（时域选项留给用户显式开）；
3. 其他 Forward+ 用 FSR1；
4. 移动端和兜底用原生。

显式选择不可用的上采样器不会报错，运行时按 auto 回退并在日志里说明一次。
FSR 3.1 的补丁/扩展契约见 [`engine/README.md`](../engine/README.md)。

**MetalFX/FSR 细节补偿**（参考 Apple WWDC22 10103 示例
`mods/imitate/ApplyingTemporalAntialiasingAndUpscalingUsingMetalFX`）：

- **纹理 LOD bias**：低分辨率输入如果仍按原分辨率 mip 采样会发糊。`DisplayPipeline`
  每帧按当前比例写 `Viewport.texture_mipmap_bias`：
  - Temporal（MetalFX Temporal / FSR2 / FSR3.1）：`log2(scale) - 1`；
  - Spatial（MetalFX Spatial / FSR1）：`log2(scale)`；
  - 0.67 比例 temporal ≈ **-1.58**（Apple 推荐公式），1.0 时 ≈ -1.0。
- **各向异性过滤 8x**：路面标线、护栏、树林在低角度下保持细节；
- **Temporal 轻锐化**：`RenderFX` 检测到时域上采样时把 unsharp 下限抬到 0.22，
  补偿 MetalFX 的柔化；Spatial/原生仍用预设值；
- **默认质量比例 0.8**：`AI_LAP_UPSCALER_SCALE` / `SHOT_SCALE` 默认 0.8；0.67
  留给性能档（1080p 输入 1296×729，0.8 为 1536×864）；
- **MSAA**：MetalFX Temporal/FSR2 与 3D MSAA 不兼容，引擎会关闭 MSAA；只有
  Spatial 才保留 2x/4x；
- **透明效果**：玻璃/粒子不写 motion vector，MetalFX 的 reactive mask 在 Godot
  4.7 尚未暴露；座舱玻璃重影明显时优先切 Spatial，或降低时域比例/关闭折射。

**动态分辨率**：

- 每 0.5 s 采样一次 `Engine.get_frames_per_second()`；
- 低于目标约 10% 持续 1 s → 降 5%，冷却 1.5 s；
- 高于目标 1 FPS 持续 3 s → 升 5%，冷却 3 s；
- 目标 `dynamic_target_fps`：`0` = 自动（最高 60），`-1` = 跟随显示器刷新率，
  正数按 FPS；默认不追 120/144 Hz，避免赛车场景为高刷压分辨率；
- 时域上采样下限统一为 **0.67**（Apple MetalFX 示例把最小输入钳在 1280×720，
  1080p 即 0.67）；MetalFX Temporal 的驱动下限若更高则以驱动为准。0.5 的低分辨率
  历史缓冲即使把 mip bias 拉满也会明显发糊。

**VRS**：

- `auto`：XR 会话用 `VRS_XR`（由 `XRInterface` 提供每眼纹理）；桌面
  Forward+/Mobile 用 `VRS_TEXTURE`；否则关闭；
- `VRS_TEXTURE` 用 32×18 的中心清晰密度图：中心 1×1、中间 2×2、外围 4×4、
  上下边缘 2×4；`vrs_update_mode = ONCE`，静态纹理不产生每帧转换开销；
- 后端/会话不支持时设置页把选项标灰；硬件不支持时 Godot 会忽略该属性。

**XR**：`DisplayPipeline` 用 `XRServer.get_primary_interface()` 判断会话，
VRS 自动切 `VRS_XR`；动态分辨率仍作用于根 Viewport 的 3D 缩放。

**相关设置键**（`user://settings.cfg`，`video` 段）：

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `upscaler` | `auto` | `auto/native/fsr1/fsr2/fsr3_1/metalfx_spatial/metalfx_temporal` |
| `vrs` | `auto` | `auto/off/texture/xr` |
| `dynamic_target_fps` | `0` | `0` 自动（≤60）、`-1` 跟随刷新率、正数 FPS |
| `fullscreen_adaptive` | `true` | 是否启用全屏动态分辨率 |
| `render_scale` | `1.0` | 关闭动态分辨率时的固定 3D 比例（50%~100%） |

调试：

```gdscript
print(DisplayPipeline.report())       # state + capabilities + 引擎扩展
print(DisplayPipeline.state())        # 当前 upscaler/scale/vrs/target
DisplayPipeline.refresh()             # 重新扫描 engine/mods 并探测能力
```

## 4. 材质映射（KN5 → StandardMaterial3D）

`gdext/src/kn5/convert.rs` 是纯数据层，`gdext/src/kn5/material.rs` 写引擎属性。
与 AC 语义相关的关键点：

| AC | Godot | 备注 |
| --- | --- | --- |
| `txDiffuse` | ALBEDO | |
| `txNormal` / `txNormalDetail` | NORMAL / DETAIL_NORMAL | |
| `txDetail` | DETAIL_ALBEDO（MULTIPLY, `uv2_scale`） | 车漆涂装颜色的来源 |
| `txMaps` | AO 贴图 | AC 的 R 通道是 specular 强度，这里近似成遮蔽 |
| `ksSpecularEXP` | `roughness = sqrt(2/(exp+2))` | 路面类下限 0.6 |
| `isAdditive == 2` | clearcoat | 车漆单独清漆层 |
| `ksEmissive` + `txEmissive`/`txDiffuse` | EMISSION + emission texture | 车灯/屏幕/招牌保留贴图细节 |
| `ksWindscreen` / glass | ALPHA + 折射（仅 Forward+） | 反射类仍按金属/电介质区分 |
| `ksTree` / `ksGrass` / `ksFlags` | 双面渲染 | 单面卡片从背面看不再消失 |

`ksPerPixelReflection` / `ksMultilayer_fresnel_nm` 的高光在加载时被压掉
（没有 AC 的 envmap，低粗糙度会反射成纯白）。湿路面由 `WeatherRig` 在运行时
换成 `wet_road.gdshader`，按统一 wetness 部分恢复介电高光并压低金属度，
避免太阳低角度反射成过曝白斑。

## 5. 天气联动

主游戏/大厅用 `game/render/weather_rig.gd`（节点 `WeatherRig`，`WorldEnvironment`），
把 `vettura-track::Weather` 的 wetness/rain 合成**单一 `wetness` 标量**，再驱动：

- 天空 uniform：天色、云量、云色，外加 10 Hz 云漂移；
- 太阳：能量/色温/角直径/阴影距离/体积雾贡献，雨天压低高光；
- 环境：AgX tonemap、环境光、雾；Forward+ 才开 SSAO/SSIL/SSR/体积雾；
- 湿路面：把赛道 `StandardMaterial3D` 换成 `wet_road.gdshader`，按
  `RenderingServer.global_shader_parameter("wetness")` 统一驱动；
- ReflectionProbe：Mobile 用探针 cubemap 代替 SSR；桌面同时用 SSR；
- 车底接触阴影、灯杆/车灯自发光增强、RainSystem 的雨量/地面湿度。

`sync_from_world()` 由 `game/fx/weather_fx.gd` 以 5 Hz 调用；玩家手动
`set_weather()/debug_set_wetness()` 后 `manual_override` 生效，不再被物理天气覆盖。
8 秒 `move_toward` 渐变，不瞬切。

`gdext/weather_visuals.gd` 是旧版天气视觉（ai_lap / desktop_probe 仍在用），
接口相同但只做天空/太阳/雾/轻量雨丝；`WeatherFX` 会优先复用场景里任何带
`sync_from_world` 的节点（即 `WeatherRig`），找不到才创建这个旧节点。

RainSystem 额外把湿路面材料改成更低粗糙度、恢复一部分 `metallic_specular`，
并在 Forward+ 下打开 SSR/体积雾；Mobile 仍用旧的低成本近似（见
`game/fx/rain/README.md`）。

雨丝本身参考 `acc-weatherfx-base` 的 Garg–Nayar 雨滴光度模型，并做了两轮
“去卡通化”：横向用二次方软剖面代替满 alpha 的矩形；纵向改成亮头 + 渐隐
拖尾；每滴按实例 scale 和世界位置散列改变长宽/亮度；默认 `shutter_time`
1/30 让拖影更接近真实雨丝；细/小雨滴占比提高、颜色从高饱和蓝白改为灰蓝。
参考 `acc-shaders/custom/rain/accRainDrops_ps.fx` 又补了深度软相交
（靠近车/路面时淡出）和边缘 Fresnel 环境反射色。

GrassFX 参考 `acc-shaders/recreated/ksGrass_ps.fx` 与 `smoothA2C.hlsl`：
草卡片用 `alpha_to_coverage` 代替硬 `discard`；世界空间低频色斑替代
`txVariation` 贴图；从 `WorldBody` 湿度同步 `wetness`，湿草变暗、降低粗糙度
并接住天空反射（`RAINFX_WET` / `RAINFX_REFLECTIVE` 的近似）。

## 6. 验证

```bash
# Rust 侧材质映射单测
cargo test -p vettura-godot

# 桌面窗口看真实效果（会写 user://fx_probe.png）
/Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn

# 晴/雨对比
FX_RAIN=0 FX_WET=0 FX_SUN=1 FX_FOG=0 \
  /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn
FX_RAIN=0.85 FX_WET=0.9 FX_SUN=0.6 \
  /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn

# 车模预览 + 赛道追逐（会写 /tmp/.../vettura_shots/*.png）
/Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/desktop_probe.tscn

# 移动端后端冒烟：确认不打印 Forward+ 专属警告
/Applications/Godot.app/Contents/MacOS/Godot --headless --rendering-method mobile \
  --path . --quit-after 30 res://game/main.tscn

# 显示管线冒烟：能力探测 / 上采样回退 / 动态分辨率范围 / VRS 状态
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/display_smoke.tscn
```

## 7. 边界

- 不执行 CSP `.fx` / Lua，不做运行时 shader 注入；
- 云/星空/湿路面是程序化近似，不是 CSP WeatherFX 的逐像素复刻；
- 材质贴图来自用户本地 mod，不随游戏分发；
- 新增桌面效果必须先判断 `RenderingServer.get_current_rendering_method()`，
  或在 `RenderQuality` 里按后端开关，不能假设一定跑 Forward+；
- FSR3.1/帧生成属于引擎级能力，补丁与契约只放 `engine/`，不把引擎源码或
  native 二进制提交进仓库；运行时必须保留 stock 引擎回退路径。
