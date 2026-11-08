use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::Wheel;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VehicleState {
    /// World position. Physics frame: +X right, +Y up, -Z forward.
    pub position: Vec3,
    pub orientation: Quat,
    /// World-frame linear velocity, m/s.
    pub linear_velocity: Vec3,
    /// World-frame angular velocity, rad/s.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_accessors_agree_with_direct_indexing() {
        let mut state = VehicleState::default();
        state.wheel_mut(Wheel::RearRight).load_n = 1234.5;

        for wheel in Wheel::ALL {
            assert_eq!(
                state.wheel(wheel) as *const WheelState,
                &state.wheels[wheel.index()] as *const WheelState,
                "{wheel:?} accessor must alias the direct slot",
            );
        }
        assert_eq!(state.wheel(Wheel::RearRight).load_n, 1234.5);
    }

    #[test]
    fn forward_rotates_with_orientation() {
        let mut state = VehicleState::default();
        assert_eq!(state.forward(), Vec3::NEG_Z);

        // 绕 Y 转 90°（yaw）：前向 -Z 转到 -X。
        state.orientation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let forward = state.forward();
        assert!(forward.x < -0.99 && forward.z.abs() < 1e-6 && forward.y.abs() < 1e-6);

        // 抬头 90°（pitch）：前向 -Z 转到 +Y。
        state.orientation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!(state.forward().y > 0.99);
    }

    #[test]
    fn default_state_rests_at_the_origin() {
        let state = VehicleState::default();
        assert_eq!(state.position, Vec3::ZERO);
        assert_eq!(state.linear_velocity, Vec3::ZERO);
        assert_eq!(state.speed_mps, 0.0);
        assert_eq!(state.gear, 1);
        // 文档约定：默认 load_n 是 0（“未初始化”），contact 为 false。
        for wheel in &state.wheels {
            assert_eq!(wheel.load_n, 0.0);
            assert!(!wheel.contact);
        }
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
