# 物理后端设计

## 目标

`vettura-physics` 不是要自己实现一套完整车辆动力学，而是做**稳定的胶水层**：

- 上游：Assetto Corsa 的 `VehicleSpec`（由 `vettura-assets` 映射）；
- 下游：Chrono::Vehicle、Jolt/Bullet raycast vehicle、独立 Pacejka 求解器，
  或外部进程；
- 对 Godot：只暴露 `Simulation`、`VehicleState`、`Controls`。

这样更换求解器不会影响内容加载和 Godot 层。

## 核心接口

```rust
pub trait VehicleBackend: Send {
    fn spec(&self) -> &VehicleSpec;
    fn state(&self) -> &VehicleState;
    fn reset(&mut self, pose: Pose);
    fn step(&mut self, dt: f32, controls: &Controls);
}
```

约定：

- `step` 恰好推进 `dt` 秒，不做可变步长；
- `Simulation` 负责固定步长累加、最大子步数、卡顿丢弃；
- 后端不直接读写 Godot 节点；
- 所有状态从 `VehicleState` 读取，后端内部状态可以更丰富。

## 为什么固定步长

Godot 的 `_physics_process` delta 通常稳定，但编辑器暂停、窗口拖动、断点都会
造成大 delta。车辆轮胎和悬架对步长非常敏感，必须固定：

```rust
sim.step(frame_dt, &controls);
// 内部：
// accumulator += frame_dt;
// while accumulator >= fixed_dt && steps < max_substeps {
//     backend.step(fixed_dt, controls);
// }
```

默认 `fixed_dt = 1/600`，`max_substeps = 8`。

## `VehicleSpec`

字段分为：

- 整车：质量、轴距、质心高度、迎风面积、风阻；
- 四轮：位置、半径、宽度、是否转向/驱动、悬架参数；
- 发动机：怠速、断油、峰值扭矩、齿比、主减速比；
- 传动：RWD/FWD/AWD、效率；
- 轮胎：当前只有全局峰值摩擦系数。

这些字段刻意保持“后端无关”。Chrono 需要的更多参数（轮胎刚度、松弛长度、
悬架几何硬点等）应放在后续的 `ChronoSpec` 扩展中，而不是污染通用结构；
该扩展属于 `vettura-physics` 的 `chrono` feature（`src/backends/chrono.rs`），
不单独开 crate。

## `SimpleBackend`

仅用于：

- 验证 Godot 节点能否被物理驱动；
- 验证固定步长和输入链路；
- 在没有 Chrono 的机器上跑测试。

它实现了：

- 油门 → 发动机扭矩 → 驱动轮推力；
- 风阻、滚阻、制动；
- 简化横向抓地（指数衰减侧向速度）；
- 自行车模型横摆率；
- 车轮转速/转向角/载荷占位。

**不要用它评估圈速或调车。**

## 车辆物理进阶（AC 数据 → 世界层）

以下各节按“数据在哪个 ini、模型怎么做、怎么测”组织。所有模型都遵守同一条
原则：**Jolt 只线性地、可确定地提供那部分，其余以平行力/乘性倍率叠加**，默认车
（没有 AC 数据的 spec）行为与之前完全一致。

### 气动（`aero.ini`）

每个 `WING_n` 段是 AC 帧下的一只翼面：`CHORD × SPAN` 是面积，`CL_GAIN` 是升力
系数，`POSITION.z` 的正负（AC 帧 +Z 向前）决定它算前翼还是后翼，
`ZONE_*_CL`/`YAW_CL_GAIN` 是侧滑侧向力来源。

- **基本模型**：`Cl·A` 求和 → 按 `front_bias` 分到前后轴，力加在**轴心**而不是
  质心，所以刹车/加速的载荷转移仍然正确；再乘 `v²/(v²+20²)`，静止的车不会被
  按进地面；离地升高按 `ride_height_fade` 折减（悬架长度对照
  `WheelGeometry::rest_length`）。
- **侧向力**：`side_force_coeff`（每弧度）+ 作用点在质心后
  `center_of_pressure_m`，于是同一个力自然产生回正横摆力矩（风向标效应）。
- **`VehicleTuning.downforce_coefficient`** 是覆盖值：非零时替换 spec 的
  `Cl·A`，为 0 时用 spec 数据。

### 气动 LUT（`LUT_AOA_CL` / `LUT_GH_CL`）

汇总成一个 `Cl·A` 会丢掉 AC 模型里最重要的两件事：攻角曲线与离地曲线。
`AeroSpec.wings` 因此为每只翼保存四张表：

```
AoA = 安装角(ANGLE) + 车身俯仰
Cl  = LUT_AOA_CL(AoA) · LUT_GH_CL(该翼离地高度)
下压力 = ½·ρ·Cl·A·v²，作用在该翼自己的安装点
```

- `vettura-acdata` 的 `Lut` 直接读 `.lut` 文件进 `WingSpec.aoa_cl/gh_cl/aoa_cd/gh_cd`；
- 离地表就是**地效**（真实 F1 前翼 0→1.14 / 0.12→0.95），攻角表尾部就是**失速**
  （扩压器表在 4° 之后掉下来）；
- 每只翼在自己的安装点施力，于是俯仰力矩与气动平衡随姿态变化，而不是恒定的
  前后轴比例；阻力同样逐翼计算（`Cd·A`），DRS 与 `drag_scale` 倍率照旧乘在末尾；
- 没有表的 spec（默认车）继续走上面的聚合路径；`downforce_coefficient` 覆盖时
  也走聚合路径（作者意图优先）。

### 差速器（`drivetrain.ini [DIFFERENTIAL]`）

`POWER`/`COAST`/`PRELOAD` 映射为 `DrivetrainSpec.diff`。Jolt 用“快轮/慢轮速度比
超过该值时扭矩全给慢轮”表达限滑，所以 AC 的锁定比例取倒数：
`POWER=0.25` → 限滑比 4.0，`POWER=1.0`（完全锁死）→ 1.0001。

### 刹车热（`brakes.ini`）

`PERF_CURVE` 解析成三个数：首次达到 1.0 的温度（`optimal_c`）、之后首次下降
的温度（`fade_c`）、最低效率。`BrakeSpec::efficiency_at` 给出“低温不足—最佳窗口
满扭矩—`fade_c` 后线性衰减”的效率曲线。

运行时每步按 `max_brake_torque × efficiency × |轮速| × dt` 生成热量，除以内阻后
升温；冷却项随车速增长（导风口流量）。刹车输入被乘以该效率，所以冷碟咬不住、
过热的长刹会衰减（`VehicleSnapshot.brake_temperatures_c`）。

### 刹车热容量 / 热浸（`brakes.ini [TEMPS_*] CORE_TRANSFER_*`）

摩擦面几毫秒就热，转子本体不会——这正是“踩了一串重刹之后松油门，刹车片还在被
煎”的**热浸（heat soak）**。刹车现在是两个热质量：

```
面温（刹车片看到的，效率曲线索引的值）   容量 C_surface（来自 TRANSFER）
核心（转子本体）                       容量 C_core
CORE_TRANSFER_IN / OUT：热“进”得慢、“出”得快（0.095 / 0.55）
→ 导温系数取决于哪边更热，热量永远沿梯度方向走
CORE_TRANSFER_AIR：核心向空气的散热
```

- 刹车产热现在只按**实际刹车输入**产生：以前油门全开也会“加热”刹车盘（bug，已修）；
- 单测直接验证：同样 100 kJ 注入，有核心时表面更凉、核心吃掉大部分热；同样初温
  冷却 10 s，有核心的表面明显更热（核心把热还回来）；
- 集成测试验证一次真实制动后四个盘的面温/核心温度都升、且表面领先核心；
- 遥测：`VehicleSnapshot.brake_core_temperatures_c` / `World::brake_core_temperatures_c()`。

### 燃油载荷/重心迁移（`car.ini [FUEL]/[FUEL_EXT]/[FUELTANK]`）

油箱容量、密度、燃油点位置进 `FuelSpec`。Jolt 的车体质量建好后不可变，所以每步
在油箱位置施加一个惯性力 `−m_fuel × a`：加油/放油的平衡变化、刹车时质量前移都
复现了，而不用去改 Jolt 的刚体。`World::set_fuel_fraction` 模拟进站加油。

### 变速箱/离合器（`drivetrain.ini [GEARBOX]/[CLUTCH]/[AUTOCLUTCH]`）

换挡/离合器时间（AC 用毫秒，spec 原样保留，世界层 ×0.001）、离合器强度
（`MAX_TORQUE/50`）、齿轮惯量、齿比与主减。Jolt 的自动变速箱据此换挡。

### 发动机扭矩曲线 / 涡轮增压 / 收油制动（`engine.ini`）

`power.lut` 整表进 `EngineSpec.torque_curve`（rpm → N·m），世界层按红线归一化成
Jolt 曲线；红线上方的点被丢弃并在红线处插值，没有表的车退回通用曲线
（`DEFAULT_NORMALIZED_CURVE`）。`peak_torque_nm` 保留给 Simple/Chrono 后端。

涡轮来自 `[TURBO_0]`：`MAX_BOOST`/`WASTEGATE`（bar，取小）、`REFERENCE_RPM`、
`GAMMA`（起压指数）、`LAG_UP`/`LAG_DN`（秒）、`TORQUE_GAIN`：

```
target = min(MAX_BOOST, WASTEGATE) · throttle · (rpm/REFERENCE_RPM)^GAMMA
boost += (target − boost)·(1 − e^(−dt/LAG))
力矩倍率 = 1 + TORQUE_GAIN · boost
```

Jolt 只接受一个最大扭矩，于是引擎按**满增压**扭矩建，油门再乘
“当前倍率/满增压倍率”——`power.lut`、涡轮压力和热保护都落在同一个油门系数里。

收油制动来自 `[COAST_REF] RPM/TORQUE/NON_LINEARITY`：
`coast(rpm) = TORQUE·[(1−n)·u + n·u²]`（`u = min(rpm,RPM)/RPM`）。throttle=0 时把
Jolt 油门写成负的小比例，泵气与摩擦损耗正好变成曲轴阻力矩；怠速几乎为零，
高转松油门明显减速。

> 顺带修正：INI 数值解析现在会去掉 `;` / `#` / `//` 行内注释。此前
> `LIMITER=19000 ; no limiter`、`SPRING_RATE=40000 ; ...` 会解析失败并静默退回
> 默认值——真实车的红线、弹簧率、阻尼率全部受影响。

### 惯性张量（`car.ini [BASIC] INERTIA`）

AC 的 `INERTIA=x,y,z`（AC 轴 = 横向/垂直/纵向，即**俯仰/横摆/侧倾**）进入
`spec.inertia`（标签 `roll, pitch, yaw`，kg·m²）。很多 mod 写的不是物理量
（F1 2008 的 `0.70` 作为俯仰惯量小三个数量级），所以每个轴先用“质量 × 整车足迹”
推导的合理带校验，带外退回碰撞盒张量并 `debug!` 记录。建体用
`BodySettings::mass_and_inertia`，`Vec3::ZERO`（默认）保持 Jolt 原有行为。

### 轮胎热 / 胎压 / 磨损（`tyres.ini [THERMAL_*]` + GRAIN/BLISTER）

单层胎体温度按“滑动摩擦生热 + 环境冷却”演化，`GRAIN_GAIN`/`BLISTER_GAIN` 叠加
在滑移与温差上；胎压按理想气体随温度变化，`PRESSURE_STATIC`/`PRESSURE_IDEAL`
决定工作窗口；磨损按滑移能量累积。三者合成一个乘性倍率进 `GripState.tyre_bits`，
再由 Jolt 的轮胎回调读取。

### 轮胎滑移曲线（Magic Formula，移植自 Project Chrono）

Jolt 的轮式车辆吃的是“摩擦系数 vs 滑移”的**表**（纵向按滑移率、侧向按滑移角 °）。
以前这张表是硬编码的，所有 mod 的车都是同一条曲线。现在它从每台车自己的数据
拟合：

```
F(x) = D · sin(C · atan(B·x − E·(B·x − atan(B·x))))
C    = 1.65                          // 形状因子
B    使峰值正好落在该车数据的峰值滑移上
E    使 3×峰值滑移处仍保留 FALLOFF_LEVEL 的抓地
```

- 公式移植自 Project Chrono 10.0.0 的 `ChPac89Tire.cpp`（BSD-3-Clause，本地
  `third_party/chrono`），核心就一行解析式，因此不必把第二套物理引擎链进来；
- 数据：`FRICTION_LIMIT_ANGLE`（侧向峰值角度，F1 前 6.1°/后 5.8°）、
  `FALLOFF_LEVEL`（0.88）、纵向峰值滑移率（AC 不发布，沿用 6%）；
- 两表归一化到峰值 1.0，真正的峰值摩擦仍由倍率链决定，曲线只负责**形状**；
- 拟合不出来的车（NaN/0）退回原有默认表，行为不变；
- 侧向峰值由硬编码的 3° 改为车的真实值，所以“轻打方向”与“接近极限”两端的响应
  更接近真实轮胎。

### 轮胎垂直刚度（`tyres.ini RATE` / `PRESSURE_SPRING_GAIN`）

车轮不是只靠悬挂弹簧连在地面上，轮胎本身就是第二根弹簧，两者**串联**：
`k = 1/(1/k_susp + 1/k_tyre)`。Jolt 只建模悬挂弹簧，所以喂给它的弹簧率必须是这个
串联值，否则行驶高度与载荷转移都偏 15~20%。

- AC 的 `RATE` 是轮胎垂向刚度（F1 前 300 kN/m、后 370 kN/m），
  `PRESSURE_SPRING_GAIN` 是每 psi 附加的刚度（相对冷态胎压）；
- 运行时胎压随温度上升，所以**热胎更硬**：17→27 psi 约多 100 kN/m；
- 没有 `RATE` 的车（默认 spec）完全不受影响。

### 零下温度与冬季工况

温度不是“0 °C 以下就夹住”的量：冷胎、冷刹车、冷引擎、结冰路面都是物理状态。

- 环境温度来自 `Weather.air_temperature_c` / `track_temperature_c`（可到
  −60 °C）；`Weather::ambient_c(on_the_road)` 区分“露天的部件用气温、
  轮胎用路面温度”，冬天路面往往比空气更冷；
- 刹车/引擎/ERS 用气温、轮胎用路温作为松弛目标；`BrakeSpec.ambient_c` 只是
  没有发布天气时的兜底；
- 胎压随橡胶温度线性变化，零下就是更低的气压（不低于 0.5 psi）；
- 有 `PERFORMANCE_CURVE` 时，曲线在最冷点以下**按第一段斜率外推**而不是
  夹住：−20 °C 的胎必须比 0 °C 更差，直到 0 截止；没有曲线时解析窗口同样
  允许负温度；
- 路面抓地在 0 °C 以下继续衰减到 −30 °C 的冬季地板（不是断崖）；
- 车辆被添加/重置时所有温度对齐到当前环境：冬天出来的车就是冷的，
  然后照常被驾驶加热。

### 轮胎三层温度 + 温度-抓地曲线（`[THERMAL_*]` / `PERFORMANCE_CURVE`）

真实轮胎是三个热质量，不是一个：

```
表面（road 接触面，~60 J/K）   毫秒级升温，风冷最强
胎面带（patch，~200 J/K）      抓地曲线读的是它 —— 所谓“轮胎热起来了”
胎体（core，~1 kJ/K）         慢热库，长时间保持热量
```

- AC 的 `SURFACE_TRANSFER` / `PATCH_TRANSFER` 就是层间传导率；于是新胎要
  **接近一圈**才进入工作温度，而摩擦面早就几百度的——这正是真实轮胎的行为；
- `PERFORMANCE_CURVE`（化合物自带的温度 → 抓地倍率表）直接替换我们原来的解析
  温度窗口：F1 软胎 70–105 °C 满抓地、40 °C 只有 0.90、300 °C 掉到 0.60；
- 没有层数据的车（默认 spec）保持原来的单质量行为，三层温度相等；
- 遥测：`VehicleSnapshot.tyre_surface_temperatures_c` / `tyre_core_temperatures_c`，
  `tyre_temperatures_c` 现在是“抓地曲线读的那个温度”（分层模式下 = patch）。
- `CORE_TRANSFER` / `INTERNAL_CORE_TRANSFER` 描述胎体内部导热，我们的单块胎体
  模型不需要，因此没有使用（已在代码注释说明）。
- `WEAR_CURVE` 没有接入：这个 mod 的 `WEAR_CURVE` 文件其实是另一条温度曲线，
  真正的磨损-抓地表（`FR_Wear.lut`）在 INI 里没有引用，强行猜测语义比不用更糟。

### 轮胎载荷敏感 / 松弛长度（`FZ0`、`LS_EXP*`、`RELAXATION_LENGTH`）

- **载荷敏感**：轮胎*力*随载荷按指数 <1 增长（侧向 `LS_EXPY`、纵向 `LS_EXPX`，
  参考载荷 `FZ0`），系数侧就是 `μ *= (load/FZ0)^(exp−1)`——轻轮系数高、重轮
  低。回调里用 Jolt 给的 `suspension_impulse/dt` 当真实载荷，逐轮逐帧生效；
  没写 `FZ0` 的车完全关闭。
- **松弛长度**：轮胎要走完 `RELAXATION_LENGTH`（≈0.1 m）侧向力才追上滑移。实现
  是“已建立的滑移”一阶滞后 `slip += (measured−slip)·(1−e^(−v·dt/L))`，再用
  `relaxed/measured` 缩放可用抓地——线性区里等价于“力 ∝ 松弛后的滑移”。

### 轮胎联合滑移（摩擦圆）

一个轮胎只有一份抓地预算：`TyreSpec::lateral_share/u` 与 `longitudinal_share/u`
给出椭圆预算 `√(1−u²)`，世界层每步从 `slip_ratio`/`slip_angle` 估两个方向的需求
（归一化点 1.0 与 0.8 rad），把 `GripState` 的纵/横向摩擦分别乘上对应份额——
同时刹车与转向会掉侧向抓地。份额地板 0.6：完全打滑的轮胎仍保留 6 成预算。

### 轮胎平点 / 抱死振动

抱死的轮子在接触斑里做真实机械功，这部分能量进橡胶形成**平点**：

```
锁定中：energy += load · (|slip|−0.9) · |v| · dt
未锁定：energy −= 250 J/s
spot   = clamp(energy/flat_spot_capacity_j, 0, 1)
grip  *= 1 − flat_spot_grip_loss · spot
```

平点每转经过接触斑一次，于是产生一次车轮频率的垂向冲击
（`flat_spot_vibration_gain × 载荷 × spot × sin(轮转角)`）——这就是抱死时方向盘
上的“抖”。松刹车后按固定速率恢复。

### 悬架非线性

Jolt 的轮式车辆只认识线性弹簧与阻尼。真实 AC 车还有渐进弹簧、bump/rebound stop、
heave 第三弹簧，部分车有主动执行器——全部以**平行力**叠加：

```
F_corner = F_jolt_linear + k_prog·x² + k_bump·max(0,x−x_bump)
                            − k_rebound·max(0,e−e_rebound) + F_active
```

- `PROGRESSIVE_SPRING_RATE` → `progressive_rate_n_per_m2`；`BUMP_STOP_RATE` →
  `bump_stop_rate_n_per_m`，触发点取 `PACKER_RANGE×0.75`（并夹到实际行程内）；
- `[HEAVE_FRONT]`/`[HEAVE_REAR]`：按该轴平均压缩量施力、作用在轴中心（只抬车身
  不增加侧倾刚度）；`ROD_LENGTH` → 预载，否则静态不承载。

#### 两级阻尼（bleeder）与非对称压缩/回弹

Jolt 只有一个对称阻尼（取压缩/回弹平均）。`DAMP_FAST_BUMP(THRESHOLD)` /
`DAMP_FAST_REBOUND(THRESHOLD)` 是泄压阀：超过阈值切到更小的快速率。补差
`−(desired − jolt)·v` 平行叠加，静止时为 0；压缩/回弹的非对称也由此生效。

### 悬架运动学中的外倾（camber）

Jolt 不会因侧倾改变外倾角，而外倾正是把接触斑压平的那一环：

```
camber(compression) = STATIC_CAMBER + camber_gain · compression
thrust              = camber_thrust_coeff · load · camber（沿该轮“向外”轴）
grip               *= 1 − camber_grip_loss · |camber − 最优外倾| / 窗口
```

外倾推力朝轮胎倾斜的一侧，所以负外倾在两侧都指向车身内侧：静态左右抵消
（停放侧向速度 <0.05 m/s），载荷转移到外侧轮时净推力指向弯心。推力作用在
**接地点**，因此同时带有真实外倾的小横摆力矩。载荷敏感/外倾/温度/胎压/磨损/
橡胶/水膜/平点全部乘在同一条倍率链上。

### 主动 / 半主动悬架

vanilla AC 没有标准主动悬架字段，这里用可选的 `[ACTIVE]` 扩展段（无段 = 被动）：

| 键 | 含义 |
| --- | --- |
| `ENABLED` | `0` 关 / `1` 全主动 / `2` 半主动 |
| `STIFFNESS`/`SPRING_RATE` | 对车高误差的刚度，N/m |
| `DAMPING` | 对悬架速度的阻尼，Ns/m |
| `MAX_FORCE` | 单角执行器上限，缺省 1.25 × 单角静载 |
| `RIDE_HEIGHT`/`RAKE` | 目标车高与前后倾角，米 |
| `SPEED_DROP`/`SPEED_REF` | 高速降车高，`v²/(v²+ref²)` 平滑 |

执行器是力，加在 Jolt 使用的轮毂点上；全主动同时抑制侧倾与俯仰，半主动只保留
阻尼。`World::set_active_suspension_mode` 可运行时切换。

### ERS / 混动部署与回收

电池容量、电机功率、部署/回收效率与每圈额度进 `ErsSpec`。`World::apply_hybrid_forces`
按 `P = F·v` 把电机功率换算成车头方向的力（`F ≤ P/v` 下限即静止时不发力），
部署受电量与每圈额度限制，回收只在制动/滑行时发生。

### 发动机热 / 功率保护

燃油能量入冷却液、水温/油温独立演化，过热/过冷都按 `EngineThermalSpec` 掉动力
（`EngineSpec::power_multiplier`），保底值让车还能开回维修站。

### ABS / 牵引力控制（车身闭环）

`Controls.abs` / `Controls.tc` 打开车辆自己的制动/牵引控制。它们只看实测轮滑移：

```
ABS：worst|slip| > 0.14 → brake  ×= clamp(0.14/worst, 0.25, 1)
TC ：驱动轮 worst|slip| > 0.12 → throttle ×= clamp(0.12/worst, 0.2, 1)
```

Jolt 只有整车刹车/油门通道，所以 ABS 缩放整车压力而不是脉冲单轮。控制律在
`vehicle.rs` 有单测直接验证，集成测试验证打开后仍刹得住、跑得动。

### 湿胎排水 / 水膜（动态）

稳态水膜 ≈ 路面水深 × 驻留时间(1/√v) × 抓持系数(载荷/胎温)；轮胎只在接触斑见水，
所以越快水膜越薄（“冲出水面”）。水膜与路面水深相加后再进 aquaplaning 计算，并按
~0.5 K/(s·mm) 带走胎热。`GripState.water_film_bits` 逐轮发布。

### 路面起伏（`SIN_HEIGHT` / `SIN_LENGTH`）

`SurfaceProperties.bump_height_m / bump_length_m`，`World::road_bump_m(position, heading)`
是位置与行进方向的确定性函数（同位置每次读数一致，回放/联机无需同步状态）；抓地按
坡度 `height/length` 折减——载荷从不稳定，有效摩擦必然下降，这就是缓冲区抓地差的
第二个原因。这里**没有**额外加“路面激励力”：悬挂已在求解轮-几何接触，再叠一份会同向
重复计算（实测会把车按进地面）。

### 赛道演进（groove）

`groove.ini` 的每个 `[GROOVE_n]` 各有 `MIN`（初始附加抓地）、`MAX`（铺满橡胶后的
附加）、`MULT`（堆积速度）。KN5 里没有把 groove 绑到三角面，所以实现是**位置驱动**的：
车轮接触地面时就地把橡胶抹上去，抓地查询只看那一格。赛线有加成、没人走的边缘没有。
`GrooveMap` 每轮发布倍率，由 Jolt 轮胎回调读取。

### 碰撞碎片（刚体）

`DebrisSpec`（盒/球/圆柱、质量、摩擦、弹性、阻尼、初速度，全参数夹紧）+ 
`World::{spawn_debris, remove_debris, clear_debris, debris_snapshot, debris_handles}`。
碎片在 `MOVING` 层，和赛道、墙、其它碎片、所有车辆共用同一个求解器——二次碰撞是真的。
接触事件只要有一方是车辆，`ImpactEvent` 的 `a`/`b` 就会出现
`ContactEntity::Debris(handle)`。世界只负责“存在且会撞”：清理时机与安全车是比赛规则。

### 遥测汇总

`VehicleSnapshot`：`wheels[i]`（接触点/载荷/滑移/滑角/外倾/悬架长度/接地）、
`brake_temperatures_c`、`tyre_temperatures_c/pressures/wear`、`fuel_fraction/kg`、
`ers_soc/energy/lap_deploy/temperature`、`coolant_c/oil_c/engine_power_multiplier`、
`turbo_boost_bar`、`water_film_mm`、`flat_spot`、`suspension_force_n`、
`active_suspension_force_n`、`heave_force_n`，以及同名 `World` getter。

## Chrono::Vehicle 接入

### 构建

Chrono 是 CMake/C++ 项目，源码放在仓库根的 `third_party/chrono/`（tag 10.0.0，
拉取命令见 [third_party/README.md](../third_party/README.md)）。实现上：

- `build.rs` 直接调本机 `cmake` + `c++`（不引入 `cmake` / `cc` crate），
  在 `OUT_DIR/chrono-build` 里构建 `Chrono::Core` + `Chrono::Vehicle`；
- 仓库默认不带 Chrono 源码（`.gitignore` 忽略 `third_party/*`），
  新克隆后需要按 `third_party/README.md` 拉一次，或用 `CHRONO_ROOT` 指向已有源码树；
- 找不到 Chrono 时链 `cpp/chrono_bridge_stub.cpp`（返回 `NotLinked`），
  workspace 始终能编译；真正的物理只在 `cargo build -p vettura-physics
  --features chrono` 且 Chrono 就位时生效。

### FFI：C ABI，不是 cxx

用 `extern "C"` 暴露最小接口，契约就是 `crates/vettura-physics/cpp/chrono_bridge.h`。
**没有用 cxx**：要传的东西全是 POD + 一个不透明句柄，用不到 cxx 的类型安全；
而 C ABI 是唯一一个所有编译器都共识的契约，可以让 Chrono 用任意 C++ 工具链编出来。

```c
// cpp/chrono_bridge.h（节选）
int32_t vettura_chrono_create(const vettura_chrono_spec* spec,
                              vettura_chrono_vehicle** out);
int32_t vettura_chrono_step(vettura_chrono_vehicle* vehicle, double dt,
                            const vettura_chrono_controls* controls);
int32_t vettura_chrono_read_state(const vettura_chrono_vehicle* vehicle,
                                  vettura_chrono_state* out_state);
```

Rust 侧手写 `#[repr(C)]` 镜像（不需要 bindgen，契约就一个头文件），
镜像与头文件的一致性由**布局测试**保证——期望值是 `cpp/layout_probe.c`
用 clang 实测出来的，不是手算的：

```rust
unsafe extern "C" {
    #[link_name = "vettura_chrono_step"]
    pub fn vettura_chrono_step(
        vehicle: *mut VetturaChronoVehicle,
        dt: f64,
        controls: *const VetturaChronoControls,
    ) -> i32;
}

impl VehicleBackend for ChronoBackend {
    fn step(&mut self, dt: f32, controls: &Controls) {
        // f32 → f64 在 Rust 侧完成，C++ 侧只见 double
        let status = unsafe {
            ffi::vettura_chrono_step(self.handle, dt as f64, &mut ffi::to_ffi_controls(controls));
            ffi::vettura_chrono_read_state(self.handle, &mut ffi_state)
        };
        debug_assert_eq!(status, ffi::VETTURA_CHRONO_OK);
    }
}
```

头文件里固化的规则（详细注释见源码）：

- 只用 POD + `double` / `int32_t`，不用 `float`、`long double`、`bool`、`size_t`；
- 异常不许跨边界，全部 `try/catch` 成 status code；
- 无全局可变状态、无线程、无内部分配泄漏（create/destroy 对称）；
- 头文件的 ABI 版本号在 `ChronoBackend::try_new` 创建前校验，不匹配直接拒绝。

### Chrono 是 JSON 驱动的 —— 所以胶水层分成两块

Chrono 的 wheeled vehicle 由 JSON 规范构造
（`chrono_vehicle/wheeled_vehicle/vehicle/WheeledVehicle::Create`：顶层车辆
JSON 引用 chassis / suspension / wheel / brake / steering / driveline /
powertrain / tire 各子系统 JSON），所以 `cpp/` 分两个文件：

| 文件 | 职责 | 依赖 |
| --- | --- | --- |
| `vettura_vehicle_spec.cpp` | `vettura_chrono_spec` → 全套 Chrono JSON，写到临时目录并 `SetChronoDataPath` | 只依赖标准 C++，**脱离 Chrono 也能编译/单测**（`cpp/bridge_dump.cpp`） |
| `bridge.cpp` | `WheeledVehicle` + `FlatTerrain` + 固定步长步进 + 状态回填 | Chrono 头文件 + 静态库 |

我们只用 `DoubleWishboneReduced` 悬架模板：我们的 `VehicleSpec` 只有弹簧/
阻尼/行程，硬点几何按胎半径等比例缩放取默认值；需要更细的悬架/轮胎参数
（硬点、衬套、Pacejka 曲线）走后续的 `ChronoSpec` 扩展，不污染 `VehicleSpec`。
AWD 暂不支持（还没接 `ShaftsDriveline4WD` 的 JSON schema），`try_new`
返回 `InvalidSpec`。

### spec → Chrono 的映射（v1 的实际取舍）

`VehicleSpec` 是后端中立的，只有整车级参数。落到 Chrono 时必须做几个明确的
近似，全部集中写在 `cpp/vettura_vehicle_spec.cpp` / `cpp/bridge.cpp` 里：

| 项 | 映射 | 原因 |
| --- | --- | --- |
| 质量 | `mass_kg` 是**整车**质量；底盘只拿簧上质量（扣掉 4 套 spindle/upright/wheel/tire + 转向拉杆，约 7%） | Chrono 把轮端建成独立刚体，底盘再写整车质量会让总质量多出一整套轮端（实测 1200 kg → 1578 kg，轮荷高 30%） |
| 质心 | 底盘 JSON 的 `Centroidal Frame` 抬高 `cg_height_m` | Chrono 的 ref frame 在质心下方，`VehicleState.position` 上报的是**质心**帧 |
| 悬架 | `DoubleWishboneReduced`，HMMWV 硬点 × `k = 胎半径 / 0.4699` | spec 没有硬点；等比缩放下轮距/行程角都保持自洽 |
| 弹簧/阻尼/行程 | 弹簧刚度与轮胎刚度**串联**成轮胎的垂直刚度，阻尼取压缩/回弹均值；静挠度上限为行程的一半 | DWA 模板不接受弹簧/阻尼，垂向柔度只能落在轮胎上；用串联等效让 spec 的弹簧/阻尼真正影响车身高度与侧倾 |
| 轮胎 | `TMsimpleTire` 的 **"Parameters"** 全参数化分支（Vertical / Longitudinal / Lateral 全套） | 走 `Load Index` / `Maximum Bearing Capacity` 分支时 `GuessPassCar70Par()` 会用 `secth + rimDia/2` 覆盖设计半径（0.33 → 0.223 m），轮心直接陷地 |
| 转向 | `RackPinion` 齿轮半径/齿条长度同样 × k（`0.1k` / `0.896k`，满舵 50°） | 齿条行程 → 转向节臂的增益随几何缩放不变，满舵约 26–35°；用固定 0.1×胎半径时满舵只有 ~10° |
| 发动机转速 | `GetMotorSpeed()` 是 rad/s，转 rpm 后夹到 `idle_rpm` | 停车时 `AutomaticTransmissionSimpleMap` 把发动机转速算成 0（模型没有怠速控制器/变矩器蠕变），但真实发动机在怠速 |
| 出生/复位高度 | 底盘 ref frame ≥ `胎半径 + 5 mm` | `DiscTerrainCollision1pt` 把「盘心低于地形」判为**无接触**，低于这个高度会直接穿地 |
| 复位 | 只保留 yaw、把 ref frame 抬到下限、清零**所有**刚体速度 | `ChVehicle::Relocate` 在 10.0 已移除；`ChBodyAuxRef::SetFrameRefToAbs` 是 Chrono 自己 `Initialize` 走的路径 |

已知偏差（v1 接受，后续 `ChronoSpec` 再修）：

- 轮距由悬架硬点决定（当前 ≈1.28 m），不完全等于 `WheelSpec.position` 的
  `|x| × 2`；
- 滚动/俯仰刚度来自悬架几何 + 轮胎垂向刚度，`SuspensionSpec::anti_roll_n_per_m`
  尚未接到防倾杆；
- 悬架弹簧/阻尼参数只进了轮胎垂向模型，更换模板前不做四分之一车校验；
- AWD、差速器锁、手动变速箱、传感器/可视化均未接；
- `Controls::shift_up` / `shift_down` / `clutch` 在 Chrono 后端是 no-op：用自动变速箱
  （`AutomaticTransmissionSimpleMap`），挡位/离合由它自己决定。

### 跨编译器一致性

让 clang / gcc / MSVC（以及不同优化选项）跑出**逐位相同**的结果，靠的是下面几条，
缺一条就可能 diverged：

| 风险 | 处理 |
| --- | --- |
| FMA 收缩（clang/gcc 默认 `-ffp-contract=fast`，MSVC 不收缩） | clang/gcc 加 `-ffp-contract=off`，MSVC 用 `/fp:strict` |
| fast-math / 不安全数学优化 | `-fno-fast-math -fno-unsafe-math-optimizations -fno-reciprocal-math -fno-ffinite-math-only`，MSVC 绝不开 `/fp:fast` |
| x87 扩展精度（仅 32 位 x86） | 只支持 x86-64 / arm64（都是 SSE/NEON + double） |
| Eigen 向量化分块随 ISA 宽度变化 | 同架构跨编译器宽度一致；**跨架构**（x86 + AVX vs arm64 + NEON）要逐位一致就加 `-DEIGEN_DONT_VECTORIZE` |
| Chrono 多线程归约顺序 | `CH_ENABLE_OPENMP=OFF`，全程单线程 |
| `-march=native` / `/arch:AVX2` | 我们的桥代码不带；Chrono 自己的 `FindSIMD` 会加 `-march=native`（CMake 选项关不掉）→ 逐位一致的范围是「同机跨编译器」 |
| NDEBUG / assert 影响代码路径 | 固定 `CMAKE_BUILD_TYPE=Release`，并记录 `vettura_chrono_build_id()` |
| Chrono 版本漂移 | `third_party/chrono` 锁 commit |

验证方式：`golden_replay_hash_is_stable` 跑固定 `VehicleSpec` + 固定输入序列
（2400 步），把每步 `VehicleState` 的位模式压成 FNV-1a 哈希；用
`VETTURA_GOLDEN_HASH=<hash>` 在另一个编译器下比对（详见 build.md §8.2）。
同进程内另有 `identical_inputs_produce_bit_identical_states` 守回归。

### 坐标与单位

- Chrono 默认也是米/秒/弧度，但坐标系需确认；
- Vettura 物理帧：`+X` 右、`+Y` 上、`-Z` 前；
- AC KN5 帧：`+X` 左、`+Y` 上、`+Z` 前；
- 在 `vettura-assets` 边界完成 AC → 物理帧转换；
- 在 Chrono bridge 边界完成物理帧 → Chrono 帧转换（如需要）。

### 状态映射

每个 step 后回填（`read_state` 无副作用，可重复调用）：

| `VehicleState` | Chrono 来源 | 说明 |
| --- | --- | --- |
| `position` | `ChassisBody()->GetPos()` | **质心**帧；`reset(pose)` 收的是 ref frame，两者差 `cg_height_m` |
| `orientation` | `ChassisBody()->GetRot()` | 经基变换回到物理帧（+X 右、+Y 上、-Z 前） |
| `linear_velocity` / `angular_velocity` | `GetPosDt()` / `GetAngVelParent()` | 都是世界系（与 SimpleBackend 一致） |
| `speed_mps` | `ChVehicle::GetSpeed()` | 有符号前进速度 |
| `wheels[i].steer_angle` | `GetSteeringAngle(axle, side)` | 正 = 左（与 `Controls::steering` 一致） |
| `wheels[i].angular_velocity` | `GetSpindleOmega` | rad/s，正 = 前进 |
| `wheels[i].rotation` | 角速度 × dt 累加 | 回绕到 `[0, 2π)`，仅给可视化 |
| `wheels[i].slip_ratio` / `slip_angle` | `ChTire::GetLongitudinalSlip()` / `GetSlipAngle()` | 轮胎模型直接给出 |
| `wheels[i].load_n` / `contact` | `ReportTireForce().force.z()` | 全局系竖轴分量；> 单轮静载 0.1% 判接地 |
| `engine_rpm` / `gear` | `GetMotorSpeed()` × 60/2π、`GetCurrentGear()` | rpm 夹到 `idle_rpm`（停车时 Chrono 报 0） |

后续：轮胎温度/磨损/压力、悬架行程、碰撞状态（需要 `ChronoSpec` 暴露更多通道）。

## 备选后端

如果 Chrono 太重：

1. **Jolt/Bullet raycast vehicle**：利用 Godot 已有 Jolt，但车辆模型通常偏街机；
2. **独立 Pacejka + 刚体**：可控性最好，但工作量大；
3. **外部进程**：物理独立进程，通过共享内存/UDP 通信，方便调试和崩溃隔离；
4. **AC 原版物理不可用**：闭源，无法直接复用。

无论选哪个，只需要实现 `VehicleBackend`。

## 待办

- [ ] 把 `SimpleBackend` 的挡位/转速逻辑改成独立 `Powertrain` 模块；
- [ ] 增加 `VehicleBackend::set_spec` 或重建机制；
- [ ] 增加碰撞/地面查询接口（Godot 侧提供 raycast）；
- [ ] 增加 `Simulation` 的可变步长模式（仅用于调试）；
- [x] 为 Chrono 后端增加确定性回放测试（同进程逐位比对）；
- [ ] 跨编译器 golden replay（clang vs gcc）、跨机器基线 ISA；
- [ ] AWD / `ShaftsDriveline4WD`、防倾杆（`anti_roll_n_per_m`）。
