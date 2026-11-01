//! `CarFolder::vehicle_spec` mapping, against synthetic car data.
//!
//! Covers the full ini/lut set, the FWD/RWD/AWD drivetrain layouts, and the
//! "half-supported mod still drives" fallback behaviour.

mod common;

use common::car_folder;
use vettura_assets::CarFolder;
use vettura_physics::{DrivetrainLayout, VehicleSpec, Wheel};

/// A complete, self-consistent synthetic car.
///
/// Index order matters: [`DRIVETRAIN_INDEX`] addresses `drivetrain.ini`.
const DRIVETRAIN_INDEX: usize = 4;

fn full_car() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "car.ini",
            "[INFO]\nSCREEN_NAME=TestCar\n[BASIC]\nTOTALMASS=1400\n",
        ),
        (
            "suspensions.ini",
            "[BASIC]\n\
             WHEELBASE=2.8\n\
             [FRONT]\n\
             TRACK=1.6\n\
             BASEY=0.1\n\
             SPRING_RATE=120000\n\
             DAMP_BUMP=3000\n\
             DAMP_REBOUND=4500\n\
             PACKER_RANGE=0.1\n\
             [REAR]\n\
             TRACK=1.5\n\
             SPRING_RATE=100000\n\
             [ARB]\n\
             FRONT=8000\n\
             REAR=6000\n",
        ),
        (
            "tyres.ini",
            "[FRONT]\n\
             RADIUS=0.35\n\
             WIDTH=0.28\n\
             DY0=1.2\n\
             [REAR]\n\
             RADIUS=0.36\n\
             WIDTH=0.30\n",
        ),
        ("engine.ini", "[ENGINE_DATA]\nMINIMUM=950\nLIMITER=8500\n"),
        (
            "drivetrain.ini",
            "[TRACTION]\n\
             TYPE=FWD\n\
             [GEARS]\n\
             COUNT=3\n\
             GEAR_1=3.5\n\
             GEAR_2=2.0\n\
             GEAR_3=1.4\n\
             FINAL=3.9\n",
        ),
        ("aero.ini", "[WING_0]\nCHORD=0.6\nSPAN=1.8\nCD_GAIN=0.32\n"),
        ("power.lut", "0|100\n1000|300\n8000|320\n"),
    ]
}

#[test]
fn maps_a_full_ini_and_lut_set() {
    let fixture = car_folder(&full_car());
    let spec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");

    assert_eq!(spec.name, "TestCar");
    assert_eq!(spec.mass_kg, 1400.0);
    assert_eq!(spec.wheelbase_m, 2.8);

    let fl = &spec.wheels[Wheel::FrontLeft.index()];
    let fr = &spec.wheels[Wheel::FrontRight.index()];
    let rl = &spec.wheels[Wheel::RearLeft.index()];
    let rr = &spec.wheels[Wheel::RearRight.index()];

    // FWD：前轮驱动；只有前轮转向。
    assert!(fl.driven && fr.driven);
    assert!(!rl.driven && !rr.driven);
    assert!(fl.steerable && fr.steerable);
    assert!(!rl.steerable && !rr.steerable);
    assert_eq!(spec.drivetrain.layout, DrivetrainLayout::Fwd);

    // 轮位：track/2 → x，radius → y，±wheelbase/2 → z。
    assert_eq!(fl.position.x, -0.8);
    assert_eq!(fl.position.y, 0.35);
    assert_eq!(fl.position.z, -1.4);
    assert_eq!(rr.position.x, 0.75);
    assert_eq!(rr.position.y, 0.36);
    assert_eq!(rr.position.z, 1.4);

    // 悬挂来自 FRONT/REAR 段；ARB 前 8000 经 0.5 系数落到每个前轮。
    assert_eq!(fl.suspension.spring_rate_n_per_m, 120_000.0);
    assert_eq!(rl.suspension.spring_rate_n_per_m, 100_000.0);
    assert_eq!(fl.suspension.travel_m, 0.1);
    assert_eq!(fl.suspension.damper_bump_ns_per_m, 3000.0);
    assert_eq!(fl.suspension.anti_roll_n_per_m, 4000.0);

    // 发动机：idle/redline/final drive/齿比；峰值扭矩取 lut 的最大 y。
    assert_eq!(spec.engine.idle_rpm, 950.0);
    assert_eq!(spec.engine.redline_rpm, 8500.0);
    assert_eq!(spec.engine.final_drive, 3.9);
    assert_eq!(spec.engine.gear_ratios, vec![3.5, 2.0, 1.4]);
    assert_eq!(spec.engine.peak_torque_nm, 320.0);

    // 气动：CHORD×SPAN，CD_GAIN 直接映射。
    assert_eq!(spec.frontal_area_m2, 0.6 * 1.8);
    assert_eq!(spec.drag_coefficient, 0.32);

    // 轮胎：FRONT.DY0 优先。
    assert_eq!(spec.tyres.peak_friction, 1.2);

    // CG 高度 = 前轮半径 + BASEY。
    assert_eq!(spec.cg_height_m, 0.35 + 0.1);
}

#[test]
fn falls_back_to_defaults_without_any_data_files() {
    // 只剩 data/ 的残包：全部走保守默认，不 panic。
    let fixture = car_folder(&[]);
    let spec = CarFolder::open(fixture.path())
        .expect("empty data dir should still open")
        .vehicle_spec()
        .expect("mapping defaults should succeed");

    let default = VehicleSpec::default();
    assert_eq!(spec.mass_kg, default.mass_kg);
    assert_eq!(spec.wheelbase_m, default.wheelbase_m);
    assert_eq!(spec.drivetrain.layout, DrivetrainLayout::Rwd);
    assert_eq!(
        spec.engine.gear_ratios.len(),
        default.engine.gear_ratios.len()
    );
    assert_eq!(spec.frontal_area_m2, 1.8);
}

#[test]
fn drivetrain_layout_drives_the_right_wheels() {
    for (traction, layout, front_driven, rear_driven) in [
        ("RWD", DrivetrainLayout::Rwd, false, true),
        ("FWD", DrivetrainLayout::Fwd, true, false),
        ("AWD", DrivetrainLayout::Awd, true, true),
        ("4WD", DrivetrainLayout::Awd, true, true),
        // 无法识别的值退化为 RWD。
        ("banana", DrivetrainLayout::Rwd, false, true),
    ] {
        let mut files = full_car();
        let traction_ini = format!("[TRACTION]\nTYPE={traction}\n");
        files[DRIVETRAIN_INDEX].1 = &traction_ini;
        let fixture = car_folder(&files);

        let spec = CarFolder::open(fixture.path())
            .expect("synthetic car folder should open")
            .vehicle_spec()
            .expect("mapping should succeed");

        assert_eq!(spec.drivetrain.layout, layout, "{traction}");
        assert_eq!(
            spec.wheels[Wheel::FrontLeft.index()].driven,
            front_driven,
            "{traction} front"
        );
        assert_eq!(
            spec.wheels[Wheel::RearLeft.index()].driven,
            rear_driven,
            "{traction} rear"
        );
    }
}

#[test]
fn mass_and_wheelbase_can_come_from_car_ini() {
    let files: Vec<(&str, &str)> = vec![("car.ini", "[BASIC]\nTOTALMASS=1400\nWHEELBASE=3.1\n")];
    let fixture = car_folder(&files);
    let spec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("mapping should succeed");

    assert_eq!(spec.mass_kg, 1400.0);
    assert_eq!(spec.wheelbase_m, 3.1);
}

#[test]
fn gear_ratios_require_a_complete_list() {
    // GEAR_3 缺失：read_gear_ratios 返回 None → 默认齿比保留。
    let mut files = full_car();
    files[DRIVETRAIN_INDEX].1 = "[GEARS]\nCOUNT=3\nGEAR_1=3.5\nGEAR_2=2.0\nFINAL=3.9\n";
    let fixture = car_folder(&files);
    let spec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("mapping should succeed");

    assert_eq!(
        spec.engine.gear_ratios,
        VehicleSpec::default().engine.gear_ratios
    );
    assert_eq!(spec.engine.final_drive, 3.9);
}
