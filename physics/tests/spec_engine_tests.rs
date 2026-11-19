//! Tests for the engine spec: torque curve, turbo, coast and limiter.

use vettura_physics::spec::{EngineSpec, TorquePoint, TurboSpec};

#[test]
fn torque_curve_normalizes_to_the_engine_redline() {
    let engine = EngineSpec {
        redline_rpm: 8_000.0,
        peak_torque_nm: 200.0,
        torque_curve: vec![
            TorquePoint {
                rpm: 0.0,
                torque_nm: 100.0,
            },
            TorquePoint {
                rpm: 4_000.0,
                torque_nm: 200.0,
            },
            TorquePoint {
                rpm: 8_000.0,
                torque_nm: 150.0,
            },
            // A trailing point past the limiter must not reach the backend.
            TorquePoint {
                rpm: 9_000.0,
                torque_nm: 0.0,
            },
        ],
        ..EngineSpec::default()
    };
    let curve = engine.normalized_torque_curve(8_000.0);
    assert_eq!(curve.first(), Some(&(0.0, 0.5)));
    let last = curve.last().copied().unwrap();
    assert!((last.0 - 1.0).abs() < 1e-6);
    assert!((last.1 - 0.75).abs() < 1e-6);
    let peak = curve
        .iter()
        .find(|(x, _)| (*x - 0.5).abs() < 1e-6)
        .copied()
        .unwrap();
    assert!((peak.1 - 1.0).abs() < 1e-6);
    assert!(curve.windows(2).all(|window| window[1].0 > window[0].0));

    // Raw interpolation follows the authored table.
    assert!((engine.torque_at_rpm(2_000.0) - 150.0).abs() < 1e-3);
    assert!((engine.torque_at_rpm(8_000.0) - 150.0).abs() < 1e-3);

    // A car without a LUT gets the generic fallback shape.
    assert_eq!(
        EngineSpec::default().normalized_torque_curve(8_000.0),
        EngineSpec::DEFAULT_NORMALIZED_CURVE.to_vec()
    );
}

#[test]
fn turbo_builds_boost_with_rpm_and_throttle() {
    let turbo = TurboSpec {
        max_boost_bar: 0.8,
        wastegate_bar: 1.0,
        reference_rpm: 4_000.0,
        gamma: 1.0,
        lag_up_s: 0.4,
        lag_down_s: 0.2,
        torque_gain_per_bar: 1.0,
    };
    assert!(turbo.is_enabled());
    // No throttle, no exhaust energy.
    assert_eq!(turbo.target_boost_bar(0.0, 8_000.0), 0.0);
    // Linear spool with gamma = 1.
    assert!((turbo.target_boost_bar(1.0, 2_000.0) - 0.4).abs() < 1e-6);
    assert!((turbo.target_boost_bar(1.0, 8_000.0) - 0.8).abs() < 1e-6);
    // The wastegate caps the pressure.
    let capped = TurboSpec {
        max_boost_bar: 1.5,
        wastegate_bar: 0.6,
        ..turbo
    };
    assert!((capped.target_boost_bar(1.0, 8_000.0) - 0.6).abs() < 1e-6);
    // Gamma > 1 keeps the turbo lazy at low rpm.
    let lazy = TurboSpec {
        gamma: 2.0,
        ..turbo
    };
    assert!(lazy.target_boost_bar(1.0, 2_000.0) < turbo.target_boost_bar(1.0, 2_000.0));
    // Boost raises torque linearly; 0.8 bar with gain 1 doubles it.
    assert_eq!(turbo.torque_multiplier(0.0), 1.0);
    assert!((turbo.torque_multiplier(0.8) - 1.8).abs() < 1e-6);
    assert!((turbo.max_torque_multiplier() - 1.8).abs() < 1e-6);
    // No authored turbo means no boost at all.
    assert!(!TurboSpec::default().is_enabled());
    assert_eq!(TurboSpec::default().target_boost_bar(1.0, 8_000.0), 0.0);
}

#[test]
fn coast_torque_rises_with_rpm_and_bends_with_non_linearity() {
    let engine = EngineSpec {
        redline_rpm: 8_000.0,
        peak_torque_nm: 300.0,
        coast_torque_nm: 120.0,
        coast_reference_rpm: 8_000.0,
        coast_non_linearity: 0.0,
        ..EngineSpec::default()
    };
    assert_eq!(engine.coast_torque_at(0.0), 0.0);
    assert!((engine.coast_torque_at(4_000.0) - 60.0).abs() < 1e-6);
    assert!((engine.coast_torque_at(8_000.0) - 120.0).abs() < 1e-6);
    // Above the reference the curve clamps, so lifting off at the limiter
    // still slows the car.
    assert!((engine.coast_torque_at(12_000.0) - 120.0).abs() < 1e-6);

    // Fully quadratic: half the rpm is a quarter of the torque.
    let quadratic = EngineSpec {
        coast_non_linearity: 1.0,
        ..engine.clone()
    };
    assert!((quadratic.coast_torque_at(4_000.0) - 30.0).abs() < 1e-6);

    // No authored coast reference means no engine braking.
    assert_eq!(EngineSpec::default().coast_torque_at(8_000.0), 0.0);
}

#[test]
fn the_soft_limiter_and_altitude_detune_the_engine() {
    let engine = EngineSpec {
        redline_rpm: 8_000.0,
        limiter_hz: 60.0,
        altitude_sensitivity: 0.14,
        ..EngineSpec::default()
    };
    // Below the redline the limiter is open.
    assert_eq!(engine.limiter_duty(8_000.0), 1.0);
    // Past it the duty falls and bottoms out at a full cut.
    let mid = engine.limiter_duty(8_200.0);
    assert!(mid > 0.0 && mid < 1.0, "{mid}");
    assert_eq!(engine.limiter_duty(9_000.0), 0.0);
    // A hard limiter (no frequency) is always open here.
    assert_eq!(EngineSpec::default().limiter_duty(20_000.0), 1.0);

    // Altitude: no loss at sea level, 14 % per kilometre with floor 20 %.
    assert_eq!(engine.altitude_multiplier(0.0), 1.0);
    assert!((engine.altitude_multiplier(1_000.0) - 0.86).abs() < 1e-6);
    assert!((engine.altitude_multiplier(5_000.0) - 0.3).abs() < 1e-6);
    assert!(engine.altitude_multiplier(20_000.0) >= 0.2);
    // No authored sensitivity: no altitude effect at all.
    assert_eq!(EngineSpec::default().altitude_multiplier(3_000.0), 1.0);
}
