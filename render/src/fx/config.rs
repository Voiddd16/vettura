//! FX 配置值模型：与 TOML / JSON 互转，避免把 `toml::Value` 泄漏到公开 API。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FxValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    List(Vec<FxValue>),
    Table(BTreeMap<String, FxValue>),
}

impl FxValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Integer(value) => Some(*value as f32),
            Self::Float(value) => Some(*value as f32),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[FxValue]> {
        match self {
            Self::List(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&BTreeMap<String, FxValue>> {
        match self {
            Self::Table(values) => Some(values),
            _ => None,
        }
    }
}

impl From<toml::Value> for FxValue {
    fn from(value: toml::Value) -> Self {
        match value {
            toml::Value::String(value) => Self::String(value),
            toml::Value::Integer(value) => Self::Integer(value),
            toml::Value::Float(value) => Self::Float(value),
            toml::Value::Boolean(value) => Self::Bool(value),
            toml::Value::Datetime(value) => Self::String(value.to_string()),
            toml::Value::Array(values) => Self::List(values.into_iter().map(Self::from).collect()),
            toml::Value::Table(values) => Self::Table(
                values
                    .into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
        }
    }
}

pub type FxConfig = BTreeMap<String, FxValue>;

/// 解析一份 FX 配置（TOML 文本）。
pub fn parse_toml(text: &str) -> Result<FxConfig, toml::de::Error> {
    let value: toml::Value = toml::from_str(text)?;
    Ok(match FxValue::from(value) {
        FxValue::Table(table) => table,
        _ => BTreeMap::new(),
    })
}

pub fn bool_at(config: &FxConfig, key: &str, default: bool) -> bool {
    config
        .get(key)
        .and_then(FxValue::as_bool)
        .unwrap_or(default)
}

pub fn f32_at(config: &FxConfig, key: &str, default: f32) -> f32 {
    config.get(key).and_then(FxValue::as_f32).unwrap_or(default)
}

pub fn str_at<'a>(config: &'a FxConfig, key: &str, default: &'a str) -> &'a str {
    config.get(key).and_then(FxValue::as_str).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_toml() {
        let config = parse_toml(
            r#"
            enabled = true
            density = 0.5
            name = "wet"
            layers = ["rain", "spray"]
            [lighting]
            max_lights = 16
            "#,
        )
        .unwrap();

        assert!(bool_at(&config, "enabled", false));
        assert!((f32_at(&config, "density", 0.0) - 0.5).abs() < 1e-6);
        assert_eq!(str_at(&config, "name", ""), "wet");
        assert!(config.get("layers").unwrap().as_list().is_some());
        assert!(config.get("lighting").unwrap().as_table().is_some());
    }
}
