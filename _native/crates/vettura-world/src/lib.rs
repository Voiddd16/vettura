//! Jolt-based world physics for Vettura.

mod ai;
mod backend;
mod debris;
mod frame;
mod groove;
mod surface;
mod timing;
mod track;
mod vehicle;
mod weather;
mod world;

pub use ai::{
    AiDriver, AiDriverTuning, AiPoint, AiPointExtra, AiProjection, AiSample, AiSpline, AiTraffic,
    SPAWN_CLEARANCE_M,
};
pub use backend::JoltBackend;
pub use debris::{DebrisHandle, DebrisShape, DebrisSnapshot, DebrisSpec};
pub use groove::{DEFAULT_GROOVE_CELL_M, GrooveDef, GrooveMap, GrooveSettings, parse_groove_ini};
pub use surface::{
    SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties, SurfaceSample, SurfaceShape,
    SurfaceTable,
};
pub use timing::{BACKWARD_TOLERANCE_M, DEFAULT_MAX_LATERAL_M, LapTimer, LapUpdate};
pub use track::{
    ColliderTriangle, DEFAULT_SPAWN_SPACING_M, MeshSurfaceIndex, SPAWN_CLEARANCE_HEIGHT_M,
    SPAWN_CLEARANCE_RADIUS_M, SPAWN_MAX_HEIGHT_VARIATION_M, SPAWN_MAX_LINE_HEIGHT_M,
    SPAWN_MAX_SLOPE, SPAWN_SNAP_RADIUS_M, SurfaceDef, TrackModelEntry, TrackModule, TrackSection,
    collect_collider_triangles, euler_deg_to_mat4, parse_models_ini, parse_sections_ini,
    parse_surfaces_ini,
};
pub use vehicle::{DriveInput, VehicleHandle, VehicleSnapshot, VehicleTuning, WheelSnapshot};
pub use weather::Weather;
pub use world::{ContactEntity, ImpactEvent, TrackObjectId, World, WorldError, WorldResult};
