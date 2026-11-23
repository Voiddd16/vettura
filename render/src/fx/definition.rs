//! FX 模块的静态定义与运行时实例契约。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::config::{FxConfig, FxValue};
use super::context::{FxContext, FxCost, FxQuality};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxScope {
    Global,
    Scene,
    Track,
    Car,
    Camera,
}

impl FxScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Scene => "scene",
            Self::Track => "track",
            Self::Car => "car",
            Self::Camera => "camera",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxCapability {
    Environment,
    Render,
    Light,
    Particle,
    Material,
    Audio,
}

impl FxCapability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Render => "render",
            Self::Light => "light",
            Self::Particle => "particle",
            Self::Material => "material",
            Self::Audio => "audio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxStage {
    Prepass,
    BeforeOpaque,
    Opaque,
    AfterOpaque,
    Transparent,
    AfterTransparent,
    PostProcess,
    Ui,
    Debug,
}

impl FxStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prepass => "prepass",
            Self::BeforeOpaque => "before_opaque",
            Self::Opaque => "opaque",
            Self::AfterOpaque => "after_opaque",
            Self::Transparent => "transparent",
            Self::AfterTransparent => "after_transparent",
            Self::PostProcess => "post_process",
            Self::Ui => "ui",
            Self::Debug => "debug",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FxMeta {
    pub id: String,
    pub version: u32,
    pub scope: FxScope,
    pub requires: Vec<String>,
    pub capabilities: Vec<FxCapability>,
    pub min_quality: FxQuality,
    pub cost: FxCost,
}

impl FxMeta {
    pub fn new(
        id: impl Into<String>,
        scope: FxScope,
        capabilities: &[FxCapability],
        min_quality: FxQuality,
    ) -> Self {
        Self {
            id: id.into(),
            version: 1,
            scope,
            requires: Vec::new(),
            capabilities: capabilities.to_vec(),
            min_quality,
            cost: FxCost::default(),
        }
    }

    pub fn require(mut self, id: impl Into<String>) -> Self {
        self.requires.push(id.into());
        self
    }

    pub fn cost(mut self, cost: FxCost) -> Self {
        self.cost = cost;
        self
    }
}

#[derive(Debug, Error)]
pub enum FxError {
    #[error("invalid config for {id}: {message}")]
    InvalidConfig { id: String, message: String },
    #[error("fx module '{0}' is already registered")]
    Duplicate(String),
    #[error("fx module '{0}' was not found")]
    NotFound(String),
}

/// 静态模块定义：元信息、默认配置、工厂。
pub trait FxDefinition: Send + Sync {
    fn meta(&self) -> FxMeta;
    fn default_config(&self) -> FxConfig;
    fn create(&self, config: &FxConfig) -> Result<Box<dyn FxInstance>, FxError>;
}

/// 运行时模块实例：只读快照 + 自己的状态。
pub trait FxInstance: Send {
    fn update(&mut self, ctx: &mut FxContext);
    fn snapshot(&self) -> BTreeMap<String, FxValue> {
        BTreeMap::new()
    }
    fn shutdown(&mut self) {}
}

/// 一条模块的启用/禁用原因，给兼容报告使用。
#[derive(Debug, Clone, Serialize)]
pub struct FxStatus {
    pub id: String,
    pub enabled: bool,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
    pub snapshot: BTreeMap<String, FxValue>,
}
