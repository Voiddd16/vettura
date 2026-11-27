# engine/ — 魔改引擎落点

这里不是 Rust crate，也不参与 `cargo build`。它是 Godot 引擎补丁、显示能力
契约和扩展 manifest 的**唯一落点**，用于把「需要改引擎才能做的显示功能」
和游戏脚本/资源隔离开。

当前游戏脚本只通过 `game/render/display_pipeline.gd`（autoload
`DisplayPipeline`）与这里发生关系：

- **不依赖补丁**：FSR 1、FSR 2.2、MetalFX Spatial/Temporal、VRS、XR
  都是 Godot 4.7 的 Viewport 能力，直接可用；
- **需要补丁**：FSR 3.1（以及后续帧生成、更多 VRS 密度）通过本目录的补丁
  序列接入；运行时检测不到时自动回退到 FSR2/FSR1/MetalFX/原生。

## 目录

```
engine/
├── README.md                  # 本文件：契约与工作流
├── display_contract.json      # 机器可读的能力契约（补丁作者/工具共用）
├── patches/
│   ├── README.md              # 补丁命名、series、验收
│   └── 4.7/
│       ├── FSR31_DESIGN.md    # FSR3.1 要碰的引擎层与验收清单
│       └── series             # 按顺序应用的 patch 列表（当前为空）
└── mods/
    ├── README.md              # manifest 格式与桥接 singleton 契约
    └── example-fsr31/         # 模板：复制成 manifest.json 并启用
        ├── README.md
        └── manifest.example.json
```

## 契约 v1（运行时认什么）

### 1. 首选：Viewport 枚举

补丁给 `Viewport.Scaling3DMode` 增加：

```cpp
SCALING_3D_MODE_FSR3_1 = 6,
SCALING_3D_MODE_MAX = 7,
```

`DisplayPipeline` 通过 `ClassDB.class_get_integer_constant("Viewport",
"SCALING_3D_MODE_FSR3_1")` 探测；存在就把它当作普通 `scaling_3d_mode`
使用，不需要额外桥接。枚举值必须 ≥ 6 且不与现有值冲突。

### 2. 备选：manifest + 桥接 singleton

如果补丁不想动 `Viewport` 枚举（例如 FSR3.1 是独立后处理链），可以：

1. 引擎注册一个 singleton（C++ `Engine::get_singleton()->add_singleton(...)`），
   例如 `VetturaEngineMod`；
2. 在 `engine/mods/<id>/manifest.json`（或 `user://engine/mods/<id>/`）声明
   `provides: ["upscaler.fsr3_1"]` 和 `bridge`；
3. 运行时探测到 singleton 后才启用，否则不显示为可用。

详细方法见 [`mods/README.md`](mods/README.md)。

### 3. 信息性 feature tag

补丁可以额外把 `vettura_fsr3_1` 写进引擎的 version features，方便日志与外部
工具识别；但**运行时可用性仍以枚举或可调用的桥接为准**，不单凭 feature tag
就启用。

## 补丁工作流

1. 用与 `project.godot` 一致的 Godot 版本（当前 4.7.2）打底：
   `git clone --branch 4.7.2-stable https://github.com/godotengine/godot`；
2. 在引擎源码里实现 `engine/patches/4.7/<NNNN-name>.patch` 描述的内容；
3. 用 `git format-patch` 生成补丁，按 `patches/4.7/series` 顺序维护；
   检查/应用可用 `python3 scripts/engine_patch.py --source <godot> --check|--apply`；
4. 构建自定义编辑器/导出模板：
   `scons platform=macos target=editor arch=arm64`（按目标平台调整）；
5. 用自定义编辑器打开本工程，跑：
   ```bash
   godot --headless --path . res://game/display_smoke.tscn
   ```
   确认 `[display] upscalers=...` 里出现 `fsr3_1`，并在设置「画面」里选中
   后日志显示 `upscaler=fsr3_1`；
6. 在 `CHANGELOG.md` 记录“需要自定义引擎”的版本与补丁提交。

## 边界

- 不提交 Godot 源码、编辑器二进制或 FidelityFX SDK 二进制；
- 补丁必须可重复应用、带动机说明和最小验证步骤；
- 运行时永远保留 stock 引擎回退路径：没有补丁也能启动、能开、能跑；
- 不把引擎扩展逻辑写进 `game/`；脚本只认能力，不认具体补丁实现；
- 新增显示特性先加 `display_contract.json` 条目，再改 `DisplayPipeline`。
