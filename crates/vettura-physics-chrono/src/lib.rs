//! Chrono::Vehicle adapter (placeholder).
//!
//! # Why this crate exists separately
//!
//! Chrono is a large C++ project with its own CMake build. Keeping the adapter
//! in its own crate means the rest of the workspace stays a normal, fast Rust
//! build. When the adapter is enabled, this crate will:
//!
//! 1. build/link the Chrono core and `Chrono::Vehicle` modules;
//! 2. expose a small `cxx` bridge (`Vehicle`, `RigidTerrain`, `DriverInputs`);
//! 3. implement [`vettura_physics::VehicleBackend`] on top of that bridge.
//!
//! The AC → `VehicleSpec` mapping is already handled by `vettura-assets`; this
//! adapter only needs to translate `VehicleSpec` into Chrono construction
//! parameters and copy `VehicleState` back each step.
//!
//! # Current status
//!
//! Not linked yet. [`ChronoVehicle::try_new`] returns
//! [`ChronoError::NotLinked`] so callers can fall back to the simple backend
//! while the C++ bridge is being developed.

use thiserror::Error;
use vettura_physics::VehicleSpec;

#[derive(Debug, Error)]
pub enum ChronoError {
    #[error(
        "Chrono::Vehicle is not linked in this build; see crates/vettura-physics-chrono/README.md"
    )]
    NotLinked,

    #[error("invalid vehicle specification: {0}")]
    InvalidSpec(String),
}

/// Handle to a future Chrono::Vehicle instance.
#[derive(Debug)]
pub struct ChronoVehicle {
    _private: (),
}

impl ChronoVehicle {
    /// Construct a Chrono vehicle from a backend-neutral spec.
    ///
    /// The signature is stable so the rest of the workspace can code against
    /// it before the C++ bridge lands.
    pub fn try_new(spec: VehicleSpec) -> Result<Self, ChronoError> {
        let _ = spec;
        Err(ChronoError::NotLinked)
    }
}
