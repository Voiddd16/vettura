//! 模块化 FX 框架：Weather / Rain / Grass / Extra / Lighting / Particles / Brake Disc。
//!
//! 设计目标：新增一个 FX 只需要实现 [`FxDefinition`] + [`FxInstance`] 并注册；
//! 不修改核心渲染代码。`ext_config` 只负责把声明式参数送进每个模块。

pub mod config;
pub mod context;
pub mod definition;
pub mod manager;
pub mod modules;
pub mod profile;

pub use config::{FxConfig, FxValue};
pub use context::{
    CameraState, CarVisualState, FxBudget, FxContext, FxEvent, FxQuality, WeatherState,
};
pub use definition::{
    FxCapability, FxDefinition, FxError, FxInstance, FxMeta, FxScope, FxStage, FxStatus,
};
pub use manager::{FxManager, FxReport, FxReportEntry};
pub use modules::builtin_definitions;
pub use profile::FxProfile;
