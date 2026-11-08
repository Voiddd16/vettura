//! Mapping against a real AC car, ignored by default because it needs the
//! local test assets.
//!
//! ```bash
//! cargo test -p vettura-assets --test real_car -- --ignored
//! ```

use std::path::PathBuf;

use vettura_assets::CarFolder;
use vettura_physics::VehicleSpec;

fn force_india_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../vettura/test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_forceindia")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(
                "../vettura/test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_forceindia",
            )
        })
}

#[test]
#[ignore]
fn real_f1_car_maps_engine_curve_and_inline_comments() {
    let root = force_india_root();
    if !root.is_dir() {
        eprintln!("skipping: {} is not available", root.display());
        return;
    }
    let spec: VehicleSpec = CarFolder::open(&root)
        .expect("real car folder should open")
        .vehicle_spec()
        .expect("real car data should map");

    // These lines carry `; comments` in the ACD; without stripping them the
    // mapper silently falls back to the default idle/redline.
    assert_eq!(spec.engine.idle_rpm, 2_350.0);
    assert_eq!(spec.engine.redline_rpm, 19_000.0);

    // The whole `power.lut` reaches the spec; 292 N·m is at 17 000 rpm.
    assert_eq!(spec.engine.torque_curve.len(), 21);
    assert!((spec.engine.peak_torque_nm - 292.0).abs() < 0.1);
    assert!((spec.engine.torque_at_rpm(17_000.0) - 292.0).abs() < 0.1);
    assert!((spec.engine.torque_at_rpm(19_000.0) - 243.0).abs() < 0.1);
    let normalized = spec.engine.normalized_torque_curve(19_000.0);
    assert!(
        normalized
            .windows(2)
            .all(|window| window[1].0 > window[0].0)
    );
    assert!((normalized.last().unwrap().0 - 1.0).abs() < 1e-6);
    assert!((normalized.last().unwrap().1 - 243.0 / 292.0).abs() < 1e-4);

    // A naturally aspirated V8: no turbo block.
    assert!(!spec.engine.turbo.is_enabled());
    assert_eq!(spec.engine.gear_ratios.len(), 7);

    // Per-wing aero: the real curve tables must reach the spec, including the
    // ground-height table that is the whole ground-effect model.
    assert_eq!(spec.aero.wings.len(), 9);
    let front = spec
        .aero
        .wings
        .iter()
        .find(|wing| wing.position.z > 0.0)
        .expect("there is a front wing");
    assert!(front.aoa_cl.len() >= 10, "{:?}", front.aoa_cl.len());
    assert!(front.gh_cl.len() >= 8, "{:?}", front.gh_cl.len());
    assert!(front.aoa_cd.len() >= 10);
    assert!(front.gh_cd.len() >= 8);
    // Front wing at 17 degrees of installation angle: a high-lift condition.
    let cl = front.cl_area(17.0, 0.05);
    assert!(cl > 0.0, "{cl}");
    println!("front wing cl*area at 17deg/50mm: {cl}");
    println!(
        "front wing cd*area at 17deg/50mm: {}",
        front.cd_area(17.0, 0.05)
    );
    println!(
        "front wing cd*area at 5deg/50mm: {}",
        front.cd_area(5.0, 0.05)
    );

    // The F1 pack's INERTIA (1.6, 0.70, 3.8) is not a physical kg·m² tensor,
    // so the mapper falls back to the collider tensor, which is.
    assert_eq!(spec.inertia, spec.collider_inertia());
    assert!(
        spec.inertia.x > 100.0 && spec.inertia.y > 200.0 && spec.inertia.z > 200.0,
        "{:?}",
        spec.inertia
    );

    // Tyre vertical stiffness: the wheel sees the suspension and the tyre in
    // series, and the tyre gets stiffer with pressure.
    assert!((spec.tyres.spring_rate_n_per_m - 335_281.5).abs() < 1.0);
    assert!((spec.tyres.pressure_spring_gain_n_per_m - 11_455.5).abs() < 1.0);
    // 26 psi hot against 17 psi cold is worth ~100 kN/m of vertical rate.
    let cold = spec
        .tyres
        .vertical_rate_n_per_m(spec.tyres.cold_pressure_psi);
    let hot = spec.tyres.vertical_rate_n_per_m(26.0);
    assert!(hot > cold + 80_000.0, "cold={cold} hot={hot}");

    // Three-layer thermal model: the rates AC publishes, and the compound's
    // own temperature-vs-grip curve.
    assert!(spec.tyres.thermal.has_layers());
    assert!((spec.tyres.thermal.surface_transfer_per_s - 0.018).abs() < 1e-6);
    assert!((spec.tyres.thermal.patch_transfer_per_s - 0.00027).abs() < 1e-6);
    assert!(spec.tyres.thermal.performance_curve.len() >= 8);
    // The curve peaks at 1.0 around 70-105 degrees and drops badly by 300.
    let peak = spec.tyres.thermal.performance_curve.iter().copied().fold(
        (0.0f32, 0.0f32),
        |best, point| {
            if point.1 > best.1 { point } else { best }
        },
    );
    assert!(
        peak.1 > 0.99 && (70.0..=110.0).contains(&peak.0),
        "{peak:?}"
    );
    assert!((spec.tyres.thermal.temperature_multiplier(85.0) - 1.0).abs() < 1e-6);
    assert!(spec.tyres.thermal.temperature_multiplier(300.0) < 0.7);

    // The tyre's own slip curve: peak slip angle and falloff, from the car's
    // data as a Magic Formula fit (Project Chrono's Pac89 formula).
    assert!((spec.tyres.lateral_peak_slip_deg - 5.95).abs() < 1e-3);
    assert!((spec.tyres.falloff_level - 0.88).abs() < 1e-3);
    let curves = spec.tyres.slip_curves();
    assert!(curves.0.windows(2).all(|pair| pair[1].0 > pair[0].0));
    assert!(curves.1.windows(2).all(|pair| pair[1].0 > pair[0].0));
    // Both tables peak at exactly 1.0, and the lateral one peaks near 6 degrees.
    assert!(curves.1.iter().any(|(_, y)| (*y - 1.0).abs() < 1e-3));
    let peak_x = curves
        .1
        .iter()
        .find(|(_, y)| (*y - 1.0).abs() < 1e-5)
        .unwrap()
        .0;
    assert!((peak_x - 5.95).abs() < 1e-3, "peak slipped to {peak_x}");

    // Engine: `LIMITER_HZ=60` for a soft limiter and `ALTITUDE_SENSITIVITY`
    // for the altitude power loss.
    assert!((spec.engine.limiter_hz - 60.0).abs() < 1e-6);
    assert!((spec.engine.altitude_sensitivity - 0.14).abs() < 1e-6);
    assert!(spec.wheels[0].suspension.caster_angle_deg > 0.0);
    assert!(spec.wheels[0].suspension.kingpin_angle_deg > 0.0);

    // Tyre load sensitivity and relaxation length, AC's `FZ0`/`LS_EXP*`
    // averaged over the axles (3705/4295 N, 0.78/0.83, 0.92/0.95).
    assert!((spec.tyres.reference_load_n - 4_000.0).abs() < 1.0);
    assert!((spec.tyres.lateral_load_sensitivity - 0.805).abs() < 1e-3);
    assert!((spec.tyres.longitudinal_load_sensitivity - 0.935).abs() < 1e-3);
    assert!((spec.tyres.relaxation_length_m - 0.104_395).abs() < 1e-3);

    // Rolling resistance and the pneumatic trail: AC authors the former, and
    // the trail is what makes the steering feel connected.
    assert!(spec.tyres.rolling_resistance_n > 5.0);
    assert!(spec.tyres.rolling_resistance_v2 > 0.0);
    assert!(spec.tyres.rolling_resistance_slip > 0.0);
    assert!(spec.tyres.pneumatic_trail_m > 0.0);

    // Steering: `STEER_LOCK=230` through `STEER_RATIO=15` is ~15.3 deg at the
    // wheel, not 30 deg of the old default. The F1 pack authors no caster or
    // kingpin, so the road-car defaults stay.
    assert!((spec.max_steer_angle_deg - 230.0 / 15.0).abs() < 1e-3);

    // The axle's roll centre, above the road and below the centre of mass.
    assert!(spec.wheels[0].suspension.roll_center_height_m > 0.0);
    assert!(spec.wheels[0].suspension.roll_center_height_m < spec.cg_height_m);

    // Drivetrain completeness: reverse ratio, upshift fuel cut-off, handbrake.
    assert!(
        (spec.gearbox.reverse_ratio - 4.0).abs() < 1e-6,
        "{:?}",
        spec.gearbox.reverse_ratio
    );
    assert!((spec.gearbox.auto_cutoff_time - 10.0).abs() < 1e-6);
    assert_eq!(spec.gearbox.handbrake_torque_nm, 0.0);

    // Brake rotor core: AC's `CORE_TRANSFER_*` rates, which is the heat soak
    // that keeps the pads cooking after the driver lifts off.
    assert!((spec.brakes.core_transfer_in_per_s - 0.095).abs() < 1e-6);
    assert!((spec.brakes.core_transfer_out_per_s - 0.55).abs() < 1e-6);
    assert!(spec.brakes.core_transfer_air_per_s > 0.0);

    // Engine braking: `[COAST_REF] RPM=12700 TORQUE=130`.
    assert_eq!(spec.engine.coast_torque_nm, 130.0);
    assert_eq!(spec.engine.coast_reference_rpm, 12_700.0);
    assert_eq!(spec.engine.coast_non_linearity, 0.0);

    // Inline comments also used to hide the suspension rates.
    let front = &spec.wheels[0].suspension;
    assert_eq!(front.spring_rate_n_per_m, 40_000.0);
    assert_eq!(front.progressive_rate_n_per_m2, 15_000.0);
    assert_eq!(front.bump_stop_rate_n_per_m, 200_000.0);
    assert_eq!(front.damper_bump_ns_per_m, 3_500.0);
    assert_eq!(front.anti_roll_n_per_m, 32_000.0);
    assert_eq!(spec.heave_springs[0].spring_rate_n_per_m, 120_000.0);
    assert_eq!(spec.heave_springs[0].preload_m, 0.015);

    // Two-rate dampers: slow rate, bleeder rate and the speed it engages at.
    let front = &spec.wheels[0].suspension;
    assert_eq!(front.damper_bump_ns_per_m, 3_500.0);
    assert_eq!(front.fast_bump_damper_ns_per_m, 1_100.0);
    assert_eq!(front.fast_bump_threshold_mps, 0.05);
    assert_eq!(front.fast_rebound_damper_ns_per_m, 1_460.0);
    assert_eq!(front.fast_rebound_threshold_mps, 0.12);
    // Static camber, negative as the car is authored.
    assert_eq!(front.static_camber_deg, -3.0);
}
