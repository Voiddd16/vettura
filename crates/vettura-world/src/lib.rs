//! Jolt-based world physics for Vettura.

mod backend;
mod frame;
mod surface;
mod track;
mod vehicle;
mod weather;
mod world;

pub use backend::JoltBackend;
pub use surface::{SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties, SurfaceSample, SurfaceShape, SurfaceTable};
pub use track::{TrackModelEntry, parse_models_ini, parse_surfaces_ini, SurfaceDef, collect_collider_triangles, euler_deg_to_mat4, MeshSurfaceIndex, TrackModule};
pub use vehicle::{DriveInput, VehicleHandle, VehicleSnapshot, VehicleTuning, WheelSnapshot};
pub use weather::Weather;
pub use world::{ContactEntity, ImpactEvent, TrackObjectId, World, WorldError, WorldResult};
