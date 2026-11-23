//! FX 启用/禁用配置与每模块参数。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::config::{FxConfig, FxValue};
use super::context::FxQuality;
use super::definition::FxError;

#[derive(Debug, Clone, Serialize)]
pub struct FxProfile {
    pub quality: FxQuality,
    /// 空表示“全部启用”。
    pub enabled: BTreeSet<String>,
    pub disabled: BTreeSet<String>,
    pub configs: BTreeMap<String, FxConfig>,
}

impl Default for FxProfile {
    fn default() -> Self {
        Self {
            quality: FxQuality::Medium,
            enabled: BTreeSet::new(),
            disabled: BTreeSet::new(),
            configs: BTreeMap::new(),
        }
    }
}

impl FxProfile {
    pub fn with_quality(mut self, quality: FxQuality) -> Self {
        self.quality = quality;
        self
    }

    pub fn enable_only(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.enabled = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn disable(mut self, id: impl Into<String>) -> Self {
        self.disabled.insert(id.into());
        self
    }

    pub fn config(&self, id: &str) -> Option<&FxConfig> {
        self.configs.get(id)
    }

    /// 合并一组参数（`ext_config` / `visual.toml` 的覆盖）。
    pub fn merge_config(&mut self, id: impl Into<String>, values: FxConfig) {
        self.configs.entry(id.into()).or_default().extend(values);
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        if self.disabled.contains(id) {
            return false;
        }
        self.enabled.is_empty() || self.enabled.contains(id)
    }

    /// 从 TOML 读取：
    ///
    /// ```toml
    /// quality = "high"
    /// enabled = ["vettura.fx.weather", "vettura.fx.rain"]
    /// disabled = ["vettura.fx.extra.ssr"]
    /// [configs."vettura.fx.rain"]
    /// intensity = 1.0
    /// ```
    pub fn parse_toml(text: &str) -> Result<Self, FxError> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            quality: Option<String>,
            #[serde(default)]
            enabled: Vec<String>,
            #[serde(default)]
            disabled: Vec<String>,
            #[serde(default)]
            configs: BTreeMap<String, toml::Value>,
        }

        let file: File = toml::from_str(text).map_err(|error| FxError::InvalidConfig {
            id: "fx.profile".to_owned(),
            message: error.to_string(),
        })?;

        let quality = match file.quality {
            Some(value) => FxQuality::parse(&value).ok_or_else(|| FxError::InvalidConfig {
                id: "fx.profile".to_owned(),
                message: format!("未知 quality '{value}'"),
            })?,
            None => FxQuality::Medium,
        };

        let configs = file
            .configs
            .into_iter()
            .map(|(id, value)| {
                let config = match FxValue::from(value) {
                    FxValue::Table(table) => table,
                    other => BTreeMap::from([("value".to_owned(), other)]),
                };
                (id, config)
            })
            .collect();

        Ok(Self {
            quality,
            enabled: file.enabled.into_iter().collect(),
            disabled: file.disabled.into_iter().collect(),
            configs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profile() {
        let profile = FxProfile::parse_toml(
            r#"
            quality = "high"
            enabled = ["vettura.fx.weather", "vettura.fx.rain"]
            disabled = ["vettura.fx.extra.ssr"]
            [configs."vettura.fx.rain"]
            intensity = 0.8
            "#,
        )
        .unwrap();

        assert_eq!(profile.quality, FxQuality::High);
        assert!(profile.is_enabled("vettura.fx.weather"));
        assert!(!profile.is_enabled("vettura.fx.extra.ssr"));
        assert!(
            (profile.config("vettura.fx.rain").unwrap()["intensity"]
                .as_f32()
                .unwrap()
                - 0.8)
                .abs()
                < 1e-6
        );
    }
}
