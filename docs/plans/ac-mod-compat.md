# AC 视觉模组兼容分支计划

分支：`feature/ac-mod-compat`

目标：**最大化 AC 视觉模组的开箱兼容**，坚持源码优先：

- 素材/参数可热加载；
- shader/渲染行为进源码；
- 不运行 `.fx`，不执行任意 Lua，不做运行时替换；
- 未知项进入兼容报告并降级。

## 已完成（P0）

- [x] 新 crate `vettura-render`（纯 Rust，不依赖 Godot）
- [x] 材质配方 `render/data/recipes.toml` + `MaterialRegistry`
- [x] `ext_config.ini` 解析：include、section 分类、材质/贴图/灯光/粒子/条件
- [x] 全部 FX section：Rain/Wind/Smoke/LightSeries/LightPollution/EmissiveLight/TyresFx/BrakeDiscFx/ExtraFx/GenericFx
- [x] `BindingPlan` + `FxConfigRecord`（保留每个 section 的独立 FX 参数）
- [x] 兼容报告
- [x] FX 框架：`FxDefinition` / `FxInstance` / `FxManager` / `FxProfile` / 预算
- [x] 内置 FX：Weather / Rain / Grass / Lighting / Particles / Brake Disc / Extra
- [x] CLI `render-inspect`（文本/JSON）
- [x] GDExtension 只读入口 `AcVisualCompat`
- [x] 单元测试 + 文档

## 下一步（P1：渲染桥）

- [ ] `gdext/src/fx`：把 `FxReport` 状态接到 Godot
- [ ] 材质：recipe → `StandardMaterial3D` / `ShaderMaterial`
- [ ] `ext_config` 覆盖实际应用到 KN5 材质/贴图
- [ ] `game/fx_lab.tscn`：开关/调参/调试视图
- [ ] 真实 mod 语料覆盖率与性能基准

## 后续（P2+）

- [ ] Grass 实例化与赛道侧区域生成
- [ ] Rain 雨丝/挡风/水花/积水
- [ ] Lighting 灯具/发光/阴影预算
- [ ] Extra FX 的 SSAO/SSR/Bloom/DoF/运动模糊
- [ ] Brake Disc 材质覆盖
- [ ] Weather 与 `weather_visuals.gd` 统一
- [ ] 动画/模型替换/驾驶员/轮胎 FX（按语料优先级）

## 验收

- `cargo fmt --all && cargo check --workspace && cargo test --workspace`
- `cargo run -p vettura-render-inspect -- "mods/F1 2008 by Cimmerian_Iter"`
- 现有画面零回归；
- 未知 shader/section 有报告；
- 移动端预算不退化。
