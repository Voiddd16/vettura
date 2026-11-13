//! The multi-vehicle Jolt world: stepping, surfaces, weather, track geometry,
//! and contact/impact reporting.
//!
//! A [`World`] owns exactly one Jolt `PhysicsWorld`. Vehicles are added in a
//! deterministic order; the world steps them together so car-to-car contacts
//! are resolved by Jolt's solver. Static boxes can be added as walls, barriers
//! or kerbs. Surface grip comes from a 2-D [`SurfaceMap`], not from per-triangle
//! collider materials.

use std::collections::HashMap;
use std::path::PathBuf;

use glam::{Quat, Vec3};
use oxijolt::{
    BodyId, BodySettings, ContactEvent, EventSettings, PhysicsWorld, RVec3, Shape,
    Vec3 as JoltVec3, WorldEvents, WorldSettings as JoltWorldSettings,
};
use vettura_physics::{Controls, Pose, VehicleSpec, VehicleState};

use crate::debris::{DebrisHandle, DebrisShape, DebrisSnapshot, DebrisSpec};
use crate::frame::{j2p, j2p_q, j2p_r, p2j, p2j_q, p2j_r};
use crate::groove::GrooveMap;
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
    #[error("unknown debris handle")]
    UnknownDebris,
    #[error("surfaces cannot be added after a vehicle exists; build the SurfaceMap first")]
    SurfacesLocked,
    #[error("track i/o error at {path}: {source}")]
    TrackIo {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid track data: {0}")]
    TrackFormat(String),
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
    /// A piece of rigid-body debris owned by the world.
    Debris(DebrisHandle),
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

    /// The debris handle, when this side of the contact is a debris body.
    pub fn debris(self) -> Option<DebrisHandle> {
        match self {
            Self::Debris(handle) => Some(handle),
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

/// One debris body owned by the world.
struct DebrisRuntime {
    handle: DebrisHandle,
    body: BodyId,
}

/// A deterministic, fixed-step multi-vehicle world.
pub struct World {
    physics: PhysicsWorld,
    surface_map: SurfaceMap,
    weather: Weather,
    weather_target: Option<Weather>,
    weather_rate: f32,
    /// Altitude above sea level of this track, metres. Thins the air for the
    /// aerodynamics and the engine.
    altitude_m: f32,
    vehicles: Vec<VehicleRuntime>,
    debris: Vec<DebrisRuntime>,
    body_to_vehicle: HashMap<BodyId, VehicleHandle>,
    body_to_debris: HashMap<BodyId, DebrisHandle>,
    track_objects: HashMap<BodyId, TrackObjectId>,
    next_vehicle: u64,
    next_debris: u64,
    next_track_object: u64,
    ground_body: Option<BodyId>,
    fixed_dt: f32,
    max_substeps: u32,
    accumulator: f32,
    impacts: Vec<ImpactEvent>,
    /// Rubber laid down by the cars on this world's track, when a track with
    /// `groove.ini` was installed.
    groove: Option<GrooveMap>,
}

impl World {
    /// Create a world with a flat static ground at `y = 0`.
    ///
    /// The ground is a large static box, not a plane, so its top face is exactly
    /// at `y = 0` for the wheel casts. Grip comes from `surface_map`; the ground
    /// body's own friction is 1.0 so Jolt's tyre-curve combination does not
    /// attenuate the callback.
    pub fn new(surface_map: SurfaceMap) -> WorldResult<Self> {
        Self::build(surface_map, true)
    }

    /// Create a world without the fallback ground plane.
    ///
    /// Use this when a track collider covers the drivable area and a second
    /// flat surface at `y = 0` would overlap the track (tunnels, bridges,
    /// banked sections below the origin). Miss the track and the car falls.
    pub fn without_ground(surface_map: SurfaceMap) -> WorldResult<Self> {
        Self::build(surface_map, false)
    }

    fn build(surface_map: SurfaceMap, with_ground: bool) -> WorldResult<Self> {
        // Single-threaded jobs: deterministic and avoids waking worker threads
        // from Godot's physics callback. `oxijolt` is built with
        // `cross-platform-deterministic`, which pins Jolt's FP settings.
        let jolt_settings = JoltWorldSettings::default().single_threaded();
        let mut physics = PhysicsWorld::new(jolt_settings)?;

        let ground_body = if with_ground {
            // Jolt caps a single shape's half extent at 2000 m; 1 km is enough
            // for a test track and leaves room for the collider's convex radius.
            let ground_shape = Shape::new_box(JoltVec3::new(1_000.0, 0.5, 1_000.0))?;
            let ground_settings = BodySettings::new_static()
                .position(RVec3::new(0.0, -0.5, 0.0))
                .friction(1.0)
                .restitution(0.0)
                .user_data(TAG_GROUND);
            Some(physics.create_body(&ground_shape, &ground_settings)?)
        } else {
            None
        };

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
            altitude_m: 0.0,
            vehicles: Vec::new(),
            debris: Vec::new(),
            body_to_vehicle: HashMap::new(),
            body_to_debris: HashMap::new(),
            track_objects: HashMap::new(),
            next_vehicle: 0,
            next_debris: 0,
            next_track_object: 0,
            ground_body,
            fixed_dt: 1.0 / 240.0,
            max_substeps: 8,
            accumulator: 0.0,
            impacts: Vec::new(),
            groove: None,
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
        runtime.set_weather_state(&self.weather);
        // The car starts at the world's ambient: a car added to a winter
        // session begins with cold tyres, cold brakes and a cold engine.
        runtime.initialize_thermals(&self.weather);
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

    /// Add one rigid body of debris (bodywork, a loose wheel, a cone, ...).
    ///
    /// The body is dynamic and collides with the track, walls, other debris and
    /// every vehicle. The world only simulates it; deciding when debris should
    /// be cleaned up or when it warrants a safety car is race logic and stays
    /// outside this layer.
    pub fn spawn_debris(&mut self, spec: DebrisSpec) -> WorldResult<DebrisHandle> {
        if !spec.position.is_finite() {
            return Err(WorldError::InvalidInput(
                "debris position must be finite".to_owned(),
            ));
        }
        let spec = spec.clamped();
        let orientation =
            if spec.orientation.is_finite() && spec.orientation.length_squared() > 1.0e-8 {
                spec.orientation.normalize()
            } else {
                Quat::IDENTITY
            };
        let shape = match spec.shape {
            // A half extent is a length, not a direction: keep the magnitudes
            // and let the frame conversion through Jolt's axis swap alone.
            DebrisShape::Box { half_extents } => {
                let half = half_extents.abs();
                Shape::new_box(JoltVec3::new(half.x, half.y, half.z))?
            }
            DebrisShape::Sphere { radius } => Shape::new_sphere(radius)?,
            DebrisShape::Cylinder {
                radius,
                half_height,
            } => Shape::new_cylinder(half_height, radius)?,
        };
        let settings = BodySettings::new_dynamic()
            .position(p2j_r(spec.position))
            .rotation(p2j_q(orientation))
            .mass(spec.mass_kg)
            .friction(spec.friction)
            .restitution(spec.restitution)
            .linear_velocity(p2j(spec.linear_velocity))
            .angular_velocity(p2j(spec.angular_velocity))
            .linear_damping(spec.linear_damping)
            .angular_damping(spec.angular_damping);
        let body = self.physics.create_body(&shape, &settings)?;
        let handle = DebrisHandle(self.next_debris);
        self.next_debris += 1;
        self.body_to_debris.insert(body, handle);
        self.debris.push(DebrisRuntime { handle, body });
        Ok(handle)
    }

    /// Remove one piece of debris and its Jolt body.
    pub fn remove_debris(&mut self, handle: DebrisHandle) -> WorldResult<()> {
        let index = self
            .debris
            .iter()
            .position(|debris| debris.handle == handle)
            .ok_or(WorldError::UnknownDebris)?;
        let runtime = self.debris.remove(index);
        self.body_to_debris.remove(&runtime.body);
        self.physics.remove_body(runtime.body)?;
        Ok(())
    }

    /// Remove every piece of debris.
    pub fn clear_debris(&mut self) -> WorldResult<()> {
        for runtime in self.debris.drain(..) {
            self.body_to_debris.remove(&runtime.body);
            self.physics.remove_body(runtime.body)?;
        }
        Ok(())
    }

    /// Number of pieces of debris currently simulated.
    pub fn debris_count(&self) -> usize {
        self.debris.len()
    }

    /// Handles of every piece of debris, in spawn order.
    pub fn debris_handles(&self) -> impl Iterator<Item = DebrisHandle> + '_ {
        self.debris.iter().map(|debris| debris.handle)
    }

    /// Pose and velocities of one piece of debris.
    pub fn debris_snapshot(&self, handle: DebrisHandle) -> Option<DebrisSnapshot> {
        let runtime = self.debris.iter().find(|debris| debris.handle == handle)?;
        let body = self.physics.body(runtime.body).ok()?;
        Some(DebrisSnapshot {
            position: j2p_r(body.position()),
            orientation: j2p_q(body.rotation()),
            linear_velocity: j2p(body.linear_velocity()),
            angular_velocity: j2p(body.angular_velocity()),
            mass_kg: body.mass().unwrap_or(0.0),
            sleeping: body.is_sleeping(),
        })
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

    /// Replace the surface map. Only valid before the first vehicle is added:
    /// vehicles keep an immutable snapshot of the surface table.
    pub fn set_surface_map(&mut self, surface_map: SurfaceMap) -> WorldResult<()> {
        if !self.vehicles.is_empty() {
            return Err(WorldError::SurfacesLocked);
        }
        self.surface_map = surface_map;
        Ok(())
    }

    /// Add a static triangle mesh (track collider, kerbs, terrain, ...).
    ///
    /// Large meshes are split into deterministic chunks so every Jolt shape
    /// stays inside its 2000 m local extent; all chunks share one
    /// [`TrackObjectId`], so impact events still identify the track as a whole.
    /// The triangles are expected in the physics frame.
    pub fn add_static_mesh(
        &mut self,
        vertices: &[Vec3],
        triangles: &[[u32; 3]],
        friction: f32,
        restitution: f32,
    ) -> WorldResult<TrackObjectId> {
        if vertices.is_empty() || triangles.is_empty() {
            return Err(WorldError::InvalidInput(
                "a static mesh needs vertices and triangles".to_owned(),
            ));
        }
        if vertices.iter().any(|vertex| !vertex.is_finite()) {
            return Err(WorldError::InvalidInput(
                "static mesh vertices must be finite".to_owned(),
            ));
        }
        if triangles
            .iter()
            .flatten()
            .any(|&index| index as usize >= vertices.len())
        {
            return Err(WorldError::InvalidInput(
                "static mesh triangle index is out of range".to_owned(),
            ));
        }

        let friction = if friction.is_finite() {
            friction.clamp(0.0, 10.0)
        } else {
            0.9
        };
        let restitution = if restitution.is_finite() {
            restitution.clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Chunk by triangle centroid. 800 m tiles keep local coordinates well
        // inside Jolt's 2000 m shape extent even for very large tracks.
        const CHUNK_SIZE: f32 = 800.0;
        let mut chunks: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, triangle) in triangles.iter().enumerate() {
            let centroid = (vertices[triangle[0] as usize]
                + vertices[triangle[1] as usize]
                + vertices[triangle[2] as usize])
                / 3.0;
            let key = (
                (centroid.x / CHUNK_SIZE).floor() as i32,
                (centroid.z / CHUNK_SIZE).floor() as i32,
            );
            chunks.entry(key).or_default().push(index);
        }

        let id = TrackObjectId(self.next_track_object);
        let mut created: Vec<BodyId> = Vec::with_capacity(chunks.len());

        // HashMap iteration order is randomized per process. Sort the chunk
        // keys so body creation order (and therefore the deterministic replay)
        // does not depend on the hasher seed.
        let mut chunk_keys: Vec<(i32, i32)> = chunks.keys().copied().collect();
        chunk_keys.sort_unstable();
        for key in chunk_keys {
            let chunk_triangles = &chunks[&key];
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for &triangle_index in chunk_triangles {
                for &vertex_index in &triangles[triangle_index] {
                    let vertex = vertices[vertex_index as usize];
                    min = min.min(vertex);
                    max = max.max(vertex);
                }
            }
            let center = (min + max) * 0.5;
            let center = Vec3::new(
                if center.x.is_finite() { center.x } else { 0.0 },
                if center.y.is_finite() { center.y } else { 0.0 },
                if center.z.is_finite() { center.z } else { 0.0 },
            );

            let mut remap: HashMap<u32, u32> = HashMap::new();
            let mut local_vertices: Vec<JoltVec3> = Vec::new();
            let mut local_triangles: Vec<[u32; 3]> = Vec::with_capacity(chunk_triangles.len());
            for &triangle_index in chunk_triangles {
                let mut local = [0u32; 3];
                for (slot, &global) in local.iter_mut().zip(&triangles[triangle_index]) {
                    *slot = *remap.entry(global).or_insert_with(|| {
                        let index = local_vertices.len() as u32;
                        local_vertices.push(p2j(vertices[global as usize] - center));
                        index
                    });
                }
                if local[0] != local[1] && local[1] != local[2] && local[2] != local[0] {
                    local_triangles.push(local);
                }
            }
            if local_triangles.is_empty() {
                continue;
            }

            let (shape, dropped) = Shape::new_mesh(&local_vertices, &local_triangles)?;
            if !dropped.is_empty() {
                tracing::debug!(
                    dropped = dropped.count(),
                    area = dropped.area(),
                    "Jolt dropped degenerate static-mesh triangles"
                );
            }
            let settings = BodySettings::new_static()
                .position(p2j_r(center))
                .friction(friction)
                .restitution(restitution)
                .user_data(tag_track_object(id));
            match self.physics.create_body(&shape, &settings) {
                Ok(body) => created.push(body),
                Err(error) => {
                    for body in created {
                        let _ = self.physics.remove_body(body);
                    }
                    return Err(error.into());
                }
            }
        }

        if created.is_empty() {
            return Err(WorldError::InvalidInput(
                "static mesh has no collidable triangles".to_owned(),
            ));
        }
        self.next_track_object += 1;
        for body in created {
            self.track_objects.insert(body, id);
        }
        Ok(id)
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
        // The road soaks up the sun (or sheds it in the rain), and every part
        // of the tyre and brake model reads its temperature. This comes first
        // for that reason.
        self.advance_track_temperature(dt);
        {
            let Self {
                physics, vehicles, ..
            } = self;
            for vehicle in vehicles.iter_mut() {
                vehicle.prepare_powertrain(dt);
            }
            for vehicle in vehicles.iter_mut() {
                vehicle.update_boost(physics, dt)?;
            }
        }
        self.apply_hybrid_forces()?;
        self.apply_external_forces(dt)?;
        let report = self.physics.step(dt)?;
        if !report.is_complete() {
            tracing::warn!("Jolt step dropped contacts; the world may need a smaller fixed step");
        }
        let events = self.physics.take_events();
        self.update_snapshots(dt)?;
        // Discs heat from the work just done; fuel burns. Both read the fresh
        // snapshot, so they run after it.
        let weather = self.weather;
        for vehicle in self.vehicles.iter_mut() {
            let reference = self.physics.body(vehicle.body)?;
            let speed = j2p(reference.linear_velocity()).length();
            vehicle.note_acceleration(dt);
            vehicle.set_weather_state(&weather);
            vehicle.update_thermals_and_fuel(&self.physics, dt, speed)?;
        }
        // Rubber is deposited where the wheels are, and the grip multiplier
        // for the next step is read back from the same cells.
        if self.groove.is_some() {
            let mut groove = self.groove.take().expect("groove present");
            for vehicle in self.vehicles.iter_mut() {
                vehicle.apply_groove(Some(&mut groove), dt);
            }
            self.groove = Some(groove);
        } else {
            for vehicle in self.vehicles.iter_mut() {
                vehicle.apply_groove(None, dt);
            }
        }
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
        for vehicle in &mut self.vehicles {
            vehicle.set_weather(&self.weather);
            vehicle.set_weather_state(&self.weather);
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

    /// Refuel to a fraction of the tank, `0..=1`. Used by pit stops.
    pub fn set_fuel_fraction(&mut self, handle: VehicleHandle, fraction: f32) -> WorldResult<()> {
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.set_fuel_fraction(fraction);
        Ok(())
    }

    /// Remaining fuel of a vehicle as a fraction of its tank, `0..=1`.
    pub fn fuel_fraction(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::fuel_fraction)
    }

    /// Remaining fuel mass of a vehicle, kg.
    pub fn fuel_kg(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::fuel_kg)
    }

    /// Brake disc temperatures of a vehicle, one per wheel, °C.
    pub fn brake_temperatures_c(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::brake_temperatures_c)
    }

    /// Install a track-evolution map, usually from `TrackModule::add_to_world`.
    ///
    /// Replaces any existing map. Rubber from the old map is lost, so this is
    /// a session start, not a mid-race change.
    pub fn set_groove_map(&mut self, groove: GrooveMap) {
        self.groove = Some(groove);
    }

    /// Whether this world tracks rubber build-up.
    pub fn has_groove(&self) -> bool {
        self.groove.is_some()
    }

    /// The rubber map, when the track has one.
    pub fn groove_map(&self) -> Option<&GrooveMap> {
        self.groove.as_ref()
    }

    /// Mutable access to the rubber map, for scripts that want to lay a line
    /// (grid starts, practice programs, session reset).
    pub fn groove_map_mut(&mut self) -> Option<&mut GrooveMap> {
        self.groove.as_mut()
    }

    /// Rubber fraction at a world position, `0..=1`.
    pub fn groove_progress_at(&self, position: Vec3) -> f32 {
        self.groove
            .as_ref()
            .map_or(0.0, |groove| groove.progress(position))
    }

    /// Grip multiplier from rubber at a world position.
    pub fn groove_grip_at(&self, position: Vec3) -> f32 {
        self.groove
            .as_ref()
            .map_or(1.0, |groove| groove.grip_at(position))
    }

    /// Tyre carcass temperatures of a vehicle, one per wheel, °C.
    pub fn tyre_temperatures_c(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::tyre_temperatures_c)
    }

    /// Tyre wear of a vehicle, one per wheel, `0..=1`.
    pub fn tyre_wear(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::tyre_wear)
    }

    /// Fit fresh tyres: ambient temperature and zero wear, as after a stop.
    pub fn reset_tyres(&mut self, handle: VehicleHandle) -> WorldResult<()> {
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.reset_tyres();
        Ok(())
    }

    /// Road undulation height at a point travelled along `heading`, metres.
    ///
    /// The value comes from the surface under the point (AC's `SIN_HEIGHT` /
    /// `SIN_LENGTH`), so a racing line over smooth asphalt reads zero while a
    /// kerb or a gravel run-off area reads centimetres.
    pub fn road_bump_m(&self, position: Vec3, heading: Vec3) -> f32 {
        let sample = self.surface_map.sample(position);
        let Some(properties) = self.surface_map.table().get(sample.surface) else {
            return 0.0;
        };
        properties.bump_m(position, heading)
    }

    /// Water film carried by each tyre of a vehicle, mm.
    pub fn water_film_mm(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::water_film_mm)
    }

    /// Current turbo boost pressure of a vehicle, bar.
    pub fn turbo_boost_bar(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::turbo_boost_bar)
    }

    /// Extra suspension force per wheel from the last step, newtons.
    ///
    /// This is the progressive spring, the bump/rebound stops and any active
    /// actuator; Jolt's own linear spring force is not included.
    pub fn suspension_force_n(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::suspension_force_n)
    }

    /// Active actuator force per wheel from the last step, newtons.
    pub fn active_suspension_force_n(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::active_suspension_force_n)
    }

    /// Heave-spring force per axle (front, rear) from the last step, newtons.
    pub fn heave_force_n(&self, handle: VehicleHandle) -> Option<[f32; 2]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::heave_force_n)
    }

    /// Flat-spot fraction per wheel, `0..=1`. A locked wheel burns its rubber
    /// into a flat, which costs that corner grip and shakes the chassis until
    /// the rubber has worn away again.
    pub fn flat_spot(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::flat_spot)
    }

    /// Chassis kick amplitude per wheel from the flat spots, newtons.
    pub fn flat_spot_force_n(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::flat_spot_force_n)
    }

    /// Change the active suspension mode at runtime (driver switch or setup).
    pub fn set_active_suspension_mode(
        &mut self,
        handle: VehicleHandle,
        mode: vettura_physics::ActiveSuspensionMode,
    ) -> WorldResult<()> {
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.set_active_suspension_mode(mode);
        Ok(())
    }

    /// Brake disc core temperatures of a vehicle, one per wheel, °C.
    pub fn brake_core_temperatures_c(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::brake_core_temperatures_c)
    }

    /// Tyre friction-surface temperatures of a vehicle, one per wheel, °C.
    pub fn tyre_surface_temperatures_c(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::tyre_surface_temperatures_c)
    }

    /// Tyre bulk-core temperatures of a vehicle, one per wheel, °C.
    pub fn tyre_core_temperatures_c(&self, handle: VehicleHandle) -> Option<[f32; 4]> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::tyre_core_temperatures_c)
    }

    /// Engine temperatures of a vehicle: coolant and oil, °C.
    pub fn engine_temperatures_c(&self, handle: VehicleHandle) -> Option<(f32, f32)> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::engine_temperatures_c)
    }

    /// Hybrid battery charge of a vehicle, joules.
    pub fn ers_energy_j(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::ers_energy_j)
    }

    /// Hybrid battery state of charge of a vehicle, `0..=1`.
    pub fn ers_soc(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::ers_soc)
    }

    /// Hybrid energy deployed by a vehicle on the current lap, joules.
    pub fn ers_lap_deploy_j(&self, handle: VehicleHandle) -> Option<f32> {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.handle == handle)
            .map(VehicleRuntime::ers_lap_deploy_j)
    }

    /// Reset the per-lap deployment allowance. Call when the lap timer
    /// completes a lap, the same way a race resets its energy budget.
    pub fn reset_ers_lap(&mut self, handle: VehicleHandle) -> WorldResult<()> {
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        vehicle.reset_ers_lap();
        Ok(())
    }

    /// Charge the hybrid battery to a fraction of its capacity, `0..=1`.
    pub fn set_ers_soc(&mut self, handle: VehicleHandle, soc: f32) -> WorldResult<()> {
        if !soc.is_finite() {
            return Err(WorldError::InvalidInput(
                "hybrid state of charge must be finite".to_owned(),
            ));
        }
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.handle == handle)
            .ok_or(WorldError::UnknownVehicle)?;
        let fraction = soc.clamp(0.0, 1.0);
        vehicle.set_ers_energy_j(vehicle.spec.ers.capacity_j.max(0.0) * fraction);
        Ok(())
    }

    /// Clear the rubber, as if the track had just been resurfaced.
    pub fn reset_groove(&mut self) {
        if let Some(groove) = &mut self.groove {
            groove.reset();
        }
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

    /// Number of logical track objects. A large static mesh may be split into
    /// several Jolt bodies internally, but all chunks share one id.
    pub fn track_object_count(&self) -> usize {
        let mut ids: Vec<TrackObjectId> = self.track_objects.values().copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
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

    /// Heat (or cool) the road surface toward the sun equilibrium.
    ///
    /// The road is a large thermal mass: it takes minutes of sunshine to heat
    /// through and a downpour to cool back, which is why the racing line
    /// changes over a session and not over a lap. The evolution is driven by
    /// `dt` and the weather only, so replays stay deterministic.
    fn advance_track_temperature(&mut self, dt: f32) {
        let tau = self.weather.track_time_constant_s.max(1.0);
        let target = self.weather.sun_equilibrium_temperature_c();
        if dt.is_finite() && dt > 0.0 {
            let alpha = 1.0 - (-dt / tau).exp();
            let next = self.weather.track_temperature_c
                + (target - self.weather.track_temperature_c) * alpha;
            self.weather.track_temperature_c = next.clamp(-60.0, 120.0);
            // The grip callbacks cache the weather they were last given, so
            // publish the evolved road temperature before the next step reads
            // it. Transitions already do this in `advance_weather`.
            for vehicle in &self.vehicles {
                vehicle.set_weather(&self.weather);
            }
        }
    }

    /// Altitude above sea level this track sits at, metres.
    pub fn altitude_m(&self) -> f32 {
        self.altitude_m
    }

    /// Set the track's altitude. Thins the air for aero and engine, and the
    /// density change is instantaneous because it is a location, not a state.
    pub fn set_altitude_m(&mut self, altitude_m: f32) {
        if altitude_m.is_finite() && altitude_m >= 0.0 {
            self.altitude_m = altitude_m.clamp(0.0, 10_000.0);
        }
    }

    /// Air density at this track's altitude, kg/m³.
    pub fn air_density_kg_m3(&self) -> f32 {
        // ISA scale height, ~8.5 km.
        let density = self.weather.air_density_kg_m3.max(0.0) * (-self.altitude_m / 8_500.0).exp();
        density.clamp(0.02, 5.0)
    }

    /// Road-surface temperature the world has evolved to, °C.
    pub fn track_temperature_c(&self) -> f32 {
        self.weather.track_temperature_c
    }

    /// Apply the hybrid machine force to every car for the upcoming step.
    ///
    /// Deploy pushes the car forward along its heading; harvest drags on it.
    /// Both act at the driven axle, so the torque split follows the layout.
    fn apply_hybrid_forces(&mut self) -> WorldResult<()> {
        let dt = self.fixed_dt;
        for vehicle in self.vehicles.iter_mut() {
            let power_w = vehicle.apply_ers(dt);
            if power_w.abs() <= 1.0e-3 {
                continue;
            }
            // P = F·v, so the force this power buys at the current speed.
            let speed = vehicle.snapshot.forward_speed_mps.abs().max(0.5);
            let force = power_w / speed;
            if !force.is_finite() {
                continue;
            }
            let forward = vehicle.snapshot.orientation * Vec3::NEG_Z;
            let mut body = self.physics.body_mut(vehicle.body)?;
            body.add_force(p2j(forward * force))?;
        }
        Ok(())
    }

    fn apply_external_forces(&mut self, dt: f32) -> WorldResult<()> {
        let weather = self.weather;
        let surface_map = &self.surface_map;
        // Dirty air: each car works out how far behind the car ahead it is, in
        // units of the wake's reach, so its downforce can be cut while it sits
        // in someone's wake. This is per-car physics, not a race rule.
        // The altitude-scaled density is read before the borrow is split.
        let altitude_density = self.air_density_kg_m3();
        let Self {
            physics, vehicles, ..
        } = self;
        let exposures = &dirty_air_exposures(vehicles)[..];

        for (vehicle, _drag_exposure) in vehicles.iter_mut().zip(exposures) {
            let linear = {
                let body = physics.body(vehicle.body)?;
                j2p(body.linear_velocity())
            };

            let relative_air = linear - weather.wind;
            let air_speed = relative_air.length();
            // Drag comes from the car's own wing tables when it has them, and
            // from the aggregate frontal area otherwise. DRS opens the rear
            // wing and cuts drag; the spec decides whether the car has one and
            // by how much.
            let wing_area = vehicle.wing_drag_area();
            let drag_area = if wing_area > 0.0 {
                wing_area
            } else {
                vehicle.spec.drag_coefficient.max(0.0) * vehicle.spec.frontal_area_m2.max(0.0)
            };
            // Altitude thins the air for the drag as well.
            let drag_factor = 0.5
                * altitude_density
                * drag_area
                * vehicle.tuning.drag_scale.max(0.0)
                * vehicle.drag_multiplier();
            let drag_force = -relative_air * drag_factor * air_speed;

            let rolling = vehicle.rolling_resistance_force_n(&weather, surface_map);
            let horizontal_velocity = Vec3::new(linear.x, 0.0, linear.z);
            let rolling_force = if horizontal_velocity.length() > 0.1 && rolling > 0.0 {
                -horizontal_velocity.normalize() * rolling
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
            if rolling_force.length_squared() > 0.0 {
                body.add_force(p2j(rolling_force))?;
            }
        }

        // Aero (per-axle downforce with ride-height fade) and the fuel load
        // inertia need the snapshot, so they are applied per vehicle.
        for (vehicle, exposure) in vehicles.iter_mut().zip(exposures) {
            let linear = vehicle.snapshot.linear_velocity;
            let relative_air = linear - weather.wind;
            vehicle.apply_aero_forces(
                physics,
                &weather,
                linear,
                relative_air,
                *exposure,
                altitude_density,
            )?;
            vehicle.apply_suspension_forces(physics, dt)?;
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
        } else if let Some(handle) = self.body_to_debris.get(&body) {
            ContactEntity::Debris(*handle)
        } else if let Some(id) = self.track_objects.get(&body) {
            ContactEntity::TrackObject(*id)
        } else if self.ground_body == Some(body) {
            ContactEntity::Ground
        } else {
            ContactEntity::Other(body.to_raw())
        }
    }
}

const TAG_GROUND: u64 = 0xffff_ffff_0000_0001;
const TAG_TRACK_OBJECT: u64 = 2 << 32;

/// How far each car is behind the car ahead, as `0..=1` of the wake's reach.
///
/// Only a car travelling roughly the same way and ahead within the wake's
/// reach counts: a car going the other way is not "ahead", and one more than a
/// car length clear is out of the wake entirely. The falloff is quadratic
/// because the wake is deepest right behind the car and thins fast.
fn dirty_air_exposures(vehicles: &[VehicleRuntime]) -> Vec<f32> {
    let mut exposures = vec![0.0f32; vehicles.len()];
    for (index, vehicle) in vehicles.iter().enumerate() {
        let spec = &vehicle.spec;
        if spec.aero.dirty_air_loss <= 0.0 {
            continue;
        }
        let reach = spec.aero.dirty_air_distance_m.max(1.0);
        let mine = vehicle.snapshot.position;
        let forward = vehicle.snapshot.orientation * Vec3::NEG_Z;
        let mut best = f32::INFINITY;
        for (other_index, other) in vehicles.iter().enumerate() {
            if other_index == index {
                continue;
            }
            // Only a car heading roughly the same way leaves a usable wake;
            // one going the other way is not "ahead" even if it is in front.
            let other_forward = other.snapshot.orientation * Vec3::NEG_Z;
            if other_forward.dot(forward) < 0.5 {
                continue;
            }
            let to_other = other.snapshot.position - mine;
            let ahead = to_other.dot(forward);
            if ahead <= 0.5 {
                continue; // level or behind: not a wake source
            }
            let offset = (to_other - forward * ahead).length().max(0.0);
            // A car alongside me is not in my wake; a car in front is.
            let distance = (ahead + offset * 2.0).min(reach * 4.0);
            best = best.min(distance);
        }
        if best.is_finite() {
            let exposure = (1.0 - best / reach).clamp(0.0, 1.0);
            exposures[index] = exposure * exposure;
        }
    }
    exposures
}

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
                bump_height_m: 0.0,
                bump_length_m: 10.0,
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
    fn wide_static_mesh_uses_one_logical_track_object() {
        let vertices = [
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(999.0, 0.0, -1.0),
            Vec3::new(1001.0, 0.0, -1.0),
            Vec3::new(1001.0, 0.0, 1.0),
            Vec3::new(999.0, 0.0, 1.0),
        ];
        let triangles = [[0, 2, 1], [0, 3, 2], [4, 6, 5], [4, 7, 6]];

        let mut world = World::without_ground(SurfaceMap::new()).unwrap();
        let id = world
            .add_static_mesh(&vertices, &triangles, 1.0, 0.0)
            .unwrap();
        assert_eq!(id.raw(), 0);
        assert!(
            world.track_objects.len() >= 2,
            "expected the mesh to be split into chunks"
        );
        assert_eq!(world.track_object_count(), 1, "chunks share one logical id");
        assert!(world.add_static_mesh(&[], &[], 1.0, 0.0).is_err());
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

    #[test]
    fn only_a_car_heading_the_same_way_leaves_a_wake() {
        let mut spec = VehicleSpec::default();
        spec.aero.dirty_air_loss = 0.3;
        spec.aero.dirty_air_distance_m = 20.0;
        // The follower sits at the origin; the other car is 10 m ahead along
        // the follower's forward axis (-Z).
        let exposure_for = |orientation: Quat| {
            let mut world = World::new(SurfaceMap::new()).unwrap();
            world.add_vehicle(spec.clone(), Pose::default()).unwrap();
            world
                .add_vehicle(
                    spec.clone(),
                    Pose {
                        position: Vec3::new(0.0, 0.0, -10.0),
                        orientation,
                    },
                )
                .unwrap();
            dirty_air_exposures(&world.vehicles)[0]
        };
        assert!(exposure_for(Quat::IDENTITY) > 0.0);
        assert_eq!(
            exposure_for(Quat::from_rotation_y(std::f32::consts::PI)),
            0.0
        );
    }
}
