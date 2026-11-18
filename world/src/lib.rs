//! Jolt-based world physics for Vettura.

mod ai;
mod backend;
mod debris;
mod frame;
mod telemetry;
mod timing;
mod vehicle;
mod world;

pub use ai::{AiDriver, AiDriverTuning, AiTraffic, DEFAULT_AI_SKILL, MAX_AI_SKILL};
pub use backend::JoltBackend;
pub use debris::{DebrisHandle, DebrisShape, DebrisSnapshot, DebrisSpec};
pub use telemetry::{
    TELEMETRY_CONFIG_PATH, TelemetryConfig, TelemetryProtocol, TelemetrySample, TelemetryScope,
    TelemetrySink, TimingTelemetry, TrackTelemetry, VehicleSample, VehicleTelemetry,
};
pub use timing::{BACKWARD_TOLERANCE_M, DEFAULT_MAX_LATERAL_M, LapTimer, LapUpdate};
pub use vehicle::{DriveInput, VehicleHandle, VehicleSnapshot, VehicleTuning, WheelSnapshot};
pub use world::{ContactEntity, ImpactEvent, TrackObjectId, World, WorldError, WorldResult};

// 赛道/环境数据层搬到了 `vettura-track`；这里保持重导出，调用方（`assets`、
// `gdext`、测试）继续可以用 `vettura_world::SurfaceMap` 这类路径，不必为了
// 一次拆分改遍全仓。
pub use vettura_track::{
    AiPoint, AiPointExtra, AiProjection, AiSample, AiSpline, ColliderTriangle,
    DEFAULT_GROOVE_CELL_M, GrooveDef, GrooveMap, GrooveSettings, MeshSurfaceIndex,
    SPAWN_CLEARANCE_M, SPLINE_VERSION, SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties,
    SurfaceSample, SurfaceShape, SurfaceTable, TrackSection, Weather, parse_groove_ini,
    triangle_height,
};
