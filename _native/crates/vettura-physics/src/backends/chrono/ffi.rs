//! C ABI 声明与 `#[repr(C)]` 镜像 —— `cpp/chrono_bridge.h` 的 Rust 侧。
//!
//! # 为什么不用 bindgen
//!
//! 契约只有一个头文件且字段全是 Pod；手写镜像 + 布局测试比引入 bindgen
//! （protoc/llvm 依赖 + 每次生成）更可控。头文件一改，
//! [`tests::layout_matches_the_c_header`] 立刻失败。
//!
//! # 布局数值从哪来
//!
//! **不是手算的**：`cpp/layout_probe.c` 由 `clang` 编译后直接实测
//! `sizeof` / `offsetof`，输出保存在 `cpp/layout_expected.txt`。下面测试里的
//! 期望值就是那份实测输出。复现命令见 `cpp/layout_probe.c` 头部注释。

use crate::{Controls, Pose, VehicleSpec, VehicleState, WheelState};

pub const VETTURA_CHRONO_OK: i32 = 0;
pub const VETTURA_CHRONO_ERR_INVALID_ARGUMENT: i32 = 1;
pub const VETTURA_CHRONO_ERR_INVALID_SPEC: i32 = 2;
pub const VETTURA_CHRONO_ERR_NOT_LINKED: i32 = 3;
pub const VETTURA_CHRONO_ERR_SOLVER: i32 = 4;

pub const VETTURA_CHRONO_WHEEL_COUNT: usize = 4;
pub const VETTURA_CHRONO_MAX_GEAR_COUNT: usize = 16;

/// ABI revision of `cpp/chrono_bridge.h`; must match the linked library.
pub const VETTURA_CHRONO_ABI_VERSION: i32 = 1;

pub const VETTURA_CHRONO_LAYOUT_RWD: i32 = 0;
pub const VETTURA_CHRONO_LAYOUT_FWD: i32 = 1;
pub const VETTURA_CHRONO_LAYOUT_AWD: i32 = 2;

// 注意：所有 `#[repr(C)]` 结构体的字段顺序必须与 `cpp/chrono_bridge.h` 一致。

/// Mirrors `vettura_chrono_vec3`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoVec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Mirrors `vettura_chrono_quat` (`(x, y, z, w)`).
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoQuat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

/// Mirrors `vettura_chrono_suspension`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoSuspension {
    pub spring_rate_n_per_m: f64,
    pub damper_bump_ns_per_m: f64,
    pub damper_rebound_ns_per_m: f64,
    pub travel_m: f64,
    pub anti_roll_n_per_m: f64,
}

/// Mirrors `vettura_chrono_wheel`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoWheel {
    pub position: VetturaChronoVec3,
    pub radius_m: f64,
    pub width_m: f64,
    pub steerable: i32,
    pub driven: i32,
    pub suspension: VetturaChronoSuspension,
}

/// Mirrors `vettura_chrono_engine`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoEngine {
    pub idle_rpm: f64,
    pub redline_rpm: f64,
    pub peak_torque_nm: f64,
    pub gear_count: i32,
    pub gear_ratios: [f64; VETTURA_CHRONO_MAX_GEAR_COUNT],
    pub final_drive: f64,
}

/// Mirrors `vettura_chrono_drivetrain`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoDrivetrain {
    pub layout: i32,
    pub efficiency: f64,
}

/// Mirrors `vettura_chrono_tyre`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoTyre {
    pub peak_friction: f64,
}

/// Mirrors `vettura_chrono_spec`. `VehicleSpec::name` 是元数据，不过界。
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoSuspensionSpec;

/// Mirrors `vettura_chrono_spec`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoSpec {
    pub mass_kg: f64,
    pub wheelbase_m: f64,
    pub cg_height_m: f64,
    pub frontal_area_m2: f64,
    pub drag_coefficient: f64,
    pub wheels: [VetturaChronoWheel; VETTURA_CHRONO_WHEEL_COUNT],
    pub engine: VetturaChronoEngine,
    pub drivetrain: VetturaChronoDrivetrain,
    pub tyres: VetturaChronoTyre,
}

/// Mirrors `vettura_chrono_pose`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoPose {
    pub position: VetturaChronoVec3,
    pub orientation: VetturaChronoQuat,
}

/// Mirrors `vettura_chrono_controls`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoControls {
    pub throttle: f64,
    pub brake: f64,
    pub clutch: f64,
    pub steering: f64,
    pub handbrake: i32,
    pub shift_up: i32,
    pub shift_down: i32,
}

/// Mirrors `vettura_chrono_wheel_state`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoWheelState {
    pub rotation: f64,
    pub steer_angle: f64,
    pub angular_velocity: f64,
    pub slip_ratio: f64,
    pub slip_angle: f64,
    pub load_n: f64,
    pub contact: i32,
}

/// Mirrors `vettura_chrono_state`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct VetturaChronoState {
    pub position: VetturaChronoVec3,
    pub orientation: VetturaChronoQuat,
    pub linear_velocity: VetturaChronoVec3,
    pub angular_velocity: VetturaChronoVec3,
    pub speed_mps: f64,
    pub wheels: [VetturaChronoWheelState; VETTURA_CHRONO_WHEEL_COUNT],
    pub engine_rpm: f64,
    pub gear: i32,
}

/// Opaque handle to a Chrono vehicle; never dereferenced from Rust.
#[derive(Debug)]
#[repr(C)]
pub struct VetturaChronoVehicle {
    _private: [u8; 0],
}

// SAFETY: bridge 返回的不透明句柄只在本实例内部按单线程契约使用；跨线程安全由
// `ChronoBackend` 的调用方保证（见 `src/backends/chrono.rs` 的线程契约注释）。
unsafe impl Send for VetturaChronoVehicle {}

unsafe extern "C" {
    #[link_name = "vettura_chrono_abi_version"]
    pub fn vettura_chrono_abi_version() -> i32;
    #[link_name = "vettura_chrono_build_id"]
    pub fn vettura_chrono_build_id(out: *mut core::ffi::c_char, out_len: i32);
    #[link_name = "vettura_chrono_create"]
    pub fn vettura_chrono_create(
        spec: *const VetturaChronoSpec,
        out: *mut *mut VetturaChronoVehicle,
    ) -> i32;
    #[link_name = "vettura_chrono_destroy"]
    pub fn vettura_chrono_destroy(vehicle: *mut VetturaChronoVehicle);
    #[link_name = "vettura_chrono_reset"]
    pub fn vettura_chrono_reset(
        vehicle: *mut VetturaChronoVehicle,
        pose: *const VetturaChronoPose,
    ) -> i32;
    #[link_name = "vettura_chrono_step"]
    pub fn vettura_chrono_step(
        vehicle: *mut VetturaChronoVehicle,
        dt: f64,
        controls: *const VetturaChronoControls,
    ) -> i32;
    #[link_name = "vettura_chrono_read_state"]
    pub fn vettura_chrono_read_state(
        vehicle: *const VetturaChronoVehicle,
        out_state: *mut VetturaChronoState,
    ) -> i32;
    #[link_name = "vettura_chrono_last_error"]
    pub fn vettura_chrono_last_error() -> *const core::ffi::c_char;
}

/* ---------------- Rust 类型 ↔ ABI Pod ---------------- */

use crate::{DrivetrainLayout, SuspensionSpec, WheelSpec};

fn vec3_from_glam(v: glam::Vec3) -> VetturaChronoVec3 {
    VetturaChronoVec3 {
        x: v.x as f64,
        y: v.y as f64,
        z: v.z as f64,
    }
}

fn vec3_to_glam(v: VetturaChronoVec3) -> glam::Vec3 {
    glam::Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

fn quat_from_glam(q: glam::Quat) -> VetturaChronoQuat {
    VetturaChronoQuat {
        x: q.x as f64,
        y: q.y as f64,
        z: q.z as f64,
        w: q.w as f64,
    }
}

fn quat_to_glam(q: VetturaChronoQuat) -> glam::Quat {
    glam::Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32)
}

/// Converts a backend-neutral [`VehicleSpec`] into the C ABI struct.
///
/// `f32` → `f64`：C++ 侧只见 `double`（编译器的浮点契约，见
/// `cpp/chrono_bridge.h` 的 determinism 规则）。
pub fn to_ffi_spec(spec: &VehicleSpec) -> VetturaChronoSpec {
    let to_ffi_wheel = |w: &WheelSpec| VetturaChronoWheel {
        position: vec3_from_glam(w.position),
        radius_m: w.radius_m as f64,
        width_m: w.width_m as f64,
        steerable: i32::from(w.steerable),
        driven: i32::from(w.driven),
        suspension: to_ffi_suspension(&w.suspension),
    };

    VetturaChronoSpec {
        mass_kg: spec.mass_kg as f64,
        wheelbase_m: spec.wheelbase_m as f64,
        cg_height_m: spec.cg_height_m as f64,
        frontal_area_m2: spec.frontal_area_m2 as f64,
        drag_coefficient: spec.drag_coefficient as f64,
        wheels: [
            to_ffi_wheel(&spec.wheels[crate::Wheel::FrontLeft.index()]),
            to_ffi_wheel(&spec.wheels[crate::Wheel::FrontRight.index()]),
            to_ffi_wheel(&spec.wheels[crate::Wheel::RearLeft.index()]),
            to_ffi_wheel(&spec.wheels[crate::Wheel::RearRight.index()]),
        ],
        engine: VetturaChronoEngine {
            idle_rpm: spec.engine.idle_rpm as f64,
            redline_rpm: spec.engine.redline_rpm as f64,
            peak_torque_nm: spec.engine.peak_torque_nm as f64,
            gear_count: spec.engine.gear_ratios.len() as i32,
            gear_ratios: {
                let mut ratios = [0.0f64; VETTURA_CHRONO_MAX_GEAR_COUNT];
                let count = spec
                    .engine
                    .gear_ratios
                    .len()
                    .min(VETTURA_CHRONO_MAX_GEAR_COUNT);
                for (dst, src) in ratios.iter_mut().zip(spec.engine.gear_ratios.iter()) {
                    *dst = *src as f64;
                }
                debug_assert_eq!(
                    count,
                    spec.engine.gear_ratios.len(),
                    "gear ratios exceed the ABI bound"
                );
                ratios
            },
            final_drive: spec.engine.final_drive as f64,
        },
        drivetrain: VetturaChronoDrivetrain {
            layout: match spec.drivetrain.layout {
                DrivetrainLayout::Rwd => VETTURA_CHRONO_LAYOUT_RWD,
                DrivetrainLayout::Fwd => VETTURA_CHRONO_LAYOUT_FWD,
                DrivetrainLayout::Awd => VETTURA_CHRONO_LAYOUT_AWD,
            },
            efficiency: spec.drivetrain.efficiency as f64,
        },
        tyres: VetturaChronoTyre {
            peak_friction: spec.tyres.peak_friction as f64,
        },
    }
}

fn to_ffi_suspension(s: &SuspensionSpec) -> VetturaChronoSuspension {
    VetturaChronoSuspension {
        spring_rate_n_per_m: s.spring_rate_n_per_m as f64,
        damper_bump_ns_per_m: s.damper_bump_ns_per_m as f64,
        damper_rebound_ns_per_m: s.damper_rebound_ns_per_m as f64,
        travel_m: s.travel_m as f64,
        anti_roll_n_per_m: s.anti_roll_n_per_m as f64,
    }
}

pub fn to_ffi_pose(pose: &Pose) -> VetturaChronoPose {
    VetturaChronoPose {
        position: vec3_from_glam(pose.position),
        orientation: quat_from_glam(pose.orientation),
    }
}

pub fn to_ffi_controls(controls: &Controls) -> VetturaChronoControls {
    VetturaChronoControls {
        throttle: controls.throttle as f64,
        brake: controls.brake as f64,
        clutch: controls.clutch as f64,
        steering: controls.steering as f64,
        handbrake: i32::from(controls.handbrake),
        shift_up: i32::from(controls.shift_up),
        shift_down: i32::from(controls.shift_down),
    }
}

pub fn from_ffi_state(state: &VetturaChronoState) -> VehicleState {
    let to_wheel = |w: &VetturaChronoWheelState| WheelState {
        rotation: w.rotation as f32,
        steer_angle: w.steer_angle as f32,
        angular_velocity: w.angular_velocity as f32,
        slip_ratio: w.slip_ratio as f32,
        slip_angle: w.slip_angle as f32,
        load_n: w.load_n as f32,
        contact: w.contact != 0,
    };
    let mut wheels = [WheelState::default(); VETTURA_CHRONO_WHEEL_COUNT];
    // Chrono 的车轮顺序与 `Wheel::ALL` 相同（FL, FR, RL, RR）：bridge 的
    // read_state 就按这个索引写入。
    for (dst, src) in wheels.iter_mut().zip(state.wheels.iter()) {
        *dst = to_wheel(src);
    }

    VehicleState {
        position: vec3_to_glam(state.position),
        orientation: quat_to_glam(state.orientation),
        linear_velocity: vec3_to_glam(state.linear_velocity),
        angular_velocity: vec3_to_glam(state.angular_velocity),
        speed_mps: state.speed_mps as f32,
        wheels,
        engine_rpm: state.engine_rpm as f32,
        gear: state.gear,
    }
}

/// The last error message, as a Rust `String` (NULL → empty).
pub fn last_error() -> String {
    // SAFETY: bridge 保证指针在返回后、下一次 bridge 调用前有效。
    unsafe {
        let ptr = vettura_chrono_last_error();
        if ptr.is_null() {
            return String::new();
        }
        core::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    }
}

/// `build_id()` 的安全包装：固定 256 字节缓冲。
pub fn build_id() -> String {
    let mut buf = [0i8; 256];
    unsafe {
        vettura_chrono_build_id(buf.as_mut_ptr(), buf.len() as i32);
    }
    let bytes: Vec<u8> = buf.iter().map(|&b| b as u8).collect();
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sizeof / offsetof 的期望值 —— 来自 `cpp/layout_expected.txt`。
    /// 生成命令见 `cpp/layout_probe.c` 头部注释。
    const EXPECTED: &[(&str, usize)] = &[
        ("VetturaChronoVec3", 24),
        ("VetturaChronoQuat", 32),
        ("VetturaChronoSuspension", 40),
        ("VetturaChronoWheel", 88),
        ("VetturaChronoEngine", 168),
        ("VetturaChronoDrivetrain", 16),
        ("VetturaChronoTyre", 8),
        ("VetturaChronoSpec", 584),
        ("VetturaChronoPose", 56),
        ("VetturaChronoControls", 48),
        ("VetturaChronoWheelState", 56),
        ("VetturaChronoState", 352),
    ];

    #[test]
    fn layout_matches_the_c_header() {
        assert_eq!(
            (
                size_of::<VetturaChronoVec3>(),
                align_of::<VetturaChronoVec3>()
            ),
            (EXPECTED[0].1, 8)
        );
        assert_eq!(
            (
                size_of::<VetturaChronoQuat>(),
                align_of::<VetturaChronoQuat>()
            ),
            (EXPECTED[1].1, 8)
        );
        assert_eq!(size_of::<VetturaChronoSuspension>(), EXPECTED[2].1);
        assert_eq!(
            (
                size_of::<VetturaChronoWheel>(),
                align_of::<VetturaChronoWheel>()
            ),
            (EXPECTED[3].1, 8)
        );
        assert_eq!(size_of::<VetturaChronoEngine>(), EXPECTED[4].1);
        assert_eq!(size_of::<VetturaChronoDrivetrain>(), EXPECTED[5].1);
        assert_eq!(size_of::<VetturaChronoTyre>(), EXPECTED[6].1);
        assert_eq!(size_of::<VetturaChronoSpec>(), EXPECTED[7].1);
        assert_eq!(size_of::<VetturaChronoPose>(), EXPECTED[8].1);
        assert_eq!(size_of::<VetturaChronoControls>(), EXPECTED[9].1);
        assert_eq!(size_of::<VetturaChronoWheelState>(), EXPECTED[10].1);
        assert_eq!(size_of::<VetturaChronoState>(), EXPECTED[11].1);
    }

    #[test]
    fn field_offsets_match_the_c_header() {
        let wheel = VetturaChronoWheel {
            position: VetturaChronoVec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            radius_m: 4.0,
            width_m: 5.0,
            steerable: 1,
            driven: 0,
            suspension: VetturaChronoSuspension {
                spring_rate_n_per_m: 0.0,
                damper_bump_ns_per_m: 0.0,
                damper_rebound_ns_per_m: 0.0,
                travel_m: 0.0,
                anti_roll_n_per_m: 0.0,
            },
        };
        let base = &wheel as *const VetturaChronoWheel as usize;
        assert_eq!(&wheel.radius_m as *const f64 as usize - base, 24);
        assert_eq!(&wheel.steerable as *const i32 as usize - base, 40);
        assert_eq!(
            &wheel.suspension as *const VetturaChronoSuspension as usize - base,
            48
        );

        let engine = VetturaChronoEngine {
            idle_rpm: 0.0,
            redline_rpm: 0.0,
            peak_torque_nm: 0.0,
            gear_count: 0,
            gear_ratios: [0.0; VETTURA_CHRONO_MAX_GEAR_COUNT],
            final_drive: 0.0,
        };
        let base = &engine as *const VetturaChronoEngine as usize;
        assert_eq!(&engine.gear_count as *const i32 as usize - base, 24);
        assert_eq!(&engine.gear_ratios as *const [f64; 16] as usize - base, 32);
        assert_eq!(&engine.final_drive as *const f64 as usize - base, 160);

        let state = unsafe { std::mem::zeroed::<VetturaChronoState>() };
        let base = &state as *const VetturaChronoState as usize;
        assert_eq!(&state.speed_mps as *const f64 as usize - base, 104);
        assert_eq!(
            &state.wheels as *const [VetturaChronoWheelState; 4] as usize - base,
            112
        );
        assert_eq!(&state.gear as *const i32 as usize - base, 344);
        assert_eq!(
            &state.wheels[0].contact as *const i32 as usize - base,
            112 + 48
        );
    }

    #[test]
    fn spec_round_trip_keeps_values() {
        let spec = VehicleSpec::default();
        let ffi = to_ffi_spec(&spec);
        assert_eq!(ffi.mass_kg, spec.mass_kg as f64);
        assert_eq!(ffi.wheelbase_m, spec.wheelbase_m as f64);
        assert_eq!(
            ffi.engine.gear_count as usize,
            spec.engine.gear_ratios.len()
        );
        assert_eq!(ffi.engine.gear_ratios[5], spec.engine.gear_ratios[5] as f64);
        assert_eq!(ffi.drivetrain.layout, VETTURA_CHRONO_LAYOUT_RWD);
        assert_eq!(ffi.tyres.peak_friction, spec.tyres.peak_friction as f64);
        // 四轮顺序必须与 Wheel::ALL 一致。
        assert_eq!(ffi.wheels[0].steerable, 1);
        assert_eq!(ffi.wheels[2].steerable, 0);
        assert_eq!(ffi.wheels[3].driven, 1);
        assert_eq!(ffi.wheels[0].position.x as f32, spec.wheels[0].position.x);
    }

    #[test]
    fn controls_and_pose_round_trip() {
        let controls = Controls::clamped(Controls {
            throttle: 0.7,
            brake: 0.2,
            clutch: 0.0,
            steering: -0.5,
            handbrake: true,
            shift_up: true,
            shift_down: false,
        });
        let ffi = to_ffi_controls(&controls);
        // f32 → f64 是精确的（每个 f32 都有唯一 f64 表示），但反向断言要小心：
        // 这里校验的是「值域 + 单精度等价」。
        assert_eq!(ffi.throttle, controls.throttle as f64);
        assert!((ffi.throttle - 0.7).abs() < 1e-7);
        assert_eq!(ffi.handbrake, 1);
        assert_eq!(ffi.shift_down, 0);
        assert_eq!(
            from_ffi_state(&unsafe { std::mem::zeroed::<VetturaChronoState>() }).gear,
            0
        );

        let pose = Pose::default();
        let ffi_pose = to_ffi_pose(&pose);
        assert_eq!(ffi_pose.position.y, 0.0);
        assert_eq!(ffi_pose.orientation.w, 1.0);
    }
}
