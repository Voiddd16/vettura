# FSR 3.1 引擎补丁设计线索（4.7）

> 目标：先把「补丁要碰哪些层」写清楚，避免以后把 FSR3.1 逻辑散落到游戏脚本。
> 文件路径以 Godot 4.7 源码为准；升级引擎版本时先重新定位。

## 两条接入路线

### A. Viewport 枚举（推荐给纯上采样）

1. `scene/main/viewport.h` / `viewport.cpp`
   - `Scaling3DMode` 增加 `SCALING_3D_MODE_FSR3_1`，`MAX` 顺延；
   - `set_scaling_3d_mode()` 接受新值。
2. `servers/rendering_server.h` / `rendering_server.cpp`
   - `ViewportScaling3DMode` 枚举同步；
   - `viewport_set_scaling_3d_mode()` 透传。
3. `servers/rendering/renderer_scene_render.h` / `renderer_scene_render_rd.cpp`
   - `set_scaling_3d_mode()` 分支；
   - 在 `render_scene()` 的 upscale 阶段调用 FSR3.1。
4. `servers/rendering/renderer_rd/effects/` 
   - 新增 `fsr31.h/.cpp`，接口对齐现有 `fsr2.h/.cpp`：
     `ensure_context()` / `upscale()` / `release()`；
   - 输入：color、depth、motion vectors、jitter、exposure、reactive mask；
   - 输出：upscaled color。
5. `servers/rendering/renderer_rd/shaders/` 或 `thirdparty/amd-fidelityfx`
   - FSR3.1 HLSL/GLSL 资源与编译规则（按目标后端）。

好处：`DisplayPipeline` 不用改，现有 `ClassDB` 探测自动识别；设置页直接出现项。

### B. 独立后处理链 + singleton（推荐给帧生成/实验特性）

1. 引擎注册 `VetturaEngineMod`，实现
   `display_available/apply/clear/status`（见
   [`../../mods/README.md`](../../mods/README.md)）；
2. 在 `engine/mods/<id>/manifest.json` 声明 `provides`；
3. 运行时通过 `DisplayPipeline` 的 `scaling_3d_scale` 与 request 字典传递参数；
4. 补丁自己管理 FSR3.1 context 生命周期，切走时 `display_clear()` 释放。

适合 FSR3.1 帧生成：它需要在最终画面之后插帧，和 Viewport 的 3D upscale
不是同一层；先做纯上采样，再评估帧生成的 UI/延迟预算。

## 资源与构建

- FidelityFX SDK 不进仓库：用脚本按 commit/版本拉取到 `third_party/` 或构建缓存；
- 每个目标后端（Metal/Vulkan/D3D12）单独验证 shader 编译；
- 不支持的平台在 `DisplayPipeline._probe_capabilities()` 里直接标不可用，
  不要让用户选到一个会崩的模式。

## 验收清单

- [ ] stock 引擎启动无变化（补丁只增加分支）；
- [ ] 新枚举在 `display_smoke` 的 upscalers 列表里出现；
- [ ] 窗口/全屏、静态/动态场景切换无残影/闪黑；
- [ ] 动态分辨率改变 scale 时 FSR3.1 context 正确重建或复用；
- [ ] 切到其他 upscaler / 退出时资源释放，无 RID/显存泄漏；
- [ ] 与 VRS、MSAA、后处理（RenderFX）叠加时输出正确；
- [ ] 记录补丁顺序、Godot commit、验证机器和帧率数据到 CHANGELOG。
