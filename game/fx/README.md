# game/fx — 真实视觉 FX 运行时

把 `vettura-render` 的 `ext_config` 绑定记录和 `WorldBody` 天气，接到真实
Godot 场景。它不是 CSP `.fx` 运行时，而是**原生 GDScript 效果近似实现**。

## 使用

把 `fx_runtime.tscn` 放到 `WorldBody` 同级（或任意场景根下）：

```gdscript
var fx := load("res://game/fx/fx_runtime.tscn").instantiate()
add_child(fx)
```

`FxRuntime` 会自动等待 `WorldBody` / `VehicleBody` 生成，然后：

- 扫描 track / car 的 `extension/ext_config.ini`；
- 读 `AcVisualCompat.inspect_json()` 的 `ext_config.records`；
- 按模块创建下面的 FX；天气从 `WorldBody.get_weather()` 每帧同步。

也可以手动：

```gdscript
fx.attach(world, [car], camera)
```

## 模块

| 文件 | 效果 | 数据来源 |
| --- | --- | --- |
| `rain_fx.gd` | 湿路面、雨丝、贴地积水、挡风玻璃雨滴、车轮水花 | `RAIN_FX` / 天气 |
| `grass_fx.gd` + `grass_blade.gdshader` | 按 GRASS_MATERIALS 三角形分块铺草 + 图集草卡片 + 风摆 | `GRASS_FX` + track mesh |
| `lighting_fx.gd` | `LIGHT_SERIES` / `EMISSIVE_LIGHT` / `LIGHT_POLLUTION` → Omni/SpotLight3D | `LIGHTING*` records |
| `particles_fx.gd` | `SMOKE_*` / `PARTICLES_*` → GPUParticles3D | particle records |
| `brake_disc_fx.gd` | 刹车盘温度 → emission | `BRAKE_DISC*` + `brake_temperature_c()` |
| `tyres_fx.gd` | `TYRES_FX_CUSTOMTEXTURE_*` → 轮胎贴图（含 zip） | tyre records |
| `extra_fx.gd` | Bloom/tonemap/adjustment，高画质 SSAO/SSIL | `EXTRA_FX*` |
| `weather_fx.gd` | 复用 `gdext/weather_visuals.gd` | `WorldBody` 天气 |

## 质量与开关

`FxRuntime` 导出：

- `enable_rain` / `grass` / `lighting` / `particles` / `brake_disc` / `tyres` /
  `extra` / `weather`
- `quality`：`off` / `low` / `medium` / `high` / `ultra`

湿材质默认最多 1400 个 surface；草 medium 6000（相机局部）、high 14000（局部高密度）、ultra 48000（全赛道底色 + 局部高密度）；灯光最多 48 盏。
这些是 v1 的移动端安全上限，后续按真实性能数据调。

## 验证

```bash
# 桌面上看雨/草/水花（会写 fx_probe.png）
/Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn

# 换赛道/车
FX_TRACK=res://mods/fn_nurburgring FX_LAYOUT=auto \
  /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn
```

```bash
FX_TRACK=res://mods/fn_nurburgring FX_LAYOUT=auto FX_QUALITY=high \
  /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/fx_probe.tscn
```

headless 也能跑通脚本与数据链路，但没有像素输出。

## 边界

- 不执行 CSP `.fx` / Lua；
- 不保证 1:1 CSP 视觉；这里是“同语义的原生近似”；
- 雨/湿路面/挡风玻璃雨滴是程序化近似，不是 CSP RainFX 的逐像素复刻；
- 车漆/轮胎贴图来自用户本地 mod，不能随游戏分发。
