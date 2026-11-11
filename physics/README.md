# vettura-physics

**与求解器无关的车辆动力学契约层**。刻意不含任何 AC 解析代码——
AC 参数映射住在 `vettura-assets`，物理核心可以用合成车测试，
将来也能接非 AC 内容。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `spec.rs` | `VehicleSpec` 全家桶：质量/惯量、车轮、悬架（含主动悬架、heave 第三弹簧）、发动机（涡轮/热/ERS）、传动、轮胎（热/压/磨）、气动（逐翼/DRS）、刹车、燃油 |
| `state.rs` | `VehicleState` / `WheelState`：位姿、速度、四轮状态、转速、挡位 |
| `controls.rs` | `Controls` / `ShiftBlip`：油门、刹车、离合、转向、换挡、手刹、DRS、ABS/TC 请求 |
| `backend.rs` | `VehicleBackend` trait：`step(dt, controls)`——所有求解器（Simple / Chrono / 将来自研）的唯一契约 |
| `simulation.rs` | 固定步长调度：accumulator + `max_substeps` 防死亡螺旋，默认 **600 Hz** |
| `tyre_model.rs` | `SlipCurve`：Magic Formula（Pac89，移植自 Project Chrono）滑移曲线拟合，按 AC 数据的峰值点/衰减点求 B/C/E |
| `backends/simple.rs` | 力基线占位后端：端到端联调与无求解器环境用，常量均为占位值 |
| `backends/chrono/` + `cpp/` | Chrono::Vehicle 适配（feature `chrono` 门控，**默认关闭**）：C ABI 桥（非 cxx）、spec → Chrono JSON；Chrono 缺席时 build.rs 自动 stub，任意机器可编译 |

## 关键入口

```rust
let sim = vettura_physics::Simulation::new(backend)
    .with_fixed_dt(1.0 / 600.0);
sim.step(delta, &controls);           // 调度层自动切固定子步
let state = sim.backend().state();    // 读位姿/车轮状态
```

## 边界（不放入）

- AC 字段名与解析（`assets/physics_mapping.rs`）；
- 赛道碰撞与多车世界（`world/`，Jolt 侧）；
- 文件 IO：核心不读文件，输入由上层构造好结构体传入；
- Godot 类型。

## 验证

```bash
cargo test -p vettura-physics
cargo build -p vettura-physics --features chrono    # 需 third_party/chrono（见其 README）
```

确定性：同输入 ⇒ 位级一致状态（`identical_inputs_produce_bit_identical_states`）。

## 相关文档

- [`docs/physics.md`](../docs/physics.md)：子系统全覆盖清单、设计取舍、Chrono 映射边界
- [`third_party/README.md`](../third_party/README.md)：Chrono 版本锁定与拉取方式
