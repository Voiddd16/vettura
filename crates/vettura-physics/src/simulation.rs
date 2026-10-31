use crate::{Controls, Pose, VehicleBackend};

/// Fixed-timestep driver for a [`VehicleBackend`].
///
/// Godot's `_physics_process` delta is usually stable, but rendering hitches
/// and debugger pauses are not. This keeps the solver on a fixed step and
/// caps catch-up work to avoid the "spiral of death".
pub struct Simulation {
    backend: Box<dyn VehicleBackend>,
    fixed_dt: f32,
    accumulator: f32,
    max_substeps: u32,
}

impl Simulation {
    pub fn new(backend: Box<dyn VehicleBackend>) -> Self {
        Self {
            backend,
            fixed_dt: 1.0 / 600.0,
            accumulator: 0.0,
            max_substeps: 8,
        }
    }

    pub fn with_fixed_dt(mut self, fixed_dt: f32) -> Self {
        self.fixed_dt = fixed_dt.max(1.0e-5);
        self
    }

    pub fn with_max_substeps(mut self, max_substeps: u32) -> Self {
        self.max_substeps = max_substeps.max(1);
        self
    }

    /// Advance by `frame_dt` seconds. Returns the number of solver substeps.
    pub fn step(&mut self, frame_dt: f32, controls: &Controls) -> u32 {
        if !frame_dt.is_finite() || frame_dt <= 0.0 {
            return 0;
        }

        self.accumulator += frame_dt;
        let controls = controls.clamped();
        let mut steps = 0;

        while self.accumulator >= self.fixed_dt && steps < self.max_substeps {
            self.backend.step(self.fixed_dt, &controls);
            self.accumulator -= self.fixed_dt;
            steps += 1;
        }

        if steps == self.max_substeps && self.accumulator > self.fixed_dt {
            // Drop the backlog instead of running an unbounded catch-up loop.
            self.accumulator = 0.0;
        }

        steps
    }

    pub fn reset(&mut self, pose: Pose) {
        self.accumulator = 0.0;
        self.backend.reset(pose);
    }

    pub fn state(&self) -> &crate::VehicleState {
        self.backend.state()
    }

    pub fn spec(&self) -> &crate::VehicleSpec {
        self.backend.spec()
    }

    pub fn backend(&self) -> &dyn VehicleBackend {
        self.backend.as_ref()
    }

    pub fn backend_mut(&mut self) -> &mut dyn VehicleBackend {
        self.backend.as_mut()
    }
}
