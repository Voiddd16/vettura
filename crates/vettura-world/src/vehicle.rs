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
    Activation, BodyId, BodySettings, DriverInput, Inertia, MotionQuality, ObjectLayer,
    PhysicsWorld, Shape, SpringSettings, TireFriction, TireMaxImpulseInput, Vec3 as JoltVec3,
    VehicleAntiRollBar, VehicleCollisionTester, VehicleDifferentialSettings, VehicleEngineSettings,
    VehicleId, VehicleTransmissionSettings, WheelSettings, WheeledVehicleSettings,
};
use vettura_physics::{
    AERO_REFERENCE_SPEED_MPS, ActiveSuspensionMode, BrakeSpec, Controls, DrivetrainLayout, ErsMode,
    Pose, TyreThermalSpec, VehicleSpec, VehicleState, Wheel, WheelState, WingSpec,
    series_spring_rate,
};

use crate::frame::{j2p, j2p_q, j2p_r, p2j, p2j_q, p2j_r};
use crate::groove::GrooveMap;
use crate::surface::{SurfaceId, SurfaceMap, SurfaceTable};
use crate::weather::Weather;
use crate::{WorldError, WorldResult};

/// Fraction of a tyre's friction budget one direction keeps at full slip.
///
/// The friction ellipse never quite empties a direction: a wheel that is
/// completely sliding still holds 60 % of its budget, so launching on a banked
/// corner or a kerb does not remove all side force and spin the car.
const FRICTION_CIRCLE_FLOOR: f32 = 0.6;

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

/// Shared, atomically-updated grip inputs for the Jolt tyre callback.
///
/// The callback runs inside Jolt's solver, where taking a lock would be both
/// slow and an unwind hazard. Each step publishes one surface/water sample per
/// wheel plus the current weather scalars; the callback only performs relaxed
/// atomic loads and a table lookup.
struct GripState {
    surfaces: Arc<SurfaceTable>,
    tyre_peak_friction: f32,
    /// Tyre model, for the friction circle in the callback.
    tyre_model: vettura_physics::TyreSpec,
    surface_ids: [AtomicU32; 4],
    water_depth_bits: [AtomicU32; 4],
    /// Rubber multiplier under each wheel, one per wheel.
    groove_bits: [AtomicU32; 4],
    /// Tyre window multiplier per wheel (temperature, pressure, wear).
    tyre_bits: [AtomicU32; 4],
    /// Water film carried by each tyre, mm.
    water_film_bits: [AtomicU32; 4],
    /// Longitudinal slip demand per wheel, `0..=1` (friction-circle usage).
    longitudinal_demand_bits: [AtomicU32; 4],
    /// Lateral slip demand per wheel, `0..=1` (friction-circle usage).
    lateral_demand_bits: [AtomicU32; 4],
    /// Relaxation multiplier on the lateral force per wheel, `0..=1`.
    relaxation_lateral_bits: [AtomicU32; 4],
    /// Relaxation multiplier on the longitudinal force per wheel, `0..=1`.
    relaxation_longitudinal_bits: [AtomicU32; 4],
    speed_bits: AtomicU32,
    wetness_bits: AtomicU32,
    track_temperature_bits: AtomicU32,
    grip_scale_bits: AtomicU32,
}

impl GripState {
    fn new(
        surfaces: Arc<SurfaceTable>,
        tyre_peak_friction: f32,
        tyre_model: vettura_physics::TyreSpec,
    ) -> Self {
        Self {
            surfaces,
            tyre_peak_friction: tyre_peak_friction.max(0.0),
            tyre_model,
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
            groove_bits: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            tyre_bits: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            water_film_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            longitudinal_demand_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            relaxation_lateral_bits: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            relaxation_longitudinal_bits: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            lateral_demand_bits: [
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
            // How much of the friction budget each direction is asking for.
            // The normalisation points are where the slip curve has given up
            // most of its grip, so a normal power-on or cornering wheel still
            // keeps almost all of its budget in both directions.
            // Normalisation points are deep into slipping, so a normal
            // power-on or cornering wheel keeps almost all of its budget.
            let longitudinal = (wheel.slip_ratio.abs() / 1.0).clamp(0.0, 1.0);
            let lateral = (wheel.slip_angle.abs() / 0.8).clamp(0.0, 1.0);
            self.longitudinal_demand_bits[index].store(longitudinal.to_bits(), Ordering::Relaxed);
            self.lateral_demand_bits[index].store(lateral.to_bits(), Ordering::Relaxed);
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
        let water_film_mm = f32::from_bits(self.water_film_bits[index].load(Ordering::Relaxed));
        let speed_mps = f32::from_bits(self.speed_bits.load(Ordering::Relaxed));

        let weather = Weather {
            wetness: f32::from_bits(self.wetness_bits.load(Ordering::Relaxed)),
            track_temperature_c: f32::from_bits(
                self.track_temperature_bits.load(Ordering::Relaxed),
            ),
            grip_scale: f32::from_bits(self.grip_scale_bits.load(Ordering::Relaxed)),
            ..Weather::dry()
        };

        // A rough road costs grip: the tyre load is never steady, so the
        // effective friction falls in proportion to how steep the undulation
        // is. AC's own run-off areas (0.03 m over 0.5 m) lose about 40%.
        let undulation = (surface.bump_height_m / surface.bump_length_m.max(0.25)).min(0.6);
        let roughness_loss = (1.0 - undulation).clamp(0.5, 1.0);

        // `input.default_friction()` is Jolt's curve already combined with the
        // ground body's friction via sqrt. The ground is 1.0, so the combined
        // peak is still 1.0 and `scale` sets the final peak exactly; only the
        // non-peak points are lifted slightly by the sqrt.
        let environmental =
            weather.grip_for_wheel(surface, speed_mps, water_depth_m, water_film_mm)
                * roughness_loss;
        // Rubber laid down by earlier laps, sampled per wheel.
        let groove = f32::from_bits(self.groove_bits[index].load(Ordering::Relaxed));
        let groove = if groove.is_finite() {
            groove.clamp(0.5, 2.0)
        } else {
            1.0
        };
        let tyre_window = f32::from_bits(self.tyre_bits[index].load(Ordering::Relaxed));
        let tyre_window = if tyre_window.is_finite() {
            tyre_window.clamp(0.5, 1.0)
        } else {
            1.0
        };
        // Combined slip: a tyre has one friction budget, so the longitudinal
        // usage eats into what is left for the lateral force and back. This is
        // the effect that makes braking into a corner understeer.
        let longitudinal_demand =
            f32::from_bits(self.longitudinal_demand_bits[index].load(Ordering::Relaxed));
        let lateral_demand =
            f32::from_bits(self.lateral_demand_bits[index].load(Ordering::Relaxed));
        let tyres = &self.tyre_model;
        // The ellipse never quite empties a direction: a wheel that is fully
        // slipping still keeps 60 % of its budget, so a launch on a banked
        // corner does not lose all side force.
        let relaxation_long =
            f32::from_bits(self.relaxation_longitudinal_bits[index].load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let relaxation_lateral =
            f32::from_bits(self.relaxation_lateral_bits[index].load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let circle_long = (FRICTION_CIRCLE_FLOOR
            + (1.0 - FRICTION_CIRCLE_FLOOR) * tyres.longitudinal_share(lateral_demand))
            * relaxation_long;
        let circle_lateral = (FRICTION_CIRCLE_FLOOR
            + (1.0 - FRICTION_CIRCLE_FLOOR) * tyres.lateral_share(longitudinal_demand))
            * relaxation_lateral;

        // Load sensitivity: the tyre raises less friction per newton the
        // heavier it is loaded. Jolt hands the solver's suspension impulse, so
        // the load is available right here.
        let load_n = input.suspension_impulse / input.delta_time.max(1.0e-6);
        let load_long = tyres.load_friction_multiplier(load_n, true);
        let load_lateral = tyres.load_friction_multiplier(load_n, false);

        let scale =
            (self.tyre_peak_friction * environmental * groove * tyre_window).clamp(0.0, 8.0);
        let base = input.default_friction();
        TireFriction {
            longitudinal: base.longitudinal * scale * circle_long * load_long,
            lateral: base.lateral * scale * circle_lateral * load_lateral,
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
    /// Maximum crank torque Jolt's engine was built with, N·m.
    engine_max_torque_nm: f32,
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

/// Slip ratio the anti-lock brake controller aims for.
const ABS_SLIP_TARGET: f32 = 0.14;
/// Slip ratio the traction controller aims for.
const TC_SLIP_TARGET: f32 = 0.12;
/// Lowest fraction of the requested brake the ABS ever applies.
const ABS_MIN_FACTOR: f32 = 0.25;
/// Lowest fraction of the requested throttle the TC ever applies.
const TC_MIN_FACTOR: f32 = 0.2;

/// Worst absolute slip ratio of the wheels that are touching the ground.
fn worst_slip(spec: &VehicleSpec, snapshot: &VehicleSnapshot, driven_only: bool) -> f32 {
    let mut worst = 0.0f32;
    for (index, wheel) in snapshot.wheels.iter().enumerate() {
        if driven_only && !spec.wheels[index].driven {
            continue;
        }
        if !wheel.contact || wheel.load_n <= 0.0 {
            continue;
        }
        worst = worst.max(wheel.slip_ratio.abs());
    }
    if worst.is_finite() { worst } else { 0.0 }
}

/// Apply the closed-loop brake and traction controllers to this step's input.
///
/// Both are the car's own systems, not race rules: they watch the wheel slip
/// they actually produced last step and release brake pressure or cut torque
/// before a wheel locks or spins up. Jolt offers one brake input for the whole
/// car, so the ABS scales that input instead of pulsing a single wheel; the
/// effect on the wheel slip is the same.
fn apply_driver_aids(spec: &VehicleSpec, input: &mut DriveInput, snapshot: &VehicleSnapshot) {
    if input.abs && input.brake > 1.0e-3 {
        let worst = worst_slip(spec, snapshot, false);
        if worst > ABS_SLIP_TARGET {
            let factor = (ABS_SLIP_TARGET / worst).clamp(ABS_MIN_FACTOR, 1.0);
            input.brake *= factor;
        }
    }
    if input.tc && input.throttle > 1.0e-3 {
        let worst = worst_slip(spec, snapshot, true);
        if worst > TC_SLIP_TARGET {
            let factor = (TC_SLIP_TARGET / worst).clamp(TC_MIN_FACTOR, 1.0);
            input.throttle *= factor;
        }
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

        let engine_torque_cap = engine_max_torque_nm(&spec, &tuning);
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
            engine_max_torque_nm: engine_torque_cap,
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
        apply_driver_aids(&self.spec, &mut input, &self.snapshot);
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
        // Lifting off: the engine's pumping and friction losses become the
        // braking torque. Jolt scales its torque by this input, so a small
        // negative value reproduces `[COAST_REF]` exactly. Only apply at zero
        // throttle: a negative throttle still means "reverse".
        if input.throttle.abs() < 1.0e-3 {
            let coast = self.spec.engine.coast_torque_at(self.snapshot.engine_rpm);
            if coast > 0.0 {
                let fraction = (coast / self.engine_max_torque_nm.max(1.0e-3)).clamp(0.0, 1.0);
                forward = forward.min(-fraction);
            }
        }
        physics
            .vehicle_mut(self.vehicle)?
            .set_driver_input(DriverInput {
                forward,
                right: -input.steering,
                brake: input.brake,
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
        self.brake_temperatures_c = [self.spec.brakes.ambient_c.max(0.0); 4];
        self.brake_core_temperatures_c = self.brake_temperatures_c;
        self.previous_suspension_lengths = [0.0; 4];
        self.suspension_force_n = [0.0; 4];
        self.active_suspension_force_n = [0.0; 4];
        self.heave_force_n = [0.0; 2];
        let ambient = self.spec.brakes.ambient_c.max(0.0);
        self.tyre_temperatures_c = [ambient; 4];
        self.tyre_surface_temperatures_c = [ambient; 4];
        self.tyre_core_temperatures_c = [ambient; 4];
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
                load_n: (state.suspension_lambda * force_scale).max(0.0),
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
            // rolls onto a wheel, its tyre leans further negative.
            self.camber_rad[index] = suspension.camber_rad(compression);
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
        weather: &Weather,
        linear: Vec3,
        relative_air: Vec3,
    ) -> WorldResult<()> {
        let spec = &self.spec;
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
                let downforce = 0.5 * weather.air_density_kg_m3 * cl_area * speed_sq * smoothing;
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
            let base = 0.5 * weather.air_density_kg_m3 * cl_area * speed_sq * smoothing;

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
                    body.add_force_at_point(p2j(down * front * front_fade), p2j_r(front_axle))?;
                }
                if rear > 1.0e-3 {
                    body.add_force_at_point(p2j(down * rear * rear_fade), p2j_r(rear_axle))?;
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
            let cooling = (engine.thermal.cooling_per_s
                + (speed_mps.abs() / 60.0).clamp(0.0, 3.0) * 0.05)
                * step;
            let ambient = self.spec.brakes.ambient_c.max(0.0);
            let capacity = engine.thermal.heat_capacity_j_per_k.max(1.0);
            self.coolant_c = (self.coolant_c
                + (heat / capacity) * (1.0 - cooling.min(0.9))
                + ambient * cooling.min(0.9))
            .clamp(ambient, 250.0);
            // Oil lags the coolant: it is further from the combustion heat and
            // has its own path to ambient.
            self.oil_c = (self.oil_c
                + (heat / (capacity * 2.0)) * (1.0 - cooling.min(0.9))
                + ambient * cooling.min(0.9))
            .clamp(ambient, 250.0);
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
                if power_w.is_finite() && power_w > 0.0 {
                    let heat = power_w * tyre.thermal.heat_share * step;
                    let ambient = self.spec.brakes.ambient_c.max(0.0);
                    // Three layers when the car authors them, one mass
                    // otherwise (see `advance_tyre_temperatures`).
                    let layered = advance_tyre_temperatures(
                        &tyre.thermal,
                        TyreTemperatures {
                            surface: self.tyre_surface_temperatures_c[index],
                            patch: self.tyre_temperatures_c[index],
                            core: self.tyre_core_temperatures_c[index],
                        },
                        heat,
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
            self.grip.water_film_bits[index]
                .store(self.water_film_mm[index].to_bits(), Ordering::Relaxed);
            // Water carries tyre heat away, which is the main reason a wet
            // lap does not overheat the tyres: a film of a few millimetres
            // raises the convective loss by an order of magnitude over air.
            let cooling = (self.water_film_mm[index] * step * 0.5).min(step * 40.0);
            if cooling > 0.0 {
                self.tyre_temperatures_c[index] =
                    (self.tyre_temperatures_c[index] - cooling).max(0.0);
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
            self.grip.tyre_bits[index].store(multiplier.to_bits(), Ordering::Relaxed);
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
            let mut multiplier = f32::from_bits(self.grip.tyre_bits[index].load(Ordering::Relaxed))
                * spec.tyres.flat_spot_grip_multiplier(fraction);
            // A tyre only grips at the camber it was designed for.
            multiplier *= spec.tyres.camber_grip_multiplier(self.camber_rad[index]);
            self.grip.tyre_bits[index].store(multiplier.to_bits(), Ordering::Relaxed);

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
                self.grip.relaxation_longitudinal_bits[index]
                    .store(longitudinal.to_bits(), Ordering::Relaxed);
                self.grip.relaxation_lateral_bits[index]
                    .store(lateral.to_bits(), Ordering::Relaxed);
            } else {
                self.grip.relaxation_longitudinal_bits[index]
                    .store(1.0f32.to_bits(), Ordering::Relaxed);
                self.grip.relaxation_lateral_bits[index].store(1.0f32.to_bits(), Ordering::Relaxed);
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
                self.grip.groove_bits[index].store(multiplier.to_bits(), Ordering::Relaxed);
            } else {
                self.grip.groove_bits[index].store(1.0f32.to_bits(), Ordering::Relaxed);
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

    /// Fit a fresh set of tyres: back to ambient temperature and zero wear.
    pub(crate) fn reset_tyres(&mut self) {
        let ambient = self.spec.brakes.ambient_c.max(0.0);
        self.tyre_temperatures_c = [ambient; 4];
        self.tyre_wear_j = [0.0; 4];
        self.water_film_mm = [0.0; 4];
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
    spec.chassis_half_extents()
}

/// Vertical rate of one tyre at its cold inflation, N/m (`0` = not authored).
///
/// The geometry and the wheel settings are both built once, so the cold
/// pressure is what they use; the warm-rate variation is a runtime effect the
/// immutable Jolt settings cannot follow.
fn wheel_vertical_rate(spec: &VehicleSpec, _index: usize) -> f32 {
    spec.tyres
        .vertical_rate_n_per_m(spec.tyres.pressure_at(spec.brakes.ambient_c))
        .max(0.0)
}

/// Suspension geometry derived from the spec and tuning.
///
/// Both [`vehicle_settings`] and the ride-height fade read the same numbers so
/// the fade always agrees with the suspension Jolt actually builds.
#[derive(Clone, Copy, Debug)]
struct WheelGeometry {
    /// Suspension length with no load, metres.
    rest_length: f32,
    /// Longest suspension length available, metres.
    max_length: f32,
    /// Shortest suspension length available, metres.
    min_length: f32,
    /// Wheel mount position relative to the centre of mass, physics frame.
    mount: Vec3,
    /// Wheel radius, metres.
    radius: f32,
}

fn wheel_geometries(spec: &VehicleSpec) -> [WheelGeometry; 4] {
    let cg = spec.cg_height_m;
    let spring_force_scale = spec.mass_kg * 9.81 * 0.25;
    std::array::from_fn(|index| {
        let wheel_spec = &spec.wheels[index];
        let radius = wheel_spec.radius_m.max(0.02);
        let travel = wheel_spec.suspension.travel_m.clamp(0.02, 0.35);
        // The wheel is connected to the ground through the suspension spring
        // *and* the tyre's own vertical stiffness, so the chassis sees the
        // series combination. Without an authored tyre rate this is just the
        // suspension spring, which is what Jolt models on its own.
        let spring = series_spring_rate(
            wheel_spec.suspension.spring_rate_n_per_m.max(1_000.0),
            wheel_vertical_rate(spec, index),
        );
        let static_compression = (spring_force_scale / spring).clamp(0.001, travel * 0.45);
        let max_length = (static_compression + travel * 0.5).max(0.02);
        let min_length = (max_length - travel - 0.001)
            .max(0.005)
            .min(max_length - 0.001);
        let rest_length = max_length - static_compression;
        WheelGeometry {
            rest_length,
            max_length,
            min_length,
            mount: Vec3::new(
                wheel_spec.position.x,
                radius + rest_length - cg,
                wheel_spec.position.z,
            ),
            radius,
        }
    })
}

/// AC wing mount position, in the physics frame.
///
/// AC authors positions as `(+X left, +Y up, +Z forward)`; the physics frame
/// keeps X and Y and flips Z, because physics forward is `-Z`.
fn wing_local_position(wing: &WingSpec) -> Vec3 {
    Vec3::new(wing.position.x, wing.position.y, -wing.position.z)
}

/// Maximum crank torque the Jolt engine is built with.
///
/// Jolt takes one maximum torque and a normalized curve. The engine is built at
/// the fully boosted torque, then the runtime scales the throttle by how much
/// boost is actually available, so at zero boost the car makes its naturally
/// aspirated `power.lut` torque. The boost torque, the engine tuning and the
/// driveline efficiency all live in this number.
fn engine_max_torque_nm(spec: &VehicleSpec, tuning: &VehicleTuning) -> f32 {
    let efficiency = spec.drivetrain.efficiency.clamp(0.3, 1.0);
    let turbo_scale = spec.engine.turbo.max_torque_multiplier().max(1.0);
    (spec.engine.peak_torque_nm * tuning.engine_scale * efficiency * turbo_scale)
        .clamp(1.0, 5_000.0)
}

fn vehicle_settings(
    spec: &VehicleSpec,
    tuning: &VehicleTuning,
) -> WorldResult<WheeledVehicleSettings> {
    let brake_scale = tuning.brake_scale.clamp(0.0, 4.0);
    let geometries = wheel_geometries(spec);

    // Same tyre shape for all four corners: the per-wheel differences (load,
    // temperature, wear, camber, water film) live in the grip callback chain.
    let curves = spec.tyres.slip_curves();
    let mut wheels = Vec::with_capacity(4);
    for wheel in Wheel::ALL {
        let wheel_spec = &spec.wheels[wheel.index()];
        let geometry = geometries[wheel.index()];
        let radius = geometry.radius;
        let width = wheel_spec.width_m.max(0.04);
        let spring = wheel_spec.suspension.spring_rate_n_per_m.max(1_000.0);
        let damper = (0.5
            * (wheel_spec.suspension.damper_bump_ns_per_m.max(0.0)
                + wheel_spec.suspension.damper_rebound_ns_per_m.max(0.0)))
        .max(100.0);
        let max_length = geometry.max_length;
        let min_length = geometry.min_length;
        let position = p2j(geometry.mount);
        // Brake torque now comes from `brakes.ini`: a peak torque shared by
        // the front/rear bias, split across the pair on each axle.
        let peak = spec.brakes.max_torque_nm.max(0.0) * brake_scale;
        let front_axle_torque = peak * spec.brakes.front_share.clamp(0.0, 1.0);
        let brake_torque = if wheel.is_front() {
            front_axle_torque
        } else {
            peak - front_axle_torque
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
            // The callback scales these curves by tyre peak × surface × weather ×
            // load × camber × …, so they only carry the tyre's own shape: where it
            // peaks and how quickly it gives the grip back. That comes from the
            // car's own data as a Magic Formula fit (see `vettura_physics`'s
            // `tyre_model`); both tables peak at exactly 1.0.
            .longitudinal_friction(curves.0.clone())
            .lateral_friction(curves.1.clone());

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

    let engine = VehicleEngineSettings::default()
        .max_torque(engine_max_torque_nm(spec, tuning))
        .min_rpm(min_rpm)
        .max_rpm(max_rpm)
        .normalized_torque(spec.engine.normalized_torque_curve(max_rpm))
        .inertia(spec.gearbox.inertia.clamp(0.005, 2.0))
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
        // `drivetrain.ini` reports shift and clutch times in milliseconds;
        // the clutch strength comes from the clutch torque capacity.
        .switch_time((spec.gearbox.switch_time * 0.001).clamp(0.01, 0.5))
        .clutch_release_time((spec.gearbox.clutch_release_time * 0.001).clamp(0.01, 1.0))
        .switch_latency(0.1)
        .shift_up_rpm(shift_up_rpm)
        .shift_down_rpm(shift_down_rpm)
        .clutch_strength((spec.gearbox.clutch_max_torque_nm.max(1.0) / 50.0).clamp(1.0, 200.0));

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

    // AC's differential lock is a fraction: `1.0` fully locked, `0` open.
    // Jolt expresses it as the fastest/slowest speed ratio at which torque
    // still gets put into the slower wheel, so an open diff needs a large
    // number and a locked one needs 1.0. Jolt has one ratio, so power and
    // coast are averaged; the preload stays in the spec for callers that
    // model it separately.
    let lock = 0.5
        * (spec.drivetrain.diff.power_lock.clamp(0.01, 1.0)
            + spec.drivetrain.diff.coast_lock.clamp(0.01, 1.0));
    // Jolt needs a ratio strictly above 1: a fully locked AC diff becomes just
    // over 1, and an open one becomes a very large number.
    let lock_ratio = (1.0 / lock).clamp(1.0001, f32::MAX);

    Ok(
        WheeledVehicleSettings::new(wheels, differentials, collision_tester)
            .engine(engine)
            .transmission(transmission)
            .anti_roll_bars(anti_roll_bars)
            .differential_limited_slip_ratio(lock_ratio)
            .max_pitch_roll_angle(70.0f32.to_radians()),
    )
}

const TAG_VEHICLE: u64 = 1 << 32;

fn tag_vehicle(handle: VehicleHandle) -> u64 {
    TAG_VEHICLE | (handle.0 & 0xffff_ffff)
}

/// One step of the tyre thermal model, °C: `(surface, patch, core)`.
///
/// The road's energy enters the friction surface and then walks down the three
/// layers with AC's own rates (`SURFACE_TRANSFER` into the patch,
/// `PATCH_TRANSFER` into the core). That is what makes a tyre take a lap or two
/// to come up to temperature: the grip curve reads the core, which lags the
/// surface badly. `CORE_TRANSFER`/`INTERNAL_CORE_TRANSFER` describe conduction
/// inside the rotor's own bulk, which this model does not split, so they are
/// not needed. Cars that author no layers keep the single-mass behaviour, where
/// all three temperatures are the same carcass.
struct TyreTemperatures {
    surface: f32,
    patch: f32,
    core: f32,
}

/// Advance the three tyre temperatures by `heat_j` over `step` seconds at
/// `speed_mps` towards `ambient_c`.
/// One step of the tyre thermal model.
fn advance_tyre_temperatures(
    spec: &TyreThermalSpec,
    state: TyreTemperatures,
    heat_j: f32,
    speed_mps: f32,
    ambient_c: f32,
    step: f32,
) -> TyreTemperatures {
    let cooling = ((spec.cooling_per_s + (speed_mps.abs() / 60.0).clamp(0.0, 3.0) * 0.06) * step)
        .clamp(0.0, 0.9);
    if !spec.has_layers() {
        // Single mass: everything happens in the carcass, as before.
        let next = state.core + (heat_j / spec.heat_capacity_j_per_k.max(1.0)) * (1.0 - cooling);
        let target = ambient_c.max(0.0);
        let core = (next + target * cooling).clamp(target, 1_400.0);
        return TyreTemperatures {
            surface: core,
            patch: core,
            core,
        };
    }

    let surface_capacity = spec.surface_capacity_j_per_k.max(1.0);
    let patch_capacity = spec.patch_capacity_j_per_k.max(1.0);
    let core_capacity = spec.heat_capacity_j_per_k.max(1.0);
    let ambient = ambient_c.max(0.0);
    let mut surface = (state.surface + heat_j / surface_capacity).clamp(ambient, 1_400.0);
    let mut patch = state.patch;
    let mut core = state.core;

    // Surface to air: the fastest channel, and it grows with the air flowing
    // over the tread.
    surface -= (surface - ambient) * cooling;

    // Down the chain. Each transfer is a fraction of the temperature
    // difference per second, so it is stable at any step size.
    let to_patch = spec.surface_transfer_per_s.max(0.0) * step;
    if to_patch > 0.0 {
        let flow = (surface - patch).clamp(-1.0e3, 1.0e3);
        surface -= flow * to_patch;
        patch += flow * to_patch * (surface_capacity / patch_capacity);
    }
    let to_core = spec.patch_transfer_per_s.max(0.0) * step;
    if to_core > 0.0 {
        let flow = (patch - core).clamp(-1.0e3, 1.0e3);
        patch -= flow * to_core;
        core += flow * to_core * (patch_capacity / core_capacity);
    }

    TyreTemperatures {
        surface: surface.clamp(ambient, 1_400.0),
        patch: patch.clamp(ambient, 1_400.0),
        core: core.clamp(ambient, 1_400.0),
    }
}

/// One step of the brake thermal model, °C.
///
/// Returns `(surface, core)` temperatures. The surface is the friction face the
/// pads see, which is what the efficiency curve is indexed by; the core is the
/// bulk of the rotor, which soaks heat in slowly and hands it back quickly, so
/// the pads keep cooking after the pedal is released.
fn advance_brake_temperatures(
    spec: &BrakeSpec,
    surface_in: f32,
    core_in: f32,
    energy_j: f32,
    speed_mps: f32,
    step: f32,
) -> (f32, f32) {
    let rise = if spec.thermal_mass_j_per_k > 1.0 {
        (energy_j * spec.heat_per_joule / spec.thermal_mass_j_per_k).max(0.0)
    } else {
        0.0
    };
    // Cooling: ambient exchange plus the air flowing over the disc.
    let speed_factor = (speed_mps / 60.0).clamp(0.05, 4.0);
    let cooling = (spec.cooling_per_s * speed_factor * step).clamp(0.0, 0.95);
    let difference = surface_in - spec.ambient_c;
    let drop = difference * cooling;
    let mut surface = surface_in + rise - drop;

    let mut core = core_in;
    let surface_capacity = spec.thermal_mass_j_per_k.max(1.0);
    let core_capacity = spec.core_capacity_j_per_k.max(1.0);
    if core_capacity > 1.0 {
        // Heat always flows down the gradient, but AC models the two directions
        // with different rates, so the conductance depends on which side is
        // hotter: soaks in slowly, comes back out quickly.
        let conductance = if surface > core {
            surface_capacity * spec.core_transfer_in_per_s.max(0.0)
        } else {
            core_capacity * spec.core_transfer_out_per_s.max(0.0)
        };
        let flow = conductance * (surface - core).clamp(-1.0e4, 1.0e4);
        surface -= flow * step / surface_capacity;
        core += flow * step / core_capacity;
        let air_flow =
            core_capacity * spec.core_transfer_air_per_s.max(0.0) * (core - spec.ambient_c);
        core -= air_flow * step / core_capacity;
    }

    (
        surface.clamp(spec.ambient_c, 1_400.0),
        core.clamp(spec.ambient_c, 1_400.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use crate::surface::SurfaceMap;
    use vettura_physics::TyreSpec;

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
        // while braking.
        let single = advance_brake_temperatures(
            &BrakeSpec {
                core_capacity_j_per_k: 0.0,
                ..spec.clone()
            },
            32.0,
            32.0,
            100_000.0,
            60.0,
            1.0 / 240.0,
        );
        let two_mass = advance_brake_temperatures(&spec, 32.0, 32.0, 100_000.0, 60.0, 1.0 / 240.0);
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
                advance_brake_temperatures(&spec, surface, core, 0.0, 60.0, 1.0 / 240.0);
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
                advance_brake_temperatures(&without_core, flat, flat, 0.0, 60.0, 1.0 / 240.0);
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
        apply_driver_aids(&spec, &mut input, &snapshot);
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
        apply_driver_aids(&spec, &mut input, &snapshot);
        assert_eq!(input.brake, ABS_MIN_FACTOR);
        assert_eq!(input.throttle, TC_MIN_FACTOR);

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
        apply_driver_aids(&spec, &mut input, &snapshot);
        assert_eq!(input.brake, 0.7);
        assert_eq!(input.throttle, 0.5);

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
        apply_driver_aids(&spec, &mut input, &snapshot);
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
