//! AC 视觉兼容层（纯 Rust，不依赖 Godot）。
//!
//! 三条主线：
//! - [`material`]：KN5 材质 → 原生材质配方，统计覆盖率；
//! - [`ext_config`]：CSP `ext_config.ini` 的声明式解析 → [`ext_config::BindingPlan`]；
//! - [`fx`]：Weather / Rain / Grass / Extra / Lighting / Particles / Brake Disc
//!   模块注册、依赖排序、质量与预算报告。
//!
//! [`report::inspect_path`] 是“开箱即用”的诊断入口：扫描一个 mod 目录，告诉
//! 你哪些材质/配置能自动适配，哪些会降级。Godot 适配在 `gdext`，CLI 在
//! `tools/render-inspect`。

pub mod ext_config;
pub mod fx;
pub mod material;
pub mod report;
pub mod warning;

pub use ext_config::{
    BindingPlan, BindingStats, ExtConfigDocument, ExtConfigKind, ExtConfigOptions,
    ExtConfigResolver, ExtConfigSection,
};
pub use fx::{
    FxCapability, FxConfig, FxContext, FxDefinition, FxInstance, FxManager, FxProfile, FxQuality,
    FxReport, FxValue,
};
pub use material::{
    MaterialCoverage, MaterialFamily, MaterialRecipe, MaterialRegistry, ResolvedMaterial,
};
pub use report::{InspectOptions, VisualReport, inspect_path};
pub use warning::{CompatWarning, WarningSeverity};
