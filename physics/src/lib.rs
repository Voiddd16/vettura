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
pub mod tyre_model;

pub use backend::VehicleBackend;
#[cfg(feature = "chrono")]
pub use backends::{ChronoBackend, ChronoError};
pub use controls::{Controls, ShiftBlip};
pub use simulation::Simulation;
pub use spec::{
    AERO_REFERENCE_SPEED_MPS, AIR_DENSITY_KG_M3, ActiveSuspensionMode, ActiveSuspensionSpec,
    AeroSpec, BrakeSpec, DiffSpec, DrivetrainLayout, DrivetrainSpec, DrsSpec, EngineSpec,
    EngineThermalSpec, ErsMode, ErsSpec, FrictionTable, FuelSpec, GearboxSpec, HeaveSpringSpec,
    Pose, SuspensionSpec, TorquePoint, TurboSpec, TyreSpec, TyreThermalSpec, TyreWearSpec,
    VehicleSpec, Wheel, WheelSpec, WingSpec, series_spring_rate,
};
pub use state::{VehicleState, WheelState};
pub use tyre_model::SlipCurve;
