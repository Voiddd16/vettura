//! A deliberately simple, force-based placeholder backend.
//!
//! This is **not** a racing tyre model. It exists so the Godot integration,
//! the fixed-step loop, and the asset mapping can be exercised end-to-end
//! before Chrono::Vehicle (or another solver) is wired in. Every physical
//! constant here is a placeholder.

use glam::{Quat, Vec3};

use crate::{Controls, Pose, VehicleBackend, VehicleSpec, VehicleState};

const AIR_DENSITY: f32 = 1.225;
const ROLLING_RESISTANCE: f32 = 0.015;
const MAX_STEER_ANGLE: f32 = 0.5;

pub struct SimpleBackend {
    spec: VehicleSpec,
    state: VehicleState,
}

impl SimpleBackend {
    pub fn new(spec: VehicleSpec) -> Self {
        Self {
            spec,
            state: VehicleState::default(),
        }
    }

    fn gear_ratio(&self) -> f32 {
        let index = (self.state.gear.max(1) as usize - 1).min(self.spec.engine.gear_ratios.len());
        let ratio = self
            .spec
            .engine
            .gear_ratios
            .get(index)
            .copied()
            .unwrap_or(1.0);
        ratio * self.spec.engine.final_drive
    }
}

impl VehicleBackend for SimpleBackend {
    fn spec(&self) -> &VehicleSpec {
        &self.spec
    }

    fn state(&self) -> &VehicleState {
        &self.state
    }

    fn reset(&mut self, pose: Pose) {
        self.state = VehicleState {
            position: pose.position,
            orientation: pose.orientation,
            ..VehicleState::default()
        };
        for wheel in &mut self.state.wheels {
            wheel.load_n = self.spec.mass_kg * 9.806_65 * 0.25;
            wheel.contact = true;
        }
    }

    fn step(&mut self, dt: f32, controls: &Controls) {
        let dt = dt.clamp(0.0, 0.05);
        if dt <= 0.0 {
            return;
        }

        let forward = self.state.orientation * Vec3::NEG_Z;
        let right = self.state.orientation * Vec3::X;
        let forward_speed = self.state.linear_velocity.dot(forward);
        let speed = self.state.linear_velocity.length();

        // Gear selection (no shift delay in the placeholder).
        if controls.shift_up {
            self.state.gear = (self.state.gear + 1).min(self.spec.engine.gear_ratios.len() as i32);
        }
        if controls.shift_down {
            self.state.gear = (self.state.gear - 1).max(1);
        }

        let gear_ratio = self.gear_ratio();
        let driven_radius = self
            .spec
            .wheels
            .iter()
            .find(|wheel| wheel.driven)
            .map(|wheel| wheel.radius_m)
            .unwrap_or(0.33);
        let wheel_rpm = forward_speed.abs() / (std::f32::consts::TAU * driven_radius) * 60.0;
        let engine_rpm = (wheel_rpm * gear_ratio)
            .max(self.spec.engine.idle_rpm)
            .min(self.spec.engine.redline_rpm);
        self.state.engine_rpm = engine_rpm;

        let torque_nm = self.spec.engine.peak_torque_nm * controls.throttle;
        let drive_force = torque_nm * gear_ratio * self.spec.drivetrain.efficiency / driven_radius;
        let brake_force = controls.brake * self.spec.mass_kg * 12.0;
        let drag_force = 0.5
            * AIR_DENSITY
            * self.spec.drag_coefficient
            * self.spec.frontal_area_m2
            * speed
            * speed;
        let rolling_force = ROLLING_RESISTANCE * self.spec.mass_kg * 9.806_65;
        let resistance = brake_force + drag_force + rolling_force;

        let direction = forward_speed.signum();
        let longitudinal_force = if speed < 0.05 && controls.throttle > 0.0 {
            drive_force
        } else {
            drive_force - resistance * direction
        };
        let acceleration = longitudinal_force / self.spec.mass_kg;
        self.state.linear_velocity += forward * acceleration * dt;

        // Fake lateral grip: kill sideways velocity exponentially.
        let lateral_speed = self.state.linear_velocity.dot(right);
        self.state.linear_velocity -= right * lateral_speed * (1.0 - (-dt * 8.0).exp());

        let speed_factor = 1.0 / (1.0 + speed * 0.04);
        let steer_angle = controls.steering * MAX_STEER_ANGLE * speed_factor;
        let yaw_rate = if speed > 0.2 {
            forward_speed / self.spec.wheelbase_m * steer_angle.tan()
        } else {
            0.0
        };
        if yaw_rate.is_finite() {
            self.state.orientation =
                (Quat::from_rotation_y(yaw_rate * dt) * self.state.orientation).normalize();
            self.state.angular_velocity = Vec3::new(0.0, yaw_rate, 0.0);
        }

        self.state.position += self.state.linear_velocity * dt;
        self.state.speed_mps = self.state.linear_velocity.dot(forward);

        for (index, wheel) in self.spec.wheels.iter().enumerate() {
            let wheel_state = &mut self.state.wheels[index];
            wheel_state.steer_angle = if wheel.steerable { steer_angle } else { 0.0 };
            wheel_state.angular_velocity = forward_speed / wheel.radius_m;
            wheel_state.rotation =
                (wheel_state.rotation + wheel_state.angular_velocity * dt) % std::f32::consts::TAU;
            wheel_state.load_n = self.spec.mass_kg * 9.806_65 * 0.25;
            wheel_state.contact = true;
        }
    }
}

impl SimpleBackend {
    pub fn state_mut(&mut self) -> &mut VehicleState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Simulation;

    const GRAVITY: f32 = 9.806_65;

    #[test]
    fn throttle_moves_the_car_forward() {
        let mut sim = Simulation::new(Box::new(SimpleBackend::new(VehicleSpec::default())));
        let start = sim.state().position;
        let controls = Controls {
            throttle: 1.0,
            ..Controls::default()
        };

        for _ in 0..600 {
            sim.step(1.0 / 60.0, &controls);
        }

        let moved = sim.state().position.distance(start);
        assert!(moved > 1.0, "car should move, moved {moved} m");
        assert!(sim.state().speed_mps > 0.0);
    }

    #[test]
    fn reset_clears_motion() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.state_mut().linear_velocity = Vec3::new(10.0, 0.0, 0.0);
        backend.reset(Pose::default());
        assert_eq!(backend.state().linear_velocity, Vec3::ZERO);
        assert_eq!(backend.state().wheels[0].load_n, 1200.0 * GRAVITY * 0.25);
    }
}
