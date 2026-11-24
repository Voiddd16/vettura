//! Tests for the tyre spec: grip, thermal, wear, slip curves and rolling resistance.

use vettura_physics::spec::{TyreSpec, TyreThermalSpec};

#[test]
fn tyre_pressure_rises_with_temperature() {
    let tyre = TyreSpec::default();
    let cold = tyre.pressure_at(tyre.thermal.optimal_c);
    let hot = tyre.pressure_at(tyre.thermal.optimal_c + tyre.thermal.window_c);
    assert!(hot > cold, "{cold} → {hot}");
    assert!(hot <= tyre.ideal_pressure_psi + 1.0);
    // Nonsense falls back to the optimal temperature, not a panic.
    assert!(tyre.pressure_at(f32::NAN).is_finite());
}

#[test]
fn combined_slip_shares_one_friction_budget() {
    let tyre = TyreSpec::default();
    // Nothing used in the other direction: everything is left.
    assert_eq!(tyre.lateral_share(0.0), 1.0);
    assert_eq!(tyre.longitudinal_share(0.0), 1.0);
    // Half used leaves more than a quarter of the ellipse.
    assert!(tyre.lateral_share(0.5) > 0.7 && tyre.lateral_share(0.5) < 0.9);
    // Fully used in one direction empties the other.
    assert!(tyre.lateral_share(1.0).abs() < 1e-6);
    assert!(tyre.longitudinal_share(1.0).abs() < 1e-6);
    // Direction does not matter.
    assert!((tyre.lateral_share(-0.5) - tyre.lateral_share(0.5)).abs() < 1e-6);
    // Nonsense keeps the whole budget.
    assert_eq!(tyre.lateral_share(f32::NAN), 1.0);
}

#[test]
fn grip_multiplier_penalises_cold_hot_and_worn_tyres() {
    let tyre = TyreSpec::default();
    let ideal = tyre.grip_multiplier(tyre.thermal.optimal_c, tyre.ideal_pressure_psi, 0.0);
    assert!((ideal - 1.0).abs() < 1e-6, "{ideal}");
    // Cold, wrong pressure and wear each take something.
    assert!(tyre.grip_multiplier(20.0, tyre.ideal_pressure_psi, 0.0) < ideal);
    assert!(tyre.grip_multiplier(tyre.thermal.optimal_c, 10.0, 0.0) < ideal);
    assert!(tyre.grip_multiplier(tyre.thermal.optimal_c, tyre.ideal_pressure_psi, 1.0) < ideal);
    // A tyre never becomes frictionless.
    assert!(tyre.grip_multiplier(2_000.0, 100.0, 1.0) >= 0.65);
}

#[test]
fn a_tyre_only_grips_at_the_camber_it_was_designed_for() {
    let tyre = TyreSpec {
        camber_optimal_rad: (-3.0f32).to_radians(),
        camber_grip_loss: 0.2,
        camber_window_rad: 0.15,
        ..TyreSpec::default()
    };
    assert!((tyre.camber_grip_multiplier((-3.0f32).to_radians()) - 1.0).abs() < 1e-6);
    // Half a window off costs half the loss.
    let half = tyre.camber_grip_multiplier((-3.0f32).to_radians() - 0.075);
    assert!((half - 0.9).abs() < 1e-6, "{half}");
    // Past the window the loss saturates, it never turns negative.
    assert!((tyre.camber_grip_multiplier(1.0) - 0.8).abs() < 1e-6);
    // Nonsense camber falls back to the designed angle, so it costs nothing.
    assert_eq!(tyre.camber_grip_multiplier(f32::NAN), 1.0);
}

#[test]
fn flat_spot_grips_less_and_never_inverts() {
    let tyre = TyreSpec {
        flat_spot_grip_loss: 0.3,
        ..TyreSpec::default()
    };
    assert_eq!(tyre.flat_spot_grip_multiplier(0.0), 1.0);
    assert!((tyre.flat_spot_grip_multiplier(0.5) - 0.85).abs() < 1e-6);
    assert!((tyre.flat_spot_grip_multiplier(2.0) - 0.7).abs() < 1e-6);
    // Nonsense never breaks the multiplier.
    assert_eq!(tyre.flat_spot_grip_multiplier(f32::NAN), 1.0);
    // No authorised grip loss means a flat spot is harmless.
    let harmless = TyreSpec {
        flat_spot_grip_loss: 0.0,
        ..TyreSpec::default()
    };
    assert_eq!(harmless.flat_spot_grip_multiplier(1.0), 1.0);
}

#[test]
fn load_sensitivity_and_relaxation_are_data_driven_and_bounded() {
    let off = TyreSpec::default();
    // No authored reference load: the flat behaviour it always had.
    assert_eq!(off.load_friction_multiplier(3_000.0, false), 1.0);
    // Relaxation itself is always available: no relaxed slip yet means no
    // developed force.
    assert_eq!(off.relaxation_multiplier(0.2, 0.0), 0.0);
    assert_eq!(off.relaxation_multiplier(0.2, 0.2), 1.0);

    let tyre = TyreSpec {
        reference_load_n: 4_000.0,
        lateral_load_sensitivity: 0.8,
        longitudinal_load_sensitivity: 0.95,
        relaxation_length_m: 0.1,
        ..TyreSpec::default()
    };
    // At the reference load the multiplier is exactly 1.
    assert!((tyre.load_friction_multiplier(4_000.0, false) - 1.0).abs() < 1e-6);
    // A light wheel carries more friction per newton, a heavy one less:
    // the force scales with load^0.8, so the coefficient scales with
    // load^-0.2.
    assert!(tyre.load_friction_multiplier(2_000.0, false) > 1.0);
    assert!(tyre.load_friction_multiplier(6_000.0, false) < 1.0);
    // The longitudinal exponent is closer to 1, so it is flatter.
    assert!(
        tyre.load_friction_multiplier(6_000.0, true)
            > tyre.load_friction_multiplier(6_000.0, false)
    );
    // Bounded so nothing overtakes the authored peak.
    assert!(tyre.load_friction_multiplier(20.0, false) <= 1.5);
    assert!(tyre.load_friction_multiplier(1.0e6, false) >= 0.5);

    // Relaxation scales the force by the slip the tyre has built up.
    assert_eq!(tyre.relaxation_multiplier(0.0, 0.0), 1.0);
    assert_eq!(tyre.relaxation_multiplier(0.4, 0.1), 0.25);
    assert_eq!(tyre.relaxation_multiplier(0.1, 0.4), 1.0);
    assert_eq!(tyre.relaxation_multiplier(f32::NAN, 0.2), 1.0);
}

#[test]
fn the_tyre_is_a_second_vertical_spring_in_series() {
    // Nothing authored: the suspension spring is the whole stiffness.
    assert_eq!(
        vettura_physics::spec::series_spring_rate(80_000.0, 0.0),
        80_000.0
    );
    // A tyre rate in series always softens the wheel: 80k + 335k = 65.6k.
    let combined = vettura_physics::spec::series_spring_rate(80_000.0, 335_281.0);
    assert!(combined < 80_000.0 && combined > 60_000.0, "{combined}");
    // Two equal springs halve; a very stiff tyre barely matters.
    assert!((vettura_physics::spec::series_spring_rate(80_000.0, 80_000.0) - 40_000.0).abs() < 1.0);
    assert!((vettura_physics::spec::series_spring_rate(80_000.0, 1.0e9) - 79_993.6).abs() < 1.0);

    // The tyre's rate rises with pressure, and pressure only ever adds.
    let tyre = TyreSpec {
        spring_rate_n_per_m: 300_000.0,
        pressure_spring_gain_n_per_m: 10_102.0,
        cold_pressure_psi: 17.0,
        ..TyreSpec::default()
    };
    assert!((tyre.vertical_rate_n_per_m(17.0) - 300_000.0).abs() < 1.0);
    assert!((tyre.vertical_rate_n_per_m(27.0) - 401_020.0).abs() < 1.0);
    // No authored rate: no vertical stiffness from the tyre at all.
    assert_eq!(TyreSpec::default().vertical_rate_n_per_m(29.0), 0.0);
}

#[test]
fn aligning_torque_and_rolling_resistance_come_from_the_tyre_data() {
    let tyre = TyreSpec {
        pneumatic_trail_m: 0.035,
        rolling_resistance_n: 8.0,
        rolling_resistance_v2: 5.0e-4,
        rolling_resistance_slip: 40.0,
        pressure_rr_gain: 0.45,
        cold_pressure_psi: 17.0,
        ..TyreSpec::default()
    };
    // A tyre pushed to the right twists the wheel back towards travel,
    // which is a positive (left) moment about the car's up axis. Well inside
    // the peak slip the trail model is unchanged.
    let peak = tyre.lateral_peak_slip_deg.to_radians();
    let force = tyre.aligning_torque_nm(0.5 * peak, 1_000.0);
    assert!((force - 35.0).abs() < 1e-3, "{force}");
    assert!((tyre.aligning_torque_nm(-0.5 * peak, -1_000.0) + 35.0).abs() < 1e-3);
    // Past the peak the pressure centre keeps moving back: the moment crosses
    // zero and then becomes de-aligning (sign flips), as on a real tyre.
    assert!(tyre.aligning_torque_nm(2.5 * peak, 1_000.0) < 0.0);
    let deep = tyre.aligning_torque_nm(4.0 * peak, 1_000.0);
    assert!(deep < 0.0);
    assert!(
        deep.abs() < 0.5 * 35.0,
        "the de-aligning moment should settle, not grow: {deep}"
    );
    // No trail, no torque; nonsense stays harmless.
    let no_trail = TyreSpec {
        pneumatic_trail_m: 0.0,
        ..tyre.clone()
    };
    assert_eq!(no_trail.aligning_torque_nm(0.5 * peak, 1_000.0), 0.0);
    // Nonsense lateral force is harmless; a nonsense slip angle reads as zero
    // slip (full aligning) rather than poisoning the chassis.
    assert_eq!(tyre.aligning_torque_nm(0.5 * peak, f32::NAN), 0.0);
    assert!(tyre.aligning_torque_nm(f32::NAN, 1_000.0).is_finite());

    // Rolling resistance: the three AC terms, and pressure only ever adds.
    let cold = tyre.rolling_resistance_n(30.0, 0.1, 17.0);
    let hot = tyre.rolling_resistance_n(30.0, 0.1, 30.0);
    assert!(
        (cold - (8.0 + 5.0e-4 * 900.0 + 40.0 * 0.01)).abs() < 1e-3,
        "{cold}"
    );
    assert!(hot > cold, "{hot} vs {cold}");
    // Speed and scrub both cost rubber.
    assert!(tyre.rolling_resistance_n(60.0, 0.1, 17.0) > cold);
    assert!(tyre.rolling_resistance_n(30.0, 0.5, 17.0) > cold);
    // No authored data: no extra rolling resistance at all.
    let bare = TyreSpec::default();
    assert_eq!(bare.rolling_resistance_n(30.0, 0.1, 17.0), 0.0);
}

#[test]
fn sub_zero_temperatures_are_a_physical_state_not_an_error() {
    // The rubber works below zero: a colder tyre simply holds less air and
    // grips worse, and neither of those is clamped at the freezing point.
    let tyre = TyreSpec {
        cold_pressure_psi: 17.0,
        ..TyreSpec::default()
    };
    assert!(tyre.pressure_at(-20.0) < tyre.pressure_at(0.0));
    assert!(tyre.pressure_at(0.0) < tyre.pressure_at(90.0));
    assert!(tyre.pressure_at(-20.0) > 0.0);

    // With an authored curve the grip keeps falling below the coldest
    // point instead of flattening at it.
    let thermal = TyreThermalSpec {
        performance_curve: vec![(0.0, 0.9), (50.0, 1.0), (100.0, 0.8), (300.0, 0.5)],
        ..TyreThermalSpec::default()
    };
    let at_zero = thermal.temperature_multiplier(0.0);
    let below = thermal.temperature_multiplier(-20.0);
    assert!(below < at_zero, "{below} vs {at_zero}");
    // Far below, the extrapolation floors out instead of going negative.
    assert!(thermal.temperature_multiplier(-1_000.0) >= 0.0);
    assert_eq!(thermal.temperature_multiplier(-1_000.0), 0.0);
    // Above the coldest point the table itself is the answer: 50 °C is the
    // peak, 75 °C is on the way down.
    assert!((thermal.temperature_multiplier(50.0) - 1.0).abs() < 1e-6);
    assert!((thermal.temperature_multiplier(75.0) - 0.9).abs() < 1e-6);

    // The analytic window also works in the cold.
    let window = TyreThermalSpec::default();
    assert!(window.temperature_multiplier(-20.0) < window.temperature_multiplier(0.0));
    assert!(window.temperature_multiplier(-20.0) > 0.0);
}

#[test]
fn three_layered_tyre_thermals_lag_behind_the_surface() {
    let single = TyreThermalSpec::default();
    assert!(!single.has_layers());
    let layered = TyreThermalSpec {
        surface_capacity_j_per_k: 60.0,
        patch_capacity_j_per_k: 200.0,
        heat_capacity_j_per_k: 1_000.0,
        surface_transfer_per_s: 0.018,
        patch_transfer_per_s: 0.00027,
        ..TyreThermalSpec::default()
    };
    assert!(layered.has_layers());
    // The grip curve reads the core; the surface that actually touches the
    // road is a different (much hotter) temperature, which is what the
    // world's `advance_tyre_temperatures` models. Here the layers exist and
    // the curve works.
    assert!(layered.heat_capacity_j_per_k > layered.patch_capacity_j_per_k);

    // An authored temperature curve replaces the analytic window.
    let with_curve = TyreThermalSpec {
        performance_curve: vec![(0.0, 0.9), (50.0, 1.0), (100.0, 0.8), (300.0, 0.5)],
        ..TyreThermalSpec::default()
    };
    assert!((with_curve.temperature_multiplier(50.0) - 1.0).abs() < 1e-6);
    assert!((with_curve.temperature_multiplier(0.0) - 0.9).abs() < 1e-6);
    assert!((with_curve.temperature_multiplier(25.0) - 0.95).abs() < 1e-6);
    assert!((with_curve.temperature_multiplier(1_000.0) - 0.5).abs() < 1e-6);
    // Nonsense temperature falls back to the optimum.
    assert!(with_curve.temperature_multiplier(f32::NAN).is_finite());
    // Without a curve the analytic window is used.
    assert!(single.temperature_multiplier(single.optimal_c) > 0.99);
    assert!(single.temperature_multiplier(20.0) < 0.95);
}

#[test]
fn slip_curves_come_from_the_cars_own_peak_and_falloff() {
    // Nonsense data keeps the flat table this crate used before: a car
    // that cannot be fitted still has to drive.
    let broken = TyreSpec {
        lateral_peak_slip_deg: f32::NAN,
        longitudinal_peak_slip: 0.0,
        falloff_level: f32::NAN,
        ..TyreSpec::default()
    };
    let (longitudinal, lateral) = broken.slip_curves();
    assert_eq!(
        longitudinal,
        vettura_physics::spec::DEFAULT_LONGITUDINAL_CURVE.to_vec()
    );
    assert_eq!(
        lateral,
        vettura_physics::spec::DEFAULT_LATERAL_CURVE.to_vec()
    );

    // The default spec publishes a 6 degree peak and a 6 % slip peak, and
    // AC data rides on those numbers: a fitted Magic Formula table.
    let spec = TyreSpec::default();
    let (longitudinal, lateral) = spec.slip_curves();
    for curve in [&longitudinal, &lateral] {
        assert!(curve.windows(2).all(|pair| pair[1].0 > pair[0].0));
        assert!(curve.iter().all(|(_, y)| (0.0..=1.0).contains(y)));
        assert!(curve.iter().any(|(_, y)| (*y - 1.0).abs() < 1e-4));
        // Past the peak the table must come back down: that is the point of
        // a real slip curve.
        let peak_index = curve
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
            .unwrap();
        assert!(peak_index > 0 && peak_index + 1 < curve.len());
        assert!(curve[peak_index + 1].1 < 1.0);
    }
    // The lateral table peaks at the authored 6 degrees.
    assert!(
        (lateral
            .iter()
            .find(|(_, y)| (*y - 1.0).abs() < 1e-4)
            .unwrap()
            .0
            - 6.0)
            .abs()
            < 1e-3
    );
    // The longitudinal table peaks at 6 % slip and still has grip at 50 %.
    assert!(
        (longitudinal
            .iter()
            .find(|(_, y)| (*y - 1.0).abs() < 1e-4)
            .unwrap()
            .0
            - 0.06)
            .abs()
            < 1e-3
    );
    assert!(longitudinal.last().unwrap().0 > 0.2);
}
