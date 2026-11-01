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
