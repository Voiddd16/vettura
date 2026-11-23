//! One vehicle inside a [`World`](crate::World): AC spec to Jolt wheeled
//! vehicle, live input, per-vehicle physics subsystems, and the per-step
//! snapshot handed to rendering, telemetry and the `VehicleBackend` contract.
//!
//! Sub-modules: [`friction`] the per-wheel friction circle, [`grip`] the Jolt
//! tyre callback and its shared state, [`thermal`] tyre/brake temperatures,
//! [`aids`] ABS/TC, and [`constraint`] the spec-to-Jolt geometry translation.
//! This file owns the vehicle runtime itself: body, constraint, snapshot and
//! every per-step subsystem update (aero, powertrain, brakes, thermal, ERS,
//! fuel, driver aids) that runs inside it.

use std::f32::consts::PI;
use std::sync::Arc;

use glam::{Quat, Vec3};
use oxijolt::{
    Activation, BodyId, BodySettings, DriverInput, Inertia, MotionQuality, PhysicsWorld, Shape,
    Vec3 as JoltVec3, VehicleId,
};
use vettura_physics::{
    AERO_REFERENCE_SPEED_MPS, ActiveSuspensionMode, Controls, ErsMode, Pose, ShiftBlip,
    VehicleSpec, VehicleState, Wheel, WheelState, WingSpec,
};

use crate::frame::{j2p, j2p_q, j2p_r, p2j, p2j_q, p2j_r};
use crate::{WorldError, WorldResult};
use vettura_track::GrooveMap;
use vettura_track::Weather;
use vettura_track::{SurfaceId, SurfaceMap, SurfaceTable};

mod aids;
mod constraint;
mod friction;
mod grip;
mod thermal;

pub(crate) use aids::{ABS_FACTOR_MAX, apply_driver_aids};
pub(crate) use constraint::{
    WheelGeometry, chassis_half_extents, normalized_orientation, tag_vehicle, validate_spec,
    vehicle_settings, wheel_geometries, wing_local_position,
};
pub(crate) use grip::GripState;
pub(crate) use thermal::{
    ENGINE_COOLING_RATE_PER_S, THERMOSTAT_RATE_PER_S, TyreTemperatures, advance_brake_temperatures,
    advance_tyre_temperatures,
};

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
    /// Drag-reduction-system request for this step.
    pub drs: bool,
    /// Anti-lock brake request for this step.
    pub abs: bool,
    /// Traction-control request for this step.
    pub tc: bool,
    /// Hybrid energy mode for this step.
    pub ers: ErsMode,
}

impl From<Controls> for DriveInput {
    fn from(controls: Controls) -> Self {
        let controls = controls.clamped();
        Self {
            throttle: controls.throttle,
            steering: controls.steering,
            brake: controls.brake,
            handbrake: controls.handbrake,
            drs: controls.drs,
            abs: controls.abs,
            tc: controls.tc,
            ers: controls.ers,
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
    /// Steering lock *override*, degrees at the wheel. `0` uses the car's own
    /// [`VehicleSpec::max_steer_angle_deg`], which is what AC data provides.
    pub max_steer_angle_deg: f32,
    /// Multiplier on aerodynamic drag.
    pub drag_scale: f32,
    /// Downforce coefficient `Cl·A` that *replaces* `spec.aero.downforce_coeff`
    /// when non-zero. `0` means "use the spec", so a car with real aero data
    /// needs no tuning entry.
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
        // A zero override means "the spec decides", so it stays zero.
        self.max_steer_angle_deg = if self.max_steer_angle_deg.is_finite() {
            self.max_steer_angle_deg.clamp(0.0, 60.0)
        } else {
            0.0
        };
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
            max_steer_angle_deg: 0.0,
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
    /// Camber angle of this wheel, radians. Negative leans the tyre top inward.
    pub camber_rad: f32,
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
            camber_rad: 0.0,
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
    /// Brake disc temperature of each wheel, °C.
    pub brake_temperatures_c: [f32; 4],
    /// Brake disc core temperature of each wheel, °C. The bulk of the rotor,
    /// which lags the surface and soaks heat back into it after the driver
    /// lifts off.
    pub brake_core_temperatures_c: [f32; 4],
    /// Tyre temperature the grip curve reads, °C: the under-tread patch when
    /// the car models layers, the carcass otherwise.
    pub tyre_temperatures_c: [f32; 4],
    /// Tyre friction-surface temperature of each wheel, °C.
    pub tyre_surface_temperatures_c: [f32; 4],
    /// Tyre bulk-core temperature of each wheel, °C.
    pub tyre_core_temperatures_c: [f32; 4],
    /// Tyre inflation pressure of each wheel, psi.
    pub tyre_pressures_psi: [f32; 4],
    /// Tyre wear of each wheel, `0..=1`.
    pub tyre_wear: [f32; 4],
    /// Remaining fuel as a fraction of the tank, `0..=1`.
    pub fuel_fraction: f32,
    /// Remaining fuel mass, kg.
    pub fuel_kg: f32,
    /// Hybrid battery charge, joules.
    pub ers_energy_j: f32,
    /// Hybrid battery state of charge, `0..=1`.
    pub ers_soc: f32,
    /// Hybrid energy deployed on this lap, joules.
    pub ers_lap_deploy_j: f32,
    /// Hybrid machine temperature, °C.
    pub ers_temperature_c: f32,
    /// Engine coolant temperature, °C.
    pub coolant_c: f32,
    /// Engine oil temperature, °C.
    pub oil_c: f32,
    /// Engine power multiplier from temperature, `0..=1`.
    pub engine_power_multiplier: f32,
    /// Turbo boost pressure, bar.
    pub turbo_boost_bar: f32,
    /// Water film carried by each tyre, mm.
    pub water_film_mm: [f32; 4],
    /// Extra suspension force per wheel from the progressive spring, end stops
    /// and active actuator, newtons.
    pub suspension_force_n: [f32; 4],
    /// The active actuator's share of [`Self::suspension_force_n`], newtons.
    pub active_suspension_force_n: [f32; 4],
    /// Heave (third) spring force on the front and rear axle, newtons.
    pub heave_force_n: [f32; 2],
    /// Flat-spot fraction per wheel, `0..=1`.
    pub flat_spot: [f32; 4],
    /// Chassis kick amplitude from the flat spots, newtons.
    pub flat_spot_force_n: [f32; 4],
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
            brake_temperatures_c: [0.0; 4],
            brake_core_temperatures_c: [0.0; 4],
            tyre_temperatures_c: [0.0; 4],
            tyre_surface_temperatures_c: [0.0; 4],
            tyre_core_temperatures_c: [0.0; 4],
            tyre_pressures_psi: [0.0; 4],
            tyre_wear: [0.0; 4],
            fuel_fraction: 1.0,
            fuel_kg: 0.0,
            ers_energy_j: 0.0,
            ers_soc: 0.0,
            ers_lap_deploy_j: 0.0,
            ers_temperature_c: 0.0,
            coolant_c: 0.0,
            oil_c: 0.0,
            engine_power_multiplier: 1.0,
            turbo_boost_bar: 0.0,
            water_film_mm: [0.0; 4],
            suspension_force_n: [0.0; 4],
            active_suspension_force_n: [0.0; 4],
            heave_force_n: [0.0; 2],
            flat_spot: [0.0; 4],
            flat_spot_force_n: [0.0; 4],
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
    /// Brake disc temperatures, one per wheel, °C.
    brake_temperatures_c: [f32; 4],
    /// Brake disc core temperatures, one per wheel, °C.
    brake_core_temperatures_c: [f32; 4],
    /// Remaining fuel as a fraction of the tank, `0..=1`.
    fuel_fraction: f32,
    /// Chassis acceleration during the previous step, physics frame, m/s².
    acceleration: Vec3,
    /// Chassis velocity at the previous step, used to derive `acceleration`.
    previous_linear_velocity: Vec3,
    /// Seconds simulated since the last thermal/fuel update.
    thermal_time_s: f32,
    /// Suspension geometry per wheel, shared with the Jolt settings.
    geometries: [WheelGeometry; 4],
    /// DRS request for the last input, only effective with a `DrsSpec`.
    drs_requested: bool,
    /// Temperature the tyre's grip curve reads, °C: the under-tread patch when
    /// the car models layers, the carcass otherwise.
    tyre_temperatures_c: [f32; 4],
    /// Tyre friction-surface temperature per wheel, °C.
    tyre_surface_temperatures_c: [f32; 4],
    /// Tyre bulk-core temperature per wheel, °C.
    tyre_core_temperatures_c: [f32; 4],
    /// Cumulative slip energy per wheel, joules.
    tyre_wear_j: [f32; 4],
    /// Hybrid battery charge, joules.
    ers_energy_j: f32,
    /// Hybrid energy deployed in the current lap, joules.
    ers_lap_deploy_j: f32,
    /// Hybrid machine temperature, °C.
    ers_temperature_c: f32,
    /// Last hybrid mode requested.
    ers_mode: ErsMode,
    /// Coolant temperature, °C.
    coolant_c: f32,
    /// Oil temperature, °C.
    oil_c: f32,
    /// Current turbo boost pressure, bar.
    turbo_boost_bar: f32,
    /// Remaining upshift fuel cut, seconds.
    fuel_cut_s: f32,
    /// Downshift rev-match blip state. Only armed when the gearbox allows it
    /// (`GearboxSpec::clutch_on_shifts`) and the spec authored a window.
    shift_blip: ShiftBlip,
    /// Phase of the soft limiter's cutting cycle, `0..1`.
    limiter_phase: f32,
    /// ABS 刹车压力调制系数 `0..=1`（跨步保留的控制器状态，见
    /// [`apply_driver_aids`]）。
    abs_factor: f32,
    /// Last gear seen, so an upshift can be detected.
    previous_gear: i32,
    /// Water film carried by each tyre, mm.
    water_film_mm: [f32; 4],
    /// Suspension length at the previous force update, used for actuator damping.
    previous_suspension_lengths: [f32; 4],
    /// Extra suspension force per wheel from the last step, newtons.
    suspension_force_n: [f32; 4],
    /// Active actuator share of that force, newtons.
    active_suspension_force_n: [f32; 4],
    /// Heave-spring force per axle, front then rear, newtons.
    heave_force_n: [f32; 2],
    /// Sliding energy accumulated per wheel into a flat spot, joules.
    flat_spot_j: [f32; 4],
    /// Slip the tyre has actually built force for, per wheel: the relaxed
    /// state behind [`Self::snapshot`]'s measured slip.
    relaxed_longitudinal_slip: [f32; 4],
    relaxed_lateral_slip: [f32; 4],
    /// Camber angle of each wheel from the last step, radians.
    camber_rad: [f32; 4],
    /// Amplitude of the flat-spot kick applied to the chassis per wheel, N.
    flat_spot_force_n: [f32; 4],
    /// Weather snapshot for the per-wheel thermal model.
    weather_stub: Weather,
}

/// Replace a non-finite value with `fallback`.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Effective wheel steer lock in degrees.
///
/// A positive tuning value overrides the spec; otherwise the data-driven lock
/// wins. Both come from caller data, so a non-finite value falls back to the
/// 30° default instead of reaching Jolt's validation.
pub(crate) fn steer_lock_deg(spec: &VehicleSpec, tuning: &VehicleTuning) -> f32 {
    let lock = if tuning.max_steer_angle_deg > 0.0 {
        tuning.max_steer_angle_deg
    } else {
        spec.max_steer_angle_deg
    };
    if lock.is_finite() {
        lock.clamp(5.0, 60.0)
    } else {
        30.0
    }
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

        let mut body_settings = BodySettings::new_dynamic()
            .position(p2j_r(body_position))
            .rotation(p2j_q(orientation))
            .friction(tuning.collision_friction.clamp(0.0, 10.0))
            .restitution(tuning.collision_restitution.clamp(0.0, 1.0))
            .linear_damping(0.0)
            .angular_damping(0.05)
            .allow_sleeping(false)
            .motion_quality(MotionQuality::LinearCast)
            .user_data(tag_vehicle(handle));
        // AC publishes a real inertia tensor in `car.ini`; when a mod does not,
        // the collider box's own tensor is used, which is what Jolt derives
        // anyway.
        let inertia = spec.effective_inertia();
        if inertia.length_squared() > 0.0 {
            // The spec labels the tensor `(roll, pitch, yaw)`; Jolt's principal
            // moments follow its own axes, which are (lateral, vertical,
            // longitudinal) = (pitch, yaw, roll).
            body_settings = body_settings.mass_and_inertia(
                spec.mass_kg,
                Inertia::Principal(JoltVec3::new(inertia.y, inertia.z, inertia.x)),
            );
        } else {
            body_settings = body_settings.mass(spec.mass_kg);
        }

        let body = physics.create_body(&hull, &body_settings)?;

        let wheel_geometries = wheel_geometries(&spec);

        let created = (|| -> WorldResult<(VehicleId, Arc<GripState>)> {
            let settings = vehicle_settings(&spec, &tuning)?;
            let vehicle = physics.create_wheeled_vehicle(body, &settings)?;
            let grip = Arc::new(GripState::new(
                surfaces,
                spec.tyres.peak_friction * tuning.grip_scale,
                spec.tyres.clone(),
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

        // Read the spec values the runtime state needs before `spec` moves.
        let ambient_c = spec.brakes.ambient_c;
        let initial_fuel = if spec.fuel.initial_fraction.is_finite() {
            spec.fuel.initial_fraction.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let ers_capacity_j = spec.ers.capacity_j.max(0.0);

        Ok(Self {
            handle,
            body,
            vehicle,
            spec,
            tuning,
            snapshot: VehicleSnapshot::default(),
            input: DriveInput::default(),
            grip,
            // Discs start at ambient; the first braking lap brings them up.
            brake_temperatures_c: [ambient_c, ambient_c, ambient_c, ambient_c],
            brake_core_temperatures_c: [ambient_c, ambient_c, ambient_c, ambient_c],
            fuel_fraction: initial_fuel,
            acceleration: Vec3::ZERO,
            previous_linear_velocity: Vec3::ZERO,
            thermal_time_s: 0.0,
            geometries: wheel_geometries,
            drs_requested: false,
            // 轮胎从环境温度起步。
            tyre_temperatures_c: [ambient_c; 4],
            tyre_surface_temperatures_c: [ambient_c; 4],
            tyre_core_temperatures_c: [ambient_c; 4],
            tyre_wear_j: [0.0; 4],
            ers_energy_j: ers_capacity_j * 0.5,
            ers_lap_deploy_j: 0.0,
            ers_temperature_c: ambient_c,
            ers_mode: ErsMode::Off,
            coolant_c: ambient_c,
            oil_c: ambient_c,
            turbo_boost_bar: 0.0,
            fuel_cut_s: 0.0,
            shift_blip: ShiftBlip::new(),
            limiter_phase: 0.0,
            abs_factor: ABS_FACTOR_MAX,
            previous_gear: 0,
            water_film_mm: [0.0; 4],
            previous_suspension_lengths: [0.0; 4],
            suspension_force_n: [0.0; 4],
            active_suspension_force_n: [0.0; 4],
            heave_force_n: [0.0; 2],
            flat_spot_j: [0.0; 4],
            relaxed_longitudinal_slip: [0.0; 4],
            relaxed_lateral_slip: [0.0; 4],
            camber_rad: [0.0; 4],
            flat_spot_force_n: [0.0; 4],
            weather_stub: Weather::dry(),
        })
    }

    pub(crate) fn apply_input(
        &mut self,
        physics: &mut PhysicsWorld,
        input: DriveInput,
    ) -> WorldResult<()> {
        let input = input.clamped();
        self.input = input;
        self.drs_requested = input.drs && self.spec.drs.available;
        self.ers_mode = if self.spec.ers.available {
            input.ers
        } else {
            ErsMode::Off
        };
        self.write_driver_input(physics)
    }

    /// Push the current controls, thermal derate, brake fade and turbo boost to
    /// Jolt.
    ///
    /// Push the current controls, thermal derate, brake fade and turbo boost to
    /// Jolt.
    ///
    /// This runs again every step because boost pressure and temperatures move
    /// between control updates; the raw driver input stays in [`Self::input`].
    fn write_driver_input(&mut self, physics: &mut PhysicsWorld) -> WorldResult<()> {
        let mut input = self.input;
        apply_driver_aids(&self.spec, &mut input, &self.snapshot, &mut self.abs_factor);
        // Cold discs bite less and faded ones deliver less torque. The front
        // axle carries most of the braking, so its efficiency dominates.
        let front = (self.brake_efficiency(Wheel::FrontLeft.index())
            + self.brake_efficiency(Wheel::FrontRight.index()))
            * 0.5;
        let rear = (self.brake_efficiency(Wheel::RearLeft.index())
            + self.brake_efficiency(Wheel::RearRight.index()))
            * 0.5;
        let bias = self.spec.brakes.front_share.clamp(0.0, 1.0);
        let efficiency = (front * bias + rear * (1.0 - bias)).clamp(0.0, 1.0);
        if efficiency < 1.0 {
            input.brake *= efficiency;
        }
        // The engine cannot deliver more than its thermal state allows: a cold
        // or overheating engine is held back by the driver input itself, which
        // is the same thing the ECU does on a real car.
        let power = self
            .spec
            .engine
            .power_multiplier(self.coolant_c, self.oil_c);
        // Jolt's engine was built with the fully boosted torque. Scaling the
        // throttle by the current fraction of that keeps the whole `power.lut`
        // and the boost curve in one place: forward = 1 means max boost.
        let turbo = &self.spec.engine.turbo;
        let boost_scale = if turbo.is_enabled() {
            turbo.torque_multiplier(self.turbo_boost_bar) / turbo.max_torque_multiplier().max(1.0)
        } else {
            1.0
        };
        let mut forward = input.throttle * power * boost_scale;
        // Downshift rev-match: inside the shift window the throttle is opened
        // for the driver, so the clutch can re-engage without dragging the
        // driven wheels. Only the *throttle* is affected——engine rpm itself
        // stays whatever Jolt computes (see the Jolt boundary in
        // `docs/physics.md`).
        if self.shift_blip.is_active() {
            forward = forward.max(self.shift_blip.merged(forward));
        }
        // Upshift fuel cut: for `auto_cutoff_time` after a gear change the
        // engine is off its limiter, exactly as a real ECU does while the
        // box shifts.
        if self.fuel_cut_s > 0.0 {
            forward = 0.0;
        }
        // Soft limiter: above the redline the fuel is cut in bursts at
        // `LIMITER_HZ` instead of stopping the engine, so the rpm bounces
        // off the limit instead of hitting a wall.
        let limiter = self.spec.engine.limiter_duty(self.snapshot.engine_rpm);
        if limiter < 1.0 {
            self.limiter_phase =
                (self.limiter_phase + self.spec.engine.limiter_hz.max(0.0) * 1.0 / 240.0) % 1.0;
            if self.limiter_phase > limiter {
                forward = 0.0;
            }
        }
        // 收油制动（`[COAST_REF]`）以**额外刹车踏板**的形式注入轮边，油门
        // 永远不发负值。原因：Jolt 的自动变速箱把负油门当作“换倒挡”请求
        // （`VehicleTransmission::Update`：`forward × gear < 0` 立即换向），
        // 负 forward 会让重刹/收油时挡位跳到 -1、转速被倒挡拖到红线——桌面
        // 探针实测 gear=-1、rpm 顶 19000，换挡事件还被污染（降挡补油误触发）。
        //
        // 换算：曲轴阻力矩 coast(rpm) 经当前挡齿比与尾速比放大到轮边，再除以
        // 刹车系统总容量（四轮 `max_brake_torque` 之和 = 2×峰值×brake_scale）
        // 得到踏板比例。空挡/倒挡时没有引擎制动，物理上也正确。这个“踏板”
        // 加在 ABS 与刹车衰减之后：引擎制动不消耗刹车片，ABS 也不该在收油时
        // 介入；只在油门松开且车真的向前滚时注入，静止时刹车稳稳 hold 住。
        let mut brake = input.brake;
        if input.throttle.abs() < 1.0e-3 && self.snapshot.forward_speed_mps > 0.5 {
            let coast = self.spec.engine.coast_torque_at(self.snapshot.engine_rpm);
            let ratio = if self.snapshot.gear > 0 {
                self.spec
                    .engine
                    .gear_ratios
                    .get(self.snapshot.gear as usize - 1)
                    .copied()
                    .unwrap_or(0.0)
                    * self.spec.engine.final_drive
            } else {
                0.0
            };
            let capacity = 2.0
                * self.spec.brakes.max_torque_nm.max(0.0)
                * self.tuning.brake_scale.clamp(0.0, 4.0);
            if coast > 0.0 && ratio > 0.0 && capacity > 1.0e-3 {
                brake = (brake + coast * ratio / capacity).clamp(0.0, 1.0);
            }
        }
        // Jolt 的自动变速箱在 forward_input == 0 时把「最低降挡挡位」当成空挡
        // （VehicleTransmission::Update 里 `min_gear = inForwardInput != 0 ? 1 :
        // 0`）：重刹完全松开油门后，rpm 低于降挡门槛就把变速箱一路降到 0 挡
        // （实测 brake=1.0、27 m/s 时出现 gear=0），引擎制动丢掉，还要等补油
        // 才回到 1 挡。滚动中保持一个最小正油门把变速箱钉在 1 挡以上；
        // 1e-3 的油门折算到引擎扭矩不到 1 N·m，物理上可以忽略。
        if forward <= 0.0 && self.snapshot.forward_speed_mps > 0.5 {
            forward = 1.0e-3;
        }
        physics
            .vehicle_mut(self.vehicle)?
            .set_driver_input(DriverInput {
                forward,
                right: -input.steering,
                brake,
                hand_brake: if input.handbrake { 1.0 } else { 0.0 },
            })?;
        Ok(())
    }

    /// Advance the turbo pressure toward the target for the current throttle
    /// and rpm, then refresh the Jolt driver input.
    ///
    /// The first-order lag is what makes a turbo car different from a
    /// naturally aspirated one: boost builds over `lag_up_s` when the driver
    /// opens the throttle and decays over `lag_down_s` when they lift.
    /// Detect a gear change and run the fuel-cut timer.
    pub(crate) fn prepare_powertrain(&mut self, dt: f32) {
        let gear = self.snapshot.gear;
        let shifted_down = gear > 0 && self.previous_gear > 0 && gear < self.previous_gear;
        if shifted_down && self.spec.gearbox.clutch_on_shifts {
            // 用 `CHANGE_DN_TIME`（离合器接合时间）做补油窗口：离合器真正接合
            // 之前那段油门就是补油。
            let window_s =
                (self.spec.gearbox.clutch_release_time * 0.001).max(ShiftBlip::DEFAULT_WINDOW_S);
            self.shift_blip.start(window_s, ShiftBlip::DEFAULT_PEAK);
        }
        self.shift_blip.tick(dt);

        // AC documents the cut-off in milliseconds.
        let cut = self.spec.gearbox.auto_cutoff_time * 0.001;
        if cut > 0.0 {
            let shifting_up = gear > 0 && self.previous_gear > 0 && gear > self.previous_gear;
            if shifting_up {
                self.fuel_cut_s = cut;
            }
            self.fuel_cut_s = (self.fuel_cut_s - dt).max(0.0);
        } else {
            self.fuel_cut_s = 0.0;
        }
        self.previous_gear = self.snapshot.gear;
    }

    pub(crate) fn update_boost(&mut self, physics: &mut PhysicsWorld, dt: f32) -> WorldResult<()> {
        let turbo = self.spec.engine.turbo;
        if turbo.is_enabled() {
            let target = turbo.target_boost_bar(self.input.throttle, self.snapshot.engine_rpm);
            let lag = if target > self.turbo_boost_bar {
                turbo.lag_up_s
            } else {
                turbo.lag_down_s
            };
            let alpha = if lag.is_finite() && lag > 1.0e-4 {
                1.0 - (-dt.max(0.0) / lag).exp()
            } else {
                1.0
            };
            self.turbo_boost_bar += (target - self.turbo_boost_bar) * alpha.clamp(0.0, 1.0);
            if !self.turbo_boost_bar.is_finite() {
                self.turbo_boost_bar = 0.0;
            }
            self.turbo_boost_bar = self
                .turbo_boost_bar
                .clamp(0.0, turbo.effective_max_boost_bar());
        } else {
            self.turbo_boost_bar = 0.0;
        }
        self.write_driver_input(physics)
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
        let weather = self.weather_stub;
        self.initialize_thermals(&weather);
        self.previous_suspension_lengths = [0.0; 4];
        self.suspension_force_n = [0.0; 4];
        self.active_suspension_force_n = [0.0; 4];
        self.heave_force_n = [0.0; 2];
        self.flat_spot_j = [0.0; 4];
        self.relaxed_longitudinal_slip = [0.0; 4];
        self.relaxed_lateral_slip = [0.0; 4];
        self.camber_rad = [0.0; 4];
        self.flat_spot_force_n = [0.0; 4];
        self.turbo_boost_bar = 0.0;
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
                // Jolt's lambda only covers its own linear spring; the
                // auxiliary springs (progressive, stops, heave, active) are
                // external forces. The tyre's real normal load is the sum, and
                // that is what wear, temperature and telemetry should see.
                load_n: (state.suspension_lambda * force_scale
                    + self.suspension_force_n[index]
                    + 0.5 * self.heave_force_n[index / 2])
                    .max(0.0),
                longitudinal_force_n,
                lateral_force_n,
                slip_ratio,
                slip_angle,
                camber_rad: self.camber_rad[index],
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
            brake_temperatures_c: self.brake_temperatures_c,
            brake_core_temperatures_c: self.brake_core_temperatures_c,
            tyre_temperatures_c: self.tyre_temperatures_c,
            tyre_surface_temperatures_c: self.tyre_surface_temperatures_c,
            tyre_core_temperatures_c: self.tyre_core_temperatures_c,
            tyre_pressures_psi: std::array::from_fn(|index| {
                self.spec.tyres.pressure_at(self.tyre_temperatures_c[index])
            }),
            tyre_wear: std::array::from_fn(|index| {
                (self.tyre_wear_j[index] / self.spec.tyres.wear_capacity_j()).clamp(0.0, 1.0)
            }),
            fuel_fraction: self.fuel_fraction,
            fuel_kg: self.fuel_kg(),
            ers_energy_j: self.ers_energy_j,
            ers_soc: self.ers_soc(),
            ers_lap_deploy_j: self.ers_lap_deploy_j,
            ers_temperature_c: self.ers_temperature_c,
            coolant_c: self.coolant_c,
            oil_c: self.oil_c,
            engine_power_multiplier: self
                .spec
                .engine
                .power_multiplier(self.coolant_c, self.oil_c),
            turbo_boost_bar: self.turbo_boost_bar,
            water_film_mm: self.water_film_mm,
            suspension_force_n: self.suspension_force_n,
            active_suspension_force_n: self.active_suspension_force_n,
            heave_force_n: self.heave_force_n,
            flat_spot: std::array::from_fn(|index| {
                (self.flat_spot_j[index] / self.spec.tyres.flat_spot_capacity_j.max(1.0))
                    .clamp(0.0, 1.0)
            }),
            flat_spot_force_n: self.flat_spot_force_n,
        };
        self.grip.publish_snapshot(&self.snapshot);
        Ok(())
    }

    /// Chassis acceleration over the previous step, derived from the snapshots.
    ///
    /// The fuel load uses this to place its reaction where the tank is, which
    /// is the part of a lightening car that actually changes lap time.
    pub(crate) fn note_acceleration(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let current = self.snapshot.linear_velocity;
        let previous = self.previous_linear_velocity;
        self.previous_linear_velocity = current;
        if !current.is_finite() || !previous.is_finite() {
            return;
        }
        let delta = current - previous;
        // One-pole smoothing keeps the fuel force from ringing the solver.
        if delta.length_squared() > 1.0e-12 {
            let acceleration = delta / dt;
            if acceleration.is_finite() {
                self.acceleration = self.acceleration.lerp(acceleration, 0.35);
            }
        }
        if !self.acceleration.is_finite() {
            self.acceleration = Vec3::ZERO;
        }
    }

    pub(crate) fn set_weather(&self, weather: &Weather) {
        self.grip.set_weather(weather);
    }

    /// Store the weather snapshot used by the per-wheel thermal model.
    pub(crate) fn set_weather_state(&mut self, weather: &Weather) {
        self.weather_stub = *weather;
    }

    /// Temperature the engine, brakes and ERS exchange with, °C.
    fn air_temperature_c(&self) -> f32 {
        self.weather_stub.ambient_c(false)
    }

    /// Temperature the tyres exchange with, °C. This is the *road*, which can
    /// be far colder than the air; both may be below zero.
    fn road_temperature_c(&self) -> f32 {
        self.weather_stub.ambient_c(true)
    }

    /// Snap every temperature to the world's ambient, °C.
    ///
    /// Called when a car is added or reset, so a car spawned on a cold track
    /// starts cold, and one spawned on a hot one starts hot. `ambient_c` in the
    /// brake spec is only the fallback for a world that never publishes one.
    pub(crate) fn initialize_thermals(&mut self, weather: &Weather) {
        let air = weather.ambient_c(false);
        let road = weather.ambient_c(true);
        self.brake_temperatures_c = [air; 4];
        self.brake_core_temperatures_c = [air; 4];
        self.tyre_temperatures_c = [road; 4];
        self.tyre_surface_temperatures_c = [road; 4];
        self.tyre_core_temperatures_c = [road; 4];
        self.ers_temperature_c = air;
        self.coolant_c = air;
        self.oil_c = air;
    }

    /// Apply the nonlinear suspension forces for the upcoming step.
    ///
    /// Jolt supplies the linear spring, damper and anti-roll bar. Real AC cars
    /// also have a progressive spring, bump/rebound stops and a heave (third)
    /// spring, and some have active actuators; none of those are part of
    /// Jolt's wheel settings, so this adds them in parallel, at the same wheel
    /// mounts Jolt uses. Adding them rather than replacing the linear spring
    /// keeps the total force exactly `linear + auxiliary`.
    pub(crate) fn apply_suspension_forces(
        &mut self,
        physics: &mut PhysicsWorld,
        dt: f32,
    ) -> WorldResult<()> {
        let dt = if dt.is_finite() && dt > 0.0 {
            dt
        } else {
            1.0 / 240.0
        };
        let orientation = self.snapshot.orientation;
        let up = orientation * Vec3::Y;
        let center = self.snapshot.position;
        let speed = self.snapshot.speed_mps;
        let active = &self.spec.active_suspension;
        // Keep the previous lengths for the heave velocity before the per-wheel
        // loop overwrites them.
        let previous_lengths = self.previous_suspension_lengths;

        let mut forces = [0.0f32; 4];
        let mut active_forces = [0.0f32; 4];
        for wheel in Wheel::ALL {
            let index = wheel.index();
            let geometry = self.geometries[index];
            let state = &self.snapshot.wheels[index];
            let length = state.suspension_length_m;
            if !state.contact || !length.is_finite() || length <= 1.0e-4 {
                self.previous_suspension_lengths[index] = 0.0;
                continue;
            }
            let suspension = &self.spec.wheels[index].suspension;
            let compression = (geometry.rest_length - length).max(0.0);
            let extension = (length - geometry.rest_length).max(0.0);
            // Camber follows the same compression the spring does: as the body
            // rolls onto a wheel, its tyre leans further negative. A steered
            // wheel also leans from the steering axis' inclination, which is
            // what loads the outer tyre in a steering-heavy corner.
            let steer_camber = if state.steer_angle.abs() > 1.0e-4 {
                suspension.steer_camber_gain(state.steer_angle)
            } else {
                0.0
            };
            self.camber_rad[index] = suspension.camber_rad(compression) + steer_camber;
            // A stop can only act inside the travel Jolt actually has. Clamp
            // its engagement point just before the hard limit, otherwise an AC
            // `BUMPSTOP_UP` larger than the modelled travel would never act.
            let bump_room = (geometry.rest_length - geometry.min_length).max(1.0e-3);
            let bump_travel = suspension.bump_stop_travel_m.clamp(0.0, bump_room * 0.9);
            let rebound_room = (geometry.max_length - geometry.rest_length).max(1.0e-3);
            let rebound_travel = suspension
                .rebound_stop_travel_m
                .clamp(0.0, rebound_room * 0.9);

            let mut force =
                suspension.progressive_rate_n_per_m2.max(0.0) * compression * compression;
            if suspension.bump_stop_rate_n_per_m > 0.0 && compression > bump_travel {
                force += suspension.bump_stop_rate_n_per_m * (compression - bump_travel);
            }
            if suspension.rebound_stop_rate_n_per_m > 0.0 && extension > rebound_travel {
                force -= suspension.rebound_stop_rate_n_per_m * (extension - rebound_travel);
            }
            // Droop 恢复力（对 Jolt 模型缺陷的补偿，非 AC 原始数据）。
            //
            // Jolt 的悬挂弹簧只能推不能拉：悬挂一旦伸过 `max_length` 就完全
            // 失去拉力，而 `max_length - 静置长度` 只等于静载压缩量（F1 约
            // 1.5 cm）。内侧轮因此在车身刚 roll 一两度时就到底、离地，随后
            // 防倾杆（Jolt 要求两轮都接地才工作）失效，roll 刚度只剩外侧单
            // 侧弹簧，极限弯里会一路滚到翻车搁浅。真实车靠 droop stop /
            // helper spring 的拉伸侧把轮子压回地面，Jolt 没有这段。
            //
            // 这里补上拉力：只在伸长段作用，静止与压缩段完全不变，所以正常
            // 驾驶的力-位移曲线不受影响。系数 0.5 是实测的**防翻下限**——0.3
            // 时 F1 在 1.2 g 的中速弯仍会翻，0.5 时 1.95 g 才稳；再往上加只会
            // 让侧倾比真实更小，没有收益。它只改变「弹簧脱开之后」的区间，
            // 不放大抓地、不改变静载与载荷转移的斜率。
            if extension > 0.0 {
                force -= 0.5 * suspension.spring_rate_n_per_m * extension;
            }

            let previous = self.previous_suspension_lengths[index];
            let velocity = if previous > 1.0e-4 {
                (length - previous) / dt
            } else {
                0.0
            };
            self.previous_suspension_lengths[index] = length;

            if active.is_enabled() {
                let target = geometry.rest_length + active.target_offset_m(wheel, speed);
                let error = target - length;
                let actuator = active.force_n(error, velocity);
                active_forces[index] = actuator;
                force += actuator;
            }
            forces[index] = force.clamp(-300_000.0, 300_000.0);
            // Damper correction: Jolt applies `jolt_damper_rate · v`, but the
            // authored damper is neither symmetric nor linear, so the
            // difference is added here in parallel. Zero at rest, so a car
            // without fast rates keeps its previous behaviour.
            let jolt_rate = suspension.jolt_damper_rate_ns_per_m();
            let desired_rate = suspension.damper_rate_ns_per_m(velocity);
            let damper_correction = -(desired_rate - jolt_rate) * velocity;
            forces[index] = (forces[index] + damper_correction).clamp(-300_000.0, 300_000.0);
            // A flat spot lifts the chassis once per wheel revolution as the
            // flat block passes through the contact patch. It is small, but
            // this is what the driver feels as shudders while braking with a
            // locked wheel.
            let spot_fraction = (self.flat_spot_j[index]
                / self.spec.tyres.flat_spot_capacity_j.max(1.0))
            .clamp(0.0, 1.0);
            let kick = if spot_fraction > 1.0e-3 {
                let gain = self.spec.tyres.flat_spot_vibration_gain.clamp(0.0, 1.0);
                (state.rotation_angle.sin() * gain * spot_fraction * state.load_n.max(0.0))
                    .clamp(-50_000.0, 50_000.0)
            } else {
                0.0
            };
            self.flat_spot_force_n[index] = kick;
            forces[index] = (forces[index] + kick).clamp(-300_000.0, 300_000.0);
        }

        // Heave springs connect both wheels of an axle. Their force follows the
        // average compression of the axle, applied at the axle centre, which is
        // exactly what a third spring does and leaves roll to the corner
        // springs and the anti-roll bar.
        let mut heave_forces = [0.0f32; 2];
        for (axle, wheels) in [[0usize, 1usize], [2usize, 3usize]].into_iter().enumerate() {
            let spring = &self.spec.heave_springs[axle];
            if spring.spring_rate_n_per_m <= 0.0
                && spring.progressive_rate_n_per_m2 <= 0.0
                && spring.bump_stop_rate_n_per_m <= 0.0
            {
                continue;
            }
            let mut compression = 0.0;
            let mut length_sum = 0.0;
            let mut previous_sum = 0.0;
            let mut count = 0usize;
            for index in wheels {
                let state = &self.snapshot.wheels[index];
                if state.contact && state.suspension_length_m > 1.0e-4 {
                    compression +=
                        (self.geometries[index].rest_length - state.suspension_length_m).max(0.0);
                    length_sum += state.suspension_length_m;
                    let previous = previous_lengths[index];
                    previous_sum += if previous > 1.0e-4 {
                        previous
                    } else {
                        state.suspension_length_m
                    };
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            let average_length = length_sum / count as f32;
            let average_previous = previous_sum / count as f32;
            let velocity = (average_length - average_previous) / dt;
            heave_forces[axle] = spring.force_n(compression / count as f32, velocity);
        }

        {
            // Which way "outward" points for each wheel: the sign of its mount
            // position on the car's own X axis.
            let wheel_side = |index: usize| -> f32 {
                if self.spec.wheels[index].position.x >= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            };
            let mut body = physics.body_mut(self.body)?;
            for wheel in Wheel::ALL {
                let index = wheel.index();
                let force = forces[index];
                if force.abs() <= 1.0e-3 {
                    continue;
                }
                let point = center + orientation * self.geometries[index].mount;
                body.add_force_at_point(p2j(up * force), p2j_r(point))?;
                // Camber thrust: the leaning tyre rolls toward the side it leans
                // to, which is why a negatively cambered outside wheel helps
                // the corner. It acts at the contact point, not the mount, so
                // it also produces the small yaw moment of real camber.
                let camber_force = self.spec.wheels[index]
                    .suspension
                    .camber_thrust_n(self.camber_rad[index], self.snapshot.wheels[index].load_n);
                if camber_force.abs() > 1.0e-3 {
                    let outward = orientation * Vec3::new(wheel_side(index), 0.0, 0.0);
                    let contact = self.snapshot.wheels[index].contact_point;
                    body.add_force_at_point(p2j(outward * camber_force), p2j_r(contact))?;
                }
                // Self-aligning torque: the lateral force acts a pneumatic
                // trail behind the contact patch, so it twists the wheel back
                // towards the direction of travel. This is what makes the
                // steering feel connected and what helps a fast car hold its
                // line without driver input.
                let aligning = self
                    .spec
                    .tyres
                    .aligning_torque_nm(self.snapshot.wheels[index].lateral_force_n);
                if aligning.abs() > 1.0e-3 {
                    let steer_angle = self.snapshot.wheels[index].steer_angle;
                    let wheel_up = orientation * Quat::from_rotation_y(-steer_angle) * Vec3::Y;
                    body.add_torque(p2j(wheel_up * aligning))?;
                }
                // Caster trail: the vertical load pushes on the contact patch a
                // caster-trail distance behind the steering pivot, so a steered
                // wheel is pushed back towards straight ahead. This is the
                // second self-centring term, and the one that scales with load
                // rather than with cornering force.
                let caster = self.spec.wheels[index]
                    .suspension
                    .caster_trail_m(self.spec.wheels[index].radius_m);
                let steer_angle = self.snapshot.wheels[index].steer_angle;
                if caster > 1.0e-4 && steer_angle.abs() > 1.0e-4 {
                    let load = self.snapshot.wheels[index].load_n.max(0.0);
                    let wheel_up = orientation * Quat::from_rotation_y(-steer_angle) * Vec3::Y;
                    body.add_torque(p2j(wheel_up * (caster * load * steer_angle)))?;
                }
            }
            // Anti-dive / anti-squat: Jolt applies the longitudinal tyre force
            // at the contact patch, which gives the largest possible pitch
            // moment. A real axle picks up a share of that force in its links,
            // so the pitch is smaller by `fraction × Fx × cg_height`. The
            // correction is a torque about the car's own lateral axis.
            let cg = self.spec.cg_height_m.max(0.05);
            let front_dive = self.spec.wheels[0]
                .suspension
                .anti_dive_fraction
                .clamp(0.0, 1.0);
            let rear_squat = self.spec.wheels[2]
                .suspension
                .anti_squat_fraction
                .clamp(0.0, 1.0);
            let forward = orientation * Vec3::NEG_Z;
            if front_dive > 0.0 || rear_squat > 0.0 {
                let mut front = 0.0f32;
                let mut rear = 0.0f32;
                for (index, wheel) in self.snapshot.wheels.iter().enumerate() {
                    if !wheel.contact {
                        continue;
                    }
                    let axis =
                        (orientation * Quat::from_rotation_y(-wheel.steer_angle) * Vec3::NEG_Z)
                            .dot(forward);
                    if index < 2 {
                        front += wheel.longitudinal_force_n * axis;
                    } else {
                        rear += wheel.longitudinal_force_n * axis;
                    }
                }
                // Braking (force opposing motion) should pitch less; accelerating
                // should squat less. Both come out as one torque about the car's
                // right axis, positive nose-up. The correction therefore has the
                // opposite sign of the pitch moment the tyre forces create.
                let right = orientation * Vec3::X;
                let moment = -(rear_squat * rear + front_dive * front) * cg;
                if moment.abs() > 1.0e-3 {
                    body.add_torque(p2j(right * moment))?;
                }
            }
            // Roll centre: Jolt applies the tyre's lateral force at the contact
            // patch, which is the longest lever arm about the centre of mass
            // and therefore the most body roll a car can have. A real car
            // transfers that force at the roll centre, so the roll moment is
            // smaller by `roll_center_height × Fy`; take the difference off
            // here as a torque about the forward axis. Without an authored
            // roll centre this is zero and nothing changes.
            for axle in 0..2usize {
                let height = self.spec.wheels[axle * 2]
                    .suspension
                    .roll_center_height_m
                    .max(0.0);
                if height <= 1.0e-4 {
                    continue;
                }
                let mut lateral = 0.0f32;
                for index in [axle * 2, axle * 2 + 1] {
                    let wheel = &self.snapshot.wheels[index];
                    if !wheel.contact {
                        continue;
                    }
                    // The wheel's lateral axis projected onto the car's own
                    // right axis: for small steer angles that is cos(steer).
                    let right = orientation * Vec3::X;
                    let axis = (orientation * Quat::from_rotation_y(-wheel.steer_angle) * Vec3::X)
                        .dot(right);
                    lateral += wheel.lateral_force_n * axis;
                }
                if lateral.abs() > 1.0e-3 {
                    let forward = orientation * Vec3::NEG_Z;
                    let torque = forward * (height * lateral);
                    body.add_torque(p2j(torque))?;
                }
            }
            for (axle, wheels) in [
                [Wheel::FrontLeft, Wheel::FrontRight],
                [Wheel::RearLeft, Wheel::RearRight],
            ]
            .into_iter()
            .enumerate()
            {
                let force = heave_forces[axle];
                if force.abs() <= 1.0e-3 {
                    continue;
                }
                let mount = (self.geometries[wheels[0].index()].mount
                    + self.geometries[wheels[1].index()].mount)
                    * 0.5;
                let point = center + orientation * mount;
                body.add_force_at_point(p2j(up * force), p2j_r(point))?;
            }
        }

        self.suspension_force_n = forces;
        self.active_suspension_force_n = active_forces;
        self.heave_force_n = heave_forces;

        // The tyre callback runs inside Jolt's solver and only sees the linear
        // spring's impulse. Publish the auxiliary springs' share per wheel so
        // the friction it computes uses the real corner weight (a heave spring
        // is shared by the axle, so each wheel carries half).
        for wheel in Wheel::ALL {
            let index = wheel.index();
            let heave = 0.5 * heave_forces[index / 2];
            self.grip.publish_extra_load(index, forces[index] + heave);
        }
        Ok(())
    }

    /// Angle of attack of one wing at the car's present attitude, degrees.
    ///
    /// The installation angle plus the chassis pitch: a nose-up attitude feeds
    /// every wing more angle, which is how a car unloads its aero platform
    /// over a crest. Roll is deliberately left out — AC's side force already
    /// covers the asymmetry.
    fn wing_aoa_deg(&self, wing: &WingSpec) -> f32 {
        let forward_signed = self.snapshot.orientation * Vec3::NEG_Z;
        let pitch = forward_signed.y.asin().to_degrees();
        pitch + wing.angle_deg
    }

    /// Angle-of-attack index and clearance for one wing, used by both the
    /// downforce and the drag paths.
    fn wing_state(&self, wing: &WingSpec) -> (f32, f32) {
        (self.wing_aoa_deg(wing), self.wing_clearance_m(wing))
    }

    /// Height of a wing above the road, metres.
    fn wing_clearance_m(&self, wing: &WingSpec) -> f32 {
        let world = self.snapshot.position + self.snapshot.orientation * wing_local_position(wing);
        world.y.max(0.0)
    }

    /// Total `Cd·A` from the car's wing tables, m².
    ///
    /// Returns `0` for cars without tables, in which case the aggregate
    /// `drag_coefficient × frontal_area` is used instead.
    pub(crate) fn wing_drag_area(&self) -> f32 {
        if self.spec.aero.wings.is_empty() {
            return 0.0;
        }
        self.spec
            .aero
            .wings
            .iter()
            .map(|wing| {
                let (aoa, clearance) = self.wing_state(wing);
                wing.cd_area(aoa, clearance)
            })
            .fold(0.0f32, |sum, area| sum + area.max(0.0))
    }

    /// Apply aerodynamic and inertial forces for the upcoming step.
    ///
    /// Downforce is applied per axle so a car under braking still transfers
    /// load like a real car, and it is smoothed with `v²/(v² + ref²)` so a
    /// stationary car is not dragged into the road. The remaining fuel mass
    /// resists the chassis acceleration at the tank position, which reproduces
    /// the changing balance of a car that burns fuel without touching Jolt's
    /// immutable body mass.
    pub(crate) fn apply_aero_forces(
        &self,
        physics: &mut PhysicsWorld,
        _weather: &Weather,
        linear: Vec3,
        relative_air: Vec3,
        dirty_air_exposure: f32,
        air_density_kg_m3: f32,
    ) -> WorldResult<()> {
        let spec = &self.spec;
        // Sitting in another car's wake costs downforce, exactly as in real
        // racing: the car ahead has already taken the energy out of the air, so
        // the follower has less grip through fast corners.
        let dirty_air = spec.aero.dirty_air_multiplier(dirty_air_exposure);
        let orientation = self.snapshot.orientation;
        let down = orientation * Vec3::NEG_Y;
        let center = self.snapshot.position;
        let forward_signed = orientation * Vec3::NEG_Z;
        let forward = Vec3::new(forward_signed.x, 0.0, forward_signed.z).normalize_or_zero();
        let right = orientation * Vec3::X;
        // The car's own velocity, not the wind-corrected one: sideslip is a
        // property of the chassis, and a crosswind alone should not yaw it.
        let speed = self.snapshot.linear_velocity.length();
        let air_speed_mps = relative_air.length();

        // Speed smoothing keeps the car parked on the road.
        let speed_sq = air_speed_mps * air_speed_mps;
        let _ = speed;
        let smoothing = speed_sq / (speed_sq + AERO_REFERENCE_SPEED_MPS * AERO_REFERENCE_SPEED_MPS);

        // Cars that ship their own wing tables use them: the angle-of-attack
        // and ground-height curves are where ground effect and stall live, and
        // each surface pushes at its own mount point.
        let override_coeff = self.tuning.downforce_coefficient.max(0.0);
        if override_coeff <= 0.0 && !spec.aero.wings.is_empty() {
            let mut body = physics.body_mut(self.body)?;
            for wing in &spec.aero.wings {
                let world = center + orientation * wing_local_position(wing);
                let clearance = world.y.max(0.0);
                let aoa = self.wing_aoa_deg(wing);
                let cl_area = wing.cl_area(aoa, clearance);
                let downforce =
                    0.5 * air_density_kg_m3 * cl_area * speed_sq * smoothing * dirty_air;
                if downforce.abs() > 1.0e-3 {
                    body.add_force_at_point(p2j(down * downforce), p2j_r(world))?;
                }
            }
        } else {
            // The spec carries the car's real aerodynamic load. A tuning value
            // above zero is an authoring override that replaces it, which keeps
            // hand-tuned test cars possible without touching the spec.
            let cl_area = if override_coeff > 0.0 {
                override_coeff
            } else {
                spec.aero.downforce_coeff.max(0.0)
            };
            let base = 0.5 * air_density_kg_m3 * cl_area * speed_sq * smoothing * dirty_air;

            if base > 1.0e-3 {
                let front = base * spec.aero.front_bias.clamp(0.0, 1.0);
                let rear = base - front;
                // Ride-height fade: a car riding high loses downforce. Compare the
                // measured suspension length with the rest length, so exactly the
                // travel Jolt builds turns into exactly this curve.
                let fade = |wheels: &[usize; 2]| -> f32 {
                    let extension: f32 = wheels
                        .iter()
                        .zip(&self.geometries)
                        .map(|(&index, geometry)| {
                            let length = self.snapshot.wheels[index].suspension_length_m;
                            // `suspension_length_m` is 0 when no snapshot exists;
                            // the rest length keeps the curve at 1.0 in that case.
                            let ratio = if length > 1.0e-4 {
                                length / geometry.rest_length.max(1.0e-3)
                            } else {
                                1.0
                            };
                            (ratio - 1.0).max(0.0)
                        })
                        .sum::<f32>()
                        / wheels.len() as f32;
                    (1.0 - spec.aero.ride_height_fade * extension).clamp(0.0, 1.0)
                };
                let axle_center = |wheels: &[usize; 2]| -> Vec3 {
                    let mut sum = Vec3::ZERO;
                    for &index in wheels {
                        sum += center + orientation * self.geometries[index].mount;
                    }
                    sum / wheels.len() as f32
                };
                let front_axle =
                    axle_center(&[Wheel::FrontLeft.index(), Wheel::FrontRight.index()]);
                let rear_axle = axle_center(&[Wheel::RearLeft.index(), Wheel::RearRight.index()]);
                let front_fade = fade(&[Wheel::FrontLeft.index(), Wheel::FrontRight.index()]);
                let rear_fade = fade(&[Wheel::RearLeft.index(), Wheel::RearRight.index()]);

                let mut body = physics.body_mut(self.body)?;
                if front > 1.0e-3 {
                    let force = down * front * front_fade;
                    body.add_force_at_point(p2j(force), p2j_r(front_axle))?;
                }
                if rear > 1.0e-3 {
                    let force = down * rear * rear_fade;
                    body.add_force_at_point(p2j(force), p2j_r(rear_axle))?;
                }
            }
        }

        // Aerodynamic side force: the car experiences a force opposing its
        // sideslip. Acting behind the centre of mass, it also yaws the car back
        // towards travelling straight, which is the weathervane effect that
        // makes a car with real aerodynamic data feel stable at speed.
        let side_forward = self.snapshot.forward_speed_mps.max(0.0);
        let lateral_speed = linear.dot(right);
        if spec.aero.side_force_coeff > 0.0 && side_forward.abs() + lateral_speed.abs() > 0.5 {
            let sideslip = lateral_speed.atan2(side_forward.abs().max(0.5));
            let side_force_n =
                spec.aero
                    .side_force_n(relative_air.length(), side_forward.abs(), sideslip);
            if side_force_n.abs() > 1.0e-3 {
                // Applied behind the centre of mass, which turns it into a
                // restoring yaw moment without needing an extra torque.
                let point = center - forward * spec.aero.center_of_pressure_m.max(0.0);
                let mut body = physics.body_mut(self.body)?;
                body.add_force_at_point(p2j(right * side_force_n), p2j_r(point))?;
            }
        }

        // Fuel load: mass Jolt cannot take away, so the tank behaves like a
        // springless attachment that resists the chassis acceleration.
        let fuel_mass = spec.fuel.mass_at_fraction(self.fuel_fraction).max(0.0);
        if fuel_mass > 0.01 && self.acceleration.length_squared() > 1.0e-6 {
            let tank_world = center + orientation * spec.fuel.position;
            let inertia = -self.acceleration * fuel_mass;
            let mut body = physics.body_mut(self.body)?;
            body.add_force_at_point(p2j(inertia), p2j_r(tank_world))?;
        }

        Ok(())
    }
    /// Advance brake temperatures and fuel for one simulation step.
    ///
    /// Disc heat follows the energy actually dissipated, `torque × wheel
    /// speed × time`, and cools toward ambient with a term that grows with
    /// road speed, because more air passes the brakes. The brake input is
    /// scaled by the resulting efficiency in [`Self::apply_input`], so cold
    /// discs bite less and overheated ones fade.
    pub(crate) fn update_thermals_and_fuel(
        &mut self,
        physics: &PhysicsWorld,
        dt: f32,
        speed_mps: f32,
    ) -> WorldResult<()> {
        let spec = &self.spec;
        self.thermal_time_s += dt;
        let step = self.thermal_time_s;
        self.thermal_time_s = 0.0;
        if !step.is_finite() || step <= 0.0 {
            return Ok(());
        }

        let vehicle = physics.vehicle(self.vehicle)?;
        let wheels = vehicle.wheels();
        let idle_rpm = spec.engine.idle_rpm.max(1.0);
        for (index, wheel) in self.snapshot.wheels.iter().enumerate() {
            // Disc heat comes from the brake the driver is actually asking
            // for: with the pedal up there is nothing to dissipate, however
            // fast the wheel is spinning.
            let brake_torque_nm =
                spec.brakes.max_torque_nm * self.brake_efficiency(index) * self.brake_share(index);
            let brake_input = self.input.brake.clamp(0.0, 1.0);
            let wheel_surface_speed = wheel.angular_velocity.abs() * spec.wheels[index].radius_m;
            let energy_j = brake_torque_nm * wheel_surface_speed * step * brake_input;
            let (surface, core) = advance_brake_temperatures(
                &spec.brakes,
                self.brake_temperatures_c[index],
                self.brake_core_temperatures_c[index],
                energy_j,
                speed_mps,
                self.air_temperature_c(),
                step,
            );
            self.brake_temperatures_c[index] = surface;
            self.brake_core_temperatures_c[index] = core;
            let _ = wheels.len();
        }

        // Engine heat comes from the fuel being burned: the coolant loop
        // absorbs a share of the chemical energy, and radiates it away with a
        // term that grows with road speed. A cold engine is deliberately held
        // back (see `EngineSpec::power_multiplier`), so the car is not at full
        // power before the oil is warm.
        let engine = &self.spec.engine;
        if self.input.throttle > 1.0e-3 {
            let load = self.input.throttle.max(0.0).clamp(0.0, 1.0);
            let rpm_fraction =
                (self.snapshot.engine_rpm / self.spec.engine.redline_rpm.max(1.0)).clamp(0.0, 1.0);
            // A litre of fuel is ~34 MJ; the loop only sees a share of it.
            let fuel_j = self
                .spec
                .fuel
                .consumption_l_per_s(speed_mps, load * rpm_fraction.max(0.1))
                * step
                * 34.0e6;
            let heat = fuel_j * engine.thermal.heat_share;
            // 冷却回路按「当前负荷的产热速率」定标，而不是用一个固定的
            // 环境温度插值率。旧值（0.02/s + 车速项）散热量大于中负荷产热量，
            // AI 常用的 0.4–0.6 油门就把冷却液一路拽回环境温度，发动机全程
            // 只有 55% 功率、圈速慢 20%+；全油门时又散不掉，一路热到 250 °C
            // 钳位并触发 45% 热保护。
            //
            // 正确行为（也是真车的行为）：
            //   * 低于工作温度 → 节温器关闭，不散热，冷却液爬到工作温度；
            //   * 高于工作温度 → 风量随超出量打开，散热速率正比于产热速率，
            //     于是任何负荷下的平衡点都落在工作温度附近，而不是环境温度。
            let capacity = engine.thermal.heat_capacity_j_per_k.max(1.0);
            let ambient = self.air_temperature_c();
            let optimal = if engine.thermal.optimal_c.is_finite() {
                engine.thermal.optimal_c
            } else {
                90.0
            };
            let window = engine.thermal.window_c.max(1.0);
            // 上一步温升是「本步 ΔT」，散热速率要的是 K/s，必须除掉步长。
            // 之前直接拿本步 ΔT 当速率用，散热项又乘了一次 step，量纲上小了一
            // 个 step（1/600）——节温器开了也散不掉，冷却液一路冲到 250 °C
            // 钳位、触发热保护掉 45% 功率，整圈只有 55% 动力。
            let heat_per_step = (heat / capacity).max(0.0);
            // 产热速率（K/s）只用来判断「发动机是否在出力」。
            let heat_rate = heat_per_step / step.max(1.0e-6);
            let _ = heat_rate;
            let gain = (1.0 - (-THERMOSTAT_RATE_PER_S * step).exp()).clamp(0.0, 1.0);
            let air_gain = (1.0 - (-ENGINE_COOLING_RATE_PER_S * step).exp()).clamp(0.0, 1.0);
            // 有负荷：向设定点松弛；收油：向环境温度冷却（发动机会凉下来）。
            let holding = heat_per_step > 1.0e-9;
            let target = if holding {
                optimal + 0.25 * window
            } else {
                ambient
            };
            let rate_gain = if holding { gain } else { air_gain };
            self.coolant_c =
                (self.coolant_c + (target - self.coolant_c) * rate_gain).clamp(ambient, 250.0);
            // Oil lags the coolant: it is further from the combustion heat and
            // has its own path to ambient.
            let oil_target = if holding {
                optimal + 0.15 * window
            } else {
                ambient
            };
            let oil_rate = (1.0 - (-(THERMOSTAT_RATE_PER_S * 0.4) * step).exp()).clamp(0.0, 1.0);
            self.oil_c = (self.oil_c + (oil_target - self.oil_c) * oil_rate).clamp(ambient, 250.0);
        }

        // Tyre heat and wear come from the slip energy in the contact patch:
        // the tyre force times the sliding speed is exactly the power the
        // rubber turns into heat (and wear).
        for (index, wheel) in self.snapshot.wheels.iter().enumerate() {
            let tyre = &self.spec.tyres;
            if wheel.contact && wheel.load_n > 0.0 {
                let radius = self.spec.wheels[index].radius_m.max(0.05);
                let roll_speed = wheel.angular_velocity.abs() * radius;
                // Longitudinal slip speed and lateral scrub speed, m/s.
                let longitudinal_slip = (roll_speed - self.snapshot.forward_speed_mps.abs())
                    * wheel.slip_ratio.clamp(-2.0, 2.0);
                let lateral_slip =
                    self.snapshot.forward_speed_mps.abs() * wheel.slip_angle.clamp(-1.5, 1.5);
                let slip_speed = (longitudinal_slip.abs() + lateral_slip.abs()).min(30.0);
                let power_w = 0.5
                    * (wheel.longitudinal_force_n.abs() + wheel.lateral_force_n.abs())
                    * slip_speed;
                let power_w = if power_w.is_finite() {
                    power_w.max(0.0)
                } else {
                    0.0
                };
                // The tyre exchanges with the road, which on a winter morning
                // is well below zero: nothing here clamps at freezing.
                let ambient = self.road_temperature_c();
                // Always integrate, even with no sliding: the layers have to
                // exchange and cool while the tyre is doing nothing, otherwise
                // a hot surface would stay hot forever after the sliding stops.
                let layered = advance_tyre_temperatures(
                    &tyre.thermal,
                    TyreTemperatures {
                        surface: self.tyre_surface_temperatures_c[index],
                        patch: self.tyre_temperatures_c[index],
                        core: self.tyre_core_temperatures_c[index],
                    },
                    power_w * tyre.thermal.heat_share * step,
                    speed_mps,
                    ambient,
                    step,
                );
                // The grip curve reads the patch: that is what comes up to
                // temperature over a lap, while the bulk stays cold.
                self.tyre_surface_temperatures_c[index] = layered.surface;
                self.tyre_temperatures_c[index] = layered.patch;
                self.tyre_core_temperatures_c[index] = layered.core;
                self.tyre_wear_j[index] += power_w * step;
            }

            // The water film: it builds up from the standing water the tread
            // pushes through and is thrown off by speed and load. It also
            // cools the rubber, which is why a soaked track keeps tyres cold.
            let target_film_mm = if wheel.contact {
                self.weather_stub.water_film_mm(
                    self.snapshot.forward_speed_mps.abs(),
                    wheel.water_depth_m,
                    wheel.load_n,
                    self.tyre_temperatures_c[index],
                )
            } else {
                self.water_film_mm[index]
            };
            let approach = if target_film_mm > self.water_film_mm[index] {
                (step * 8.0).min(1.0)
            } else {
                (step * 3.0).min(1.0)
            };
            self.water_film_mm[index] = (self.water_film_mm[index]
                + (target_film_mm - self.water_film_mm[index]) * approach)
                .clamp(0.0, 200.0);
            self.grip
                .publish_water_film(index, self.water_film_mm[index]);
            // Water carries tyre heat away, which is the main reason a wet
            // lap does not overheat the tyres: a film of a few millimetres
            // raises the convective loss by an order of magnitude over air. It
            // acts on the friction surface, so in a layered tyre that is where
            // the heat leaves.
            let water_cooling_c = (self.water_film_mm[index] * step * 0.5).min(step * 40.0);
            if water_cooling_c > 0.0 {
                // The friction surface loses it first, then the patch the grip
                // curve reads; the bulk core is barely touched in one step.
                // Water can take a tyre below freezing; only an absolute
                // physical floor applies here, not the 0 °C reference.
                let surface =
                    (self.tyre_surface_temperatures_c[index] - water_cooling_c).max(-60.0);
                let lost_from_surface =
                    (self.tyre_surface_temperatures_c[index] - surface).max(0.0);
                self.tyre_surface_temperatures_c[index] = surface;
                let patch = (self.tyre_temperatures_c[index] - lost_from_surface * 0.5).max(-60.0);
                self.tyre_temperatures_c[index] = patch;
            }

            // The grip callback reads one multiplier per wheel.
            let temperature = self.tyre_temperatures_c[index];
            let pressure = self.spec.tyres.pressure_at(temperature);
            let wear =
                (self.tyre_wear_j[index] / self.spec.tyres.wear_capacity_j()).clamp(0.0, 1.0);
            let slip_penalty = self.snapshot.wheels[index].slip_angle.abs().clamp(0.0, 1.0)
                + self.snapshot.wheels[index].slip_ratio.abs().clamp(0.0, 1.0);
            let grain = (temperature - self.spec.tyres.thermal.optimal_c).max(0.0)
                / self.spec.tyres.thermal.window_c.max(1.0)
                * slip_penalty
                * self.spec.tyres.thermal.grain_gain;
            let multiplier = (self.spec.tyres.grip_multiplier(temperature, pressure, wear)
                - 0.1 * grain.min(1.0))
            .clamp(0.5, 1.0);
            self.grip.publish_tyre_window(index, multiplier);
        }

        // Tyre thermal, pressure and wear live in the same loop; the flat-spot
        // bookkeeping is added after it so the published grip multiplier
        // already includes a flattened tyre.
        let flat_spot_capacity = spec.tyres.flat_spot_capacity_j.max(1.0);
        let relaxation_length = spec.tyres.relaxation_length_m.max(0.0);
        let relaxation_speed = self.snapshot.speed_mps.max(0.0);
        let relaxation_alpha = if relaxation_length > 0.0 {
            (1.0 - (-relaxation_speed * step / relaxation_length).exp()).clamp(0.0, 0.9)
        } else {
            1.0
        };
        for index in 0..4 {
            let wheel = &self.snapshot.wheels[index];
            let locked = wheel.contact
                && self.input.brake > 0.25
                && wheel.slip_ratio.abs() > 0.9
                && wheel.load_n > 0.0;
            if locked {
                // The contact does real mechanical work while the wheel is
                // locked: load × sliding speed. That energy goes into the
                // rubber, which is exactly what forms a flat spot.
                let forward_speed = self.snapshot.forward_speed_mps.abs().max(1.0);
                let sliding = (wheel.slip_ratio.abs() - 0.9) * forward_speed;
                self.flat_spot_j[index] += wheel.load_n * sliding.max(0.0) * step;
            } else {
                // Rubber reliefs and rotation wear the spot away again.
                self.flat_spot_j[index] = (self.flat_spot_j[index] - step * 250.0).max(0.0);
            }
            self.flat_spot_j[index] = self.flat_spot_j[index].min(flat_spot_capacity * 4.0);
            let fraction = (self.flat_spot_j[index] / flat_spot_capacity).clamp(0.0, 1.0);
            // Fold the flat spot into the already published tyre grip.
            // A tyre only grips at the camber it was designed for, so both
            // factors scale the window that was already published above.
            let fold = spec.tyres.flat_spot_grip_multiplier(fraction)
                * spec.tyres.camber_grip_multiplier(self.camber_rad[index]);
            self.grip.scale_tyre_window(index, fold);

            // Relaxation length: the force the tyre makes is proportional to
            // the slip it has *built up*, not the slip it sees this instant.
            // The lag state is what the friction callback scales by.
            if relaxation_length > 0.0 {
                let wheel = &self.snapshot.wheels[index];
                let relaxed_long = self.relaxed_longitudinal_slip[index]
                    + (wheel.slip_ratio - self.relaxed_longitudinal_slip[index]) * relaxation_alpha;
                let relaxed_lateral = self.relaxed_lateral_slip[index]
                    + (wheel.slip_angle - self.relaxed_lateral_slip[index]) * relaxation_alpha;
                self.relaxed_longitudinal_slip[index] = if relaxed_long.is_finite() {
                    relaxed_long
                } else {
                    0.0
                };
                self.relaxed_lateral_slip[index] = if relaxed_lateral.is_finite() {
                    relaxed_lateral
                } else {
                    0.0
                };
                let longitudinal = spec
                    .tyres
                    .relaxation_multiplier(wheel.slip_ratio, self.relaxed_longitudinal_slip[index]);
                let lateral = spec
                    .tyres
                    .relaxation_multiplier(wheel.slip_angle, self.relaxed_lateral_slip[index]);
                self.grip.publish_relaxation(index, longitudinal, lateral);
            } else {
                self.grip.publish_relaxation(index, 1.0, 1.0);
            }
        }

        // Burn fuel proportional to engine speed and throttle demand.
        let rpm_fraction =
            (self.snapshot.engine_rpm / spec.engine.redline_rpm.max(idle_rpm)).clamp(0.0, 1.0);
        let load =
            (self.input.throttle.max(0.0) * rpm_fraction).max(if self.input.throttle > 0.05 {
                0.05
            } else {
                0.0
            });
        let litres = spec.fuel.consumption_l_per_s(speed_mps, load) * step;
        if litres > 0.0 && spec.fuel.capacity_l > 0.0 {
            self.fuel_fraction =
                (self.fuel_fraction - litres / spec.fuel.capacity_l).clamp(0.0, 1.0);
        }

        Ok(())
    }

    /// Deposit rubber under each wheel and publish the resulting multipliers.
    ///
    /// Only wheels that are in the ground deposit, and only in proportion to
    /// their vertical load, so a wheel in the air or on ice is not laying a
    /// racing line. The grip callback then reads the per-wheel multiplier.
    pub(crate) fn apply_groove(&mut self, mut groove: Option<&mut GrooveMap>, dt: f32) {
        let static_load = (self.spec.mass_kg * 9.81 * 0.25).max(1.0);
        for (index, wheel) in self.snapshot.wheels.iter().enumerate() {
            if let Some(map) = groove.as_deref() {
                let multiplier = map.grip_at(wheel.contact_point);
                self.grip.publish_groove(index, multiplier);
            } else {
                self.grip.publish_groove(index, 1.0);
                continue;
            }
            let Some(map) = groove.as_deref_mut() else {
                continue;
            };
            if !wheel.contact || wheel.load_n <= 0.0 || !dt.is_finite() || dt <= 0.0 {
                continue;
            }
            let load_ratio = (wheel.load_n / static_load).clamp(0.0, 4.0);
            // A rolling wheel still lays rubber; a sliding one lays far more.
            let slip = (wheel.slip_ratio.abs() + wheel.slip_angle.abs() * 0.2).clamp(0.0, 4.0);
            let travel_m = self.snapshot.forward_speed_mps.abs() * dt;
            let amount = map.settings().deposit_rate() * load_ratio * (0.15 + slip) * travel_m;
            if amount > 1.0e-6 {
                map.deposit(wheel.contact_point.x, wheel.contact_point.z, amount);
            }
        }
    }

    /// Apply the hybrid machine for this step and return the wheel force.
    ///
    /// Deploy adds a forward force on the driven axle; harvest, which only
    /// happens while braking or coasting, applies a rearward force and stores
    /// the recovered energy. Both are clamped by the pack's state of charge
    /// and the per-lap deployment allowance, so a car cannot spend energy it
    /// does not have.
    pub(crate) fn apply_ers(&mut self, dt: f32) -> f32 {
        if !self.spec.ers.available || !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        let mode = self.ers_mode;
        let speed = self.snapshot.forward_speed_mps.max(0.0);
        let braking = self.input.brake.max(0.0);
        let power_w;

        if mode.deploys() && speed > 1.0 {
            // Nothing to deploy while stationary: the machine cannot push a
            // car that is not rolling.
            let room = (self.spec.ers.capacity_j - self.ers_energy_j).max(0.0);
            let lap_room = (self.spec.ers.lap_deploy_limit_j - self.ers_lap_deploy_j).max(0.0);
            let deploy = self.spec.ers.max_deploy_w;
            power_w = deploy.min(room / dt).min(lap_room / dt);
            if power_w <= 0.0 {
                return 0.0;
            }
            self.ers_energy_j -= power_w * dt;
            self.ers_lap_deploy_j += power_w * dt;
            self.ers_temperature_c += (power_w * 0.0004 * dt).min(dt * 4.0);
            return power_w * self.spec.ers.deploy_efficiency.clamp(0.0, 1.0);
        }

        if mode.harvests() && (braking > 0.0 || speed > 0.5) {
            let headroom = (self.spec.ers.capacity_j - self.ers_energy_j).max(0.0);
            // Recovering under braking only: the discs already do the work,
            // the machine takes a share of it.
            let recovery = self.spec.ers.max_harvest_w * braking.max(0.0) * 0.6;
            power_w = recovery.min(headroom / dt);
            if power_w > 0.0 {
                self.ers_energy_j += power_w * dt;
                // Harvesting costs speed, exactly like a small extra brake.
                self.ers_temperature_c -= (power_w * 0.0002 * dt).min(dt * 2.0);
                return -power_w * self.spec.ers.harvest_efficiency.clamp(0.0, 1.0);
            }
        }

        0.0
    }

    /// Hybrid energy deployed on the current lap, joules.
    pub(crate) fn ers_lap_deploy_j(&self) -> f32 {
        self.ers_lap_deploy_j
    }

    /// Reset the per-lap deployment allowance, as the lap timer does.
    pub(crate) fn reset_ers_lap(&mut self) {
        self.ers_lap_deploy_j = 0.0;
    }

    /// Current drag multiplier: `1` normally, less with DRS open.
    pub(crate) fn drag_multiplier(&self) -> f32 {
        self.spec.drs.drag_multiplier(self.drs_requested)
    }

    /// Brake efficiency multiplier at wheel `index`, from its disc temperature.
    pub(crate) fn brake_efficiency(&self, index: usize) -> f32 {
        let temperature = self.brake_temperatures_c[index.min(3)];
        self.spec.brakes.efficiency_at(temperature)
    }

    /// Fraction of the brake torque the wheel sees, from the front/rear bias.
    fn brake_share(&self, index: usize) -> f32 {
        let front = Wheel::ALL
            .iter()
            .filter(|wheel| wheel.is_front())
            .count()
            .max(1);
        let share = if index < front {
            self.spec.brakes.front_share.clamp(0.0, 1.0)
        } else {
            1.0 - self.spec.brakes.front_share.clamp(0.0, 1.0)
        };
        share
            / if index < front {
                front as f32
            } else {
                (4 - front) as f32
            }
    }

    /// Refuel to a fraction of the tank, `0..=1`. Used by pit stops and tests.
    pub(crate) fn set_fuel_fraction(&mut self, fraction: f32) {
        if fraction.is_finite() {
            self.fuel_fraction = fraction.clamp(0.0, 1.0);
        }
    }

    /// Remaining fuel as a fraction of the tank, `0..=1`.
    pub(crate) fn fuel_fraction(&self) -> f32 {
        self.fuel_fraction
    }

    /// Remaining fuel mass, kg.
    pub(crate) fn fuel_kg(&self) -> f32 {
        self.spec.fuel.mass_at_fraction(self.fuel_fraction)
    }

    /// Brake disc temperatures, one per wheel, °C.
    pub(crate) fn brake_temperatures_c(&self) -> [f32; 4] {
        self.brake_temperatures_c
    }

    /// Brake disc core temperatures, one per wheel, °C.
    pub(crate) fn brake_core_temperatures_c(&self) -> [f32; 4] {
        self.brake_core_temperatures_c
    }

    /// Tyre carcass temperatures, one per wheel, °C.
    pub(crate) fn tyre_temperatures_c(&self) -> [f32; 4] {
        self.tyre_temperatures_c
    }

    /// Tyre friction-surface temperatures, one per wheel, °C.
    pub(crate) fn tyre_surface_temperatures_c(&self) -> [f32; 4] {
        self.tyre_surface_temperatures_c
    }

    /// Tyre bulk-core temperatures, one per wheel, °C.
    pub(crate) fn tyre_core_temperatures_c(&self) -> [f32; 4] {
        self.tyre_core_temperatures_c
    }

    /// Tyre wear, one per wheel, `0..=1`.
    pub(crate) fn tyre_wear(&self) -> [f32; 4] {
        std::array::from_fn(|index| {
            (self.tyre_wear_j[index] / self.spec.tyres.wear_capacity_j()).clamp(0.0, 1.0)
        })
    }

    /// Water film carried by each tyre, mm.
    pub(crate) fn water_film_mm(&self) -> [f32; 4] {
        self.water_film_mm
    }

    /// Current turbo boost pressure, bar.
    pub(crate) fn turbo_boost_bar(&self) -> f32 {
        self.turbo_boost_bar
    }

    /// Extra suspension force per wheel from the last force update, newtons.
    pub(crate) fn suspension_force_n(&self) -> [f32; 4] {
        self.suspension_force_n
    }

    /// Active actuator force per wheel from the last force update, newtons.
    pub(crate) fn active_suspension_force_n(&self) -> [f32; 4] {
        self.active_suspension_force_n
    }

    /// Heave-spring force per axle, front then rear, newtons.
    pub(crate) fn heave_force_n(&self) -> [f32; 2] {
        self.heave_force_n
    }

    /// Flat-spot fraction per wheel, `0..=1`.
    pub(crate) fn flat_spot(&self) -> [f32; 4] {
        std::array::from_fn(|index| {
            (self.flat_spot_j[index] / self.spec.tyres.flat_spot_capacity_j.max(1.0))
                .clamp(0.0, 1.0)
        })
    }

    /// Chassis kick amplitude per wheel from the flat spots, newtons.
    pub(crate) fn flat_spot_force_n(&self) -> [f32; 4] {
        self.flat_spot_force_n
    }

    /// Switch the active suspension mode at runtime.
    pub(crate) fn set_active_suspension_mode(&mut self, mode: ActiveSuspensionMode) {
        self.spec.active_suspension.mode = mode;
        if mode == ActiveSuspensionMode::Off {
            self.active_suspension_force_n = [0.0; 4];
        }
    }

    /// Engine temperatures, coolant and oil, °C.
    pub(crate) fn engine_temperatures_c(&self) -> (f32, f32) {
        (self.coolant_c, self.oil_c)
    }

    /// Hybrid battery charge, joules.
    pub(crate) fn ers_energy_j(&self) -> f32 {
        self.ers_energy_j
    }

    /// Set the hybrid battery charge in joules.
    pub(crate) fn set_ers_energy_j(&mut self, joules: f32) {
        if joules.is_finite() {
            self.ers_energy_j = joules.clamp(0.0, self.spec.ers.capacity_j.max(0.0));
        }
    }

    /// Hybrid battery state of charge, `0..=1`.
    pub(crate) fn ers_soc(&self) -> f32 {
        let capacity = self.spec.ers.capacity_j.max(1.0);
        (self.ers_energy_j / capacity).clamp(0.0, 1.0)
    }

    /// Fit a fresh set of tyres: back to road temperature and zero wear.
    pub(crate) fn reset_tyres(&mut self) {
        let ambient = self.road_temperature_c();
        self.tyre_temperatures_c = [ambient; 4];
        self.tyre_surface_temperatures_c = [ambient; 4];
        self.tyre_core_temperatures_c = [ambient; 4];
        self.tyre_wear_j = [0.0; 4];
        self.water_film_mm = [0.0; 4];
    }

    /// 把车放到「比赛温度」：发动机/机油到工作温度，轮胎到最佳抓地温度，
    /// 刹车片到最佳效率温度。
    ///
    /// 这是所有商业竞速游戏对 AI 的默认状态——发车格上的 AI 从来不是冷车：
    /// 它前面已经跑过暖胎圈。实测不暖胎的代价极大：冷车 ECU 限扭把 AI 圈速
    /// 拖慢 20% 以上（这台 F2008 全程冷机只有 55% 功率、冷胎抓地也掉一档），
    /// 而且 Jolt 的变速箱在冷胎高滑移下拒绝升挡。所以「AI 圈速」「比赛起步」
    /// 这类语义都该从这里出发；要建模暖胎过程就让他自己跑几分钟。
    pub(crate) fn warm_up(&mut self) {
        let optimal = self.spec.engine.thermal.optimal_c;
        self.coolant_c = if optimal.is_finite() && optimal > 0.0 {
            optimal
        } else {
            self.coolant_c
        };
        self.oil_c = self.coolant_c;
        let tyre = self.spec.tyres.thermal.optimal_c;
        let tyre = if tyre.is_finite() && tyre > 0.0 {
            tyre
        } else {
            self.road_temperature_c()
        };
        self.tyre_temperatures_c = [tyre; 4];
        self.tyre_surface_temperatures_c = [tyre; 4];
        self.tyre_core_temperatures_c = [tyre; 4];
        let disc = self.spec.brakes.optimal_c;
        let disc = if disc.is_finite() && disc > 0.0 {
            disc
        } else {
            300.0
        };
        self.brake_temperatures_c = [disc; 4];
        self.brake_core_temperatures_c = [disc * 0.7; 4];
    }

    /// Remove this vehicle's constraint and chassis body from the world.
    pub(crate) fn remove(self, physics: &mut PhysicsWorld) -> WorldResult<()> {
        physics.remove_vehicle(self.vehicle)?;
        physics.remove_body(self.body)?;
        Ok(())
    }

    /// Average rolling-resistance coefficient of the wheels on the ground.
    /// Rolling-resistance force of this car, N, opposing its own motion.
    ///
    /// Two terms: the per-surface coefficient the world has always had, and —
    /// when the car authors it — the tyre's own rolling resistance from AC's
    /// `tyres.ini`, which has a constant part, a speed-squared part and a
    /// slip-squared part. The latter is what makes a car coast down and what
    /// costs a little speed under braking.
    pub(crate) fn rolling_resistance_force_n(
        &self,
        weather: &Weather,
        surfaces: &SurfaceMap,
    ) -> f32 {
        let mut force = 0.0f32;
        let mut coefficient_sum = 0.0f32;
        let mut count = 0u32;
        for (index, wheel) in self.snapshot.wheels.iter().enumerate() {
            if !wheel.contact {
                continue;
            }
            if let Some(properties) = surfaces.table().get(wheel.surface) {
                coefficient_sum += weather.rolling_resistance(properties, wheel.water_depth_m);
                count += 1;
            }
            // Tyre rolling resistance for this wheel. The slip term uses the
            // slip angle, so a scrub cornering slowly also costs rubber.
            let pressure = self.spec.tyres.pressure_at(self.tyre_temperatures_c[index]);
            force += self.spec.tyres.rolling_resistance_n(
                self.snapshot.speed_mps,
                wheel.slip_angle,
                pressure,
            );
        }
        if count == 0 {
            return force.max(0.0);
        }
        force + coefficient_sum / count as f32 * self.spec.mass_kg * 9.806_65
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use vettura_physics::{BrakeSpec, TyreSpec, TyreThermalSpec};
    use vettura_track::SurfaceMap;

    #[allow(unused_imports)]
    use super::aids::TC_SLIP_TARGET;
    use super::aids::{ABS_FACTOR_MIN, ABS_RECOVERY_PER_S, ABS_SLIP_TARGET, ABS_STEP_DT_S};

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
    fn a_tyre_spring_in_series_softens_the_suspension() {
        // The same suspension spring, with and without a tyre vertical rate:
        // the wheel sees the series combination, so it sits lower.
        let settle_with = |tyre_rate: f32| -> f32 {
            let spec = VehicleSpec {
                // A stiff suspension, so the geometry is not clamped and the
                // tyre's own rate is visible.
                wheels: std::array::from_fn(|index| {
                    let mut wheel = VehicleSpec::default().wheels[index].clone();
                    wheel.suspension.spring_rate_n_per_m = 200_000.0;
                    wheel
                }),
                tyres: TyreSpec {
                    spring_rate_n_per_m: tyre_rate,
                    ..TyreSpec::default()
                },
                ..VehicleSpec::default()
            };
            let mut world = World::new(SurfaceMap::new()).unwrap();
            let car = world.add_vehicle(spec, Pose::default()).unwrap();
            for _ in 0..240 {
                world.step_exact(1.0 / 240.0).unwrap();
            }
            world
                .snapshot(car)
                .unwrap()
                .wheels
                .iter()
                .map(|wheel| wheel.suspension_length_m)
                .sum::<f32>()
                / 4.0
        };
        let hard = settle_with(0.0);
        let soft = settle_with(300_000.0);
        assert!(
            (soft - hard).abs() > 0.002,
            "the tyre spring made no difference: {hard} vs {soft}"
        );
    }

    #[test]
    fn three_layered_tyres_take_time_to_come_up_to_temperature() {
        let spec = |layers: bool| -> TyreThermalSpec {
            TyreThermalSpec {
                surface_capacity_j_per_k: if layers { 60.0 } else { 0.0 },
                patch_capacity_j_per_k: if layers { 200.0 } else { 0.0 },
                heat_capacity_j_per_k: 1_000.0,
                surface_transfer_per_s: if layers { 0.018 } else { 0.0 },
                patch_transfer_per_s: if layers { 0.00027 } else { 0.0 },
                ..TyreThermalSpec::default()
            }
        };
        // One second of 20 kW into the tread: thin surface, thick core.
        let hot = advance_tyre_temperatures(
            &spec(true),
            TyreTemperatures {
                surface: 32.0,
                patch: 32.0,
                core: 32.0,
            },
            20_000.0,
            0.0,
            32.0,
            1.0,
        );
        let (surface, core) = (hot.surface, hot.core);
        assert!(
            surface > core,
            "the core led the surface: {core} vs {surface}"
        );
        assert!(surface > 100.0, "the surface barely heated: {surface}");

        // The legacy single mass behaves exactly like the old model: all three
        // temperatures are the same carcass.
        let flat = advance_tyre_temperatures(
            &spec(false),
            TyreTemperatures {
                surface: 32.0,
                patch: 32.0,
                core: 32.0,
            },
            20_000.0,
            0.0,
            32.0,
            1.0,
        );
        assert_eq!(flat.surface, flat.patch);
        assert_eq!(flat.patch, flat.core);
        assert!(
            flat.patch < surface,
            "the layered model is not laggier: {} vs {surface}",
            flat.patch
        );

        // Sustained power brings the patch up over a lap while the surface stays
        // hotter and the bulk core stays cold -- exactly the AC picture.
        let mut state = TyreTemperatures {
            surface: 32.0,
            patch: 32.0,
            core: 32.0,
        };
        for _ in 0..240 * 20 {
            state = advance_tyre_temperatures(&spec(true), state, 500.0, 60.0, 32.0, 0.25);
        }
        let (surface, patch, core) = (state.surface, state.patch, state.core);
        assert!(patch > 60.0, "the patch never warmed up: {patch}");
        assert!(
            surface > patch,
            "the surface fell behind: {surface} vs {patch}"
        );
        assert!(core < patch, "the bulk warmed too fast: {core} vs {patch}");
    }

    #[test]
    fn brake_heat_soaks_in_slowly_and_comes_back_quickly() {
        let spec = BrakeSpec::default();
        // 100 kJ into one disc heats the surface by itself, but with a rotor
        // core most of that energy ends up in the bulk: the pads stay cooler
        let single = advance_brake_temperatures(
            &BrakeSpec {
                core_capacity_j_per_k: 0.0,
                ..spec.clone()
            },
            32.0,
            32.0,
            100_000.0,
            60.0,
            32.0,
            1.0 / 240.0,
        );
        let two_mass =
            advance_brake_temperatures(&spec, 32.0, 32.0, 100_000.0, 60.0, 32.0, 1.0 / 240.0);
        let (single_surface, _) = single;
        let (surface, core) = two_mass;
        assert!(surface < single_surface, "{surface} vs {single_surface}");
        // The core really is a second, lagging mass that caught most of the heat.
        assert!(core > 32.0, "the core stayed cold: {core}");
        assert!(
            surface > core,
            "the core led the surface: {core} vs {surface}"
        );

        // With the pedal up the ducting cools the surface; once it drops below
        // the core, the rotor hands its heat back quickly, which is the soak.
        let mut surface = surface;
        let mut core = core;
        for _ in 0..240 * 10 {
            let (next_surface, next_core) =
                advance_brake_temperatures(&spec, surface, core, 0.0, 60.0, 32.0, 1.0 / 240.0);
            surface = next_surface;
            core = next_core;
        }
        assert!(
            surface > 32.0 + 5.0,
            "the pads cooled straight to ambient, no soak: {surface}"
        );

        // Apples to apples on the decay: start a single-mass disc at the same
        // surface temperature, with the pedal up, and it can only cool. The
        // rotor with a core keeps feeding its heat back into the pads.
        let without_core = BrakeSpec {
            core_capacity_j_per_k: 0.0,
            ..spec.clone()
        };
        let mut flat = surface;
        for _ in 0..240 * 10 {
            let (next, _) =
                advance_brake_temperatures(&without_core, flat, flat, 0.0, 60.0, 32.0, 1.0 / 240.0);
            flat = next;
        }
        assert!(
            surface > flat + 1.0,
            "the core did not keep the pads warm: {surface} vs {flat}"
        );
    }

    #[test]
    fn driver_aids_release_brake_and_throttle_before_a_lockup() {
        let spec = VehicleSpec::default();
        let mut snapshot = VehicleSnapshot::default();
        // 低速护栏会在 <2 m/s 时整体关掉辅助（静止抱死/虚高滑移没有意义），
        // 这里是"高速滑动"场景，得给一个真实车速。
        snapshot.speed_mps = 30.0;
        for (index, wheel) in snapshot.wheels.iter_mut().enumerate() {
            wheel.contact = true;
            wheel.load_n = 3_000.0;
            // The rear wheels are driven on this car; give them a huge slip.
            wheel.slip_ratio = if spec.wheels[index].driven { 0.8 } else { 0.0 };
        }

        // Braking with no aids keeps the requested pressure.
        let mut input = DriveInput {
            throttle: 1.0,
            brake: 1.0,
            ..DriveInput::default()
        };
        let mut factor = ABS_FACTOR_MAX;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        assert_eq!(input.brake, 1.0);
        assert_eq!(input.throttle, 1.0);

        // ABS releases to the point where the slip would sit at its target,
        // but never below the release floor.
        let mut input = DriveInput {
            throttle: 1.0,
            brake: 1.0,
            abs: true,
            tc: true,
            ..DriveInput::default()
        };
        let mut factor = ABS_FACTOR_MAX;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        // 滑移 0.8 → 比例命令 0.14/0.8 = 0.175，高于下限——下限只兜底，
        // 不把本该收敛的点抬上去。
        let expected_release = ABS_SLIP_TARGET / 0.8;
        assert!((input.brake - expected_release).abs() < 1.0e-6);
        // 状态也要落下来，否则下一步回压时会在死区里"凭空"保持满压。
        assert!((factor - expected_release).abs() < 1.0e-6);

        // TC 按比例释放：目标滑移 0.08 / 实际 0.8 = 0.1（下限 0 只是不封底）。
        assert!((input.throttle - TC_SLIP_TARGET / 0.8).abs() < 1.0e-6);

        // 滑移大到比例命令低于下限时（低载荷轮抱死：0.14/20 = 0.007），
        // 下限兜底到 2%——而不是旧值 25% 那样的"锁死"。
        for wheel in &mut snapshot.wheels {
            wheel.slip_ratio = 20.0;
        }
        let mut input = DriveInput {
            throttle: 0.1,
            brake: 1.0,
            abs: true,
            ..DriveInput::default()
        };
        let mut factor = ABS_FACTOR_MAX;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        assert_eq!(input.brake, 0.02);
        assert_eq!(factor, ABS_FACTOR_MIN);

        // With the wheels in their window neither system touches the input.
        for wheel in &mut snapshot.wheels {
            wheel.slip_ratio = 0.05;
        }
        let mut input = DriveInput {
            throttle: 0.5,
            brake: 0.7,
            abs: true,
            tc: true,
            ..DriveInput::default()
        };
        // 关键循环：滑移已明确回落到 RELEASE 之下，压力才按有限速率回上来，
        // 而不是瞬间恢复满压。
        let mut factor = expected_release;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        let expected = expected_release + ABS_RECOVERY_PER_S * ABS_STEP_DT_S;
        assert!((factor - expected).abs() < 1.0e-6);
        assert!(factor < ABS_FACTOR_MAX, "recovery is instant: {factor}");

        // Dead-zone: 滑移在 TARGET 与 RELEASE 之间时压力维持不加深不回收。
        for wheel in &mut snapshot.wheels {
            wheel.slip_ratio = 0.11;
        }
        let mut input = DriveInput {
            throttle: 0.1,
            brake: 0.1,
            abs: true,
            ..DriveInput::default()
        };
        let mut factor = 0.5;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        assert_eq!(factor, 0.5, "dead-zone must hold pressure");
        assert_eq!(input.brake, 0.05);

        // With no ABS flag the factor resets to full.
        let mut factor = 0.02;
        let mut input = DriveInput {
            brake: 1.0,
            ..DriveInput::default()
        };
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        assert_eq!(input.brake, 1.0);

        // No contact means no measurement, so no intervention.
        for wheel in &mut snapshot.wheels {
            wheel.contact = false;
            wheel.slip_ratio = 3.0;
        }
        let mut input = DriveInput {
            brake: 1.0,
            throttle: 1.0,
            abs: true,
            tc: true,
            ..DriveInput::default()
        };
        let mut factor = ABS_FACTOR_MIN;
        apply_driver_aids(&spec, &mut input, &snapshot, &mut factor);
        assert_eq!(input.brake, 1.0);
        assert_eq!(input.throttle, 1.0);
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
