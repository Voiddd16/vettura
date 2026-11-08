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
            "[INFO]\nSCREEN_NAME=TestCar\n[BASIC]\nTOTALMASS=1400\n\
             INERTIA=700,900,1200\n",
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
             FZ0=3000\n\
             LS_EXPY=0.80\n\
             LS_EXPX=0.90\n\
             RELAXATION_LENGTH=0.10\n\
             [REAR]\n\
             RADIUS=0.36\n\
             WIDTH=0.30\n\
             FZ0=5000\n\
             LS_EXPY=0.90\n\
             LS_EXPX=0.95\n\
             RELAXATION_LENGTH=0.14\n",
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
    // The full `power.lut` is carried, not just the peak.
    assert_eq!(spec.engine.torque_curve.len(), 3);
    assert_eq!(spec.engine.torque_curve[1].rpm, 1000.0);
    assert_eq!(spec.engine.torque_curve[1].torque_nm, 300.0);
    assert!(!spec.engine.turbo.is_enabled());

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

#[test]
fn maps_aero_brakes_fuel_and_gearbox_data() {
    let files = full_car();
    // 在同一批合成 ini 上追加 aero/brakes/fuel/gearbox 数据。
    let mut files = files;
    files.push((
        "aero.ini",
        "[WING_1]\nCHORD=0.5\nSPAN=1.0\nCL_GAIN=2.0\nPOSITION=-0.4,-0.1,2.3\n\
         [WING_2]\nCHORD=0.4\nSPAN=1.2\nCL_GAIN=1.0\nPOSITION=0.0,-0.2,-1.1\n",
    ));
    files.push((
        "brakes.ini",
        "[DATA]\nMAX_TORQUE=3800\nFRONT_SHARE=0.6\n\
         [TEMPS_FRONT]\nTRANSFER=0.5\nCOOL_TRANSFER=0.002\nTORQUE_K=0.6\n\
         PERF_CURVE=(|0=0.50|100=0.7|300=1.0|500=1|700=1|800=0.95|1000=0.85|)\n",
    ));
    files.push(("fuel_cons.ini", "[FUEL_EVAL]\nKM_PER_LITER=2.5\n"));

    let fixture = car_folder(&files);
    let spec: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");

    // 下压力 = 各翼面 CL_GAIN × CHORD × SPAN 之和；4 号翼面在前，所以前占比 > 0。
    let expected_cl = 2.0 * 0.5 * 1.0 + 1.0 * 0.4 * 1.2;
    assert!(
        (spec.aero.downforce_coeff - expected_cl).abs() < 1.0e-6,
        "{} vs {expected_cl}",
        spec.aero.downforce_coeff
    );
    // 前翼 1.0 / 总计 1.48 → 前占比约 0.68。
    assert!(
        (spec.aero.front_bias - 1.0 / expected_cl).abs() < 1.0e-6,
        "{} vs {}",
        spec.aero.front_bias,
        1.0 / expected_cl
    );
    // Load sensitivity: AC quotes them per axle, the spec keeps the average.
    assert!((spec.tyres.reference_load_n - 4_000.0).abs() < 1e-6);
    assert!((spec.tyres.lateral_load_sensitivity - 0.85).abs() < 1e-6);
    assert!((spec.tyres.longitudinal_load_sensitivity - 0.925).abs() < 1e-6);
    assert!((spec.tyres.relaxation_length_m - 0.12).abs() < 1e-6);
    assert!(spec.aero.ride_height_fade > 0.0);

    // 刹车峰值、前后配平与温度窗口都来自 brakes.ini。
    assert_eq!(spec.brakes.max_torque_nm, 3800.0);
    assert_eq!(spec.brakes.front_share, 0.6);
    assert_eq!(spec.brakes.optimal_c, 300.0);
    assert_eq!(spec.brakes.fade_c, 800.0);
    // 最佳点以下刹车效率低于 1，最佳点等于 1。
    assert!(spec.brakes.efficiency_at(100.0) < 1.0);
    assert!((spec.brakes.efficiency_at(300.0) - 1.0).abs() < 1.0e-6);
    // 高温衰减。
    assert!(spec.brakes.efficiency_at(1000.0) < 0.9);

    // fuel_cons.ini 的每升公里数覆盖默认值。
    assert_eq!(spec.fuel.km_per_liter, 2.5);
    assert!(spec.fuel.full_mass_kg() > 0.0);
}

#[test]
fn drivetrain_ini_fills_diff_and_gearbox() {
    // full_car() 已经带一份 drivetrain.ini；把它替换成带差速器/变速箱数据的版本。
    let files: Vec<(&str, &str)> = full_car()
        .into_iter()
        .filter(|(name, _)| *name != "drivetrain.ini")
        .chain([(
            "drivetrain.ini",
            "[TRACTION]\nTYPE=RWD\n[GEARS]\nCOUNT=2\nGEAR_1=3.0\nGEAR_2=1.5\nFINAL=4.0\n\
             [DIFFERENTIAL]\nPOWER=0.25\nCOAST=0.55\nPRELOAD=15\n\
             [GEARBOX]\nCHANGE_UP_TIME=10\nCHANGE_DN_TIME=20\nINERTIA=0.012\n\
             VALID_SHIFT_RPM_WINDOW=1200\n\
             [CLUTCH]\nMAX_TORQUE=1150\n[AUTOCLUTCH]\nUSE_ON_CHANGES=1\n",
        )])
        .collect();

    let fixture = car_folder(&files);
    let spec: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");

    assert_eq!(spec.drivetrain.diff.power_lock, 0.25);
    assert_eq!(spec.drivetrain.diff.coast_lock, 0.55);
    assert_eq!(spec.drivetrain.diff.preload_nm, 15.0);
    // AC 的毫秒值原样保留，世界层再换算成秒。
    assert_eq!(spec.gearbox.switch_time, 10.0);
    assert_eq!(spec.gearbox.clutch_release_time, 20.0);
    assert_eq!(spec.gearbox.clutch_max_torque_nm, 1150.0);
    assert_eq!(spec.gearbox.inertia, 0.012);
    assert_eq!(spec.gearbox.rev_match_rpm, 1200.0);
    assert!(spec.gearbox.clutch_on_shifts);
    assert!(spec.gearbox.automatic);
}

#[test]
fn suspension_ini_fills_progressive_stops_heave_and_active_actuator() {
    let suspensions = "[BASIC]\nWHEELBASE=2.8\n\
        [FRONT]\nTRACK=1.6\nBASEY=0.1\nSPRING_RATE=120000\n\
        DAMP_BUMP=3000\nDAMP_REBOUND=4500\nPACKER_RANGE=0.1\n\
        PROGRESSIVE_SPRING_RATE=15000\nBUMP_STOP_RATE=200000\n\
        [REAR]\nTRACK=1.5\nSPRING_RATE=100000\nPACKER_RANGE=0.08\n\
        PROGRESSIVE_SPRING_RATE=10000\nBUMP_STOP_RATE=180000\n\
        [HEAVE_FRONT]\nSPRING_RATE=90000\nPROGRESSIVE_SPRING_RATE=5000\n\
        ROD_LENGTH=0.012\nPACKER_RANGE=0.06\nDAMP_BUMP=2000\nDAMP_REBOUND=3000\n\
        [HEAVE_REAR]\nSPRING_RATE=70000\nROD_LENGTH=0.006\n\
        [ACTIVE]\nENABLED=1\nSTIFFNESS=80000\nDAMPING=5000\n\
        MAX_FORCE=12000\nRIDE_HEIGHT=0.01\nRAKE=0.004\n\
        SPEED_DROP=0.03\nSPEED_REF=40\n\
        [ARB]\nFRONT=8000\nREAR=6000\n";
    let files: Vec<(&str, &str)> = full_car()
        .into_iter()
        .map(|(name, contents)| {
            if name == "suspensions.ini" {
                (name, suspensions)
            } else {
                (name, contents)
            }
        })
        .collect();
    let fixture = car_folder(&files);
    let spec: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");

    let front = &spec.wheels[Wheel::FrontLeft.index()].suspension;
    assert_eq!(front.progressive_rate_n_per_m2, 15_000.0);
    assert_eq!(front.bump_stop_rate_n_per_m, 200_000.0);
    assert!(front.bump_stop_travel_m > 0.0);
    let rear = &spec.wheels[Wheel::RearLeft.index()].suspension;
    assert_eq!(rear.progressive_rate_n_per_m2, 10_000.0);

    // Heave springs are axle-level, not duplicated on the corner wheels.
    assert_eq!(spec.heave_springs[0].spring_rate_n_per_m, 90_000.0);
    assert_eq!(spec.heave_springs[0].preload_m, 0.012);
    assert_eq!(spec.heave_springs[0].damper_bump_ns_per_m, 2_000.0);
    assert_eq!(spec.heave_springs[1].spring_rate_n_per_m, 70_000.0);

    // The optional ACTIVE block turns the actuator on and maps every limit.
    assert_eq!(
        spec.active_suspension.mode,
        vettura_physics::ActiveSuspensionMode::Active
    );
    assert_eq!(spec.active_suspension.stiffness_n_per_m, 80_000.0);
    assert_eq!(spec.active_suspension.damping_ns_per_m, 5_000.0);
    assert_eq!(spec.active_suspension.max_force_n, 12_000.0);
    assert_eq!(spec.active_suspension.ride_height_m, 0.01);
    assert_eq!(spec.active_suspension.rake_m, 0.004);
    assert_eq!(spec.active_suspension.speed_drop_m, 0.03);
    assert_eq!(spec.active_suspension.speed_ref_mps, 40.0);
}

#[test]
fn a_car_without_an_active_section_stays_passive() {
    let fixture = car_folder(&full_car());
    let spec: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");
    assert_eq!(
        spec.active_suspension.mode,
        vettura_physics::ActiveSuspensionMode::Off
    );
    assert!(!spec.active_suspension.is_enabled());
    // The fixture has no heave sections either.
    assert_eq!(spec.heave_springs[0].spring_rate_n_per_m, 0.0);
    assert_eq!(spec.heave_springs[1].spring_rate_n_per_m, 0.0);
}

#[test]
fn engine_ini_maps_turbo_and_ignores_inline_comments() {
    let files: Vec<(&str, &str)> = full_car()
        .into_iter()
        .map(|(name, contents)| {
            if name == "engine.ini" {
                (
                    name,
                    "[ENGINE_DATA]\nMINIMUM=950 ; idle\nLIMITER=8500 ; redline\n\
                     [TURBO_0]\nMAX_BOOST=0.8\nWASTEGATE=1.0\nREFERENCE_RPM=3500\n\
                     GAMMA=1.5\nLAG_UP=0.45\nLAG_DN=0.25\nTORQUE_GAIN=0.9\n\
                     [COAST_REF]\nRPM=12700\nTORQUE=130\nNON_LINEARITY=0.5\n",
                )
            } else {
                (name, contents)
            }
        })
        .collect();
    let fixture = car_folder(&files);
    let spec: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");

    // Inline comments must not push these back to the defaults.
    assert_eq!(spec.engine.idle_rpm, 950.0);
    assert_eq!(spec.engine.redline_rpm, 8500.0);

    let turbo = spec.engine.turbo;
    assert!(turbo.is_enabled());
    assert_eq!(turbo.max_boost_bar, 0.8);
    assert_eq!(turbo.wastegate_bar, 1.0);
    assert_eq!(turbo.reference_rpm, 3500.0);
    assert_eq!(turbo.gamma, 1.5);
    assert_eq!(turbo.lag_up_s, 0.45);
    assert_eq!(turbo.lag_down_s, 0.25);
    assert_eq!(turbo.torque_gain_per_bar, 0.9);

    // Engine braking from the coast reference, inline comment and all.
    assert_eq!(spec.engine.coast_torque_nm, 130.0);
    assert_eq!(spec.engine.coast_reference_rpm, 12_700.0);
    assert_eq!(spec.engine.coast_non_linearity, 0.5);
}

#[test]
fn car_ini_inertia_is_used_when_it_is_plausible_and_ignored_when_not() {
    // A plausible tensor (kg·m²) reaches the spec.
    let plausible = car_folder(&full_car());
    let spec: VehicleSpec = CarFolder::open(plausible.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");
    // AC's tuple is (pitch, yaw, roll) in its own axes; the spec labels the
    // same tensor (roll, pitch, yaw).
    assert_eq!(spec.inertia, glam::Vec3::new(1_200.0, 700.0, 900.0));

    // Anything outside the physical band falls back to the collider tensor.
    let nonsense: Vec<(&str, &str)> = full_car()
        .into_iter()
        .map(|(name, contents)| {
            if name == "car.ini" {
                ("car.ini", "[BASIC]\nTOTALMASS=1400\nINERTIA=1.6,0.70,3.8\n")
            } else {
                (name, contents)
            }
        })
        .collect();
    let fixture = car_folder(&nonsense);
    let fallback: VehicleSpec = CarFolder::open(fixture.path())
        .expect("synthetic car folder should open")
        .vehicle_spec()
        .expect("full data should map");
    assert_eq!(fallback.inertia, fallback.collider_inertia());
    // The collider tensor itself is physically sane for a 1400 kg car.
    assert!(fallback.inertia.y > 500.0 && fallback.inertia.z > 500.0);
}
