# 3D 雨天系统（Godot 4.3+ / Forward+ 与 Mobile）

目录：

```text
game/fx/rain/
  rain_system.tscn       # 根节点 RainSystem（挂到 Camera3D 下）
  rain_system.gd         # 参数、风场、粒子、地面、雾、屏幕雨痕、可选音频
  rain_drop.gdshader     # 3D 雨丝：billboard + 速度方向 + 软 alpha + 最小屏幕宽度
  rain_splash.gdshader   # 可选水花（当前实现改用小 SphereMesh，shader 保留给 quad 变体）
  rain_ripple.gdshader   # MultiMesh 地面涟漪（不用 Decal）
  rain_screen.gdshader   # 可选全屏程序化雨痕（默认关闭）
  rain_mist.gdshader     # 贴地薄雾/溅射水雾带
  rain_refraction.gdshader # 可选镜头折射（BackBufferCopy，默认关闭）
```

> 参考：`mods/imitate/acc-shaders/custom/rain/accRainDrops_*`（深度软相交、
> 法线/Fresnel 环境反射）与 `acc-weatherfx-base`（Garg–Nayar 光度模型）。
> `RenderingServer.get_current_rendering_method()` 在 4.2+ 可用，返回
> `forward_plus` / `mobile` / `gl_compatibility`。

## 技术选型

| 部分 | 选择 | 理由 |
| --- | --- | --- |
| 雨滴 | **GPUParticles3D + 自定义 spatial shader** | 粒子模拟在 GPU；GDScript 不逐帧遍历粒子。Mobile 的 Vulkan/Metal 有 compute 通道，GPUParticles3D 可用。 |
| 雨丝形状 | QuadMesh + vertex billboard + 屏幕空间倾斜 | 细长 quad 比 3D BoxMesh 便宜；按全局合速度倾斜；软 alpha + 亮头/拖尾抑制“白棍感”。 |
| 雨丝颜色 | 背景亮度 + 边缘 Fresnel 环境色 | 参考 `accRainDrops_ps.fx`：边缘反射天空、中心偏折射，不再整条纯白。 |
| 雨丝相交 | `hint_depth_texture` 深度软相交 | 靠近车/路面时淡出，避免硬切进几何体。 |
| 地面涟漪 | **MultiMesh quad + shader** | Mobile 每 Mesh 最多 8 个 Decal；MultiMesh 无 Decal 投影开销，同屏最多 6 个实例，CPU 只更新这 6 个。 |
| 水花 | GPUParticles3D + 小 SphereMesh | GPU 模拟；避免近处 quad 变白块。 |
| 远景 | WorldEnvironment depth fog + height fog；Forward+ 加 volumetric fog | Mobile 不付体积雾开销；桌面雨幕有体积层次。 |
| 全屏雨痕 | CanvasLayer + 程序化 canvas shader | 不读屏幕纹理；默认关闭，低画质强制关闭。 |
| 抗锯齿 | MSAA 2x + FXAA | 不使用 TAA/FSR2；雨线靠软 alpha + 最小屏幕宽度 + 亮度抖动。 |

### 为什么不用 CPU 粒子

CPU 逐粒子会占用主线程，Mobile 上直接吃掉 16.7ms 预算的很大一块；而且
不能把雨滴数量、长度、风场实时调到我们想要的密度。这里只有 **<=6 个涟漪**
在 GDScript 里更新，雨滴/水花全在 GPU。

## 性能预算（示例）

目标：1080p、Mobile、60fps（16.7ms）/ 30fps（33.3ms）。

- medium：桌面总雨滴 10000（Large/Medium/Small/Fine 按粒径谱分配），水花约 12，涟漪 6。
- high：桌面总雨滴 16000，水花约 24，涟漪 6；移动端约 0.5 倍预算（medium 5000 / high 8000）。
- 单根雨丝在 10m 处约 1.2–2px 宽、20–50px 长；平均覆盖约 50–100px²。
- 目标 GPU 雨效：medium ≤1.5ms、high ≤3ms（真机 profiler 为准）。
- 更多更小、更细的雨滴在 tile GPU 上观感更好，overdraw 也更可控。

## 节点树

```text
RainSystem (Node3D, top_level=true)
├─ RainLarge / RainMedium / RainSmall / RainFine (GPUParticles3D, local_coords=false)
├─ RainSplash (GPUParticles3D, local_coords=false)
├─ RainMist (GPUParticles3D, local_coords=false)
├─ RainRipples (MultiMeshInstance3D, <=6)
├─ RainScreen (CanvasLayer → ColorRect, 默认隐藏)
├─ RainRefraction (BackBufferCopy + ColorRect, 默认关闭)
└─ RainEnvironment (可选：只复用已有 WorldEnvironment；没有才创建)
```

运行时脚本还会查找 `WorldBody` / `VehicleBody` 用于地面高度与湿材质扫描。

## 挂载与接入

1. 把 `rain_system.tscn` 实例化到 **Camera3D 下**：

```gdscript
var rain := load("res://game/fx/rain/rain_system.tscn").instantiate()
camera.add_child(rain)
```

2. 如果不想挂相机下，也可以放在场景根；脚本会自动查找当前 Camera3D。
3. 与现有 `FxRuntime` 已自动接入：`game/fx/fx_runtime.gd` 会创建它并调用
   `setup(world, cars, records, camera)`。
4. 世界环境：脚本使用 `fog_enabled` / `fog_density` / `fog_light_color` /
   `fog_height` / `fog_height_density`；Forward+ 下额外按雨量开
   `volumetric_fog_*` 与 `ssr_enabled`，Mobile 下显式关闭。
5. Project Settings：
   - 桌面 `renderer/rendering_method = forward_plus`，
     移动端 `rendering_method.mobile = mobile`
   - `rendering/anti_aliasing/quality/msaa_3d = 2x`（或按档位）
   - `rendering/anti_aliasing/quality/screen_space_aa = FXAA`
   - 不要启用 TAA / FSR2。
6. 地面材质：脚本会复制匹配 `extension/ext_config.ini [RAIN_FX]` 的
   `PUDDLES/SOAKING/SMOOTH/ROUGH/LINES_MATERIALS` 的 `StandardMaterial3D`，
   按 `ground_wetness` 调整 albedo/roughness；没有筛选条件时才作用于全部赛道
   材质（最多 `max_wet_materials` 个 surface）。Forward+ 下把粗糙度降到
   0.24 左右、恢复一部分 `metallic_specular`（0.10）并压低金属度，配合 SSR
   让湿沥青接住天空/护栏反射；移动端没有 SSR，改为 0.22~0.24 粗糙度 +
   0.20 介电高光，靠天空 cubemap 出高光。金属度封顶是为了避免低角度太阳
   反射成白斑。

## Inspector 参数

| 分组 | 参数 | 默认 | 说明 |
| --- | --- | --- | --- |
| Rain | `rain_amount` | 0.6 | 0–1；联动粒子数量、水花、雾、湿度 |
| Rain | `fall_speed_mps` | 13 | 整体下落速度；13 = 粒径谱的基准速度，其余按比例缩放 |
| Rain | `wind_speed` / `wind_direction_deg` | 3 / 25 | m/s、度；决定雨丝倾角 |
| Rain | `turbulence` | 0.25 | 低频水平扰动 |
| Drop Shape | `drop_width` | 0.011 | 世界宽度（m） |
| Drop Shape | `drop_length` | 0.34 | 拖影长度倍率（0.34 = 物理 `\|v\|·T` 的 1.0×） |
| Drop Shape | `stretch_multiplier` | 1.15 | 额外长度倍率（1.15 = 1.0×） |
| Drop Shape | `min_screen_width_px` | 1.25 | 最小屏幕宽度，抑制闪烁 |
| Drop Shape | `drop_brightness` / `drop_color` | 1 / 灰蓝 | 亮度/颜色（默认不再用高饱和蓝白） |
| Drop Shape | `jitter_strength` | 0.18 | 每滴亮度抖动，幅度 ±`jitter_strength/2` |
| Ground | `ground_wetness` | 0.75 | 湿度；`auto_wetness_from_rain` 可自动跟随雨量 |
| Ground | `splash_strength` | 0.65 | 水花/涟漪生成率 |
| Ground | `ripple_size` / `max_ripples` | 0.45 / 6 | 涟漪尺寸/上限（硬上限 6） |
| Atmosphere | `fog_density` | 0.012 | 雾最大密度 |
| Atmosphere | `rain_visibility_distance` | 70 | 雨幕可见距离 |
| Optics | `shutter_time` | 1/30 | 曝光积分窗口，范围 1/500–1/15；1/30 的拖影更接近雨丝 |
| Optics | `exposure_gain` | 7 | τ/T 亮度增益；旧版 8 + 硬边 + 高亮让雨丝像白棍 |
| Optics | `drop_size_scale` | 1 | 粒径谱整体缩放 |
| Optics | `streak_softness` | 0.45 | 横向软剖面/近景虚化 |
| Optics | `internal_pattern_strength` | 0.40 | 纵向亮度扰动（单个弱高光） |
| Optics | `background_response` | 0.85 | Garg–Nayar 的 β 近似 |
| Optics | `backlit_boost` | 1.0 | 逆光边缘增强 |
| Optics | `dof_enabled` | false | 可选 CameraAttributes DOF，Mobile 默认关 |
| Gust | `gust_strength/period` | 0.25 / 3 | 阵风包络 |
| Gust | `shaft_scale/contrast` | 0.08 / 0.35 | 雨幕条带 |
| Optional | `screen_refraction_enabled` | false | BackBufferCopy 镜头折射，HIGH 才允许 |
| Quality | `quality` | MEDIUM | LOW/MEDIUM/HIGH |
| Quality | `mobile_low_end` | false | 低配再降数量/分辨率 |
| Optional | `screen_drops_enabled` | false | 屏幕雨痕；低画质强制关闭 |
| Optional | `enable_audio` / `rain_audio_db` | false / -24 | 程序化雨声，默认关闭 |

## 四组预设

### 小雨

```text
rain_amount=0.25, wind_speed=1.5, wind_direction_deg=20,
fall_speed_mps=12, drop_length=0.22, drop_width=0.008,
shutter_time=1.0/60.0, exposure_gain=7.0, background_response=0.85,
ground_wetness=0.35, splash_strength=0.3, fog_density=0.004,
rain_visibility_distance=45, quality=LOW
```

### 暴雨

```text
rain_amount=0.95, wind_speed=6, wind_direction_deg=35,
fall_speed_mps=16, drop_length=0.5, drop_width=0.014,
shutter_time=1.0/50.0, exposure_gain=8.0, background_response=0.85,
ground_wetness=0.95, splash_strength=0.9, fog_density=0.025,
rain_visibility_distance=65, quality=HIGH
```

### 斜风暴雨

```text
rain_amount=0.9, wind_speed=18, wind_direction_deg=75,
fall_speed_mps=14, drop_length=0.55, stretch_multiplier=1.3,
shutter_time=1.0/45.0, exposure_gain=7.5, gust_strength=0.55,
ground_wetness=0.95, splash_strength=0.8, fog_density=0.02,
rain_visibility_distance=60, quality=HIGH
```

### 细雨雾

```text
rain_amount=0.35, wind_speed=0.5, wind_direction_deg=0,
fall_speed_mps=10, drop_length=0.18, drop_width=0.007,
shutter_time=1.0/60.0, exposure_gain=6.5, background_response=0.9,
ground_wetness=0.45, splash_strength=0.2, fog_density=0.04,
rain_visibility_distance=35, quality=MEDIUM
```

## 验证清单

- [ ] `RenderingServer.get_current_rendering_method()` 在导出真机上打印 `mobile`，不是 `gl_compatibility`。
- [ ] 切到 `gl_compatibility`：必须出现 warning，并看到屏幕雨痕降级；GPUParticles 不依赖。
- [ ] 相机快速跑动/转头：雨丝不粘屏，看不到发射盒边界。
- [ ] 风为 0：雨丝垂直；风大：整体一致倾斜，不是各自乱飘。
- [ ] 近处雨滴能分辨单滴，远处叠成半透明雨幕，没有巨大白条/方块。
- [ ] 地面有涟漪/水花，湿材质变暗、roughness 下降，但不过曝。
- [ ] 关闭 `screen_drops_enabled`、LOW 画质时无屏幕雨痕。
- [ ] 同屏 Decal 数量为 0（本实现用 MultiMesh，不触发 8 个上限）。
- [ ] Android/iOS Mobile 导出预设真机测试；编辑器 Forward+ 不能替代。
- [ ] 用 GPU profiler 看雨效 GPU ms；60fps 目标 ≤3ms，30fps ≤6ms。
- [ ] `Engine.get_frames_per_second()` 只作粗测；正式用 Android GPU Inspector / Xcode Metal System Trace。

常见坑：

- 改了 `rain_amount` 但地湿没变：检查 `auto_wetness_from_rain` 或 `set_weather`。
- 粒子为零：检查渲染方法是否 `gl_compatibility`。
- 雨丝糊成白板：距离相机太近；shader 会在约 2.0–5.0m 处淡出，并把长宽比限制在 2:1 以上，别把发射盒缩到相机内部。
- 雾过头：`fog_density` 是最大值，`rain_amount` 会插值；不要同时开 volumetric fog。
- 移动时雨跟着飘：保持 `local_coords=false`，只更新根节点位置，不继承相机旋转。

## 光度/物理模型

- 雨滴按接近球形小透镜处理；折射为主，镜面/内反射作为高光项。Mobile 不做逐滴屏幕采样。
- streak 亮度使用 Garg & Nayar 近似：
  `I_d = τ·Ē_d + (T−τ)·E_bg`，`ΔI = τ(Ē_d−E_bg)`；背景亮度用屏幕高度 + sky/ground uniform 近似，亮天空前雨会淡出。
- 粒径谱分 Large/Medium/Small/Fine 四档，终端速度使用
  `v(D) ≈ 9.65 − 10.3·exp(−0.6·D_mm)` 量级（Marshall–Palmer/Atlas 类经验关系），不是所有雨滴同速。
- 雨丝长度由 `|v|·T` 决定，T = max(shutter_time, frame_dt)；低帧率拖影自然变长、亮度按 τ/T 摊薄。
- 阵风 = 低频包络 × 空间相干噪声；shader 内再乘雨幕条带密度，避免均匀白线。

## 反向“卡通感”自检

1. 暗背景（湿柏油/暗植被）前雨清楚；亮天空前雨明显变淡甚至消失。若天空前仍一片白线，检查 `background_response` 与 `sky_brightness`。
2. 放大单条拖影：两端渐隐、内部有散斑/1–3 个拖尾高光；不是均匀矩形。
3. rain_amount 从 0.2 拉到 0.95：大滴占比、速度方差、雨幕条带、雾、水花都应变化，不只是数量变多。
4. 近景雨滴更大更虚，中景最清晰，远景融进雾；转头时三层不跳变。
5. 关掉场景灯：雨不应继续发白；打开路灯/车灯：只有局部雨被照亮并带色（最多 3 个局部光源 uniform）。
6. 60fps → 30fps：拖影变长、亮度按 τ/T 变淡，而不是断续跳点。
7. 开 `screen_refraction_enabled` 前后对比 Android/iOS GPU ms；关闭时不应有 Viewport copy 成本。
8. 截图去饱和后：应是低对比灰蓝画面里的雨，不是高对比白线压在彩色场景上。

## 已知取舍

- 没有体积雨幕、逐滴碰撞、SSR/SSAO/SSIL，也没有 TAA/FSR2；Mobile 下用 depth/height fog、roughness、颜色调整近似。
- 屏幕折射是共享 BackBufferCopy 层，不是逐滴真实折射；默认关闭，HIGH 才允许；开启时单列一次 Viewport copy 成本。
- 水花只在地面附近平面发射，不做逐滴射线检测；坡度大的赛道会有少量悬浮感。
- 涟漪是 MultiMesh quad，不是 Decal；没有真正的积水 mask 纹理。
- 屏幕雨痕是程序化叠加，默认关闭，避免 tile GPU 全屏带宽。
- 可选音频是低通白噪声，不是真实雨声采样；默认关闭，避免 CPU 音频填充成本。
