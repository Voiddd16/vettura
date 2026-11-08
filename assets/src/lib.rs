//! Assetto Corsa content resolution.
//!
//! This is the glue layer between raw AC files and the rest of Vettura:
//!
//! * find the car's data source (`data.acd` or an extracted `data/` directory)
//! * expose INI/LUT files by name
//! * map AC physics data onto [`vettura_physics::VehicleSpec`]
//! * load the car model (`*.kn5`) from the car folder
//!
//! Rendering is intentionally absent. The Godot crate consumes the parsed
//! KN5 and turns it into engine resources.

pub mod car;
pub mod error;
pub mod physics_mapping;

pub use car::{CarData, CarFolder, DataSource};
pub use error::{AssetError, Result};
