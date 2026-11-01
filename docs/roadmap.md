# 路线图

按“先能读、再能开、最后能像 AC”排序。

## Phase 0：地基（当前）

- [x] Rust workspace 与 crate 边界
- [x] 坐标系/单位约定
- [x] KN5 v5/v6 解析：贴图、材质、节点、静态/蒙皮网格
- [x] `data.acd` 解密
- [x] INI / LUT
- [x] AC 车目录与初版 `VehicleSpec` 映射
- [x] 物理后端接口 + 固定步长
- [x] SimpleBackend 占位
- [x] CLI：`kn5-inspect`、`acd-inspect`
- [x] Godot 扩展骨架（`Kn5File` / `AcCar` / `VehicleBody`）

## Phase 1：把车显示出来

- [x] KN5 节点树 → Godot `Node3D` 层级（dummy / mesh，重名自动加后缀）
- [x] Mesh → `ArrayMesh`（批量顶点/索引/切线；蒙皮网格带 bones/weights + Skeleton）
- [x] DDS → `Image`/`ImageTexture`（含 KN5 mip 链不完整的兜底）
- [x] 基础材质映射（diffuse/normal/specular/AO/detail/alpha）
- [x] AC 车目录 → 主模型 `.kn5` 选择（`lods.ini` → 最大文件）
- [x] 冒烟测试：`godot/kn5_probe.tscn` 用真实 mod 车实例化三台车
- [ ] `ResourceFormatLoader` 支持 `.kn5`
- [ ] 大文件加载缓存与进度反馈

验收：能把一台 F1 2008 mod 车完整显示在 Godot 场景中，车轮位置正确。
（已完成：`kn5_probe` 对 F1 2008 的三台车输出 125/136/129 个 `MeshInstance3D`、
全部 surface 有贴图、四个车轮 dummy 位置与 KN5 一致。）

## Phase 2：把车开起来（占位物理）

- [ ] `VehicleBody` 接入 `AcCar` 的 `VehicleSpec`
- [ ] 车轮节点按 dummy 名称自动绑定
- [ ] 键盘/手柄输入映射
- [ ] 简单地面 raycast + 车身姿态
- [ ] 相机跟随/车内视角
- [ ] 基础 HUD（速度、挡位、转速）

验收：能沿平面直行、转向、刹车，车轮转动和转向可见。

## Phase 3：AC 物理数据完整映射

- [ ] `power.lut` 完整扭矩曲线
- [ ] 涡轮/混动/能量回收
- [ ] 变速箱换挡时间、离合、自动补油
- [ ] 差速器（power/coast/preload）
- [ ] 轮胎温度/磨损/压力
- [ ] 悬架几何硬点、防倾杆、 bump stop
- [ ] 气动多翼面 + 高度/DRS 曲线
- [ ] 制动/ABS/TC

验收：同一辆车在相同输入下，关键状态（速度、转速、挡位）与 AC 接近。

## Phase 4：Chrono::Vehicle 后端

- [x] 固定 Chrono 版本与构建脚本（10.0.0，`third_party/chrono` + `build.rs` 调 CMake）
- [x] C ABI bridge（`cpp/chrono_bridge.h`，不是 cxx）：车辆创建、reset、step、状态读取
- [x] `VehicleSpec` → Chrono 参数（整车质量预算、质心、胎/悬架/转向/传动 JSON）
- [x] 轮胎/悬架/传动映射（v1 近似，见 [physics.md](physics.md) 的「spec → Chrono 的映射」）
- [ ] 与 SimpleBackend 的 A/B 对比测试
- [x] 同进程确定性回放（`identical_inputs_produce_bit_identical_states`）
- [ ] 跨编译器 golden replay（clang vs gcc 位模式对拍）
- [ ] 性能基准

验收：同一 `VehicleSpec` 可无缝切换后端，Godot 层无感知。

## Phase 5：赛道与内容

- [ ] 赛道 KN5 / `surfaces.ini` / `fast_lane.ai`
- [ ] 碰撞体（`collider.kn5`）
- [ ] 起点/维修区/计时
- [ ] AI 车辆
- [ ] 天气/时间
- [ ] 声音（`sfx/`）
- [ ] 涂装/皮肤

## Phase 6：工具与工程化

- [ ] `xtask` 构建/复制扩展产物
- [ ] CI：fmt + clippy + test（无 Godot）
- [ ] 示例车/赛道测试资产（不提交大文件）
- [ ] 性能分析（解析、网格上传、物理 step）
- [ ] 文档：格式、API、迁移指南

## 非目标（当前阶段）

- 自研渲染器
- 自研完整轮胎模型（优先复用成熟实现）
- 支持所有 AC 特殊 shader
- 编辑器内实时调车
