//! Rigid-body debris: loose bodywork, wheels, cones and gravel chunks.
//!
//! The world owns the bodies and steps them with the same Jolt solver as the
//! cars, so debris collides with the track, walls, other debris and vehicles.
//! Cleanup, scoring and safety-car rules deliberately live outside this module:
//! here a piece of debris only has mass, a shape and a pose.

use glam::{Quat, Vec3};

/// A piece of debris is identified by this handle while it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebrisHandle(pub(crate) u64);

impl DebrisHandle {
    /// Raw handle value, stable only within the world that created it.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Shape used for a piece of debris.
///
/// Cones and chunks are approximated by a box or a sphere; a wheel can use the
/// cylinder. The mass is supplied separately so a hollow bodywork panel can be
/// light without shrinking its collision shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DebrisShape {
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
    Cylinder { radius: f32, half_height: f32 },
}

impl DebrisShape {
    /// Clamp every dimension into a range Jolt accepts.
    pub fn clamped(self) -> Self {
        let clamp_extent = |value: f32| {
            if value.is_finite() {
                value.clamp(0.01, 5.0)
            } else {
                0.1
            }
        };
        match self {
            Self::Box { half_extents } => Self::Box {
                half_extents: Vec3::new(
                    clamp_extent(half_extents.x),
                    clamp_extent(half_extents.y),
                    clamp_extent(half_extents.z),
                ),
            },
            Self::Sphere { radius } => Self::Sphere {
                radius: clamp_extent(radius),
            },
            Self::Cylinder {
                radius,
                half_height,
            } => Self::Cylinder {
                radius: clamp_extent(radius),
                half_height: clamp_extent(half_height),
            },
        }
    }
}

impl Default for DebrisShape {
    fn default() -> Self {
        Self::Box {
            half_extents: Vec3::splat(0.15),
        }
    }
}

/// Everything needed to create one rigid body of debris.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebrisSpec {
    pub shape: DebrisShape,
    /// Centre of mass, physics frame.
    pub position: Vec3,
    pub orientation: Quat,
    pub mass_kg: f32,
    pub friction: f32,
    pub restitution: f32,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub linear_damping: f32,
    pub angular_damping: f32,
}

impl Default for DebrisSpec {
    fn default() -> Self {
        Self {
            shape: DebrisShape::default(),
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            mass_kg: 2.0,
            friction: 0.6,
            restitution: 0.1,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            linear_damping: 0.05,
            angular_damping: 0.1,
        }
    }
}

impl DebrisSpec {
    /// Replace non-finite values and clamp every physical parameter.
    pub fn clamped(mut self) -> Self {
        self.shape = self.shape.clamped();
        self.mass_kg = if self.mass_kg.is_finite() {
            self.mass_kg.clamp(0.05, 2_000.0)
        } else {
            2.0
        };
        self.friction = if self.friction.is_finite() {
            self.friction.clamp(0.0, 10.0)
        } else {
            0.6
        };
        self.restitution = if self.restitution.is_finite() {
            self.restitution.clamp(0.0, 1.0)
        } else {
            0.1
        };
        self.linear_damping = if self.linear_damping.is_finite() {
            self.linear_damping.clamp(0.0, 5.0)
        } else {
            0.05
        };
        self.angular_damping = if self.angular_damping.is_finite() {
            self.angular_damping.clamp(0.0, 5.0)
        } else {
            0.1
        };
        if !self.linear_velocity.is_finite() {
            self.linear_velocity = Vec3::ZERO;
        }
        if !self.angular_velocity.is_finite() {
            self.angular_velocity = Vec3::ZERO;
        }
        self
    }
}

/// Read-only state of one piece of debris.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebrisSnapshot {
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub mass_kg: f32,
    /// Jolt has put the body to sleep because it has been still for a while.
    pub sleeping: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_clamping_covers_every_physical_parameter() {
        let spec = DebrisSpec {
            shape: DebrisShape::Box {
                half_extents: Vec3::new(f32::NAN, 99.0, -0.5),
            },
            position: Vec3::splat(f32::INFINITY),
            orientation: Quat::from_xyzw(f32::NAN, 0.0, 0.0, 0.0),
            mass_kg: -5.0,
            friction: 1_000.0,
            restitution: 3.0,
            linear_velocity: Vec3::splat(f32::NAN),
            angular_velocity: Vec3::ZERO,
            linear_damping: -1.0,
            angular_damping: f32::INFINITY,
        }
        .clamped();

        let DebrisShape::Box { half_extents } = spec.shape else {
            panic!("box stayed a box");
        };
        assert_eq!(half_extents, Vec3::new(0.1, 5.0, 0.01));
        assert_eq!(spec.mass_kg, 0.05);
        assert_eq!(spec.friction, 10.0);
        assert_eq!(spec.restitution, 1.0);
        assert_eq!(spec.linear_damping, 0.0);
        // A non-finite damping falls back to the default, not to a clamp.
        assert_eq!(spec.angular_damping, 0.1);
        assert_eq!(spec.linear_velocity, Vec3::ZERO);
    }

    #[test]
    fn shape_clamping_handles_spheres_and_cylinders() {
        let sphere = DebrisShape::Sphere {
            radius: f32::NEG_INFINITY,
        }
        .clamped();
        assert!(matches!(sphere, DebrisShape::Sphere { radius } if radius == 0.1));
        let cylinder = DebrisShape::Cylinder {
            radius: 0.0,
            half_height: -4.0,
        }
        .clamped();
        assert!(
            matches!(cylinder, DebrisShape::Cylinder { radius, half_height } if radius == 0.01 && half_height == 0.01)
        );
    }
}
