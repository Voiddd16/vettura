//! Tests for vehicle-level spec types: `VehicleSpec`, `Wheel`, `Pose`.

use glam::{Quat, Vec3};
use vettura_physics::spec::{DrivetrainLayout, EngineSpec, SuspensionSpec, VehicleSpec};

#[test]
fn wheel_indices_and_axle_predicates_agree() {
    for wheel in vettura_physics::spec::Wheel::ALL {
        assert_eq!(wheel.index(), wheel as usize);
        // 索引必须直接可用来寻址 wheels 数组。
        assert_eq!(vettura_physics::spec::Wheel::ALL[wheel.index()], wheel);
    }

    assert!(
        vettura_physics::spec::Wheel::FrontLeft.is_front()
            && vettura_physics::spec::Wheel::FrontLeft.is_left()
    );
    assert!(
        vettura_physics::spec::Wheel::FrontRight.is_front()
            && !vettura_physics::spec::Wheel::FrontRight.is_left()
    );
    assert!(
        !vettura_physics::spec::Wheel::RearLeft.is_front()
            && vettura_physics::spec::Wheel::RearLeft.is_left()
    );
    assert!(
        !vettura_physics::spec::Wheel::RearRight.is_front()
            && !vettura_physics::spec::Wheel::RearRight.is_left()
    );
}

#[test]
fn default_spec_is_symmetric_and_rwd() {
    let spec = VehicleSpec::default();

    assert_eq!(spec.mass_kg, 1200.0);
    assert_eq!(spec.wheels.len(), 4);

    let fl = &spec.wheels[vettura_physics::spec::Wheel::FrontLeft.index()];
    let fr = &spec.wheels[vettura_physics::spec::Wheel::FrontRight.index()];
    let rl = &spec.wheels[vettura_physics::spec::Wheel::RearLeft.index()];
    let rr = &spec.wheels[vettura_physics::spec::Wheel::RearRight.index()];

    // 前后轴距 = wheelbase；左右对称。
    assert_eq!(fl.position.z, -spec.wheelbase_m * 0.5);
    assert_eq!(rr.position.z, spec.wheelbase_m * 0.5);
    assert_eq!(fl.position.x, -fr.position.x);
    assert_eq!(rl.position.x, -rr.position.x);

    // RWD：前轮转向不驱动，后轮驱动不转向。
    assert!(fl.steerable && !fl.driven);
    assert!(fr.steerable && !fr.driven);
    assert!(!rl.steerable && rl.driven);
    assert!(!rr.steerable && rr.driven);
    assert_eq!(spec.drivetrain.layout, DrivetrainLayout::Rwd);

    // 四个轮子共用合理的默认值。
    for wheel in &spec.wheels {
        assert!(wheel.radius_m > 0.0);
        assert_eq!(
            wheel.suspension.spring_rate_n_per_m,
            SuspensionSpec::default().spring_rate_n_per_m
        );
    }
}

#[test]
fn pose_forward_points_along_negative_z_when_identity() {
    let pose = vettura_physics::spec::Pose::default();
    assert_eq!(pose.forward(), Vec3::NEG_Z);

    // 绕 Y 转 90°：-Z 前向变成 -X。
    let turned = vettura_physics::spec::Pose {
        orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        ..vettura_physics::spec::Pose::default()
    };
    let forward = turned.forward();
    assert!(forward.x < -0.99 && forward.z.abs() < 1e-6 && forward.y.abs() < 1e-6);
}

#[test]
fn engine_defaults_cover_a_sane_gearbox() {
    let engine = EngineSpec::default();
    assert_eq!(engine.gear_ratios.len(), 6);
    // 齿比必须逐级递减（每个更高的档位都是减速更少的超比档）。
    for pair in engine.gear_ratios.windows(2) {
        assert!(pair[0] > pair[1], "{:?} must descend", engine.gear_ratios);
    }
    assert!(engine.final_drive > 0.0);
    assert!(engine.idle_rpm < engine.redline_rpm);
}

#[test]
fn default_spec_carries_new_phase_six_sections() {
    let spec = VehicleSpec::default();
    assert!((spec.drivetrain.diff.average_lock() - 1.0).abs() < 1.0e-6);
    assert_eq!(spec.aero.front_bias, 0.4);
    assert_eq!(spec.brakes.front_share, 0.55);
    assert_eq!(spec.aero.downforce_coeff, 0.0);
}

#[test]
fn the_inertia_tensor_falls_back_to_the_collider_when_unusable() {
    let spec = VehicleSpec::default();
    // Nothing authored: the collider tensor, and it is physically sane.
    assert_eq!(spec.effective_inertia(), Vec3::ZERO);
    let derived = spec.collider_inertia();
    assert!(derived.y > 500.0 && derived.z > 500.0, "{derived:?}");
    // Roll is the smallest for a car.
    assert!(derived.x < derived.y, "{derived:?}");

    let authored = VehicleSpec {
        inertia: Vec3::new(400.0, 900.0, 1_100.0),
        ..VehicleSpec::default()
    };
    assert_eq!(
        authored.effective_inertia(),
        Vec3::new(400.0, 900.0, 1_100.0)
    );
    // Nonsense is ignored.
    let broken = VehicleSpec {
        inertia: Vec3::splat(f32::NAN),
        ..VehicleSpec::default()
    };
    assert_eq!(broken.effective_inertia(), Vec3::ZERO);
}

#[test]
fn the_roll_centre_and_steer_lock_are_data_driven() {
    let spec = VehicleSpec::default();
    // The spec carries its own lock; the world uses it unless the tuning
    // overrides it.
    assert_eq!(spec.max_steer_angle_deg, 30.0);
    // A real AC car: 230 degrees at the wheel through a 15:1 ratio.
    let lock = (230.0f32 / 15.0).clamp(3.0, 60.0);
    assert!((lock - 15.333_333).abs() < 1e-4);

    // The roll centre is above the road and well below the centre of mass,
    // which is what makes the correction a reduction in roll moment.
    let suspension = SuspensionSpec::default();
    assert!(suspension.roll_center_height_m > 0.0);
    assert!(suspension.roll_center_height_m < spec.cg_height_m);
    // A mod can author it, within sanity.
    let authored = SuspensionSpec {
        roll_center_height_m: 0.08,
        ..SuspensionSpec::default()
    };
    assert_eq!(authored.roll_center_height_m, 0.08);
}
