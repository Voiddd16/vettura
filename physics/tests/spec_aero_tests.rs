//! Tests for aero, brake, DRS and fuel spec types.

use vettura_physics::spec::{AIR_DENSITY_KG_M3, AeroSpec, BrakeSpec, DrsSpec, FuelSpec};

#[test]
fn side_force_opposes_sideslip_and_scales_with_speed() {
    let aero = AeroSpec {
        side_force_coeff: 2.0,
        ..AeroSpec::default()
    };
    // A sideslip of 0.1 rad at 30 m/s pushes against it.
    let force = aero.side_force_n(AIR_DENSITY_KG_M3, 30.0, 0.1);
    assert!(force < 0.0, "the force must oppose the sideslip: {force}");
    assert!(aero.side_force_n(AIR_DENSITY_KG_M3, 30.0, -0.1) > 0.0);
    // Stationary: nothing.
    assert_eq!(aero.side_force_n(AIR_DENSITY_KG_M3, 0.0, 0.1), 0.0);
    // The restoring yaw moment comes from acting behind the CG.
    let moment = aero.side_force_yaw_moment_nm(AIR_DENSITY_KG_M3, 30.0, 0.1);
    assert!(moment.abs() > 0.0);
}

#[test]
fn brake_efficiency_is_cold_fade_and_optimal() {
    let brakes = BrakeSpec::default();
    assert!(brakes.efficiency_at(brakes.ambient_c) < 1.0);
    assert!(brakes.efficiency_at(brakes.optimal_c) == 1.0);
    assert!(brakes.efficiency_at(brakes.fade_c) == 1.0);
    assert!(brakes.efficiency_at(brakes.fade_c * 2.0) < 1.0);
    assert!(brakes.efficiency_at(brakes.fade_c * 2.0) >= brakes.min_efficiency);
}

#[test]
fn drs_cuts_drag_by_the_authored_fraction() {
    let drs = DrsSpec {
        available: true,
        drag_reduction: 0.2,
        ..DrsSpec::default()
    };
    assert_eq!(drs.drag_multiplier(false), 1.0);
    assert!((drs.drag_multiplier(true) - 0.8).abs() < 1e-6);
    // A car without DRS ignores the request.
    assert_eq!(DrsSpec::default().drag_multiplier(true), 1.0);
}

#[test]
fn fuel_scales_with_load_and_speed() {
    let fuel = FuelSpec::default();
    assert!(fuel.full_mass_kg() > 40.0);
    // No load, no burn.
    assert_eq!(fuel.consumption_l_per_s(30.0, 0.0), 0.0);
    // More load, more burn; more speed, more burn.
    assert!(fuel.consumption_l_per_s(30.0, 0.5) < fuel.consumption_l_per_s(30.0, 1.0));
    assert!(fuel.consumption_l_per_s(10.0, 1.0) < fuel.consumption_l_per_s(30.0, 1.0));
    // The mass the tank carries follows the fraction.
    assert!((fuel.mass_at_fraction(0.0)).abs() < 1e-6);
    assert!((fuel.mass_at_fraction(0.5) - fuel.full_mass_kg() * 0.5).abs() < 1e-6);
}

#[test]
fn dirty_air_costs_downforce_only_directly_behind_a_car() {
    let aero = AeroSpec {
        dirty_air_loss: 0.3,
        dirty_air_distance_m: 20.0,
        ..AeroSpec::default()
    };
    // Clean air: nothing lost.
    assert_eq!(aero.dirty_air_multiplier(0.0), 1.0);
    // Right on the gearbox: the full authored loss.
    assert!((aero.dirty_air_multiplier(1.0) - 0.7).abs() < 1e-6);
    // Half a car length back: half the loss.
    assert!((aero.dirty_air_multiplier(0.5) - 0.85).abs() < 1e-6);
    // A car that does not author it is unaffected in traffic.
    assert_eq!(AeroSpec::default().dirty_air_multiplier(1.0), 1.0);
    // Nonsense reads as clean air.
    assert_eq!(aero.dirty_air_multiplier(f32::NAN), 1.0);
}

#[test]
fn wing_tables_give_ground_effect_and_stall() {
    let wing = vettura_physics::spec::WingSpec {
        chord_m: 0.5,
        span_m: 1.0,
        cl_gain: 0.25,
        aoa_cl: vec![(0.0, 0.5), (5.0, 0.9), (10.0, 0.3)],
        gh_cl: vec![(0.0, 1.2), (0.05, 1.0), (0.20, 0.5)],
        aoa_cd: vec![(0.0, 0.2), (10.0, 0.6)],
        gh_cd: vec![(0.0, 0.9), (0.10, 1.0)],
        ..vettura_physics::spec::WingSpec::default()
    };
    // Area 0.5 m²; the tables give Cl and then the height multiplier.
    assert!((wing.cl_area(0.0, 0.05) - 0.5 * 1.0 * 0.5).abs() < 1e-6);
    // Closer to the road is more ground effect.
    assert!(wing.cl_area(0.0, 0.0) > wing.cl_area(0.0, 0.2));
    // High angle of attack stalls the table.
    assert!(wing.cl_area(10.0, 0.05) < wing.cl_area(5.0, 0.05));
    // Out-of-domain queries clamp like AC's own LUT sampling.
    assert!((wing.cl_area(0.0, 1.0) - 0.5 * 0.5 * 0.5).abs() < 1e-6);
    assert!((wing.cl_area(0.0, -1.0) - 0.5 * 1.2 * 0.5).abs() < 1e-6);
    // Drag follows the same two tables.
    assert!((wing.cd_area(0.0, 0.05) - 0.2 * 0.95 * 0.5).abs() < 1e-6);
    assert!((wing.cd_area(10.0, 0.10) - 0.6 * 1.0 * 0.5).abs() < 1e-6);
    // No tables: the authored gain stands alone.
    let plain = vettura_physics::spec::WingSpec {
        chord_m: 0.5,
        span_m: 1.0,
        cl_gain: 0.25,
        cd_gain: 0.1,
        ..vettura_physics::spec::WingSpec::default()
    };
    assert!((plain.cl_area(0.0, 0.05) - 0.125).abs() < 1e-6);
    assert!((plain.cd_area(7.0, 0.05) - 0.05).abs() < 1e-6);
}
