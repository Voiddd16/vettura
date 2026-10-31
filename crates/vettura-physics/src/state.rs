use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::Wheel;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VehicleState {
    /// World position. Physics frame: +X right, +Y up, -Z forward.
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    /// Signed forward speed in m/s.
    pub speed_mps: f32,
    pub wheels: [WheelState; 4],
    pub engine_rpm: f32,
    pub gear: i32,
}

impl Default for VehicleState {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            speed_mps: 0.0,
            wheels: [WheelState::default(); 4],
            engine_rpm: 0.0,
            gear: 1,
        }
    }
}

impl VehicleState {
    pub fn forward(&self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }

    pub fn wheel(&self, wheel: Wheel) -> &WheelState {
        &self.wheels[wheel.index()]
    }

    pub fn wheel_mut(&mut self, wheel: Wheel) -> &mut WheelState {
        &mut self.wheels[wheel.index()]
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct WheelState {
    /// Accumulated rotation angle, radians.
    pub rotation: f32,
    /// Current steering angle, radians. `0` when the wheel is not steerable.
    pub steer_angle: f32,
    /// Wheel spin rate, rad/s.
    pub angular_velocity: f32,
    /// Longitudinal slip ratio, dimensionless. `0` for a free-rolling wheel.
    pub slip_ratio: f32,
    /// Lateral slip angle, radians. `0` for pure rolling.
    pub slip_angle: f32,
    /// Tyre vertical load, newtons.
    ///
    /// The derived `Default` is `0`, which represents "resting on loose
    /// ground" rather than a physically accurate weight. Backends that model
    /// load transfer / suspension (see
    /// [`crate::backends::SimpleBackend::reset`]) overwrite this on the first
    /// step, so consumers should treat it as "uninitialised until reset".
    pub load_n: f32,
    /// Whether the tyre is currently touching the road.
    pub contact: bool,
}
