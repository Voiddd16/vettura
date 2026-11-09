# 变更记录

本文件记录 Vettura 的版本级变更。版本号遵循 SemVer：`0.x` 阶段允许在次版本间
调整内部结构，公共 API（GDExtension 暴露给 GDScript 的类与方法）以“能用”为准。

## 0.0.1 —— 2026-11-10

第一个可运行版本：Rust workspace + Godot 4.7 GDExtension 打通
“读 AC 数据 → 物理 → 显示”，`gdext/baseline.tscn` 能在真实赛道 + 真实车上跑通。

### 新增

- **按文件共享的加载缓存**（`gdext/src/kn5/cache.rs`）：同一路径的 `.kn5` 只解析
  一次，每张内嵌 DDS 只解码一次，每个 KN5 材质只创建一份 `StandardMaterial3D`；
  同一 `Kn5File` / `AcCar` 的重复实例化因此快约 4 倍。
- **分帧加载器 `Kn5SceneBuilder`**：`start()` → 多次 `step(预算, parent)` →
  `root()`，带 `progress()` 与 `remaining_tasks()`，把一次长加载切成多帧小块
  （`scripts/load_probe.py` 可量化对比）。
- **合成模型生成器** `kn5/src/synth.rs`（合法 DXT1 DDS + 截断 mip 链），bench、
  测试与 `kn5-inspect --synth` 共用一个实现，加载探针不再依赖本地十几 GB 的真车
  资产。
- **加载探针场景** `gdext/load_probe.tscn`：分别测量解析、冷实例化、缓存命中、
  分帧加载四段耗时。
- `NodeKind::Mesh/SkinnedMesh` 改为 `Arc<Mesh>` / `Arc<SkinnedMesh>`：几何可在
  解析结果、任务列表、多个实例之间零拷贝共享。

### 修复

- 预览页在本地缺少 `test/` 真车资产、或 GDExtension 尚未注册时不再刷一片
  ERROR，而是降级为警告并跳过模型加载（`--install-android-build-template` 等
  工程级命令不再被噪音淹没）。
- `test_visual/smoke.tscn` 无车可测时只验证相机循环，输出 `PASS（部分）`
  而不是 `FAIL`。
- 合成 DDS 头改用 `DDSD_LINEARSIZE`：Godot 会拒绝“未声明线性长度却给了非零
  pitch”的压缩贴图，此前合成贴图全部加载失败（探针的 `albedo=0` 由此而来）。

### 已知边界

- 冷路径首帧仍要解码全部贴图；把 DDS 解码搬进 Rust（BC1/BC3）以并行化/后台化，
  是下一个版本的首要候选。
- Android 导出链路尚未验证（见 README「环境要求」）。
