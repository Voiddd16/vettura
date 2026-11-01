//! Unit conventions.
//!
//! Vettura uses SI units internally. Assetto Corsa data is mixed: distances
//! are often meters, mass is kilograms, but some INI values use km/h, degrees,
//! or normalized 0..1 ranges. Convert at the asset boundary, never deep in the
//! physics code.

pub type Meters = f32;
pub type Seconds = f32;
pub type Kilograms = f32;
pub type Newtons = f32;
pub type NewtonMeters = f32;
pub type Radians = f32;
pub type RadiansPerSecond = f32;
pub type MetersPerSecond = f32;

/// Standard gravity (ISO 80000-3).
pub const GRAVITY: f32 = 9.806_65;

pub const KMH_PER_MPS: f32 = 3.6;

#[inline]
pub fn mps_to_kmh(value: MetersPerSecond) -> f32 {
    value * KMH_PER_MPS
}

#[inline]
pub fn kmh_to_mps(value: f32) -> MetersPerSecond {
    value / KMH_PER_MPS
}

#[inline]
pub fn deg_to_rad(value: f32) -> Radians {
    value.to_radians()
}

#[inline]
pub fn rad_to_deg(value: Radians) -> f32 {
    value.to_degrees()
}

/// Clamp a normalized input (throttle, brake, ...) to `0..=1`.
#[inline]
pub fn normalize_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_conversions_round_trip() {
        const KNOWN: [(MetersPerSecond, f32); 3] = [(0.0, 0.0), (10.0, 36.0), (27.777_778, 100.0)];

        for (mps, kmh) in KNOWN {
            assert!((mps_to_kmh(mps) - kmh).abs() < 1e-4);
            assert!((kmh_to_mps(kmh) - mps).abs() < 1e-4);
        }
    }

    #[test]
    fn angle_conversions_round_trip() {
        assert_eq!(deg_to_rad(0.0), 0.0);
        assert!((deg_to_rad(180.0) - std::f32::consts::PI).abs() < 1e-6);
        assert!((rad_to_deg(std::f32::consts::FRAC_PI_2) - 90.0).abs() < 1e-5);
    }

    #[test]
    fn normalize_unit_clamps_to_zero_one() {
        for value in [-1.0, -0.001, 0.0, 0.25, 0.999, 1.0, 2.0] {
            let clamped = normalize_unit(value);
            assert!((0.0..=1.0).contains(&clamped), "{value} -> {clamped}");
            if (0.0..=1.0).contains(&value) {
                assert_eq!(clamped, value);
            }
        }
        // NaN 会原样带出——调用方负责过滤非有限输入。
        assert!(normalize_unit(f32::NAN).is_nan());
    }
}
