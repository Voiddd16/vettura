//! Vehicle specification types and derived data.
//!
//! This module is the single source of truth for every parameter a physics
//! backend needs.  All structures are plain data (`Clone`, `Serialize`,
//! `Deserialize`) so they can cross the FFI boundary into Godot and be saved
//! as resources.

pub mod aero;
pub mod brakes;
pub mod constants;
pub mod drivetrain;
pub mod drs;
pub mod engine;
pub mod ers;
pub mod fuel;
pub mod pose;
pub mod suspension;
pub mod tyre;
pub mod tyre_thermal;
pub mod tyre_wear;
pub mod vehicle_spec;
pub mod wheel;

pub use aero::{AeroSpec, WingSpec};
pub use brakes::BrakeSpec;
pub use constants::{
    AERO_REFERENCE_SPEED_MPS, AIR_DENSITY_KG_M3, DEFAULT_LATERAL_CURVE,
    DEFAULT_LATERAL_PEAK_SLIP_DEG, DEFAULT_LONGITUDINAL_CURVE, FrictionTable, series_spring_rate,
};
pub use drivetrain::{DiffSpec, DrivetrainLayout, DrivetrainSpec, GearboxSpec};
pub use drs::DrsSpec;
pub use engine::{EngineSpec, EngineThermalSpec, TorquePoint, TurboSpec};
pub use ers::{ErsMode, ErsSpec};
pub use fuel::FuelSpec;
pub use pose::Pose;
pub use suspension::{ActiveSuspensionMode, ActiveSuspensionSpec, HeaveSpringSpec, SuspensionSpec};
pub use tyre::TyreSpec;
pub use tyre_thermal::TyreThermalSpec;
pub use tyre_wear::TyreWearSpec;
pub use vehicle_spec::{VehicleSpec, WheelSpec};
pub use wheel::Wheel;
