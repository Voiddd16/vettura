# stylized — 风格化渲染框架

画面风格增强子项目：**纯 shader（`.gdshader`）+ 少量 GDScript**，
不依赖 Rust/GDExtension 构建，按用途分成六类 + 公共工具。

> 背景：主渲染管线（`gdext/kn5`）全部使用内置 `StandardMaterial3D`，
> 无自定义 shader、无 WorldEnvironment——本子项目补的就是这块。

## 分类（按用途）

| 目录 | 用途 | 文件 |
| --- | --- | --- |
| `glass/` | 透明与玻璃 | `racing_glass`（车窗/风挡：透射+菲涅尔反射+边缘辉光）、`fence_glass`（护栏玻璃：网格线，近似 ksGlassFence） |
| `hud/` | 界面与仪表 | `hud_panel`（圆角面板+发光描边）、`gauge_ring`（270° 进度环仪表，超阈值变警告色）、`scanline`（扫描线/故障叠层） |
| `color/` | 颜色控制 | `paint_hue`（HSV 车漆换色，带清漆）、`livery_mask`（遮罩分区涂装换色） |
| `shape/` | 形状与位移 | `outline`（反壳轮廓描边）、`wind_sway`（植被/旗帜风摆，世界空间顶点位移） |
| `lighting/` | 质感光照 | `rim_light`（边缘光叠层）、`toon_ramp`（卡通分档光照）、`fake_reflection`（流光假反射车漆，无 cubemap） |
| `post/` | 全屏滤镜 | `vignette`、`color_grade`（调色）、`speed_lines`（速度线）、`heat_haze`（热浪扭曲）、`chromatic`（色差）+ `fx_layer.gd`（滤镜链挂载器） |
| `common/` | 公共工具 | `noise.gdshaderinc`（hash/value/fbm）、`color_utils.gdshaderinc`（HSV 互转）、`fresnel_utils.gdshaderinc`（菲涅尔） |

## 四种集成方式（按改动成本从小到大）

1. **`next_pass` 叠加**（不改原材质）：`outline`、`rim_light`——
   在现有 `StandardMaterial3D` 上设 `next_pass` 即可，KN5 管线材质不动；
2. **`material_override` 整件替换**：`racing_glass`、`paint_hue`、`livery_mask`、
   `toon_ramp`、`fake_reflection`、`wind_sway`——适合"这一件要换风格"；
3. **`Control.material`**（HUD）：`hud_panel`、`gauge_ring` 挂到 ColorRect/Panel；
4. **`FxLayer` 全屏滤镜链**（`post/fx_layer.gd`）：

   ```gdscript
   const FxLayer := preload("res://stylized/post/fx_layer.gd")
   var fx := FxLayer.new()
   fx.layer = 100
   add_child(fx)
   var rect := fx.push(preload("res://stylized/post/vignette.gdshader"), {"strength": 0.4})
   # 后推的滤镜采样前面滤镜的输出，链式生效；fx.pop() / fx.clear() 移除
   ```

## 约定

- uniform 用 **snake_case**；公共函数带 **`vfx_` 前缀**（防冲突）；
- 每个 shader 文件头注释写清**用途 + 关键 uniform + 推荐集成方式**；
- 透明类统一 `blend_mix`；不透明类 `depth_draw_opaque`；
- HUD 的 `hud_panel` 需要脚本把 `panel_size` 设成 Control 像素尺寸（否则圆角随长宽比变形）。

## 移动端约束（`mobile` 渲染器）

- 全部 shader 避免了 spatial 侧读屏幕纹理（screen 读取只出现在 canvas_item 滤镜里）；
- 滤镜叠层不宜同时超过 3 层（每层一次 backbuffer copy）；
- 未用 volumetric fog / SDFGI / SSAO 等 mobile 受限特性；
- CLEARCOAT 仅 Forward+/Mobile 支持，Compatibility 会静默忽略（可接受）。

## 验证

```bash
# 桌面窗口看效果（每个展台一类效果）
/Applications/Godot.app/Contents/MacOS/Godot --path . res://stylized/showcase.tscn --quit-after 600
# 冒烟：headless 验证资源/脚本/场景树（shader 编译错误需窗口模式才报）
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://stylized/showcase.tscn --quit-after 60
```

## 待接真资产的落点

- `livery_mask`：真车遮罩需按车身 UV 绘制（可由 `gdext` 管线作为 override 材质下发）；
- `wind_sway`：赛道植被（树/草）接入 `gdext` KN5 管线时按节点名/材质名匹配下发；
- 赛道湿滑/橡胶线 shader（配合 `world/` groove 数据做参数注入）——后续类别候选 `track/`。
