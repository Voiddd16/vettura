//! Jolt-based world physics for Vettura.

mod ai;
mod backend;
mod debris;
mod frame;
mod groove;
mod surface;
mod telemetry;
mod timing;
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
    ColliderTriangle, MeshSurfaceIndex, SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties,
    SurfaceSample, SurfaceShape, SurfaceTable, triangle_height,
};
pub use telemetry::{
    TELEMETRY_CONFIG_PATH, TelemetryConfig, TelemetryProtocol, TelemetrySample, TelemetryScope,
    TelemetrySink, TimingTelemetry, TrackTelemetry, VehicleSample, VehicleTelemetry,
};
pub use timing::{BACKWARD_TOLERANCE_M, DEFAULT_MAX_LATERAL_M, LapTimer, LapUpdate, TrackSection};
pub use vehicle::{DriveInput, VehicleHandle, VehicleSnapshot, VehicleTuning, WheelSnapshot};
pub use weather::Weather;
pub use world::{ContactEntity, ImpactEvent, TrackObjectId, World, WorldError, WorldResult};
