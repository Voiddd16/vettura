//! Jolt-based **world** physics for Vettura.
//!
//! The design goal is deliberately different from a full internal vehicle model:
//! this crate focuses on what a racing game needs from the car's environment:
//!
//! - **car ↔ track**: surface-dependent grip, rolling resistance, kerbs, walls and
//!   barriers;
//! - **car ↔ car**: rigid-body contact with impulse-aware impact events;
//! - **car ↔ weather**: wetness, standing water/aquaplaning, track temperature,
//!   air density, wind and aerodynamic drag/downforce.
//!
//! The mature solver is [Jolt Physics](https://github.com/jrouwe/JoltPhysics) 5.6,
//! reached through the safe [`oxijolt`] API. Jolt owns collision detection, the
//! contact solver, the wheeled-vehicle constraint and its suspension/tyre step.
//! This crate adapts Assetto Corsa-derived [`vettura_physics::VehicleSpec`] data to
//! Jolt, adds the environment model above, and exposes a fixed-step multi-car world.
//!
//! # Frame
//!
//! The public frame is the Vettura physics frame: `+X` right, `+Y` up, `-Z`
//! forward. Jolt internally uses `+X` left, `+Y` up, `+Z` forward; the conversion
//! is a 180° yaw and is confined to the crate's private `frame` module.
//!
//! # Fixed step
//!
//! [`World::step`] accumulates wall-clock frames and runs [`World::step_exact`] in
//! fixed increments. The default is 1/240 s and 8 substeps, which keeps a 60 Hz
//! frame at 4 substeps and a 30 Hz frame at 8. A deterministic replay should use
//! the same fixed step and input sequence.
//!
//! # Minimal example
//!
//! ```
//! use vettura_physics::{Pose, VehicleSpec};
//! use vettura_world::{SurfaceMap, VehicleTuning, World};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut world = World::new(SurfaceMap::new())?;
//! let car = world.add_vehicle(VehicleSpec::default(), Pose::default())?;
//!
//! // Settle on the suspension.
//! for _ in 0..240 {
//!     world.step_exact(1.0 / 240.0)?;
//! }
//! assert!(world.vehicle(car).unwrap().snapshot.wheels.iter().all(|w| w.contact));
//! # Ok(())
//! # }
//! ```
//!
//! # Scope and non-goals
//!
//! This is not a clutch/engine-internals simulator. Jolt's built-in engine,
//! automatic transmission and differentials provide a plausible drivetrain; the
//! interesting work here is the environment and multi-body interaction. The
//! interfaces are intentionally wide enough to add a custom tyre model, a track
//! mesh loader or a damage/deformation layer later.

mod backend;
mod frame;
mod surface;
mod vehicle;
mod weather;
mod world;

pub use backend::JoltBackend;
pub use surface::{
    SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties, SurfaceSample, SurfaceShape,
    SurfaceTable,
};
pub use vehicle::{DriveInput, VehicleHandle, VehicleSnapshot, VehicleTuning, WheelSnapshot};
pub use weather::Weather;
pub use world::{ContactEntity, ImpactEvent, TrackObjectId, World, WorldError, WorldResult};
