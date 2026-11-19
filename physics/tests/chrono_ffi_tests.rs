#![cfg(feature = "chrono")]
//! Layout and round-trip tests for the Chrono FFI ABI.
//!
//! These tests ensure the Rust `#[repr(C)]` structs match `cpp/chrono_bridge.h`
//! bit-for-bit. Expected sizes come from `cpp/layout_expected.txt` (generated
//! by `clang` via `cpp/layout_probe.c`).

use vettura_physics::backends::chrono::ffi::{
    VetturaChronoControls, VetturaChronoDrivetrain, VetturaChronoEngine, VetturaChronoPose,
    VetturaChronoQuat, VetturaChronoSpec, VetturaChronoState, VetturaChronoSuspension,
    VetturaChronoTyre, VetturaChronoVec3, VetturaChronoWheel, VetturaChronoWheelState,
    from_ffi_state, to_ffi_controls, to_ffi_pose, to_ffi_spec,
};
use vettura_physics::{Controls, Pose, VehicleSpec};

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
            std::mem::size_of::<VetturaChronoVec3>(),
            std::mem::align_of::<VetturaChronoVec3>()
        ),
        (EXPECTED[0].1, 8)
    );
    assert_eq!(
        (
            std::mem::size_of::<VetturaChronoQuat>(),
            std::mem::align_of::<VetturaChronoQuat>()
        ),
        (EXPECTED[1].1, 8)
    );
    assert_eq!(
        std::mem::size_of::<VetturaChronoSuspension>(),
        EXPECTED[2].1
    );
    assert_eq!(
        (
            std::mem::size_of::<VetturaChronoWheel>(),
            std::mem::align_of::<VetturaChronoWheel>()
        ),
        (EXPECTED[3].1, 8)
    );
    assert_eq!(std::mem::size_of::<VetturaChronoEngine>(), EXPECTED[4].1);
    assert_eq!(
        std::mem::size_of::<VetturaChronoDrivetrain>(),
        EXPECTED[5].1
    );
    assert_eq!(std::mem::size_of::<VetturaChronoTyre>(), EXPECTED[6].1);
    assert_eq!(std::mem::size_of::<VetturaChronoSpec>(), EXPECTED[7].1);
    assert_eq!(std::mem::size_of::<VetturaChronoPose>(), EXPECTED[8].1);
    assert_eq!(std::mem::size_of::<VetturaChronoControls>(), EXPECTED[9].1);
    assert_eq!(
        std::mem::size_of::<VetturaChronoWheelState>(),
        EXPECTED[10].1
    );
    assert_eq!(std::mem::size_of::<VetturaChronoState>(), EXPECTED[11].1);
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
        gear_ratios: [0.0; vettura_physics::backends::chrono::ffi::VETTURA_CHRONO_MAX_GEAR_COUNT],
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
    assert_eq!(ffi.drivetrain.layout, 0); // RWD
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
        ..Controls::default()
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
