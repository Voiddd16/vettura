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
