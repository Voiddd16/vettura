# Mobile 3D 雨天系统（Godot 4.3+ / Mobile）

目录：

```text
game/fx/rain/
  rain_system.tscn       # 根节点 RainSystem（挂到 Camera3D 下）
  rain_system.gd         # 参数、风场、粒子、地面、雾、屏幕雨痕、可选音频
  rain_drop.gdshader     # 3D 雨丝：billboard + 速度方向 + 软 alpha + 最小屏幕宽度
  rain_splash.gdshader   # 可选水花（当前实现改用小 SphereMesh，shader 保留给 quad 变体）
  rain_ripple.gdshader   # MultiMesh 地面涟漪（不用 Decal）
  rain_screen.gdshader   # 可选全屏程序化雨痕（默认关闭）
```

> 依赖：Godot 4.3+（工程 4.7 验证），Mobile / Vulkan / Metal。  
> `RenderingServer.get_current_rendering_method()` 在 4.2+ 可用，返回
> `forward_plus` / `mobile` / `gl_compatibility`。

## 技术选型

| 部分 | 选择 | 理由 |
| --- | --- | --- |
| 雨滴 | **GPUParticles3D + 自定义 spatial shader** | 粒子模拟在 GPU；GDScript 不逐帧遍历粒子。Mobile 的 Vulkan/Metal 有 compute 通道，GPUParticles3D 可用。 |
| 雨丝形状 | QuadMesh + vertex billboard + 屏幕空间倾斜 | 细长 quad 比 3D BoxMesh 便宜；按全局合速度倾斜；软 alpha 抑制亚像素闪烁。 |
| 地面涟漪 | **MultiMesh quad + shader** | Mobile 每 Mesh 最多 8 个 Decal；MultiMesh 无 Decal 投影开销，同屏最多 6 个实例，CPU 只更新这 6 个。 |
| 水花 | GPUParticles3D + 小 SphereMesh | GPU 模拟；避免近处 quad 变白块。 |
| 远景 | WorldEnvironment depth fog + height fog | 禁用 volumetric fog / FogVolume；Mobile 的替代方案。 |
| 全屏雨痕 | CanvasLayer + 程序化 canvas shader | 不读屏幕纹理；默认关闭，低画质强制关闭。 |
| 抗锯齿 | MSAA 2x + FXAA | 不使用 TAA/FSR2；雨线靠软 alpha + 最小屏幕宽度 + 亮度抖动。 |

### 为什么不用 CPU 粒子

CPU 逐粒子会占用主线程，Mobile 上直接吃掉 16.7ms 预算的很大一块；而且
不能把雨滴数量、长度、风场实时调到我们想要的密度。这里只有 **<=6 个涟漪**
在 GDScript 里更新，雨滴/水花全在 GPU。

## 性能预算（示例）

目标：1080p、Mobile、60fps（16.7ms）/ 30fps（33.3ms）。

- medium：近层 1800、远层 550、水花 22、涟漪 4。
- high：近层 3600、远层 1300、水花 42、涟漪 6。
- 单根雨丝在 10m 处约 1.5–2px 宽、30–45px 长；平均覆盖约 60–100px²。
- medium 雨滴覆盖约 1800×80 ≈ 0.14M px + 远层 ≈ 0.04M px，
  加两层透明度叠加约 0.3–0.5M px；高画质控制在 ~1M px 以内。
- 目标 GPU 雨效：medium ≤1.5ms、high ≤3ms（以真机 GPU profiler 为准）。
- 不要把所有预算花在“少量巨大长条”上：更多更小、更细的雨滴在 tile GPU 上
  观感更好，overdraw 也更可控。

## 节点树

```text
RainSystem (Node3D, top_level=true)
├─ RainNear (GPUParticles3D, local_coords=false)
├─ RainFar (GPUParticles3D, local_coords=false)
├─ RainSplash (GPUParticles3D, local_coords=false)
├─ RainRipples (MultiMeshInstance3D, <=6)
├─ RainScreen (CanvasLayer → ColorRect, 默认隐藏)
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
4. 世界环境：脚本只使用 `fog_enabled` / `fog_density` / `fog_light_color` /
   `fog_height` / `fog_height_density`。不要打开 `volumetric_fog_enabled`。
5. Project Settings：
   - `renderer/rendering_method = mobile`
   - `rendering/anti_aliasing/quality/msaa_3d = 2x`（或按档位）
   - `rendering/anti_aliasing/quality/screen_space_aa = FXAA`
   - 不要启用 TAA / FSR2。
6. 地面材质：脚本会复制匹配 `extension/ext_config.ini [RAIN_FX]` 的
   `PUDDLES/SOAKING/SMOOTH/ROUGH/LINES_MATERIALS` 的 `StandardMaterial3D`，
   按 `ground_wetness` 调整 albedo/roughness；没有筛选条件时才作用于全部赛道
   材质（最多 `max_wet_materials` 个 surface）。

## Inspector 参数

| 分组 | 参数 | 默认 | 说明 |
| --- | --- | --- | --- |
| Rain | `rain_amount` | 0.6 | 0–1；联动粒子数量、水花、雾、湿度 |
| Rain | `wind_speed` / `wind_direction_deg` | 3 / 25 | m/s、度；决定雨丝倾角 |
| Rain | `fall_speed_mps` | 13 | 雨滴下落速度 |
| Rain | `turbulence` | 0.25 | 低频水平扰动 |
| Drop Shape | `drop_width` | 0.011 | 世界宽度（m） |
| Drop Shape | `drop_length` | 0.34 | 世界长度（m） |
| Drop Shape | `stretch_multiplier` | 1.15 | 速度拉伸倍率 |
| Drop Shape | `min_screen_width_px` | 1.25 | 最小屏幕宽度，抑制闪烁 |
| Drop Shape | `drop_brightness` / `drop_color` | 1 / 蓝白 | 亮度/颜色 |
| Drop Shape | `jitter_strength` | 0.18 | 每滴亮度抖动 |
| Ground | `ground_wetness` | 0.75 | 湿度；`auto_wetness_from_rain` 可自动跟随雨量 |
| Ground | `splash_strength` | 0.65 | 水花/涟漪生成率 |
| Ground | `ripple_size` / `max_ripples` | 0.45 / 4 | 涟漪尺寸/上限（硬上限 6） |
| Atmosphere | `fog_density` | 0.012 | 雾最大密度 |
| Atmosphere | `rain_visibility_distance` | 70 | 雨幕可见距离 |
| Quality | `quality` | MEDIUM | LOW/MEDIUM/HIGH |
| Quality | `mobile_low_end` | false | 低配再降数量/分辨率 |
| Optional | `screen_drops_enabled` | false | 屏幕雨痕；低画质强制关闭 |
| Optional | `enable_audio` / `rain_audio_db` | false / -24 | 程序化雨声，默认关闭 |

## 四组预设

### 小雨

```text
rain_amount=0.25, wind_speed=1.5, wind_direction_deg=20,
fall_speed_mps=12, drop_length=0.22, drop_width=0.008,
ground_wetness=0.35, splash_strength=0.3, fog_density=0.004,
rain_visibility_distance=45, quality=LOW
```

### 暴雨

```text
rain_amount=0.95, wind_speed=6, wind_direction_deg=35,
fall_speed_mps=16, drop_length=0.5, drop_width=0.014,
ground_wetness=0.95, splash_strength=0.9, fog_density=0.025,
rain_visibility_distance=65, quality=HIGH
```

### 斜风暴雨

```text
rain_amount=0.9, wind_speed=18, wind_direction_deg=75,
fall_speed_mps=14, drop_length=0.55, stretch_multiplier=1.3,
ground_wetness=0.95, splash_strength=0.8, fog_density=0.02,
rain_visibility_distance=60, quality=HIGH
```

### 细雨雾

```text
rain_amount=0.35, wind_speed=0.5, wind_direction_deg=0,
fall_speed_mps=10, drop_length=0.18, drop_width=0.007,
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
- 雨丝糊成白板：距离相机太近；shader 会在 <0.8m 淡出，别把发射盒缩到相机内部。
- 雾过头：`fog_density` 是最大值，`rain_amount` 会插值；不要同时开 volumetric fog。
- 移动时雨跟着飘：保持 `local_coords=false`，只更新根节点位置，不继承相机旋转。

## 已知取舍

- 没有体积雨幕、逐滴碰撞、SSR/SSAO/SSIL，也没有 TAA/FSR2；Mobile 下用 depth/height fog、roughness、颜色调整近似。
- 水花只在地面附近平面发射，不做逐滴射线检测；坡度大的赛道会有少量悬浮感。
- 涟漪是 MultiMesh quad，不是 Decal；没有真正的积水 mask 纹理。
- 屏幕雨痕是程序化叠加，默认关闭，避免 tile GPU 全屏带宽。
- 可选音频是低通白噪声，不是真实雨声采样；默认关闭，避免 CPU 音频填充成本。
