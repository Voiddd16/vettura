//! Backend-neutral vehicle dynamics.
//!
//! This crate intentionally contains **no Assetto Corsa parsing code**. It
//! defines the contract that a simulation backend (Chrono::Vehicle, a custom
//! tyre solver, or the simple test backend) must satisfy, plus the data
//! structures used to feed it.
//!
//! The AC-specific mapping lives in `vettura-assets`, so the physics core can
//! be tested with synthetic cars and reused for non-AC content later.

pub mod backend;
pub mod backends;
pub mod controls;
pub mod simulation;
pub mod spec;
pub mod state;

pub use backend::VehicleBackend;
pub use controls::Controls;
pub use simulation::Simulation;
pub use spec::{
    DrivetrainLayout, DrivetrainSpec, EngineSpec, Pose, SuspensionSpec, TyreSpec, VehicleSpec,
    Wheel, WheelSpec,
};
pub use state::{VehicleState, WheelState};
