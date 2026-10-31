//! Coordinate frames.
//!
//! # Assetto Corsa (authoring / KN5)
//!
//! KN5 geometry is right-handed with:
//!
//! * `+X` = left
//! * `+Y` = up
//! * `+Z` = forward
//!
//! This was verified against the sample F1 2008 KN5: `WHEEL_LF` is at
//! `(+0.71, 0.30, +1.53)` while `WHEEL_RR` is at `(-0.70, 0.30, -1.65)`.
//!
//! # Godot
//!
//! Godot is right-handed with `+X` right, `+Y` up, `-Z` forward.
//!
//! Converting AC to Godot is a 180 degree rotation around Y:
//! `(x, y, z) -> (-x, y, -z)`. The transformation is its own inverse, so the
//! same helper works in both directions.

use glam::Vec3;

pub const AC_LEFT: Vec3 = Vec3::X;
pub const AC_UP: Vec3 = Vec3::Y;
pub const AC_FORWARD: Vec3 = Vec3::Z;

pub const GODOT_RIGHT: Vec3 = Vec3::X;
pub const GODOT_UP: Vec3 = Vec3::Y;
pub const GODOT_FORWARD: Vec3 = Vec3::NEG_Z;

/// Convert a point (or direction) from AC space to Godot space.
#[inline]
pub fn ac_to_godot(value: Vec3) -> Vec3 {
    Vec3::new(-value.x, value.y, -value.z)
}

/// Convert a point (or direction) from Godot space to AC space.
#[inline]
pub fn godot_to_ac(value: Vec3) -> Vec3 {
    ac_to_godot(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_is_an_involution() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(godot_to_ac(ac_to_godot(p)), p);
    }

    #[test]
    fn ac_forward_maps_to_godot_forward() {
        assert_eq!(ac_to_godot(AC_FORWARD), GODOT_FORWARD);
    }
}
