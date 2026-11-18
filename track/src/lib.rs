//! 赛道/环境纯数据层：表面、橡胶进化、天气、AI 线与计时分段。
//!
//! 本 crate **不依赖 Jolt，也不解析任何 AC 文件**：`vettura-assets` 负责把
//! 车/赛道目录解析成这里的纯数据（`SurfaceMap` / `GrooveMap` / `AiSpline` /
//! `TrackSection`），`vettura-world` 再把它们装进 Jolt 世界。这样赛道数据
//! 可以被无物理的环境消费（工具、分析脚本、测试夹具），也避免 `assets`
//! 为了几种数据类型而拖进整个物理世界。

mod groove;
mod section;
mod spline;
mod surface;
mod weather;

pub use groove::{DEFAULT_GROOVE_CELL_M, GrooveDef, GrooveMap, GrooveSettings, parse_groove_ini};
pub use section::TrackSection;
pub use spline::{
    AiPoint, AiPointExtra, AiProjection, AiSample, AiSpline, SPAWN_CLEARANCE_M, SPLINE_VERSION,
};
pub use surface::{
    ColliderTriangle, MeshSurfaceIndex, SurfaceId, SurfaceKind, SurfaceMap, SurfaceProperties,
    SurfaceSample, SurfaceShape, SurfaceTable, triangle_height,
};
pub use weather::Weather;

/// 赛道纯数据解析错误（INI / AI 线文件）。
///
/// 这些错误和 Jolt 世界无关，所以不再复用 `WorldError`：`vettura-world`
/// 通过 `WorldError::Track(#[from] TrackError)` 再包一层。
#[derive(Debug, thiserror::Error)]
pub enum TrackError {
    #[error("track i/o error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid track data: {0}")]
    Format(String),
}

/// 本 crate 的结果别名。
pub type TrackResult<T> = std::result::Result<T, TrackError>;
