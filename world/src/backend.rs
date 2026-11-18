//! `VehicleBackend` adapter so the Jolt world can drive the existing
//! `vettura_physics::Simulation` and the Godot `VehicleBody` node.
//!
//! The adapter owns a one-vehicle [`World`], which makes it a drop-in
//! replacement for `SimpleBackend` or `ChronoBackend`:
//!
//! ```ignore
//! let backend = JoltBackend::try_new(spec)?;
//! let mut simulation = Simulation::new(Box::new(backend));
//! ```
//!
//! For multi-car racing, use [`World`] directly instead; the adapter is the
//! single-car convenience path.

use vettura_physics::{Controls, Pose, VehicleBackend, VehicleSpec, VehicleState};

use crate::vehicle::VehicleHandle;
use crate::world::{World, WorldResult};
use vettura_track::SurfaceMap;

/// Single-vehicle Jolt backend implementing the common `VehicleBackend` trait.
pub struct JoltBackend {
    world: World,
    handle: VehicleHandle,
    spec: VehicleSpec,
    state: VehicleState,
}

impl JoltBackend {
    /// Create a backend on a flat dry-asphalt ground.
    pub fn try_new(spec: VehicleSpec) -> WorldResult<Self> {
        Self::with_surface_map(spec, SurfaceMap::new())
    }

    /// Create a backend on a custom surface map (grass, wet patches, puddles,
    /// custom asphalt, ...).
    pub fn with_surface_map(spec: VehicleSpec, surface_map: SurfaceMap) -> WorldResult<Self> {
        let mut world = World::new(surface_map)?;
        let handle = world.add_vehicle(spec.clone(), Pose::default())?;
        let mut backend = Self {
            world,
            handle,
            spec,
            state: VehicleState::default(),
        };
        backend.sync_state();
        Ok(backend)
    }

    /// Create a backend inside an existing world. The caller owns the world and
    /// is responsible for weather and track setup.
    pub fn from_world(world: World, handle: VehicleHandle) -> WorldResult<Self> {
        let spec = world
            .vehicle(handle)
            .map(|vehicle| vehicle.spec.clone())
            .ok_or(crate::WorldError::UnknownVehicle)?;
        let mut backend = Self {
            world,
            handle,
            spec,
            state: VehicleState::default(),
        };
        backend.sync_state();
        Ok(backend)
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn handle(&self) -> VehicleHandle {
        self.handle
    }

    fn sync_state(&mut self) {
        if let Some(state) = self.world.vehicle_state(self.handle) {
            self.state = state;
        }
    }
}

impl VehicleBackend for JoltBackend {
    fn spec(&self) -> &VehicleSpec {
        &self.spec
    }

    fn state(&self) -> &VehicleState {
        &self.state
    }

    fn reset(&mut self, pose: Pose) {
        if self.world.reset_vehicle(self.handle, pose).is_ok() {
            self.sync_state();
        }
    }

    fn step(&mut self, dt: f32, controls: &Controls) {
        if self.world.set_controls(self.handle, *controls).is_err() {
            return;
        }
        if self.world.step_exact(dt).is_ok() {
            self.sync_state();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_physics::Simulation;

    #[test]
    fn jolt_backend_drives_through_the_shared_simulation() {
        let backend = JoltBackend::try_new(VehicleSpec::default()).unwrap();
        let mut simulation = Simulation::new(Box::new(backend));
        simulation.reset(Pose::default());

        let controls = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        for _ in 0..600 {
            simulation.step(1.0 / 600.0, &controls);
        }

        let state = simulation.state();
        assert!(
            state.speed_mps > 0.5,
            "backend did not accelerate: {}",
            state.speed_mps
        );
        assert!(
            state.position.z < -0.1,
            "backend did not move forward: {}",
            state.position.z
        );
    }
}
