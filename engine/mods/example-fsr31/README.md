# example-fsr31 — FSR 3.1 扩展模板

这是**模板**，不会被运行时加载（文件名是 `manifest.example.json`，且
`enabled: false`）。真正的 FSR3.1 需要：

1. 按 [`../../patches/README.md`](../../patches/README.md) 维护引擎补丁；
2. 在引擎里注册 `VetturaEngineMod` singleton，实现
   [`../../mods/README.md`](../../mods/README.md) 的四个方法；
3. 把本文件复制成 `manifest.json`，把 `enabled` 改成 `true`；
4. 重新跑 `godot --headless --path . res://game/display_smoke.tscn`，日志里
   应出现 `fsr3_1`；
5. 桌面窗口设置「画面 → 上采样 → AMD FSR 3.1」确认无报错。

如果补丁选择的是给 `Viewport.Scaling3DMode` 增加
`SCALING_3D_MODE_FSR3_1` 枚举，则**不需要**这个 manifest——枚举探测优先。
