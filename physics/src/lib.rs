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
pub mod coords;
pub mod simulation;
pub mod spec;
pub mod state;
pub mod tyre_model;
pub mod units;

pub use backend::VehicleBackend;
// 全工作区共享的坐标系/单位约定（原 `vettura-core`）。
#[cfg(feature = "chrono")]
pub use backends::{ChronoBackend, ChronoError};
pub use controls::{Controls, ShiftBlip};
pub use coords::{
    AC_FORWARD, AC_LEFT, AC_UP, GODOT_FORWARD, GODOT_RIGHT, GODOT_UP, ac_to_godot, godot_to_ac,
};
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
pub use units::GRAVITY;
