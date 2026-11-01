//! Chrono::Vehicle backend.
//!
//! 该模块取代了原先独立的 `vettura-physics-chrono` crate：拆分得过细，而 Chrono
//! 的 C++ 源码统一放在仓库根的 `third_party/chrono`，由本 crate 的 `chrono`
//! feature 门控链接。
//!
//! # Glue 层：C ABI，不是 cxx
//!
//! 边界是 `cpp/chrono_bridge.h` 声明的 `extern "C"` 接口，Rust 侧手写
//! `#[repr(C)]` 镜像（[`ffi`]，带布局测试）。选 C ABI 的理由：只要能跨界的全是
//! Pod 加一个不透明句柄，cxx 的代码生成买不到安全；而 C ABI 是唯一所有编译器
//! 都共识的契约，Chrono 可以用任意 C++ 工具链编出来。
//!
//! # C++ 侧职责（`cpp/`）
//!
//! - [`bridge.cpp`](../../../cpp/bridge.cpp)：`WheeledVehicle` + `FlatTerrain`
//!   + 固定步长步进 + 状态回填；
//! - [`vettura_vehicle_spec.cpp`](../../../cpp/vettura_vehicle_spec.cpp)：
//!   spec → Chrono JSON（Chrono 的 wheeled vehicle 是 JSON 驱动的），纯标准
//!   C++，可脱离 Chrono 单测；
//! - [`chrono_bridge_stub.cpp`](../../../cpp/chrono_bridge_stub.cpp)：Chrono
//!   没就位时的替代实现，返回 `NotLinked`，让 workspace 依然能编译/测试。
//!
//! # 只接 vehicle 部分
//!
//! 我们只做车辆动力学：平地地形 (`FlatTerrain`)、底盘/悬架/轮胎/传动/转向、
//! 驻车制动。SCM/Granular/FEA 地形、传感器、可视化都不接。
//!
//! # 线程契约
//!
//! Chrono 以 `ENABLE_OPENMP=OFF` 构建，bridge 侧无全局状态；`ChronoBackend`
//! 因此是 `Send`，但**不是** `Sync`：多线程同时 step 同一个实例没有意义也没有
//! 实现。`Simulation` 单持有者，正好匹配。

pub mod ffi;

use thiserror::Error;

use crate::{Controls, Pose, VehicleBackend, VehicleSpec, VehicleState};

pub use ffi::{
    VetturaChronoControls, VetturaChronoVehicle, to_ffi_controls, to_ffi_pose, to_ffi_spec,
};

#[derive(Debug, Error)]
pub enum ChronoError {
    #[error("Chrono::Vehicle is not linked in this build; see third_party/README.md")]
    NotLinked,

    #[error("invalid vehicle specification: {0}")]
    InvalidSpec(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("solver error inside Chrono::Vehicle: {0}")]
    Solver(String),
}

/// Chrono::Vehicle backend, driven through the C ABI in `cpp/chrono_bridge.h`.
///
/// `try_new` 返回 [`ChronoError::NotLinked`] 时（Chrono 源码没就位，或链接到
/// 的是 stub），调用方应回退到 [`crate::backends::SimpleBackend`]。
#[derive(Debug)]
pub struct ChronoBackend {
    /// Opaque handle owned by the bridge; null until `create` succeeds.
    handle: Option<*mut VetturaChronoVehicle>,
    spec: VehicleSpec,
    state: VehicleState,
    last_error_message: String,
}

impl ChronoBackend {
    /// Build a Chrono vehicle from a backend-neutral spec.
    ///
    /// 失败时保留 `NotLinked` / `InvalidSpec` 的区分，调用方（或日志）可以据此
    /// 决定回退策略；`vettura_chrono_last_error()` 的原文通过
    /// [`ChronoBackend::last_error`] 暴露。
    pub fn try_new(spec: VehicleSpec) -> Result<Self, ChronoError> {
        // 创建前先校验 ABI 版本：跨版本链接一律拒绝，比崩溃好。
        // SAFETY: 纯读取，不依赖指针参数。
        let abi = unsafe { ffi::vettura_chrono_abi_version() };
        if abi != ffi::VETTURA_CHRONO_ABI_VERSION {
            return Err(ChronoError::InvalidArgument(format!(
                "bridge ABI version {abi} != expected {}",
                ffi::VETTURA_CHRONO_ABI_VERSION
            )));
        }

        let mut handle: *mut VetturaChronoVehicle = std::ptr::null_mut();
        let ffi_spec = ffi::to_ffi_spec(&spec);
        // SAFETY: spec 是栈上 Pod，生命周期覆盖本次调用；handle 由 bridge 分配。
        let status = unsafe { ffi::vettura_chrono_create(&ffi_spec, &mut handle as *mut *mut _) };
        let message = ffi::last_error();
        match status {
            ffi::VETTURA_CHRONO_OK => Ok(Self {
                handle: Some(handle),
                spec,
                state: VehicleState::default(),
                last_error_message: message,
            }),
            ffi::VETTURA_CHRONO_ERR_NOT_LINKED => Err(ChronoError::NotLinked),
            ffi::VETTURA_CHRONO_ERR_INVALID_SPEC => Err(ChronoError::InvalidSpec(message)),
            ffi::VETTURA_CHRONO_ERR_INVALID_ARGUMENT => Err(ChronoError::InvalidArgument(message)),
            other => Err(ChronoError::Solver(format!("status {other}: {message}"))),
        }
    }

    /// Bridge 侧最后一次错误的原文（用于诊断；`create` 失败时也有效）。
    pub fn last_error(&self) -> &str {
        &self.last_error_message
    }

    /// ABI/编译器/flags 标识（golden replay 测试用它定位 divergence）。
    pub fn bridge_build_id() -> String {
        ffi::build_id()
    }
}

impl VehicleBackend for ChronoBackend {
    fn spec(&self) -> &VehicleSpec {
        &self.spec
    }

    fn state(&self) -> &VehicleState {
        &self.state
    }

    fn reset(&mut self, pose: Pose) {
        let Some(handle) = self.handle else {
            // 没链接时保持 SimpleBackend 相同语义：原地归零。
            self.state = VehicleState::default();
            self.state.position = pose.position;
            self.state.orientation = pose.orientation;
            return;
        };
        let ffi_pose = ffi::to_ffi_pose(&pose);
        // SAFETY: handle 由本实例持有；pose 生命周期覆盖调用。
        let status = unsafe { ffi::vettura_chrono_reset(handle, &ffi_pose) };
        debug_assert_eq!(
            status,
            ffi::VETTURA_CHRONO_OK,
            "chrono reset failed: {}",
            ffi::last_error()
        );
    }

    fn step(&mut self, dt: f32, controls: &Controls) {
        let Some(handle) = self.handle else {
            self.state = VehicleState::default();
            return;
        };
        let mut ffi_state = unsafe { std::mem::zeroed::<ffi::VetturaChronoState>() };
        // SAFETY: handle 由本实例持有；controls 在调用期间有效；dt 已由
        // Simulation 钳到合理量级。
        let ffi_controls = ffi::to_ffi_controls(controls);
        let status = unsafe {
            ffi::vettura_chrono_step(handle, dt as f64, &ffi_controls);
            // bridge 的 read_state 需要非 const 句柄（ReportTireForce 不是 const）。
            ffi::vettura_chrono_read_state(handle, &mut ffi_state)
        };
        debug_assert_eq!(
            status,
            ffi::VETTURA_CHRONO_OK,
            "chrono read_state failed: {}",
            ffi::last_error()
        );
        if status == ffi::VETTURA_CHRONO_OK {
            self.state = ffi::from_ffi_state(&ffi_state);
        }
    }
}

impl Drop for ChronoBackend {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: 句柄只归还给 bridge 一次；destroy(NULL) 是空操作。
            unsafe { ffi::vettura_chrono_destroy(handle) };
        }
    }
}

// SAFETY: bridge 无全局状态、Chrono 以单线程构建；句柄随本实例一起被 drop，
// 因此可以跨线程移动。多线程并发 step 同一实例是被禁止的（无 Sync）。
unsafe impl Send for ChronoBackend {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrono 没链接时（stub）返回 `NotLinked`；链接了就构造成功。
    #[test]
    fn try_new_reports_a_status_that_callers_can_handle() {
        match ChronoBackend::try_new(VehicleSpec::default()) {
            Ok(backend) => assert!(backend.handle.is_some()),
            Err(ChronoError::NotLinked) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn bridge_build_id_is_never_empty() {
        let id = ChronoBackend::bridge_build_id();
        assert!(
            !id.is_empty(),
            "build_id must be usable for divergence triage"
        );
    }

    /// 取一个真的连上 Chrono 的后端；stub 链接时返回 `None` 让测试跳过。
    fn linked_backend() -> Option<ChronoBackend> {
        match ChronoBackend::try_new(VehicleSpec::default()) {
            Ok(backend) => Some(backend),
            Err(ChronoError::NotLinked) => None,
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    const DT: f32 = 1.0 / 600.0;

    /// 跑 `steps` 步，返回最后的状态。
    fn run(backend: &mut ChronoBackend, steps: usize, controls: &Controls) -> VehicleState {
        for _ in 0..steps {
            backend.step(DT, controls);
        }
        *backend.state()
    }

    /// 端到端物理健全性：车落地、四轮受载、油门能推动车前进。
    ///
    /// 这是整条链路的回归测试：spec → Chrono JSON → WheeledVehicle → VehicleState。
    #[test]
    fn vehicle_settles_on_the_ground_and_drives_forward_on_throttle() {
        let Some(mut backend) = linked_backend() else {
            return;
        };

        // 2 秒静置：重力把车压到悬架上。
        backend.reset(Pose::default());
        let settled = run(&mut backend, 1200, &Controls::default());

        assert!(
            (0.3..1.2).contains(&settled.position.y),
            "COG should rest at a sane height, got y = {}",
            settled.position.y
        );
        for (index, wheel) in settled.wheels.iter().enumerate() {
            assert!(wheel.contact, "wheel {index} should be in contact");
            assert!(
                wheel.load_n > 1000.0,
                "wheel {index} should carry load, got {} N",
                wheel.load_n
            );
        }
        let total_load: f32 = settled.wheels.iter().map(|w| w.load_n).sum();
        let expected = 1200.0 * 9.81;
        assert!(
            (total_load - expected).abs() < 0.1 * expected,
            "total wheel load {total_load} should match the vehicle weight {expected}"
        );

        // 1 秒全油门：车必须往前动，发动机转速要超过怠速。
        let throttle = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let driven = run(&mut backend, 600, &throttle);
        assert!(
            driven.speed_mps > 0.5,
            "full throttle should accelerate the car, speed = {} m/s",
            driven.speed_mps
        );
        assert!(
            driven.engine_rpm > 1000.0,
            "engine should spin above idle under load, rpm = {}",
            driven.engine_rpm
        );
        assert!(
            driven.gear >= 1,
            "automatic gearbox should have selected a forward gear"
        );
        // 车轮转角必须在涨（可视化/里程表依赖这个积分）。
        assert!(
            driven.wheels.iter().any(|w| w.rotation > 0.0),
            "wheel rotation should accumulate while driving"
        );
        // 状态必须有限：NaN/Inf 都说明坐标转换或状态回填断了。
        assert!(driven.position.x.is_finite() && driven.position.y.is_finite());
        assert!(driven.orientation.to_array().iter().all(|v| v.is_finite()));
        // 车确实沿 -Z（物理帧前方）走了。
        assert!(
            driven.position.z < -1.0,
            "car should have moved forward (-Z), position = {:?}",
            driven.position
        );
    }

    /// 刹车：从行驶中全力制动 1 秒，速度必须明显下降。
    #[test]
    fn braking_slows_the_vehicle() {
        let Some(mut backend) = linked_backend() else {
            return;
        };
        backend.reset(Pose::default());
        let throttle = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let cruising = run(&mut backend, 1200, &throttle);
        assert!(cruising.speed_mps > 1.0);

        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        let braked = run(&mut backend, 600, &brake);
        assert!(
            braked.speed_mps < cruising.speed_mps - 1.0,
            "braking should slow the car: {} -> {} m/s",
            cruising.speed_mps,
            braked.speed_mps
        );
    }

    /// 转向：直线行驶中打方向，航向角必须变化（左转 = yaw 增大）。
    #[test]
    fn steering_changes_heading() {
        let Some(mut backend) = linked_backend() else {
            return;
        };
        backend.reset(Pose::default());
        let throttle = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let straight = run(&mut backend, 900, &throttle);
        // 物理帧 +Y 上：yaw 增大 = 向左转（与 Controls::steering 正方向一致）。
        let yaw_before = straight.orientation.to_euler(glam::EulerRot::YXZ).0;

        let turning = Controls {
            throttle: 0.5,
            steering: 0.5,
            ..Controls::default()
        };
        let turned = run(&mut backend, 900, &turning);
        let yaw_after = turned.orientation.to_euler(glam::EulerRot::YXZ).0;

        assert!(
            yaw_after > yaw_before + 0.05,
            "steering left should increase yaw: {yaw_before} -> {yaw_after}"
        );
        assert!(
            turned.wheels[0].steer_angle > 0.01,
            "front wheels should report a positive (left) steer angle, got {}",
            turned.wheels[0].steer_angle
        );
    }

    /// 同进程内的确定性：同一 spec + 同一输入序列，两次运行必须逐位相同。
    ///
    /// 跨编译器/跨机器的逐位一致性靠 docs/build.md §8.2 的浮点 flag 保证，
    /// 这个测试只守住「后端自身没有引入不确定性」（时间戳、随机数、线程等）。
    #[test]
    fn identical_inputs_produce_bit_identical_states() {
        let (Some(mut a), Some(mut b)) = (linked_backend(), linked_backend()) else {
            return;
        };
        a.reset(Pose::default());
        b.reset(Pose::default());

        // 一段混合输入：怠速 → 加速 → 左转 → 刹车。
        let phases = [
            (120, Controls::default()),
            (
                240,
                Controls {
                    throttle: 0.8,
                    ..Controls::default()
                },
            ),
            (
                240,
                Controls {
                    throttle: 0.4,
                    steering: 0.3,
                    ..Controls::default()
                },
            ),
            (
                240,
                Controls {
                    brake: 0.7,
                    ..Controls::default()
                },
            ),
        ];
        for (steps, controls) in phases {
            let sa = run(&mut a, steps, &controls);
            let sb = run(&mut b, steps, &controls);
            assert_eq!(state_bits(&sa), state_bits(&sb), "states diverged");
        }
    }

    /// 跨编译器对拍的钩子：把固定场景的状态位模式压成 FNV-1a 哈希。
    ///
    /// 同一台机器上用两个编译器各跑一次：
    ///
    /// ```text
    /// cargo test -p vettura-physics --features chrono golden_replay -- --nocapture
    /// # 记下 printed hash，然后在另一个编译器下：
    /// VETTURA_GOLDEN_HASH=<hash> cargo test -p vettura-physics --features chrono golden_replay
    /// ```
    ///
    /// 哈希依赖 Chrono 版本和机器 ISA（Chrono 自己带 `-march=native`），所以不写死
    /// 在仓库里；它的用途是「同一台机器、不同编译器」的逐位对拍。
    #[test]
    fn golden_replay_hash_is_stable() {
        let Some(mut backend) = linked_backend() else {
            return;
        };
        backend.reset(Pose::default());

        let phases = [
            (600, Controls::default()),
            (
                600,
                Controls {
                    throttle: 0.5,
                    ..Controls::default()
                },
            ),
            (
                600,
                Controls {
                    throttle: 1.0,
                    steering: 0.4,
                    ..Controls::default()
                },
            ),
            (
                600,
                Controls {
                    brake: 0.8,
                    ..Controls::default()
                },
            ),
        ];

        // FNV-1a：实现简单，不引入依赖，跨编译器行为明确。
        fn fnv1a(mut hash: u64, bits: &[u32]) -> u64 {
            for value in bits {
                for byte in value.to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x1000_0000_01b3);
                }
            }
            hash
        }

        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for (steps, controls) in phases {
            for _ in 0..steps {
                backend.step(DT, &controls);
                hash = fnv1a(hash, &state_bits(backend.state()));
            }
        }

        let hash = format!("{hash:016x}");
        match std::env::var("VETTURA_GOLDEN_HASH") {
            Ok(expected) => assert_eq!(
                hash,
                expected,
                "golden replay hash diverged; build id = {}",
                ChronoBackend::bridge_build_id()
            ),
            Err(_) => eprintln!("golden replay hash = {hash}"),
        }
    }

    /// 把一个状态拍平成可逐位比较的整数序列。
    fn state_bits(state: &VehicleState) -> Vec<u32> {
        fn push(bits: &mut Vec<u32>, value: f32) {
            bits.push(value.to_bits());
        }
        let mut bits = Vec::new();
        for v in [
            state.position,
            state.linear_velocity,
            state.angular_velocity,
        ] {
            push(&mut bits, v.x);
            push(&mut bits, v.y);
            push(&mut bits, v.z);
        }
        for v in state.orientation.to_array() {
            push(&mut bits, v);
        }
        push(&mut bits, state.speed_mps);
        push(&mut bits, state.engine_rpm);
        bits.push(state.gear as u32);
        for wheel in &state.wheels {
            push(&mut bits, wheel.rotation);
            push(&mut bits, wheel.steer_angle);
            push(&mut bits, wheel.angular_velocity);
            push(&mut bits, wheel.slip_ratio);
            push(&mut bits, wheel.slip_angle);
            push(&mut bits, wheel.load_n);
            bits.push(wheel.contact as u32);
        }
        bits
    }
}
