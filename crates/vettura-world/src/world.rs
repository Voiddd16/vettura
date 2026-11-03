//! The multi-vehicle Jolt world: stepping, surfaces, weather, track geometry,
//! and contact/impact reporting.
//!
//! A [`World`] owns exactly one Jolt `PhysicsWorld`. Vehicles are added in a
//! deterministic order; the world steps them together so car-to-car contacts
//! are resolved by Jolt's solver. Static boxes can be added as walls, barriers
//! or kerbs. Surface grip comes from a 2-D [`SurfaceMap`], not from per-triangle
//! collider materials.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use oxijolt::{
    BodyId, BodySettings, ContactEvent, EventSettings, PhysicsWorld, RVec3, Shape,
    Vec3 as JoltVec3, WorldEvents, WorldSettings as JoltWorldSettings,
};
use vettura_physics::{Controls, Pose, VehicleSpec, VehicleState};

use crate::frame::{j2p, j2p_r, p2j, p2j_q, p2j_r};
use crate::surface::{SurfaceId, SurfaceMap, SurfaceProperties};
use crate::vehicle::{DriveInput, VehicleHandle, VehicleRuntime, VehicleSnapshot, VehicleTuning};
use crate::weather::Weather;

/// Errors from world construction, vehicle creation, or stepping.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("Jolt world error: {0}")]
    JoltWorld(#[from] oxijolt::WorldError),
    #[error("Jolt shape error: {0}")]
    JoltShape(#[from] oxijolt::ShapeError),
    #[error("Jolt body error: {0}")]
    JoltBody(#[from] oxijolt::BodyError),
    #[error("Jolt vehicle error: {0}")]
    JoltVehicle(#[from] oxijolt::VehicleError),
    #[error("Jolt step error: {0}")]
    JoltStep(#[from] oxijolt::StepError),
    #[error("invalid vehicle specification: {0}")]
    InvalidVehicle(String),
    #[error("invalid world input: {0}")]
    InvalidInput(String),
    #[error("unknown vehicle handle")]
    UnknownVehicle,
    #[error("surfaces cannot be added after a vehicle exists; build the SurfaceMap first")]
    SurfacesLocked,
}

/// Result alias for this crate.
pub type WorldResult<T> = std::result::Result<T, WorldError>;

/// Identifies a static track object (wall, barrier, kerb box) in impact events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackObjectId(pub(crate) u64);

impl TrackObjectId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// One side of an [`ImpactEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactEntity {
    Vehicle(VehicleHandle),
    TrackObject(TrackObjectId),
    /// The static ground body.
    Ground,
    /// A body not registered as a vehicle or track object. The raw Jolt body id
    /// is kept for debugging.
    Other(u32),
}

impl ContactEntity {
    pub fn vehicle(self) -> Option<VehicleHandle> {
        match self {
            Self::Vehicle(handle) => Some(handle),
            _ => None,
        }
    }
}

/// A contact involving at least one vehicle, reported after Jolt's solver.
///
/// `normal_impulse_ns` is the total impulse along the contact normal. Dividing
/// it by the bodies' reduced mass gives `impact_speed_mps`, the normal velocity
/// change of the impact. That is a better severity signal than raw closing
/// speed because a glancing contact on a wall can have a large tangential
/// component but little normal impulse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactEvent {
    pub a: ContactEntity,
    pub b: ContactEntity,
    /// Average contact point on body A, physics frame.
    pub position: Vec3,
    /// Contact normal, pointing from A toward B, physics frame.
    pub normal: Vec3,
    pub normal_impulse_ns: f32,
    pub friction_impulse_ns: f32,
    /// Normal velocity change (`normal impulse / reduced mass`), m/s.
    pub impact_speed_mps: f32,
    /// Normalized severity in `0..=1`, saturating around 12 m/s of normal
    /// velocity change.
    pub severity: f32,
}

/// Borrowed view of one vehicle.
#[derive(Clone, Copy, Debug)]
pub struct VehicleView<'a> {
    pub handle: VehicleHandle,
    pub spec: &'a VehicleSpec,
    pub tuning: VehicleTuning,
    pub snapshot: &'a VehicleSnapshot,
}

/// A deterministic, fixed-step multi-vehicle world.
pub struct World {
    physics: PhysicsWorld,
    surface_map: SurfaceMap,
    weather: Weather,
    weather_target: Option<Weather>,
    weather_rate: f32,
    vehicles: Vec<VehicleRuntime>,
    body_to_vehicle: HashMap<BodyId, VehicleHandle>,
    track_objects: HashMap<BodyId, TrackObjectId>,
    next_vehicle: u64,
    next_track_object: u64,
    ground_body: BodyId,
    fixed_dt: f32,
    max_substeps: u32,
    accumulator: f32,
    impacts: Vec<ImpactEvent>,
}

impl World {
    /// Create a world with a flat static ground at `y = 0`.
    ///
    /// The ground is a large static box, not a plane, so its top face is exactly
    /// at `y = 0` for the wheel casts. Grip comes from `surface_map`; the ground
    /// body's own friction is 1.0 so Jolt's tyre-curve combination does not
    /// attenuate the callback.
    pub fn new(surface_map: SurfaceMap) -> WorldResult<Self> {
        // Single-threaded jobs: deterministic and avoids waking worker threads
        // from Godot's physics callback. `oxijolt` is built with
        // `cross-platform-deterministic`, which pins Jolt's FP settings.
        let jolt_settings = JoltWorldSettings::default().single_threaded();
        let mut physics = PhysicsWorld::new(jolt_settings)?;

        // Jolt caps a single shape's half extent at 2000 m; 1 km is enough for a
        // test track and leaves room for the collider's convex radius.
        let ground_shape = Shape::new_box(JoltVec3::new(1_000.0, 0.5, 1_000.0))?;
        let ground_settings = BodySettings::new_static()
            .position(RVec3::new(0.0, -0.5, 0.0))
            .friction(1.0)
            .restitution(0.0)
            .user_data(TAG_GROUND);
        let ground_body = physics.create_body(&ground_shape, &ground_settings)?;

        physics.set_event_settings(
            EventSettings::default()
                .contacts(true)
                .collision_estimates(true),
        );

        Ok(Self {
            physics,
            surface_map,
            weather: Weather::dry(),
            weather_target: None,
            weather_rate: 1.0,
            vehicles: Vec::new(),
            body_to_vehicle: HashMap::new(),
            track_objects: HashMap::new(),
            next_vehicle: 0,
            next_track_object: 0,
            ground_body,
            fixed_dt: 1.0 / 240.0,
            max_substeps: 8,
            accumulator: 0.0,
            impacts: Vec::new(),
        })
    }

    /// Add a vehicle with default [`VehicleTuning`].
    ///
    /// `pose.position` is the **ground-reference** position (the chassis
    /// reference frame used by `VehicleState`), not the centre of mass. The
    /// centre of mass is placed `cg_height_m` above it along the local up axis.
    pub fn add_vehicle(&mut self, spec: VehicleSpec, pose: Pose) -> WorldResult<VehicleHandle> {
        self.add_vehicle_with(spec, pose, VehicleTuning::default())
    }

    /// Add a vehicle with explicit tuning.
    pub fn add_vehicle_with(
        &mut self,
        spec: VehicleSpec,
        pose: Pose,
        tuning: VehicleTuning,
    ) -> WorldResult<VehicleHandle> {
        let handle = VehicleHandle(self.next_vehicle);
        let mut runtime = VehicleRuntime::create(
            &mut self.physics,
            handle,
            spec,
            pose,
            tuning,
            self.surface_map.table_arc(),
        )?;
        runtime.set_weather(&self.weather);
        let dt = self.fixed_dt;
        if let Err(error) = runtime.update_snapshot(&self.physics, dt, &self.surface_map) {
            let _ = runtime.remove(&mut self.physics);
            return Err(error);
        }
        self.next_vehicle += 1;
        self.body_to_vehicle.insert(runtime.body, handle);
        self.vehicles.push(runtime);
        Ok(handle)
    }

    /// Remove a vehicle and its Jolt body. The handle becomes invalid.
    pub fn remove_vehicle(&mut self, handle: VehicleHandle) -> WorldResult<()> {
        let index = self
            .vehicles
            .iter()
            .position(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        let runtime = self.vehicles.remove(index);
        self.body_to_vehicle.remove(&runtime.body);
        runtime.remove(&mut self.physics)
    }

    /// Append a custom surface. Must be done before any vehicle is created,
    /// because existing vehicles hold an immutable snapshot of the table for
    /// their tyre callback.
    pub fn add_surface(&mut self, properties: SurfaceProperties) -> WorldResult<SurfaceId> {
        if !self.vehicles.is_empty() {
            return Err(WorldError::SurfacesLocked);
        }
        Ok(self.surface_map.add_surface(properties))
    }

    /// Change the fallback surface for points outside every patch. Safe after
    /// vehicles exist (the map itself is consulted every step).
    pub fn set_default_surface(&mut self, surface: SurfaceId) {
        self.surface_map.set_default_surface(surface);
    }

    /// Add a static box. Useful for walls, barriers, kerbs and simple ramps.
    /// The box is axis-aligned in its local frame and then yawed by `yaw`
    /// radians (physics frame).
    pub fn add_static_box(
        &mut self,
        center: Vec3,
        half_extents: Vec3,
        yaw_radians: f32,
        friction: f32,
        restitution: f32,
    ) -> WorldResult<TrackObjectId> {
        if !center.is_finite() {
            return Err(WorldError::InvalidInput(
                "static box centre must be finite".to_owned(),
            ));
        }
        if !half_extents.is_finite() || half_extents.min_element() <= 0.0 {
            return Err(WorldError::InvalidInput(
                "static box half extents must be finite and positive".to_owned(),
            ));
        }
        if !yaw_radians.is_finite() {
            return Err(WorldError::InvalidInput(
                "static box yaw must be finite".to_owned(),
            ));
        }
        if !yaw_radians.is_finite() {
            return Err(WorldError::InvalidInput(
                "static box yaw must be finite".to_owned(),
            ));
        }

        let shape = Shape::new_box(JoltVec3::new(
            half_extents.x,
            half_extents.y,
            half_extents.z,
        ))?;
        let id = TrackObjectId(self.next_track_object);
        let friction = if friction.is_finite() {
            friction.clamp(0.0, 10.0)
        } else {
            0.7
        };
        let restitution = if restitution.is_finite() {
            restitution.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let settings = BodySettings::new_static()
            .position(p2j_r(center))
            .rotation(p2j_q(Quat::from_rotation_y(yaw_radians)))
            .friction(friction)
            .restitution(restitution)
            .user_data(tag_track_object(id));
        let body = self.physics.create_body(&shape, &settings)?;
        self.next_track_object += 1;
        self.track_objects.insert(body, id);
        Ok(id)
    }

    /// A wall with typical barrier friction and low restitution.
    pub fn add_wall(
        &mut self,
        center: Vec3,
        half_extents: Vec3,
        yaw_radians: f32,
    ) -> WorldResult<TrackObjectId> {
        self.add_static_box(center, half_extents, yaw_radians, 0.7, 0.05)
    }

    /// Advance by a wall-clock frame delta, accumulating into fixed steps.
    ///
    /// Returns the number of fixed steps actually run. If the accumulator still
    /// holds more than one step after `max_substeps`, the excess is dropped,
    /// matching `vettura_physics::Simulation`'s anti-spiral-of-death policy.
    pub fn step(&mut self, frame_dt: f32) -> WorldResult<u32> {
        if !frame_dt.is_finite() || frame_dt <= 0.0 {
            return Ok(0);
        }
        self.accumulator += frame_dt;

        let mut steps = 0;
        while self.accumulator + 1.0e-9 >= self.fixed_dt && steps < self.max_substeps {
            self.step_exact(self.fixed_dt)?;
            self.accumulator -= self.fixed_dt;
            steps += 1;
        }
        if steps == self.max_substeps && self.accumulator > self.fixed_dt {
            self.accumulator = 0.0;
        }
        Ok(steps)
    }

    /// Advance exactly once by `dt` seconds. Intended for replay and tests; the
    /// caller is responsible for a consistent fixed step.
    pub fn step_exact(&mut self, dt: f32) -> WorldResult<()> {
        if !dt.is_finite() || dt <= 0.0 {
            return Err(WorldError::InvalidInput(
                "dt must be finite and positive".to_owned(),
            ));
        }
        if !(PhysicsWorld::MIN_DELTA_TIME..=PhysicsWorld::MAX_DELTA_TIME).contains(&dt) {
            return Err(WorldError::InvalidInput(format!(
                "dt must be within [{}, {}]",
                PhysicsWorld::MIN_DELTA_TIME,
                PhysicsWorld::MAX_DELTA_TIME
            )));
        }

        self.advance_weather(dt);
        self.apply_external_forces()?;
        let report = self.physics.step(dt)?;
        if !report.is_complete() {
            tracing::warn!("Jolt step dropped contacts; the world may need a smaller fixed step");
        }
        let events = self.physics.take_events();
        self.update_snapshots(dt)?;
        self.record_impacts(&events);
        Ok(())
    }

    pub fn fixed_dt(&self) -> f32 {
        self.fixed_dt
    }

    pub fn set_fixed_dt(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.fixed_dt = dt.clamp(1.0e-4, 0.05);
        }
    }

    pub fn max_substeps(&self) -> u32 {
        self.max_substeps
    }

    pub fn set_max_substeps(&mut self, max_substeps: u32) {
        self.max_substeps = max_substeps.max(1);
    }

    pub fn weather(&self) -> Weather {
        self.weather
    }

    /// Set the current weather immediately.
    pub fn set_weather(&mut self, weather: Weather) {
        self.weather = weather.clamped();
        self.weather_target = None;
        for vehicle in &self.vehicles {
            vehicle.set_weather(&self.weather);
        }
    }

    /// Blend the current weather toward `target` over `seconds`. A non-positive
    /// or non-finite duration assigns it immediately.
    pub fn transition_weather(&mut self, target: Weather, seconds: f32) {
        let target = target.clamped();
        if !seconds.is_finite() || seconds <= 0.0 {
            self.set_weather(target);
            return;
        }
        self.weather_target = Some(target);
        self.weather_rate = 1.0 / seconds;
    }

    pub fn surface_map(&self) -> &SurfaceMap {
        &self.surface_map
    }

    /// Set the driver input for a vehicle.
    pub fn set_controls(&mut self, handle: VehicleHandle, controls: Controls) -> WorldResult<()> {
        self.set_input(handle, DriveInput::from(controls))
    }

    /// Set the low-level input for a vehicle.
    pub fn set_input(&mut self, handle: VehicleHandle, input: DriveInput) -> WorldResult<()> {
        let Self {
            physics, vehicles, ..
        } = self;
        let vehicle = vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.apply_input(physics, input)
    }

    /// Set the chassis linear and angular velocity in the physics frame.
    pub fn set_vehicle_velocity(
        &mut self,
        handle: VehicleHandle,
        linear: Vec3,
        angular: Vec3,
    ) -> WorldResult<()> {
        if !linear.is_finite() || !angular.is_finite() {
            return Err(WorldError::InvalidInput(
                "vehicle velocities must be finite".to_owned(),
            ));
        }
        let Self {
            physics, vehicles, ..
        } = self;
        let vehicle = vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.set_velocity(physics, linear, angular)
    }

    /// Teleport a vehicle to `pose` and clear its velocities. The pose is the
    /// ground-reference pose, matching [`VehicleState::position`].
    pub fn reset_vehicle(&mut self, handle: VehicleHandle, pose: Pose) -> WorldResult<()> {
        let dt = self.fixed_dt;
        let Self {
            physics,
            vehicles,
            surface_map,
            ..
        } = self;
        let vehicle = vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.reset(physics, pose)?;
        vehicle.update_snapshot(physics, dt, surface_map)
    }

    pub fn vehicle(&self, handle: VehicleHandle) -> Option<VehicleView<'_>> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(|vehicle| VehicleView {
                handle: vehicle.handle,
                spec: &vehicle.spec,
                tuning: vehicle.tuning,
                snapshot: &vehicle.snapshot,
            })
    }

    pub fn snapshot(&self, handle: VehicleHandle) -> Option<&VehicleSnapshot> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(|vehicle| &vehicle.snapshot)
    }

    /// Snapshot converted to the backend-neutral state used by Godot.
    pub fn vehicle_state(&self, handle: VehicleHandle) -> Option<VehicleState> {
        self.snapshot(handle).map(VehicleSnapshot::to_vehicle_state)
    }

    pub fn vehicle_count(&self) -> usize {
        self.vehicles.len()
    }

    pub fn vehicle_handles(&self) -> impl Iterator<Item = VehicleHandle> + '_ {
        self.vehicles.iter().map(|vehicle| vehicle.handle)
    }

    pub fn track_object_count(&self) -> usize {
        self.track_objects.len()
    }

    /// Impacts accumulated since the last [`drain_impacts`](Self::drain_impacts).
    pub fn impacts(&self) -> &[ImpactEvent] {
        &self.impacts
    }

    /// Take all accumulated impacts, leaving the buffer empty.
    pub fn drain_impacts(&mut self) -> Vec<ImpactEvent> {
        std::mem::take(&mut self.impacts)
    }

    pub fn clear_impacts(&mut self) {
        self.impacts.clear();
    }

    fn advance_weather(&mut self, dt: f32) {
        let Some(target) = self.weather_target else {
            return;
        };
        let blend = (self.weather_rate * dt).clamp(0.0, 1.0);
        self.weather = Weather::lerp(&self.weather, &target, blend);
        for vehicle in &self.vehicles {
            vehicle.set_weather(&self.weather);
        }
        if blend >= 1.0 {
            self.weather_target = None;
        }
    }

    fn apply_external_forces(&mut self) -> WorldResult<()> {
        let weather = self.weather;
        let surface_map = &self.surface_map;
        let Self {
            physics, vehicles, ..
        } = self;

        for vehicle in vehicles.iter_mut() {
            let linear = {
                let body = physics.body(vehicle.body)?;
                j2p(body.linear_velocity())
            };

            let relative_air = linear - weather.wind;
            let air_speed = relative_air.length();
            let drag_factor = 0.5
                * weather.air_density_kg_m3
                * vehicle.spec.drag_coefficient.max(0.0)
                * vehicle.spec.frontal_area_m2.max(0.0)
                * vehicle.tuning.drag_scale.max(0.0);
            let drag_force = -relative_air * drag_factor * air_speed;

            let downforce = 0.5
                * weather.air_density_kg_m3
                * vehicle.tuning.downforce_coefficient.max(0.0)
                * vehicle.spec.frontal_area_m2.max(0.0)
                * air_speed
                * air_speed;

            let rolling = vehicle.rolling_resistance(&weather, surface_map);
            let horizontal_velocity = Vec3::new(linear.x, 0.0, linear.z);
            let rolling_force = if horizontal_velocity.length() > 0.1 {
                -horizontal_velocity.normalize() * rolling * vehicle.spec.mass_kg * 9.81
            } else {
                Vec3::ZERO
            };

            let mut body = physics.body_mut(vehicle.body)?;
            if drag_force.length_squared() > 0.0 {
                // Drag acts a little above the centre of mass, so it adds a
                // small pitch moment instead of being pure translation.
                let position = body.position();
                let point = RVec3::new(position.x, position.y + 0.2, position.z);
                body.add_force_at_point(p2j(drag_force), point)?;
            }
            if downforce > 0.0 {
                body.add_force(p2j(Vec3::NEG_Y * downforce))?;
            }
            if rolling_force.length_squared() > 0.0 {
                body.add_force(p2j(rolling_force))?;
            }
        }
        Ok(())
    }

    fn update_snapshots(&mut self, dt: f32) -> WorldResult<()> {
        let Self {
            physics,
            vehicles,
            surface_map,
            ..
        } = self;
        for vehicle in vehicles.iter_mut() {
            vehicle.update_snapshot(physics, dt, surface_map)?;
        }
        Ok(())
    }

    fn record_impacts(&mut self, events: &WorldEvents) {
        for event in &events.contacts {
            let ContactEvent::Added {
                manifold, estimate, ..
            } = event
            else {
                continue;
            };

            let a = self.entity(manifold.pair.body1);
            let b = self.entity(manifold.pair.body2);
            if a.vehicle().is_none() && b.vehicle().is_none() {
                continue;
            }

            let (normal_impulse_ns, friction_impulse_ns) = estimate
                .as_ref()
                .map(|estimate| {
                    let normal = estimate
                        .contact_impulses
                        .iter()
                        .copied()
                        .filter(|impulse| impulse.is_finite())
                        .sum::<f32>();
                    let friction = (estimate.friction_impulse1 * estimate.friction_impulse1
                        + estimate.friction_impulse2 * estimate.friction_impulse2)
                        .sqrt();
                    (normal, friction)
                })
                .unwrap_or((0.0, 0.0));

            if normal_impulse_ns < 0.5 && friction_impulse_ns < 0.5 {
                continue;
            }

            let mass_a = self
                .physics
                .body(manifold.pair.body1)
                .ok()
                .and_then(|body| body.mass());
            let mass_b = self
                .physics
                .body(manifold.pair.body2)
                .ok()
                .and_then(|body| body.mass());
            let reduced_mass = match (mass_a, mass_b) {
                (Some(a), Some(b)) if a + b > 0.0 => a * b / (a + b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                _ => 0.0,
            };
            let impact_speed_mps = if reduced_mass > 0.0 {
                normal_impulse_ns / reduced_mass
            } else {
                0.0
            };
            let severity = ((impact_speed_mps - 0.5) / 12.0).clamp(0.0, 1.0);

            let position = if manifold.points.is_empty() {
                Vec3::ZERO
            } else {
                let sum = manifold
                    .points
                    .iter()
                    .fold(Vec3::ZERO, |sum, point| sum + j2p_r(point.point_on1));
                sum / manifold.points.len() as f32
            };

            self.impacts.push(ImpactEvent {
                a,
                b,
                position,
                normal: j2p(manifold.normal),
                normal_impulse_ns,
                friction_impulse_ns,
                impact_speed_mps,
                severity,
            });
        }
    }

    fn entity(&self, body: BodyId) -> ContactEntity {
        if let Some(handle) = self.body_to_vehicle.get(&body) {
            ContactEntity::Vehicle(*handle)
        } else if let Some(id) = self.track_objects.get(&body) {
            ContactEntity::TrackObject(*id)
        } else if body == self.ground_body {
            ContactEntity::Ground
        } else {
            ContactEntity::Other(body.to_raw())
        }
    }
}

const TAG_GROUND: u64 = 0xffff_ffff_0000_0001;
const TAG_TRACK_OBJECT: u64 = 2 << 32;

fn tag_track_object(id: TrackObjectId) -> u64 {
    TAG_TRACK_OBJECT | (id.0 & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::SurfaceKind;

    fn settle(world: &mut World, steps: usize) {
        for _ in 0..steps {
            world.step_exact(1.0 / 240.0).unwrap();
        }
    }

    #[test]
    fn world_starts_with_ground_and_no_vehicles() {
        let world = World::new(SurfaceMap::new()).unwrap();
        assert_eq!(world.vehicle_count(), 0);
        assert_eq!(world.track_object_count(), 0);
        assert_eq!(world.weather(), Weather::dry());
    }

    #[test]
    fn surface_addition_locks_after_vehicle_creation() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let id = world
            .add_surface(SurfaceProperties {
                kind: SurfaceKind::Asphalt,
                dry_grip: 1.5,
                wet_grip: 1.0,
                rolling_resistance: 0.01,
                roughness: 0.1,
            })
            .unwrap();
        assert_eq!(id, SurfaceId(10));

        world
            .add_vehicle(VehicleSpec::default(), Pose::default())
            .unwrap();
        assert!(world.add_surface(SurfaceKind::Grass.properties()).is_err());
    }

    #[test]
    fn static_box_count_tracks_successful_creation() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        world
            .add_wall(Vec3::new(0.0, 0.5, -10.0), Vec3::new(5.0, 0.5, 0.2), 0.0)
            .unwrap();
        assert_eq!(world.track_object_count(), 1);
        assert!(
            world
                .add_static_box(Vec3::ZERO, Vec3::new(0.0, 0.5, 0.2), 0.0, 0.7, 0.1)
                .is_err()
        );
    }

    #[test]
    fn settling_a_car_reports_four_wheel_contacts() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(VehicleSpec::default(), Pose::default())
            .unwrap();
        settle(&mut world, 240);

        let snapshot = world.snapshot(car).unwrap();
        assert!(snapshot.wheels.iter().all(|wheel| wheel.contact));
        assert!((snapshot.position.y - 0.35).abs() < 0.08);
        assert!(snapshot.reference_position.y.abs() < 0.08);
        assert!(snapshot.wheels.iter().all(|wheel| wheel.load_n > 0.0));
    }

    #[test]
    fn throttle_moves_the_car_forward() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(VehicleSpec::default(), Pose::default())
            .unwrap();
        settle(&mut world, 120);

        world
            .set_controls(
                car,
                Controls {
                    throttle: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        for _ in 0..240 {
            world.step_exact(1.0 / 240.0).unwrap();
        }

        let snapshot = world.snapshot(car).unwrap();
        assert!(
            snapshot.forward_speed_mps > 2.0,
            "forward speed was {}",
            snapshot.forward_speed_mps
        );
        assert!(
            snapshot.position.z < -0.5,
            "car did not move along physics forward (-Z): {}",
            snapshot.position.z
        );
    }

    #[test]
    fn removing_a_vehicle_unregisters_it() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(VehicleSpec::default(), Pose::default())
            .unwrap();
        assert_eq!(world.vehicle_count(), 1);
        world.remove_vehicle(car).unwrap();
        assert_eq!(world.vehicle_count(), 0);
        assert!(world.snapshot(car).is_none());
        assert!(matches!(
            world.set_controls(car, Controls::default()),
            Err(WorldError::UnknownVehicle)
        ));
        assert!(matches!(
            world.remove_vehicle(car),
            Err(WorldError::UnknownVehicle)
        ));
    }

    #[test]
    fn unknown_vehicle_is_reported() {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        assert!(matches!(
            world.set_controls(VehicleHandle(999), Controls::default()),
            Err(WorldError::UnknownVehicle)
        ));
        assert!(world.snapshot(VehicleHandle(999)).is_none());
    }
}
