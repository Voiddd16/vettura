# game/render — 渲染层

画面渲染的“全局配置 + 后处理 + 天空”。与 `game/fx/` 的分工：

- `game/render/`：所有场景共享的渲染基线（后端档位、天空、环境、屏幕后处理）；
- `game/fx/`：赛道/车辆级效果（雨、草、灯光、粒子、刹车盘、轮胎）。

## 文件

| 文件 | 作用 |
| --- | --- |
| `render_quality.gd` | autoload `RenderQuality`：按 Forward+/Mobile 打开或关闭 SSAO/SSIL/SSR/体积雾参数；推进默认环境的云层漂移 |
| `post_fx.gd` | autoload `RenderFX`：桌面完整后处理；移动端只挂廉价暗角 |
| `post_fx.gdshader` | 桌面：暗角、径向色散、胶片颗粒、轻锐化、可选桶形畸变 |
| `post_fx_vignette.gdshader` | 移动端：不读屏幕纹理的径向暗角 |
| `sky.gdshader` | 程序化天空：大气渐变、太阳/月亮、两层云、夜间星空 |
| `weather_rig.gd` / `weather_rig.tscn` | 天气状态机：wetness 驱动天空/太阳/雾/湿路面/ReflectionProbe/接触阴影，10 Hz 云漂移 |
| `wet_road.gdshader` | 湿路面：albedo/normal/AO + 统一 wetness 的粗糙度/反射 |
| `contact_shadow.gdshader` | 车底接触阴影（跟随车辆，随湿度加深） |

环境资源是工程根目录的 `game_environment.tres`（`project.godot` 的
`environment/defaults/default_environment`）。**屏幕空间特性的 enabled 开关
不要写进 `.tres`**，否则 Android 的 Mobile 后端会刷“仅 Forward+ 可用”警告；
统一由 `render_quality.gd` 在运行时处理。

## 调试

```bash
# 关闭后处理做 A/B
VETTURA_POST_FX=0 godot --path . res://gdext/fx_probe.tscn

# 强制移动端后端，确认没有桌面专属警告
godot --headless --rendering-method mobile --path . --quit-after 30 res://game/main.tscn
```

运行期切换后处理预设：

```gdscript
RenderFX.set_preset("cinematic")   # off / subtle / cinematic
RenderFX.set_parameters({"vignette_strength": 0.1})
```

## 边界

- 不把 CSP `.fx` / Lua 搬进运行时；
- 后处理层低于 UI，避免给 HUD 做暗角；
- 移动端用单独的无屏幕采样暗角 shader，带宽留给 MSAA/雨/草；
- 新增桌面效果必须按 `RenderingServer.get_current_rendering_method()` 分支。

详见 [`docs/rendering.md`](../../docs/rendering.md)。
