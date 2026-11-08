//! Conversion between the Vettura physics frame and Jolt's frame.
//!
//! Physics frame: `+X` right, `+Y` up, `-Z` forward.
//! Jolt frame: `+X` left, `+Y` up, `+Z` forward.
//!
//! The two frames differ by a 180° rotation about `+Y`. Positions and ordinary
//! vectors transform with `R`; orientations (quaternions) transform by
//! conjugation, `q_j = R * q_p * R⁻¹`, because both the body's local axes and the
//! world axes are re-expressed. The rotation is its own inverse, so the same
//! functions convert both ways.

use glam::{Quat, Vec3};
use oxijolt::{Quat as JoltQuat, RVec3, Vec3 as JoltVec3};

/// Jolt's `Vec3` from a physics-frame vector.
pub(crate) fn p2j(v: Vec3) -> JoltVec3 {
    JoltVec3::new(-v.x, v.y, -v.z)
}

/// Physics-frame vector from Jolt's `Vec3`.
pub(crate) fn j2p(v: JoltVec3) -> Vec3 {
    Vec3::new(-v.x, v.y, -v.z)
}

/// Jolt's position type from a physics-frame position.
pub(crate) fn p2j_r(v: Vec3) -> RVec3 {
    RVec3::new(-v.x, v.y, -v.z)
}

/// Physics-frame position from Jolt's position type.
pub(crate) fn j2p_r(v: RVec3) -> Vec3 {
    Vec3::new(-v.x, v.y, -v.z)
}

/// Jolt quaternion from a physics-frame quaternion.
pub(crate) fn p2j_q(q: Quat) -> JoltQuat {
    let r = JoltQuat::from_xyzw(0.0, 1.0, 0.0, 0.0);
    let r_inv = JoltQuat::from_xyzw(0.0, -1.0, 0.0, 0.0);
    mul(mul(r, JoltQuat::from_xyzw(q.x, q.y, q.z, q.w)), r_inv)
}

/// Physics-frame quaternion from a Jolt quaternion.
pub(crate) fn j2p_q(q: JoltQuat) -> Quat {
    let r = JoltQuat::from_xyzw(0.0, 1.0, 0.0, 0.0);
    let r_inv = JoltQuat::from_xyzw(0.0, -1.0, 0.0, 0.0);
    let p = mul(mul(r, q), r_inv);
    Quat::from_xyzw(p.x, p.y, p.z, p.w).normalize()
}

/// Quaternion product `a * b` (apply `b`, then `a`).
fn mul(a: JoltQuat, b: JoltQuat) -> JoltQuat {
    JoltQuat::from_xyzw(
        a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
        a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
        a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a:?} != {b:?}");
    }

    #[test]
    fn axes_map_to_jolt_convention() {
        // Physics right (+X) is Jolt left (-X).
        close(j2p(p2j(Vec3::X)), Vec3::X);
        // Physics forward (-Z) is Jolt forward (+Z).
        close(j2p(p2j(Vec3::NEG_Z)), Vec3::NEG_Z);
        // Up is unchanged.
        close(j2p(p2j(Vec3::Y)), Vec3::Y);
        assert_eq!(p2j(Vec3::NEG_Z), JoltVec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn identity_orientation_stays_identity() {
        let q = p2j_q(Quat::IDENTITY);
        assert!((q.x).abs() < 1e-6 && (q.y).abs() < 1e-6);
        assert!((q.z).abs() < 1e-6 && (q.w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn yaw_and_forward_agree_after_conversion() {
        // Physics yaw +90° turns forward (-Z) to -X.
        let qp = Quat::from_rotation_y(FRAC_PI_2);
        assert!((qp * Vec3::NEG_Z - Vec3::NEG_X).length() < 1e-6);

        let qj = p2j_q(qp);
        let forward_j = rotate(qj, JoltVec3::new(0.0, 0.0, 1.0));
        // Jolt sees the same world direction after the world transform:
        // physics -X maps to Jolt +X.
        close(j2p(forward_j), Vec3::NEG_X);
    }

    #[test]
    fn conjugation_round_trips_an_arbitrary_rotation() {
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.4, 0.7);
        let back = j2p_q(p2j_q(q));
        assert!((back * Vec3::NEG_Z - q * Vec3::NEG_Z).length() < 1e-5);
        assert!((back * Vec3::X - q * Vec3::X).length() < 1e-5);
    }

    fn rotate(q: JoltQuat, v: JoltVec3) -> JoltVec3 {
        let p = mul(
            mul(q, JoltQuat::from_xyzw(v.x, v.y, v.z, 0.0)),
            JoltQuat::from_xyzw(-q.x, -q.y, -q.z, q.w),
        );
        JoltVec3::new(p.x, p.y, p.z)
    }
}
