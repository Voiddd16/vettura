//! Tests for the suspension spec: springs, dampers, active suspension and geometry.

use vettura_physics::spec::{
    ActiveSuspensionMode, ActiveSuspensionSpec, HeaveSpringSpec, SuspensionSpec,
};

#[test]
fn progressive_springs_and_end_stops_are_nonlinear() {
    let passive = SuspensionSpec::default();
    // The default has no nonlinear elements, so it adds nothing.
    assert_eq!(passive.auxiliary_force_n(0.03, 0.0), 0.0);

    let spring = SuspensionSpec {
        progressive_rate_n_per_m2: 20_000.0,
        bump_stop_rate_n_per_m: 200_000.0,
        bump_stop_travel_m: 0.05,
        rebound_stop_rate_n_per_m: 100_000.0,
        rebound_stop_travel_m: 0.03,
        ..SuspensionSpec::default()
    };
    // The progressive part grows with the square of compression.
    let small = spring.auxiliary_force_n(0.02, 0.0);
    let double = spring.auxiliary_force_n(0.04, 0.0);
    assert!((small - 8.0).abs() < 1e-3, "{small}");
    assert!((double - 32.0).abs() < 1e-3, "{double}");
    // The bump stop engages only after its travel and then adds linearly.
    let before = spring.auxiliary_force_n(0.05, 0.0);
    let after = spring.auxiliary_force_n(0.06, 0.0);
    assert!(after - before > 1_900.0, "{before} → {after}");
    // The rebound stop pulls down when over-extended.
    assert!(spring.auxiliary_force_n(0.0, 0.05) < 0.0);
    assert_eq!(spring.auxiliary_force_n(0.0, 0.01), 0.0);
    // Nonsense input stays finite.
    assert!(spring.auxiliary_force_n(f32::NAN, f32::NAN).abs() < 1e-3);
}

#[test]
fn a_heave_spring_stiffens_both_wheels_together() {
    let heave = HeaveSpringSpec {
        spring_rate_n_per_m: 120_000.0,
        progressive_rate_n_per_m2: 10_000.0,
        bump_stop_rate_n_per_m: 0.0,
        bump_stop_travel_m: 0.05,
        preload_m: 0.0,
        damper_bump_ns_per_m: 3_000.0,
        damper_rebound_ns_per_m: 5_000.0,
    };
    // Static compression of 1 cm gives the linear and progressive parts.
    assert!((heave.force_n(0.01, 0.0) - (1_200.0 + 1.0)).abs() < 1e-3);
    assert_eq!(heave.force_n(-0.01, 0.0), 0.0);
    // The damper resists compression upward and extension downward.
    assert!(heave.force_n(0.01, -0.2) > heave.force_n(0.01, 0.0));
    assert!(heave.force_n(0.01, 0.2) < heave.force_n(0.01, 0.0));
    // The rod length is a preload: compression minus preload carries no
    // load, so it shows up as extra force even at zero measured travel.
    let preloaded = HeaveSpringSpec {
        preload_m: 0.02,
        ..heave
    };
    assert!((preloaded.force_n(0.0, 0.0) - 2_404.0).abs() < 1e-3);
    // No authored spring means no force at all.
    assert_eq!(HeaveSpringSpec::default().force_n(0.05, 0.0), 0.0);
}

#[test]
fn active_suspension_holds_ride_height_and_damps_motion() {
    // A passive car has no actuator at all.
    let passive = ActiveSuspensionSpec::default();
    assert!(!passive.is_enabled());
    assert_eq!(passive.force_n(0.05, -0.2), 0.0);

    let active = ActiveSuspensionSpec {
        mode: ActiveSuspensionMode::Active,
        stiffness_n_per_m: 80_000.0,
        damping_ns_per_m: 4_000.0,
        max_force_n: 12_000.0,
        ..ActiveSuspensionSpec::default()
    };
    assert!(active.is_enabled());
    // Compressed below the target: push the chassis up.
    assert!(active.force_n(0.02, 0.0) > 0.0);
    // Extended above it: pull it down.
    assert!(active.force_n(-0.02, 0.0) < 0.0);
    // Moving in compression (negative length velocity): resist upward.
    assert!(active.force_n(0.0, -0.5) > 0.0);
    assert!(active.force_n(0.0, 0.5) < 0.0);
    // The force never exceeds the actuator limit.
    assert_eq!(active.force_n(1.0, 0.0), 12_000.0);
    assert_eq!(active.force_n(-1.0, 0.0), -12_000.0);

    // Semi-active only damps; it does not hold a static ride height.
    let semi = ActiveSuspensionSpec {
        mode: ActiveSuspensionMode::SemiActive,
        ..active.clone()
    };
    assert_eq!(semi.force_n(0.02, 0.0), 0.0);
    assert!(semi.force_n(0.02, -0.5) > 0.0);

    // Speed lowers the body, smoothly and toward the authored drop.
    let lowering = ActiveSuspensionSpec {
        speed_drop_m: 0.04,
        speed_ref_mps: 50.0,
        ..active.clone()
    };
    assert_eq!(lowering.speed_drop_at(0.0), 0.0);
    assert!((lowering.speed_drop_at(50.0) - 0.02).abs() < 1e-6);
    assert!(lowering.speed_drop_at(100.0) > lowering.speed_drop_at(50.0));
    assert!(lowering.speed_drop_at(100.0) < 0.04);

    // Rake raises the front relative to the rear.
    let raked = ActiveSuspensionSpec {
        rake_m: 0.02,
        ..active
    };
    assert!(raked.target_offset_m(vettura_physics::spec::Wheel::FrontLeft, 0.0) > 0.0);
    assert!(raked.target_offset_m(vettura_physics::spec::Wheel::RearLeft, 0.0) < 0.0);
}

#[test]
fn two_rate_dampers_switch_at_their_threshold_and_stay_symmetric_by_default() {
    let slow = SuspensionSpec {
        damper_bump_ns_per_m: 4_000.0,
        damper_rebound_ns_per_m: 6_000.0,
        ..SuspensionSpec::default()
    };
    // Jolt works at the symmetric average.
    assert!((slow.jolt_damper_rate_ns_per_m() - 5_000.0).abs() < 1e-6);
    // No fast rates: the authored bump/rebound split in both directions.
    assert!((slow.damper_rate_ns_per_m(-0.1) - 4_000.0).abs() < 1e-6);
    assert!((slow.damper_rate_ns_per_m(0.1) - 6_000.0).abs() < 1e-6);
    assert_eq!(slow.damper_rate_ns_per_m(0.0), 0.0);
    // Nonsense velocity reads as standstill, not as a fast rate.
    assert_eq!(slow.damper_rate_ns_per_m(f32::NAN), 0.0);

    let two_rate = SuspensionSpec {
        damper_bump_ns_per_m: 4_000.0,
        damper_rebound_ns_per_m: 6_000.0,
        fast_bump_damper_ns_per_m: 1_000.0,
        fast_bump_threshold_mps: 0.05,
        fast_rebound_damper_ns_per_m: 1_500.0,
        fast_rebound_threshold_mps: 0.05,
        ..SuspensionSpec::default()
    };
    // Below the threshold the slow rates still apply.
    assert!((two_rate.damper_rate_ns_per_m(-0.05) - 4_000.0).abs() < 1e-6);
    assert!((two_rate.damper_rate_ns_per_m(0.05) - 6_000.0).abs() < 1e-6);
    // Above it the bleeder takes over in both directions.
    assert!((two_rate.damper_rate_ns_per_m(-0.5) - 1_000.0).abs() < 1e-6);
    assert!((two_rate.damper_rate_ns_per_m(0.5) - 1_500.0).abs() < 1e-6);
    // A fast rate of zero is "off", so the slow rate stays in charge.
    let off = SuspensionSpec {
        fast_bump_damper_ns_per_m: 0.0,
        ..two_rate
    };
    assert!((off.damper_rate_ns_per_m(-0.5) - 4_000.0).abs() < 1e-6);
}

#[test]
fn camber_leans_in_under_load_and_pulls_the_car_inward() {
    let suspension = SuspensionSpec {
        static_camber_deg: -3.0,
        camber_gain_rad_per_m: -0.5,
        camber_thrust_coeff: 0.3,
        ..SuspensionSpec::default()
    };
    // No load keeps the static angle.
    let static_rad = suspension.camber_rad(0.0);
    assert!((static_rad - (-3.0f32).to_radians()).abs() < 1e-6);
    // Compression adds negative camber.
    let compressed = suspension.camber_rad(0.05);
    assert!(compressed < static_rad);
    assert!((compressed - static_rad + 0.025).abs() < 1e-6);

    // Negative camber on a loaded wheel pulls that wheel inward, which is
    // what makes negative camber help a corner.
    let thrust = suspension.camber_thrust_n(compressed, 4_000.0);
    assert!(thrust < 0.0, "camber thrust direction: {thrust}");
    // No camber, no thrust; nonsense stays harmless.
    assert_eq!(suspension.camber_thrust_n(0.0, 4_000.0), 0.0);
    assert_eq!(suspension.camber_thrust_n(f32::NAN, f32::NAN), 0.0);
    // A car with no camber stiffness cannot make camber thrust.
    let straight = SuspensionSpec {
        camber_thrust_coeff: 0.0,
        ..SuspensionSpec::default()
    };
    assert_eq!(straight.camber_thrust_n(-0.1, 1_000.0), 0.0);
}

#[test]
fn steering_axis_geometry_adds_camber_and_self_centering() {
    let suspension = SuspensionSpec::default();
    // Straight ahead: no camber from the axis and no trail effect.
    assert_eq!(suspension.steer_camber_gain(0.0), 0.0);
    // Lock on: the wheel leans.
    let steer_gain = suspension.steer_camber_gain(0.3);
    assert!(steer_gain.abs() > 0.01, "{steer_gain}");
    assert!(suspension.caster_trail_m(0.33) > 0.01);
    // Nonsense or a disabled axis do nothing.
    assert_eq!(suspension.steer_camber_gain(f32::NAN), 0.0);
    let flat = SuspensionSpec {
        caster_angle_deg: 0.0,
        kingpin_angle_deg: 0.0,
        ..SuspensionSpec::default()
    };
    assert_eq!(flat.caster_trail_m(0.33), 0.0);
    assert_eq!(flat.steer_camber_gain(0.3), 0.0);
}

#[test]
fn anti_dive_and_squat_are_bounded_fractions() {
    let suspension = SuspensionSpec::default();
    assert!(suspension.anti_dive_fraction > 0.0 && suspension.anti_dive_fraction <= 1.0);
    assert!(suspension.anti_squat_fraction > 0.0 && suspension.anti_squat_fraction <= 1.0);
    // Both can be turned off for a car that transfers everything through
    // the springs, and both can be authored.
    let off = SuspensionSpec {
        anti_dive_fraction: 0.0,
        anti_squat_fraction: 0.0,
        ..SuspensionSpec::default()
    };
    assert_eq!(off.anti_dive_fraction, 0.0);
    assert_eq!(off.anti_squat_fraction, 0.0);
}
