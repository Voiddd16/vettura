//! 兼容性诊断：所有“看过但没完全支持”的东西都进这里，不静默吞掉。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompatWarning {
    pub source: String,
    pub section: String,
    pub message: String,
    pub severity: WarningSeverity,
}

impl CompatWarning {
    pub fn info(
        source: impl Into<String>,
        section: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            section: section.into(),
            message: message.into(),
            severity: WarningSeverity::Info,
        }
    }

    pub fn warning(
        source: impl Into<String>,
        section: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            section: section.into(),
            message: message.into(),
            severity: WarningSeverity::Warning,
        }
    }

    pub fn error(
        source: impl Into<String>,
        section: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            section: section.into(),
            message: message.into(),
            severity: WarningSeverity::Error,
        }
    }
}
