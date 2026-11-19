#![cfg(feature = "chrono")]
//! Integration tests for the Chrono backend.
//!
//! These require the `chrono` feature and a linked Chrono library.
//! When Chrono is not linked, tests return early (they don't fail).

use vettura_physics::backends::chrono::{ChronoBackend, ChronoError};
use vettura_physics::{Controls, Pose, VehicleBackend, VehicleSpec};

/// Chrono 没链接时（stub）返回 `NotLinked`；链接了就构造成功。
#[test]
fn try_new_reports_a_status_that_callers_can_handle() {
    match ChronoBackend::try_new(VehicleSpec::default()) {
        Ok(backend) => assert!(backend.is_linked()),
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
fn run(
    backend: &mut ChronoBackend,
    steps: usize,
    controls: &Controls,
) -> vettura_physics::VehicleState {
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
fn state_bits(state: &vettura_physics::VehicleState) -> Vec<u32> {
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
