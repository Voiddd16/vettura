use glam::{Quat, Vec3};

/// Convenience pose used by `VehicleBackend::reset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, 0.0),
            orientation: Quat::IDENTITY,
        }
    }
}

impl Pose {
    pub fn forward(self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }
}
