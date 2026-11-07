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

- [x] `VehicleBody` 接入 `AcCar` 的 `VehicleSpec`
- [x] 车轮节点按 dummy 名称自动绑定
- [x] 输入 API 完整暴露（油门/刹车/转向/手刹/DRS/ABS/TC，`set_controls`）
- [x] 赛道地面/车身姿态由 `WorldBody` 的 Jolt 碰撞体与轮上 `VehicleSnapshot` 驱动
- [ ] 键盘/手柄输入映射（Godot Input Map）
- [ ] 相机跟随/车内视角
- [ ] 基础 HUD（速度、挡位、转速）

验收：能沿平面直行、转向、刹车，车轮转动和转向可见；`godot/baseline.tscn`
进一步验收“真实赛道 + 真实车能跑”，输出 `[baseline] PASS`。

## Phase 3：AC 物理数据完整映射

- [x] `power.lut` 完整扭矩曲线 + 车体惯性张量（`car.ini INERTIA`，带合理性校验）
- [x] 涡轮/混动/能量回收
- [x] 变速箱换挡时间、离合（`[GEARBOX]`/`[CLUTCH]`/`[AUTOCLUTCH]` → Jolt 切换/离合时间、离合器强度、转动惯量）
- [ ] 自动补油（降挡油门跟随）
- [x] 差速器（power/coast/preload）
- [x] 轮胎温度/磨损/压力 + 载荷敏感（`FZ0`/`LS_EXP*`）+ 松弛长度（`RELAXATION_LENGTH`）
- [x] 轮胎垂直刚度（`RATE`/`PRESSURE_SPRING_GAIN`，与悬挂串联）+ 三层胎温 + `PERFORMANCE_CURVE`
- [x] 轮胎滑移曲线：Magic Formula 公式移植自 Project Chrono `ChPac89Tire`，按 `FRICTION_LIMIT_ANGLE`/`FALLOFF_LEVEL` 拟合
- [x] 悬架几何硬点、防倾杆、 bump stop（硬点仍由 Jolt `DoubleWishboneReduced` 近似；防倾杆/bump stop/渐进弹簧/限位/动态外倾与外倾抓地窗口/两级阻尼与非对称压缩回弹已落地）
- [x] 气动多翼面 + 高度/DRS 曲线（含 `LUT_AOA_CL`/`LUT_GH_CL` 逐翼攻角×离地表 = 地效与失速）
- [x] 制动/ABS/TC（刹车热衰减 + 转子核心热浸 `CORE_TRANSFER_*` + ABS/TC 闭环控制，见 docs/physics.md）

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

- [x] 赛道 `models*.ini` / `surfaces.ini` 解析（含多布局目录）
- [x] 碰撞体（所有模型中 `1` 开头的物理节点 → Jolt static mesh，分块 + 表面索引）
- [x] `fast_lane.ai` / `ideal_line.ai` v7 解析 + 起点位姿
- [x] 安全发车格（AI 线吸附 + 碰撞体坡度/净空检查）
- [x] 维修区（`pit_lane.ai` 复用 fast_lane 驱动/发车）
- [x] 计时（`sections.ini` 分段 + `LapTimer` 圈速/最佳圈/无效圈判定）
- [x] AI 车辆控制（沿 `fast_lane.ai` 纯跟踪 + 速度提示 + 周边车感知/超越与跟车）
- [x] 天气/时间对表面抓地的影响（`SurfaceMap` + `Weather`）
- [x] 赛道演进（`groove.ini` → `GrooveMap` 位置驱动橡胶堆积 → 每个车轮的抓地倍率）
- [x] 声音运行时：`sfx/GUIDs.txt` 事件表 + bank 定位 + FMOD Studio runtime 直达（`vettura-audio` / `FmodSystem`；bank 由 FMOD 加载，不做 FSB 解析）
- [ ] 声浪状态映射：rpm / load / 挡位 / turbo / 路面 / 打滑 → FMOD 事件参数与混音
- [ ] 音频资源策略：bank 预加载、内存上限、总线和音量矩阵
- [ ] 涂装/皮肤

## Phase 6：车辆物理进阶（核心驾驶感）

- [x] **气动套件**：`aero.ini` 汇总 `Cl·A` → 前后轴独立下压力、离地高度衰减、`v²/(v²+ref²)` 平滑
- [x] **差速器/锁止**：`drivetrain.ini [DIFFERENTIAL]` POWER/COAST/PRELOAD → Jolt 限滑比映射
- [x] **刹车热衰减/温度**：`brakes.ini` PERF_CURVE → 每碟升温/风冷、踏板效率随温度变化
- [x] **燃油载荷/重心迁移**：`car.ini [FUEL]/[FUEL_EXT]/[FUELTANK]` → 油箱位置惯性力 + `World::set_fuel_fraction`
- [x] **变速箱/离合器**：`drivetrain.ini [GEARBOX]/[CLUTCH]/[AUTOCLUTCH]` → 换挡/离合时间、离合器强度、齿轮惯量
- [x] **DRS/可调空气动力学**：`DrsSpec` + `Controls.drs` → 阻力按倍数下降（赛道激活区规则待补）

## Phase 7：高级车辆物理（耐力赛/现代方程式/专项）

- [x] **气动侧向力/横摆稳定性**：`YAW_CL_GAIN`/`ZONE_*_CL` → 侧滑侧向力，作用点在质心后产生回正力矩
- [x] **轮胎联合滑移（摩擦圆）**：椭圆预算 `√(1−u²)`，制动+转向互抢抓地，份额地板 0.6
- [x] **轮胎载荷敏感/松弛长度**：`FZ0`/`LS_EXP*` 载荷→摩擦系数、`RELAXATION_LENGTH` 侧向力随行驶距离建立
- [x] **气动 LUT**：逐翼 `LUT_AOA_CL` × `LUT_GH_CL`（攻角曲线 + 离地曲线）= 地效与失速，逐翼施力
- [x] **路面起伏**：`SIN_HEIGHT`/`SIN_LENGTH` → 确定性 bump 函数 + 坡度抓地折减
- [x] **轮胎热/压/磨损**：胎温窗口（冷/热都掉抓地）、理想胎压、滑动生热、磨损累积 → `GripState` 逐轮倍率
- [x] **ERS/混动**：电池 + 电机部署/回收，受电量与每圈额度限制，回收只在制动时
- [x] **发动机热/功率保护**：燃油能量入冷却液、水温/油温窗口、冷启动掉动力、过热保底
- [x] **主动悬挂/高度控制**：半主动/全主动执行器、目标车高/倾角、随速降低车高
- [x] **车体惯性张量**：`car.ini INERTIA` 带物理合理性校验映射进 Jolt 刚体
- [ ] **路面颠簸/3D 扫描路面**：PSD 颠簸谱、单独颠簸激励通道
- [x] **湿胎排水/水膜动力学**：轮胎动态水膜（驻留时间模型）+ 与路面水深叠加 + 水膜冷却胎热
- [x] **轮胎平点/抱死振动**：锁死轮真实功耗 → 平点损伤、每转一次的垂向冲击、抓地折减与恢复
- [x] **碰撞碎片**：世界持有刚体碎片（车身件/轮胎/锥桶/碎石），与赛道/墙体/其它碎片/车辆二次碰撞（清理与安全车属比赛规则，不在物理层）

## Phase 8：工具与工程化

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
