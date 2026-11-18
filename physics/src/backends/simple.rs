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

/// Fallback driven-wheel radius, metre. Guards against division by zero when a
/// [`VehicleSpec`] has no driven wheel at all.
const FALLBACK_WHEEL_RADIUS: f32 = 0.33;

/// Speed (m/s) below which the car is treated as stationary when computing the
/// direction of rolling-resistance / drag forces.
const MIN_DIRECTION_SPEED: f32 = 0.05;

pub struct SimpleBackend {
    spec: VehicleSpec,
    state: VehicleState,
}

impl SimpleBackend {
    /// 按给定规格创建 backend；内部状态从 [`VehicleState::default`] 起，
    /// 首次 [`VehicleBackend::step`] 前应调用 [`VehicleBackend::reset`]。
    pub fn new(spec: VehicleSpec) -> Self {
        Self {
            spec,
            state: VehicleState::default(),
        }
    }

    /// 当前档位的总齿比（档位齿比 × 主减速比）。
    ///
    /// 档位越界时 clamp：`gear <= 1` 取一档，超过档位数取最后一个，
    /// 列表为空时按 1.0 处理——不会返回 0 造成除零。
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
            .filter(|radius| *radius > 0.0)
            .unwrap_or(FALLBACK_WHEEL_RADIUS);
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

        // Below MIN_DIRECTION_SPEED the sign of `forward_speed` is meaningless
        // (and may jitter between +/- when the car is rolling backwards at a
        // crawl), so resistance is not applied in that regime. Throttle still
        // works so the car can pull away from rest.
        let direction = if forward_speed.abs() > MIN_DIRECTION_SPEED {
            forward_speed.signum()
        } else {
            0.0
        };
        let longitudinal_force = if controls.throttle > 0.0 && speed < MIN_DIRECTION_SPEED {
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
        // Steering is only meaningful once the car actually rolls; below this
        // threshold the bicycle model would divide by a near-zero forward speed
        // and produce a huge, meaningless yaw rate.
        let yaw_rate = if speed >= MIN_DIRECTION_SPEED {
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
    /// 直接改内部状态（布置测试场景、搬运门控用）。
    pub fn state_mut(&mut self) -> &mut VehicleState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Simulation;

    use crate::units::GRAVITY;

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

    /// 怠速边界：静止时轮速为 0，发动机转速必须被夹到怠速而不是 0。
    #[test]
    fn engine_rpm_rests_at_idle_when_stationary() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());

        backend.step(1.0 / 60.0, &Controls::default());

        assert_eq!(backend.state().engine_rpm, backend.spec().engine.idle_rpm);
    }

    /// 不同档位 → 不同齿比；越界档位 clamp 而不是 panic 或变负。
    #[test]
    fn gear_ratio_tracks_selected_gear_and_clamps() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());
        let spec = backend.spec().clone();
        let final_drive = spec.engine.final_drive;
        let dt = 1.0 / 60.0;

        // 一档：ratios[0] * final_drive。
        backend.state_mut().gear = 1;
        assert_eq!(
            backend.gear_ratio(),
            spec.engine.gear_ratios[0] * final_drive
        );

        // 顶档：最后一个齿比。
        let top = spec.engine.gear_ratios.len() as i32;
        backend.state_mut().gear = top;
        assert_eq!(
            backend.gear_ratio(),
            spec.engine.gear_ratios[top as usize - 1] * final_drive
        );

        // 0 / 负数 / 远超档位数：都 clamp 到合法区间，齿比仍是正的最终传动比。
        for gear in [0, -3, i32::MAX, top + 100] {
            backend.state_mut().gear = gear;
            assert!(backend.gear_ratio() > 0.0, "gear {gear}");
        }

        // 转速必须落在 [idle, redline] 内，哪怕给很大的轮速。
        backend.state_mut().gear = 1;
        backend.state_mut().linear_velocity = Vec3::NEG_Z * 200.0;
        backend.step(dt, &Controls::default());
        let rpm = backend.state().engine_rpm;
        assert!(
            (spec.engine.idle_rpm..=spec.engine.redline_rpm).contains(&rpm),
            "rpm {rpm} out of range"
        );
    }

    /// 刹车必须让前进速度下降。
    #[test]
    fn brake_decelerates_a_rolling_car() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());
        backend.state_mut().linear_velocity = Vec3::NEG_Z * 10.0;

        let controls = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        for _ in 0..60 {
            backend.step(1.0 / 60.0, &controls);
        }

        assert!(
            backend.state().linear_velocity.length() < 10.0,
            "brake should slow the car, speed = {}",
            backend.state().linear_velocity.length()
        );
    }

    /// 低于 MIN_DIRECTION_SPEED 时阻力方向无意义：松油门滑行车速度不被
    /// 阻力项反向推快，也不抖动方向。
    #[test]
    fn coasting_below_direction_threshold_preserves_speed() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());
        let creep = MIN_DIRECTION_SPEED * 0.5;
        backend.state_mut().linear_velocity = Vec3::NEG_Z * creep;

        let controls = Controls::default(); // 全松
        backend.step(1.0 / 60.0, &controls);

        assert!(
            (backend.state().speed_mps - creep).abs() < 1e-6,
            "creep speed should be untouched, got {}",
            backend.state().speed_mps
        );
    }

    /// 转向偏航只在滚动时产生：静止打方向车身不会原地乱转。
    #[test]
    fn steering_yaws_only_when_rolling() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        let controls = Controls {
            steering: 1.0,
            ..Controls::default()
        };

        // 静止：yaw_rate 守卫为 0。
        backend.reset(Pose::default());
        backend.step(1.0 / 60.0, &controls);
        assert_eq!(backend.state().angular_velocity, Vec3::ZERO);

        // 滚动后：出现绕 Y 的偏航角速度。
        backend.state_mut().linear_velocity = Vec3::NEG_Z * 10.0;
        backend.step(1.0 / 60.0, &controls);
        assert!(
            backend.state().angular_velocity.y.abs() > 0.0,
            "rolling steering should produce yaw"
        );
    }

    /// 换挡控制移动档位并 clamp 在 [1, len]。
    #[test]
    fn shift_controls_move_gear_within_range() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());
        let top = backend.spec().engine.gear_ratios.len() as i32;
        let dt = 1.0 / 60.0;

        let up = Controls {
            shift_up: true,
            ..Controls::default()
        };
        let down = Controls {
            shift_down: true,
            ..Controls::default()
        };

        backend.step(dt, &down);
        assert_eq!(backend.state().gear, 1, "already in first gear");

        backend.step(dt, &up);
        assert_eq!(backend.state().gear, 2);

        for _ in 0..100 {
            backend.step(dt, &up);
        }
        assert_eq!(backend.state().gear, top, "must clamp at the top gear");

        for _ in 0..100 {
            backend.step(dt, &down);
        }
        assert_eq!(backend.state().gear, 1, "must clamp at first gear");
    }

    /// 重置后每个轮子都应带上静态载荷并标记接触。
    #[test]
    fn reset_seeds_wheel_loads() {
        let mut backend = SimpleBackend::new(VehicleSpec::default());
        backend.reset(Pose::default());

        let expected = backend.spec().mass_kg * GRAVITY * 0.25;
        for (index, wheel) in backend.state().wheels.iter().enumerate() {
            assert!((wheel.load_n - expected).abs() < 1e-3, "wheel {index}");
            assert!(wheel.contact);
        }
    }
}
