//! Assetto Corsa content resolution.
//!
//! This is the glue layer between raw AC files and the rest of Vettura:
//!
//! * find the car's data source (`data.acd` or an extracted `data/` directory)
//! * expose INI/LUT files by name
//! * map AC physics data onto [`vettura_physics::VehicleSpec`]
//! * load the car model (`*.kn5`) from the car folder
//! * load a track directory ([`TrackModule`]: `models*.ini` / `surfaces.ini` /
//!   collision KN5 → collision mesh, surfaces, spawn grid, AI line)
//!
//! Rendering is intentionally absent. The Godot crate consumes the parsed
//! KN5 and turns it into engine resources.

pub mod car;
pub mod error;
pub mod physics_mapping;
pub mod track;

pub use car::{CarData, CarFolder, DataSource};
pub use error::{AssetError, Result};
pub use track::{
    DEFAULT_SPAWN_SPACING_M, SPAWN_CLEARANCE_HEIGHT_M, SPAWN_CLEARANCE_RADIUS_M,
    SPAWN_MAX_HEIGHT_VARIATION_M, SPAWN_MAX_LINE_HEIGHT_M, SPAWN_MAX_SLOPE, SPAWN_SNAP_RADIUS_M,
    SurfaceDef, TrackLoadError, TrackLoadResult, TrackModelEntry, TrackModule,
    collect_collider_triangles, euler_deg_to_mat4, parse_models_ini, parse_sections_ini,
    parse_surfaces_ini,
};
