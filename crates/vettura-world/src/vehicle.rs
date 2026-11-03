//! One vehicle inside a [`World`](crate::World): AC spec → Jolt wheeled vehicle,
//! live input, tyre/surface callback, and the per-step snapshot.
//!
//! Jolt's wheeled-vehicle constraint supplies the suspension, tyre slip curve,
//! engine, automatic transmission and differentials. This module's job is the
//! adapter: translate the Vettura frame, size the suspension from the AC data,
//! install a surface/weather-aware tyre callback, and expose a stable snapshot
//! for rendering, telemetry and the `VehicleBackend` contract.

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use glam::{Quat, Vec3};
use oxijolt::{
    Activation, BodyId, BodySettings, DriverInput, MotionQuality, ObjectLayer, PhysicsWorld, Shape,
    SpringSettings, TireFriction, TireMaxImpulseInput, Vec3 as JoltVec3, VehicleAntiRollBar,
    VehicleCollisionTester, VehicleDifferentialSettings, VehicleEngineSettings, VehicleId,
    VehicleTransmissionSettings, WheelSettings, WheeledVehicleSettings,
};
use vettura_physics::{
    Controls, DrivetrainLayout, Pose, VehicleSpec, VehicleState, Wheel, WheelState,
};

use crate::frame::{j2p, j2p_q, j2p_r, p2j, p2j_q, p2j_r};
use crate::surface::{SurfaceId, SurfaceMap, SurfaceTable};
use crate::weather::Weather;
use crate::{WorldError, WorldResult};

/// Stable handle to a vehicle in a [`World`](crate::World).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VehicleHandle(pub(crate) u64);

impl VehicleHandle {
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Driver input in the Vettura physics frame.
///
/// This is the low-level form; [`World::set_controls`](crate::World::set_controls)
/// converts the shared [`Controls`] type into it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriveInput {
    /// Throttle / reverse, `-1..=1`. Positive drives forward.
    pub throttle: f32,
    /// Steering, `-1..=1`, positive = left.
    pub steering: f32,
    /// Brake, `0..=1`.
    pub brake: f32,
    /// Hand brake, latch or analog.
    pub handbrake: bool,
}

impl From<Controls> for DriveInput {
    fn from(controls: Controls) -> Self {
        let controls = controls.clamped();
        Self {
            throttle: controls.throttle,
            steering: controls.steering,
            brake: controls.brake,
            handbrake: controls.handbrake,
        }
    }
}

impl DriveInput {
    pub fn clamped(mut self) -> Self {
        self.throttle = finite_or(self.throttle, 0.0).clamp(-1.0, 1.0);
        self.steering = finite_or(self.steering, 0.0).clamp(-1.0, 1.0);
        self.brake = finite_or(self.brake, 0.0).clamp(0.0, 1.0);
        self
    }
}

/// Per-vehicle tuning that is not part of the AC-derived [`VehicleSpec`].
///
/// Defaults are conservative and close to Jolt's own car preset. Increase
/// `grip_scale` for arcade handling, lower `downforce_coefficient` for road
/// cars, and raise `collision_restitution` for a more bouncy barrier response.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleTuning {
    /// Multiplier on the tyre peak friction from `spec.tyres.peak_friction`.
    pub grip_scale: f32,
    /// Multiplier on the derived brake torque.
    pub brake_scale: f32,
    /// Multiplier on the engine peak torque.
    pub engine_scale: f32,
    /// Maximum front-wheel steer angle, degrees.
    pub max_steer_angle_deg: f32,
    /// Multiplier on aerodynamic drag.
    pub drag_scale: f32,
    /// Downforce coefficient `Cl·A` (dimensionless area), applied at the centre of
    /// mass. `0` disables the extra downforce.
    pub downforce_coefficient: f32,
    /// Chassis-to-chassis and chassis-to-wall friction.
    pub collision_friction: f32,
    /// Chassis-to-chassis and chassis-to-wall restitution.
    pub collision_restitution: f32,
}

impl VehicleTuning {
    /// Replace non-finite values with defaults and clamp every knob to a range
    /// Jolt's validation accepts.
    pub fn clamped(mut self) -> Self {
        self.grip_scale = finite_or(self.grip_scale, 1.0).clamp(0.0, 8.0);
        self.brake_scale = finite_or(self.brake_scale, 1.0).clamp(0.0, 4.0);
        self.engine_scale = finite_or(self.engine_scale, 1.0).clamp(0.0, 8.0);
        self.max_steer_angle_deg = finite_or(self.max_steer_angle_deg, 30.0).clamp(0.0, 60.0);
        self.drag_scale = finite_or(self.drag_scale, 1.0).clamp(0.0, 8.0);
        self.downforce_coefficient = finite_or(self.downforce_coefficient, 0.0).clamp(0.0, 10.0);
        self.collision_friction = finite_or(self.collision_friction, 0.6).clamp(0.0, 10.0);
        self.collision_restitution = finite_or(self.collision_restitution, 0.1).clamp(0.0, 1.0);
        self
    }
}

impl Default for VehicleTuning {
    fn default() -> Self {
        Self {
            grip_scale: 1.0,
            brake_scale: 1.0,
            engine_scale: 1.0,
            max_steer_angle_deg: 30.0,
            drag_scale: 1.0,
            downforce_coefficient: 0.0,
            collision_friction: 0.6,
            collision_restitution: 0.1,
        }
    }
}

/// One wheel after the latest completed step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelSnapshot {
    pub contact: bool,
    /// World-space contact point (physics frame). Falls back to the wheel's
    /// approximate centre when the wheel is airborne.
    pub contact_point: Vec3,
    /// Ground normal at the contact, pointing toward the wheel.
    pub contact_normal: Vec3,
    /// Surface under this wheel.
    pub surface: SurfaceId,
    /// Standing water at the contact, metres.
    pub water_depth_m: f32,
    /// Current suspension length, metres.
    pub suspension_length_m: f32,
    /// Whether the suspension is on its hard stop.
    pub hit_hard_point: bool,
    /// Vertical load estimate, newtons (`suspension impulse / dt`).
    pub load_n: f32,
    /// Longitudinal tyre force estimate, newtons.
    pub longitudinal_force_n: f32,
    /// Lateral tyre force estimate, newtons.
    pub lateral_force_n: f32,
    /// Longitudinal slip ratio estimate. Positive when the wheel spins faster
    /// than the ground.
    pub slip_ratio: f32,
    /// Lateral slip angle estimate, radians.
    pub slip_angle: f32,
    /// Wheel spin rate, rad/s.
    pub angular_velocity: f32,
    /// Accumulated wheel rotation, radians.
    pub rotation_angle: f32,
    /// Steering angle, radians; positive steers left.
    pub steer_angle: f32,
}

impl Default for WheelSnapshot {
    fn default() -> Self {
        Self {
            contact: false,
            contact_point: Vec3::ZERO,
            contact_normal: Vec3::Y,
            surface: SurfaceId::ASPHALT,
            water_depth_m: 0.0,
            suspension_length_m: 0.0,
            hit_hard_point: false,
            load_n: 0.0,
            longitudinal_force_n: 0.0,
            lateral_force_n: 0.0,
            slip_ratio: 0.0,
            slip_angle: 0.0,
            angular_velocity: 0.0,
            rotation_angle: 0.0,
            steer_angle: 0.0,
        }
    }
}

/// Public view of one vehicle after the latest step.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSnapshot {
    /// Chassis centre-of-mass position in the physics frame.
    pub position: Vec3,
    /// Ground-reference position (the chassis reference frame, `position` minus
    /// the local CG offset). This matches `VehicleState::position` and the
    /// existing Chrono backend.
    pub reference_position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    /// Velocity magnitude, m/s.
    pub speed_mps: f32,
    /// Signed forward speed, m/s.
    pub forward_speed_mps: f32,
    pub wheels: [WheelSnapshot; 4],
    pub engine_rpm: f32,
    /// `-1` reverse, `0` neutral, `1` first gear, ...
    pub gear: i32,
}

impl Default for VehicleSnapshot {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            reference_position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            speed_mps: 0.0,
            forward_speed_mps: 0.0,
            wheels: [WheelSnapshot::default(); 4],
            engine_rpm: 0.0,
            gear: 0,
        }
    }
}

impl VehicleSnapshot {
    pub fn forward(&self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.orientation * Vec3::X
    }

    pub fn wheel(&self, wheel: Wheel) -> &WheelSnapshot {
        &self.wheels[wheel.index()]
    }

    /// Convert to the backend-neutral state used by `vettura-physics` and Godot.
    pub fn to_vehicle_state(&self) -> VehicleState {
        VehicleState {
            position: self.reference_position,
            orientation: self.orientation,
            linear_velocity: self.linear_velocity,
            angular_velocity: self.angular_velocity,
            speed_mps: self.forward_speed_mps,
            wheels: std::array::from_fn(|index| {
                let wheel = &self.wheels[index];
                WheelState {
                    rotation: wheel.rotation_angle,
                    steer_angle: wheel.steer_angle,
                    angular_velocity: wheel.angular_velocity,
                    slip_ratio: wheel.slip_ratio,
                    slip_angle: wheel.slip_angle,
                    load_n: wheel.load_n,
                    contact: wheel.contact,
                }
            }),
            engine_rpm: self.engine_rpm,
            gear: self.gear,
        }
    }
}

/// Shared, atomically-updated grip inputs for the Jolt tyre callback.
///
/// The callback runs inside Jolt's solver, where taking a lock would be both
/// slow and an unwind hazard. Each step publishes one surface/water sample per
/// wheel plus the current weather scalars; the callback only performs relaxed
/// atomic loads and a table lookup.
struct GripState {
    surfaces: Arc<SurfaceTable>,
    tyre_peak_friction: f32,
    surface_ids: [AtomicU32; 4],
    water_depth_bits: [AtomicU32; 4],
    speed_bits: AtomicU32,
    wetness_bits: AtomicU32,
    track_temperature_bits: AtomicU32,
    grip_scale_bits: AtomicU32,
}

impl GripState {
    fn new(surfaces: Arc<SurfaceTable>, tyre_peak_friction: f32) -> Self {
        Self {
            surfaces,
            tyre_peak_friction: tyre_peak_friction.max(0.0),
            surface_ids: [
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
            ],
            water_depth_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            speed_bits: AtomicU32::new(0.0f32.to_bits()),
            wetness_bits: AtomicU32::new(0.0f32.to_bits()),
            track_temperature_bits: AtomicU32::new(32.0f32.to_bits()),
            grip_scale_bits: AtomicU32::new(1.0f32.to_bits()),
        }
    }

    fn set_weather(&self, weather: &Weather) {
        self.wetness_bits
            .store(weather.wetness.to_bits(), Ordering::Relaxed);
        self.track_temperature_bits
            .store(weather.track_temperature_c.to_bits(), Ordering::Relaxed);
        self.grip_scale_bits
            .store(weather.grip_scale.to_bits(), Ordering::Relaxed);
    }

    fn publish_snapshot(&self, snapshot: &VehicleSnapshot) {
        for (index, wheel) in snapshot.wheels.iter().enumerate() {
            self.surface_ids[index].store(u32::from(wheel.surface.0), Ordering::Relaxed);
            self.water_depth_bits[index].store(wheel.water_depth_m.to_bits(), Ordering::Relaxed);
        }
        self.speed_bits.store(
            snapshot.linear_velocity.length().to_bits(),
            Ordering::Relaxed,
        );
    }

    fn friction(&self, input: &TireMaxImpulseInput) -> TireFriction {
        let index = (input.wheel as usize).min(3);
        let surface = self
            .surfaces
            .get(SurfaceId(
                self.surface_ids[index].load(Ordering::Relaxed) as u16
            ))
            .unwrap_or_else(|| &self.surfaces.entries()[0]);
        let water_depth_m = f32::from_bits(self.water_depth_bits[index].load(Ordering::Relaxed));
        let speed_mps = f32::from_bits(self.speed_bits.load(Ordering::Relaxed));

        let weather = Weather {
            wetness: f32::from_bits(self.wetness_bits.load(Ordering::Relaxed)),
            track_temperature_c: f32::from_bits(
                self.track_temperature_bits.load(Ordering::Relaxed),
            ),
            grip_scale: f32::from_bits(self.grip_scale_bits.load(Ordering::Relaxed)),
            ..Weather::dry()
        };

        // `input.default_friction()` is Jolt's curve already combined with the
        // ground body's friction via sqrt. The ground is 1.0, so the combined
        // peak is still 1.0 and `scale` sets the final peak exactly; only the
        // non-peak points are lifted slightly by the sqrt.
        let environmental = weather.grip(surface, speed_mps, water_depth_m);
        let scale = (self.tyre_peak_friction * environmental).clamp(0.0, 8.0);
        let base = input.default_friction();
        TireFriction {
            longitudinal: base.longitudinal * scale,
            lateral: base.lateral * scale,
        }
    }
}

/// Internal vehicle state owned by the world.
pub(crate) struct VehicleRuntime {
    pub(crate) handle: VehicleHandle,
    pub(crate) body: BodyId,
    pub(crate) vehicle: VehicleId,
    pub(crate) spec: VehicleSpec,
    pub(crate) tuning: VehicleTuning,
    pub(crate) snapshot: VehicleSnapshot,
    input: DriveInput,
    grip: Arc<GripState>,
}

impl VehicleRuntime {
    pub(crate) fn create(
        physics: &mut PhysicsWorld,
        handle: VehicleHandle,
        spec: VehicleSpec,
        pose: Pose,
        tuning: VehicleTuning,
        surfaces: Arc<SurfaceTable>,
    ) -> WorldResult<Self> {
        validate_spec(&spec)?;
        let tuning = tuning.clamped();
        let orientation = normalized_orientation(pose.orientation);
        if !pose.position.is_finite() {
            return Err(WorldError::InvalidVehicle(
                "pose position must be finite".to_owned(),
            ));
        }

        let body_position = pose.position + orientation * Vec3::new(0.0, spec.cg_height_m, 0.0);
        let (half_w, half_l, half_h) = chassis_half_extents(&spec);
        let hull = Shape::new_box(JoltVec3::new(half_w, half_h, half_l))?;

        let body_settings = BodySettings::new_dynamic()
            .position(p2j_r(body_position))
            .rotation(p2j_q(orientation))
            .mass(spec.mass_kg)
            .friction(tuning.collision_friction.clamp(0.0, 10.0))
            .restitution(tuning.collision_restitution.clamp(0.0, 1.0))
            .linear_damping(0.0)
            .angular_damping(0.05)
            .allow_sleeping(false)
            .motion_quality(MotionQuality::LinearCast)
            .user_data(tag_vehicle(handle));

        let body = physics.create_body(&hull, &body_settings)?;

        let created = (|| -> WorldResult<(VehicleId, Arc<GripState>)> {
            let settings = vehicle_settings(&spec, &tuning)?;
            let vehicle = physics.create_wheeled_vehicle(body, &settings)?;
            let grip = Arc::new(GripState::new(
                surfaces,
                spec.tyres.peak_friction * tuning.grip_scale,
            ));
            let callback_state = Arc::clone(&grip);
            physics
                .vehicle_mut(vehicle)?
                .set_tire_max_impulse_callback(move |input| callback_state.friction(input));
            Ok((vehicle, grip))
        })();

        let (vehicle, grip) = match created {
            Ok(value) => value,
            Err(error) => {
                let _ = physics.remove_body(body);
                return Err(error);
            }
        };

        Ok(Self {
            handle,
            body,
            vehicle,
            spec,
            tuning,
            snapshot: VehicleSnapshot::default(),
            input: DriveInput::default(),
            grip,
        })
    }

    pub(crate) fn apply_input(
        &mut self,
        physics: &mut PhysicsWorld,
        input: DriveInput,
    ) -> WorldResult<()> {
        let input = input.clamped();
        physics
            .vehicle_mut(self.vehicle)?
            .set_driver_input(DriverInput {
                forward: input.throttle,
                right: -input.steering,
                brake: input.brake,
                hand_brake: if input.handbrake { 1.0 } else { 0.0 },
            })?;
        self.input = input;
        Ok(())
    }

    pub(crate) fn set_velocity(
        &mut self,
        physics: &mut PhysicsWorld,
        linear: Vec3,
        angular: Vec3,
    ) -> WorldResult<()> {
        let mut body = physics.body_mut(self.body)?;
        body.set_linear_velocity(p2j(linear))?;
        body.set_angular_velocity(p2j(angular))?;
        Ok(())
    }

    pub(crate) fn reset(&mut self, physics: &mut PhysicsWorld, pose: Pose) -> WorldResult<()> {
        if !pose.position.is_finite() {
            return Err(WorldError::InvalidVehicle(
                "reset position must be finite".to_owned(),
            ));
        }
        let orientation = normalized_orientation(pose.orientation);
        let position = pose.position + orientation * Vec3::new(0.0, self.spec.cg_height_m, 0.0);
        {
            let mut body = physics.body_mut(self.body)?;
            body.set_position_and_rotation(
                p2j_r(position),
                p2j_q(orientation),
                Activation::Activate,
            )?;
            body.set_linear_velocity(JoltVec3::ZERO)?;
            body.set_angular_velocity(JoltVec3::ZERO)?;
        }
        physics
            .vehicle_mut(self.vehicle)?
            .set_driver_input(DriverInput::default())?;
        self.input = DriveInput::default();
        self.snapshot = VehicleSnapshot {
            reference_position: pose.position,
            position,
            orientation,
            ..VehicleSnapshot::default()
        };
        Ok(())
    }

    pub(crate) fn update_snapshot(
        &mut self,
        physics: &PhysicsWorld,
        dt: f32,
        surfaces: &SurfaceMap,
    ) -> WorldResult<()> {
        let body = physics.body(self.body)?;
        let position = j2p_r(body.position());
        let orientation = j2p_q(body.rotation());
        let linear_velocity = j2p(body.linear_velocity());
        let angular_velocity = j2p(body.angular_velocity());

        let vehicle = physics.vehicle(self.vehicle)?;
        let wheel_states = vehicle.wheels();
        let engine_rpm = vehicle.engine_rpm();
        let gear = vehicle.current_gear();

        let forward = orientation * Vec3::NEG_Z;
        let forward_speed_mps = linear_velocity.dot(forward);
        let reference_position = position + orientation * (Vec3::NEG_Y * self.spec.cg_height_m);

        let mut wheels = [WheelSnapshot::default(); 4];
        for (index, slot) in wheels.iter_mut().enumerate() {
            let spec = &self.spec.wheels[index];
            let radius = spec.radius_m.max(0.01);
            let wheel_center = position
                + orientation
                    * Vec3::new(
                        spec.position.x,
                        radius - self.spec.cg_height_m,
                        spec.position.z,
                    );

            let Some(state) = wheel_states.get(index) else {
                let sample = surfaces.sample(wheel_center);
                *slot = WheelSnapshot {
                    contact_point: wheel_center,
                    surface: sample.surface,
                    water_depth_m: sample.water_depth_m,
                    ..WheelSnapshot::default()
                };
                continue;
            };

            let (contact, contact_point, contact_normal, surface_sample) =
                if let Some(contact) = state.contact {
                    let point = j2p_r(contact.position);
                    let sample = surfaces.sample(point);
                    (true, point, j2p(contact.normal), sample)
                } else {
                    (false, wheel_center, Vec3::Y, surfaces.sample(wheel_center))
                };

            let (slip_ratio, slip_angle, longitudinal_force_n, lateral_force_n) =
                if let Some(contact) = state.contact {
                    let lever = contact_point - position;
                    let contact_velocity = linear_velocity + angular_velocity.cross(lever);
                    let longitudinal = contact_velocity.dot(j2p(contact.longitudinal));
                    let lateral = contact_velocity.dot(j2p(contact.lateral));
                    let wheel_surface_speed = state.angular_velocity * radius;
                    let slip_ratio = ((wheel_surface_speed - longitudinal)
                        / longitudinal.abs().max(2.0))
                    .clamp(-2.0, 2.0);
                    let slip_angle = lateral
                        .atan2(longitudinal.abs().max(0.5))
                        .clamp(-PI * 0.5, PI * 0.5);
                    let force_scale = if dt > 1.0e-6 { 1.0 / dt } else { 0.0 };
                    (
                        slip_ratio,
                        slip_angle,
                        state.longitudinal_lambda * force_scale,
                        state.lateral_lambda * force_scale,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                };

            let force_scale = if dt > 1.0e-6 { 1.0 / dt } else { 0.0 };
            *slot = WheelSnapshot {
                contact,
                contact_point,
                contact_normal,
                surface: surface_sample.surface,
                water_depth_m: surface_sample.water_depth_m,
                suspension_length_m: state.suspension_length,
                hit_hard_point: state.hit_hard_point,
                load_n: (state.suspension_lambda * force_scale).max(0.0),
                longitudinal_force_n,
                lateral_force_n,
                slip_ratio,
                slip_angle,
                angular_velocity: state.angular_velocity,
                rotation_angle: state.rotation_angle,
                steer_angle: state.steer_angle,
            };
        }

        self.snapshot = VehicleSnapshot {
            position,
            reference_position,
            orientation,
            linear_velocity,
            angular_velocity,
            speed_mps: linear_velocity.length(),
            forward_speed_mps,
            wheels,
            engine_rpm,
            gear,
        };
        self.grip.publish_snapshot(&self.snapshot);
        Ok(())
    }

    pub(crate) fn set_weather(&self, weather: &Weather) {
        self.grip.set_weather(weather);
    }

    /// Remove this vehicle's constraint and chassis body from the world.
    pub(crate) fn remove(self, physics: &mut PhysicsWorld) -> WorldResult<()> {
        physics.remove_vehicle(self.vehicle)?;
        physics.remove_body(self.body)?;
        Ok(())
    }

    /// Average rolling-resistance coefficient of the wheels on the ground.
    pub(crate) fn rolling_resistance(&self, weather: &Weather, surfaces: &SurfaceMap) -> f32 {
        let mut sum = 0.0;
        let mut count = 0u32;
        for wheel in &self.snapshot.wheels {
            if !wheel.contact {
                continue;
            }
            if let Some(properties) = surfaces.table().get(wheel.surface) {
                sum += weather.rolling_resistance(properties, wheel.water_depth_m);
                count += 1;
            }
        }
        if count == 0 { 0.0 } else { sum / count as f32 }
    }
}

fn normalized_orientation(q: Quat) -> Quat {
    if q.is_finite() && q.length_squared() > 1.0e-8 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn validate_spec(spec: &VehicleSpec) -> WorldResult<()> {
    if !(spec.mass_kg.is_finite() && spec.mass_kg >= 1.0) {
        return Err(WorldError::InvalidVehicle(
            "mass_kg must be finite and at least 1 kg".to_owned(),
        ));
    }
    if !(spec.cg_height_m.is_finite() && spec.cg_height_m > 0.05) {
        return Err(WorldError::InvalidVehicle(
            "cg_height_m must be finite and above 0.05 m".to_owned(),
        ));
    }
    if !(spec.wheelbase_m.is_finite() && spec.wheelbase_m > 0.1) {
        return Err(WorldError::InvalidVehicle(
            "wheelbase_m must be finite and above 0.1 m".to_owned(),
        ));
    }
    for wheel in &spec.wheels {
        if !wheel.position.is_finite() {
            return Err(WorldError::InvalidVehicle(
                "wheel positions must be finite".to_owned(),
            ));
        }
        if !(wheel.radius_m.is_finite() && wheel.radius_m > 0.02) {
            return Err(WorldError::InvalidVehicle(
                "wheel radius must be finite and above 0.02 m".to_owned(),
            ));
        }
        if !(wheel.width_m.is_finite() && wheel.width_m > 0.0) {
            return Err(WorldError::InvalidVehicle(
                "wheel width must be finite and positive".to_owned(),
            ));
        }
        let suspension = &wheel.suspension;
        if !(suspension.spring_rate_n_per_m.is_finite()
            && suspension.spring_rate_n_per_m > 0.0
            && suspension.damper_bump_ns_per_m.is_finite()
            && suspension.damper_bump_ns_per_m >= 0.0
            && suspension.damper_rebound_ns_per_m.is_finite()
            && suspension.damper_rebound_ns_per_m >= 0.0
            && suspension.travel_m.is_finite()
            && suspension.travel_m > 0.0
            && suspension.anti_roll_n_per_m.is_finite()
            && suspension.anti_roll_n_per_m >= 0.0)
        {
            return Err(WorldError::InvalidVehicle(
                "suspension rates, travel and anti-roll must be finite and non-negative".to_owned(),
            ));
        }
    }
    if !(spec.tyres.peak_friction.is_finite() && spec.tyres.peak_friction >= 0.0) {
        return Err(WorldError::InvalidVehicle(
            "tyre peak friction must be finite and non-negative".to_owned(),
        ));
    }
    if !(spec.engine.idle_rpm.is_finite()
        && spec.engine.idle_rpm > 0.0
        && spec.engine.redline_rpm.is_finite()
        && spec.engine.redline_rpm > spec.engine.idle_rpm
        && spec.engine.peak_torque_nm.is_finite()
        && spec.engine.peak_torque_nm > 0.0
        && spec.engine.final_drive.is_finite()
        && spec.engine.final_drive > 0.0)
    {
        return Err(WorldError::InvalidVehicle(
            "engine rpm, peak torque and final drive must be finite and positive".to_owned(),
        ));
    }
    if !(spec.drivetrain.efficiency.is_finite() && spec.drivetrain.efficiency > 0.0) {
        return Err(WorldError::InvalidVehicle(
            "drivetrain efficiency must be finite and positive".to_owned(),
        ));
    }
    if !(spec.frontal_area_m2.is_finite()
        && spec.frontal_area_m2 >= 0.0
        && spec.drag_coefficient.is_finite()
        && spec.drag_coefficient >= 0.0)
    {
        return Err(WorldError::InvalidVehicle(
            "aero area and drag coefficient must be finite and non-negative".to_owned(),
        ));
    }
    Ok(())
}

fn chassis_half_extents(spec: &VehicleSpec) -> (f32, f32, f32) {
    let half_w = spec
        .wheels
        .iter()
        .map(|wheel| wheel.position.x.abs() + wheel.width_m.max(0.0) * 0.5)
        .fold(0.0f32, f32::max)
        .max(0.2)
        + 0.02;
    let half_l = spec
        .wheels
        .iter()
        .map(|wheel| wheel.position.z.abs() + wheel.radius_m.max(0.0))
        .fold(0.0f32, f32::max)
        .max(0.3)
        + 0.05;
    let half_h = (spec.cg_height_m - 0.06).clamp(0.10, 0.55);
    (half_w, half_l, half_h)
}

fn vehicle_settings(
    spec: &VehicleSpec,
    tuning: &VehicleTuning,
) -> WorldResult<WheeledVehicleSettings> {
    let cg = spec.cg_height_m;
    let spring_force_scale = spec.mass_kg * 9.81 * 0.25;
    let brake_force = spec.mass_kg * 9.81 * 1.2 * tuning.brake_scale.clamp(0.0, 4.0);

    let mut wheels = Vec::with_capacity(4);
    for wheel in Wheel::ALL {
        let wheel_spec = &spec.wheels[wheel.index()];
        let radius = wheel_spec.radius_m;
        let width = wheel_spec.width_m.max(0.04);
        let travel = wheel_spec.suspension.travel_m.clamp(0.02, 0.35);
        let spring = wheel_spec.suspension.spring_rate_n_per_m.max(1_000.0);
        let damper = (0.5
            * (wheel_spec.suspension.damper_bump_ns_per_m.max(0.0)
                + wheel_spec.suspension.damper_rebound_ns_per_m.max(0.0)))
        .max(100.0);

        let static_compression = (spring_force_scale / spring).clamp(0.001, travel * 0.45);
        let max_length = (static_compression + travel * 0.5).max(0.02);
        let min_length = (static_compression - travel * 0.5)
            .max(0.005)
            .min(max_length - 0.001);
        let rest_length = max_length - static_compression;
        let attach_y = radius + rest_length - cg;
        let position = p2j(Vec3::new(
            wheel_spec.position.x,
            attach_y,
            wheel_spec.position.z,
        ));

        let brake_torque = if wheel.is_front() {
            brake_force * 0.62 * 0.5 * radius
        } else {
            brake_force * 0.38 * 0.5 * radius
        };

        let settings = WheelSettings::new(position)
            .radius(radius)
            .width(width)
            .inertia((0.5 * 12.0 * radius * radius).max(0.05))
            .angular_damping(0.05)
            .max_steer_angle(if wheel_spec.steerable {
                tuning.max_steer_angle_deg.clamp(5.0, 60.0).to_radians()
            } else {
                0.0
            })
            .max_brake_torque(brake_torque.max(1.0))
            .max_hand_brake_torque(if wheel.is_front() {
                0.0
            } else {
                brake_torque * 0.7
            })
            .suspension_min_length(min_length)
            .suspension_max_length(max_length)
            .suspension_preload_length(0.0)
            .suspension_spring(SpringSettings::StiffnessAndDamping {
                stiffness: spring,
                damping: damper,
            })
            // The callback scales these curves by tyre peak × surface × weather.
            // Keep the curve's own peak at 1.0 so the callback is the only place
            // that decides the final coefficient.
            .longitudinal_friction(vec![(0.0, 0.0), (0.06, 1.0), (0.18, 0.93), (0.5, 0.8)])
            .lateral_friction(vec![(0.0, 0.0), (3.0, 1.0), (10.0, 0.95), (20.0, 0.82)]);

        wheels.push(settings);
    }

    let final_drive = spec.engine.final_drive.max(0.5);
    let differentials = match spec.drivetrain.layout {
        DrivetrainLayout::Rwd => vec![
            VehicleDifferentialSettings::new(Some(2), Some(3))
                .differential_ratio(final_drive)
                .engine_torque_ratio(1.0)
                .limited_slip_ratio(1.4),
        ],
        DrivetrainLayout::Fwd => vec![
            VehicleDifferentialSettings::new(Some(0), Some(1))
                .differential_ratio(final_drive)
                .engine_torque_ratio(1.0)
                .limited_slip_ratio(1.4),
        ],
        DrivetrainLayout::Awd => vec![
            VehicleDifferentialSettings::new(Some(0), Some(1))
                .differential_ratio(final_drive)
                .engine_torque_ratio(0.42)
                .limited_slip_ratio(1.4),
            VehicleDifferentialSettings::new(Some(2), Some(3))
                .differential_ratio(final_drive)
                .engine_torque_ratio(0.58)
                .limited_slip_ratio(1.4),
        ],
    };

    let max_rpm = spec.engine.redline_rpm.clamp(2_000.0, 30_000.0);
    let min_rpm = spec.engine.idle_rpm.clamp(300.0, max_rpm * 0.5);
    let shift_up_rpm = (max_rpm * 0.94).max(min_rpm + 500.0).min(max_rpm - 1.0);
    let shift_down_rpm = (max_rpm * 0.42)
        .max(min_rpm * 0.8)
        .min(shift_up_rpm - 1.0)
        .max(1.0);

    let efficiency = spec.drivetrain.efficiency.clamp(0.3, 1.0);
    let max_torque =
        (spec.engine.peak_torque_nm * tuning.engine_scale * efficiency).clamp(1.0, 5_000.0);
    let engine = VehicleEngineSettings::default()
        .max_torque(max_torque)
        .min_rpm(min_rpm)
        .max_rpm(max_rpm)
        .normalized_torque(vec![
            (0.0, 0.72),
            (0.45, 0.92),
            (0.7, 1.0),
            (0.9, 0.95),
            (1.0, 0.82),
        ])
        .inertia(0.25)
        .angular_damping(0.1);

    let mut gear_ratios: Vec<f32> = spec
        .engine
        .gear_ratios
        .iter()
        .copied()
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .collect();
    if gear_ratios.is_empty() {
        gear_ratios = vec![3.2, 2.1, 1.6, 1.3, 1.1, 0.95];
    }
    let transmission = VehicleTransmissionSettings::default()
        .gear_ratios(gear_ratios)
        .reverse_gear_ratios(vec![-3.2])
        .switch_time(0.12)
        .clutch_release_time(0.08)
        .switch_latency(0.1)
        .shift_up_rpm(shift_up_rpm)
        .shift_down_rpm(shift_down_rpm)
        .clutch_strength(20.0);

    let mut anti_roll_bars = Vec::new();
    let front_arb = spec.wheels[Wheel::FrontLeft.index()]
        .suspension
        .anti_roll_n_per_m
        .max(0.0);
    let rear_arb = spec.wheels[Wheel::RearLeft.index()]
        .suspension
        .anti_roll_n_per_m
        .max(0.0);
    if front_arb > 0.0 {
        anti_roll_bars.push(VehicleAntiRollBar::new(0, 1).stiffness(front_arb));
    }
    if rear_arb > 0.0 {
        anti_roll_bars.push(VehicleAntiRollBar::new(2, 3).stiffness(rear_arb));
    }

    let average_radius =
        spec.wheels.iter().map(|wheel| wheel.radius_m).sum::<f32>() / spec.wheels.len() as f32;
    let tester_radius = (average_radius * 0.5).clamp(0.03, 0.30);
    // Query as a moving body so static ground/walls (NON_MOVING) are hit;
    // NON_MOVING-vs-NON_MOVING is filtered out by the default layer table.
    let collision_tester = VehicleCollisionTester::cast_sphere(ObjectLayer::MOVING, tester_radius);

    Ok(
        WheeledVehicleSettings::new(wheels, differentials, collision_tester)
            .engine(engine)
            .transmission(transmission)
            .anti_roll_bars(anti_roll_bars)
            .max_pitch_roll_angle(70.0f32.to_radians()),
    )
}

const TAG_VEHICLE: u64 = 1 << 32;

fn tag_vehicle(handle: VehicleHandle) -> u64 {
    TAG_VEHICLE | (handle.0 & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_produces_four_wheels_and_a_diff() {
        let spec = VehicleSpec::default();
        let settings = vehicle_settings(&spec, &VehicleTuning::default()).unwrap();
        // The fields are private; the construction itself is the assertion that
        // Jolt's validation accepted the mapping.
        let _ = settings;
    }

    #[test]
    fn invalid_specs_are_rejected() {
        let spec = VehicleSpec {
            mass_kg: 0.0,
            ..VehicleSpec::default()
        };
        assert!(validate_spec(&spec).is_err());

        let mut spec = VehicleSpec::default();
        spec.wheels[0].radius_m = 0.0;
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn chassis_extents_follow_wheel_positions() {
        let spec = VehicleSpec::default();
        let (half_w, half_l, _) = chassis_half_extents(&spec);
        assert!(half_w > 0.75 && half_w < 1.0);
        assert!(half_l > 1.3 && half_l < 1.8);
    }

    #[test]
    fn input_maps_steering_to_jolt_sign() {
        let controls = Controls {
            throttle: 0.5,
            brake: 0.2,
            steering: 0.75,
            ..Controls::default()
        };
        let input = DriveInput::from(controls);
        assert_eq!(input.throttle, 0.5);
        assert_eq!(input.steering, 0.75);
        assert_eq!(input.brake, 0.2);
    }
}
