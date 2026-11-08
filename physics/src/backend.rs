use crate::{Controls, Pose, VehicleSpec, VehicleState};

/// Contract implemented by every physics backend.
///
/// Backends own their internal solver state. `step` advances exactly `dt`
/// seconds; scheduling and fixed-step accumulation belong to [`crate::Simulation`].
pub trait VehicleBackend: Send {
    fn spec(&self) -> &VehicleSpec;
    fn state(&self) -> &VehicleState;
    fn reset(&mut self, pose: Pose);
    fn step(&mut self, dt: f32, controls: &Controls);
}
