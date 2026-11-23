//! FX 注册、依赖排序、生命周期与兼容报告。

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::warning::{CompatWarning, WarningSeverity};

use super::config::FxValue;
use super::context::{FxContext, FxQuality};
use super::definition::{FxCapability, FxDefinition, FxError, FxInstance, FxMeta, FxScope};
use super::modules::builtin_definitions;
use super::profile::FxProfile;

#[derive(Debug, Clone, Serialize)]
pub struct FxReportEntry {
    pub id: String,
    pub scope: FxScope,
    pub capabilities: Vec<FxCapability>,
    pub enabled: bool,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
    pub snapshot: BTreeMap<String, FxValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FxReport {
    pub quality: FxQuality,
    pub entries: Vec<FxReportEntry>,
    pub warnings: Vec<CompatWarning>,
}

impl FxReport {
    pub fn enabled_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.enabled).count()
    }

    pub fn disabled_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.enabled).count()
    }
}

struct RunningFx {
    meta: FxMeta,
    instance: Box<dyn FxInstance>,
}

/// FX 运行时管理器。实例在 `build()` 时创建，`update()` 每帧推进。
pub struct FxManager {
    definitions: Vec<Box<dyn FxDefinition>>,
    running: Vec<RunningFx>,
    report: FxReport,
}

impl Default for FxManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FxManager {
    /// 注册所有内置模块。
    pub fn new() -> Self {
        let mut manager = Self {
            definitions: Vec::new(),
            running: Vec::new(),
            report: FxReport {
                quality: FxQuality::Medium,
                entries: Vec::new(),
                warnings: Vec::new(),
            },
        };
        for definition in builtin_definitions() {
            manager.definitions.push(definition);
        }
        manager
    }

    /// 注册一个额外模块（社区/测试）。
    pub fn register(&mut self, definition: Box<dyn FxDefinition>) -> Result<(), FxError> {
        let id = definition.meta().id;
        if self
            .definitions
            .iter()
            .any(|existing| existing.meta().id == id)
        {
            return Err(FxError::Duplicate(id));
        }
        self.definitions.push(definition);
        Ok(())
    }

    pub fn definitions(&self) -> &[Box<dyn FxDefinition>] {
        &self.definitions
    }

    /// 按 profile 创建实例。缺失依赖/质量不足/被禁用都会进报告，不返回全局错误。
    /// 把 `ext_config` 的 FX 参数合并进 profile 后再构建。
    pub fn build_with_plan(&mut self, profile: &FxProfile, plan: &crate::ext_config::BindingPlan) {
        let mut merged = profile.clone();
        for patch in &plan.fx_patches {
            merged.merge_config(patch.module.clone(), patch.values.clone());
        }
        self.build(&merged);
    }

    pub fn build(&mut self, profile: &FxProfile) {
        self.running.clear();
        self.report = FxReport {
            quality: profile.quality,
            entries: Vec::new(),
            warnings: Vec::new(),
        };

        let order = self.dependency_order();
        let mut enabled_ids = BTreeSet::new();

        for index in order {
            let definition = &self.definitions[index];
            let meta = definition.meta();
            let id = meta.id.clone();

            if !profile.is_enabled(&id) {
                self.disabled(meta, "disabled by profile".to_owned());
                continue;
            }
            if !profile.quality.at_least(meta.min_quality) {
                let reason = format!(
                    "quality '{}' below minimum '{}'",
                    profile.quality.label(),
                    meta.min_quality.label()
                );
                self.disabled(meta, reason);
                continue;
            }

            let missing = meta
                .requires
                .iter()
                .filter(|required| !enabled_ids.contains(*required))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                let reason = format!("missing dependency: {}", missing.join(", "));
                self.report
                    .warnings
                    .push(CompatWarning::warning("fx", &id, reason.clone()));
                self.disabled(meta, reason);
                continue;
            }

            let config = profile.config(&id).cloned().unwrap_or_default();
            match definition.create(&config) {
                Ok(instance) => {
                    enabled_ids.insert(id.clone());
                    self.report.entries.push(FxReportEntry {
                        id,
                        scope: meta.scope,
                        capabilities: meta.capabilities.clone(),
                        enabled: true,
                        reason: None,
                        warnings: Vec::new(),
                        snapshot: BTreeMap::new(),
                    });
                    self.running.push(RunningFx { meta, instance });
                }
                Err(error) => {
                    self.disabled(meta, error.to_string());
                }
            }
        }

        // 报告按注册 ID 排序，方便 diff。
        self.report.entries.sort_by(|a, b| a.id.cmp(&b.id));
    }

    fn disabled(&mut self, meta: FxMeta, reason: String) {
        self.report.entries.push(FxReportEntry {
            id: meta.id,
            scope: meta.scope,
            capabilities: meta.capabilities,
            enabled: false,
            reason: Some(reason),
            warnings: Vec::new(),
            snapshot: BTreeMap::new(),
        });
    }

    /// 每帧更新；实例顺序由依赖排序保证（比如 weather 在 rain 之前）。
    pub fn update(&mut self, ctx: &mut FxContext) {
        for running in &mut self.running {
            running.instance.update(ctx);
        }
        for entry in &mut self.report.entries {
            if let Some(running) = self
                .running
                .iter()
                .find(|running| running.meta.id == entry.id)
            {
                entry.snapshot = running.instance.snapshot();
            }
        }
    }

    pub fn report(&self) -> &FxReport {
        &self.report
    }

    pub fn running_ids(&self) -> Vec<&str> {
        self.running
            .iter()
            .map(|running| running.meta.id.as_str())
            .collect()
    }

    pub fn shutdown(&mut self) {
        for running in &mut self.running {
            running.instance.shutdown();
        }
        self.running.clear();
    }

    fn dependency_order(&mut self) -> Vec<usize> {
        let ids = self
            .definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (definition.meta().id, index))
            .collect::<BTreeMap<_, _>>();
        let mut state = vec![0u8; self.definitions.len()];
        let mut order = Vec::with_capacity(self.definitions.len());
        let mut warnings = Vec::new();

        for index in 0..self.definitions.len() {
            visit(
                index,
                &self.definitions,
                &ids,
                &mut state,
                &mut order,
                &mut warnings,
            );
        }

        self.report.warnings.extend(warnings);
        order
    }
}

fn visit(
    index: usize,
    definitions: &[Box<dyn FxDefinition>],
    ids: &BTreeMap<String, usize>,
    state: &mut [u8],
    order: &mut Vec<usize>,
    warnings: &mut Vec<CompatWarning>,
) {
    match state[index] {
        2 => return,
        1 => {
            let id = definitions[index].meta().id;
            warnings.push(CompatWarning {
                source: "fx".to_owned(),
                section: id.clone(),
                message: format!("cyclic dependency at '{id}'"),
                severity: WarningSeverity::Error,
            });
            return;
        }
        _ => {}
    }

    state[index] = 1;
    let meta = definitions[index].meta();
    for required in &meta.requires {
        if let Some(required_index) = ids.get(required) {
            visit(*required_index, definitions, ids, state, order, warnings);
        } else {
            warnings.push(CompatWarning::warning(
                "fx",
                &meta.id,
                format!("unknown dependency '{required}'"),
            ));
        }
    }
    state[index] = 2;
    order.push(index);
}

#[cfg(test)]
mod tests {
    use super::super::config::{FxConfig, FxValue};
    use super::super::context::FxContext;
    use super::super::definition::FxError;
    use super::super::definition::{FxInstance, FxMeta};
    use super::*;

    #[test]
    fn builds_all_builtin_modules_at_medium() {
        let mut manager = FxManager::new();
        manager.build(&FxProfile::default());
        let report = manager.report();
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.weather" && entry.enabled)
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.rain" && entry.enabled)
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.grass" && entry.enabled)
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.extra.bloom" && entry.enabled)
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.extra.ssao" && !entry.enabled)
        );
    }

    #[test]
    fn high_quality_enables_expensive_modules() {
        let mut manager = FxManager::new();
        manager.build(&FxProfile::default().with_quality(FxQuality::High));
        let report = manager.report();
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.extra.ssao" && entry.enabled)
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.id == "vettura.fx.extra.ssr" && entry.enabled)
        );
    }

    #[test]
    fn disabling_weather_disables_rain_but_not_particles() {
        let mut manager = FxManager::new();
        manager.build(&FxProfile::default().disable("vettura.fx.weather"));
        let report = manager.report();
        let rain = report
            .entries
            .iter()
            .find(|entry| entry.id == "vettura.fx.rain")
            .unwrap();
        assert!(!rain.enabled);
        assert!(
            rain.reason
                .as_deref()
                .unwrap_or("")
                .contains("vettura.fx.weather")
        );
        let particles = report
            .entries
            .iter()
            .find(|entry| entry.id == "vettura.fx.particles")
            .unwrap();
        assert!(particles.enabled);
    }

    #[test]
    fn weather_advances_time_and_rain_follows_wetness() {
        let mut profile = FxProfile::default();
        let mut weather_config = FxConfig::new();
        weather_config.insert("time_of_day".to_owned(), FxValue::Float(10.0));
        weather_config.insert("rain".to_owned(), FxValue::Float(0.5));
        weather_config.insert("wetness".to_owned(), FxValue::Float(0.5));
        profile
            .configs
            .insert("vettura.fx.weather".to_owned(), weather_config);

        let mut manager = FxManager::new();
        manager.build(&profile);
        let mut ctx = FxContext {
            dt: 3600.0,
            ..FxContext::default()
        };
        manager.update(&mut ctx);

        assert!((ctx.weather.time_of_day - 11.0).abs() < 1e-4);
        let report = manager.report();
        let weather = report
            .entries
            .iter()
            .find(|entry| entry.id == "vettura.fx.weather")
            .unwrap();
        assert_eq!(
            weather
                .snapshot
                .get("time_of_day")
                .and_then(FxValue::as_f32),
            Some(11.0)
        );
        let rain = report
            .entries
            .iter()
            .find(|entry| entry.id == "vettura.fx.rain")
            .unwrap();
        assert_eq!(
            rain.snapshot.get("intensity").and_then(FxValue::as_f32),
            Some(0.5)
        );
        assert_eq!(
            rain.snapshot.get("spray").and_then(FxValue::as_bool),
            Some(true)
        );
    }

    struct FakeModule {
        meta: FxMeta,
    }

    struct FakeInstance;

    impl FxInstance for FakeInstance {
        fn update(&mut self, _ctx: &mut FxContext) {}
    }

    impl FxDefinition for FakeModule {
        fn meta(&self) -> FxMeta {
            self.meta.clone()
        }

        fn default_config(&self) -> FxConfig {
            FxConfig::new()
        }

        fn create(&self, _config: &FxConfig) -> Result<Box<dyn FxInstance>, FxError> {
            Ok(Box::new(FakeInstance))
        }
    }

    #[test]
    fn missing_dependency_disables_module() {
        let mut manager = FxManager::new();
        manager
            .register(Box::new(FakeModule {
                meta: FxMeta::new(
                    "test.needs_missing",
                    FxScope::Scene,
                    &[FxCapability::Render],
                    FxQuality::Low,
                )
                .require("test.missing"),
            }))
            .unwrap();
        manager.build(&FxProfile::default());
        let entry = manager
            .report()
            .entries
            .iter()
            .find(|entry| entry.id == "test.needs_missing")
            .unwrap();
        assert!(!entry.enabled);
        assert!(
            entry
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("test.missing")
        );
    }

    #[test]
    fn build_with_plan_merges_ext_config_patch() {
        let plan = crate::ext_config::BindingPlan {
            fx_patches: vec![crate::ext_config::FxConfigPatch {
                module: "vettura.fx.weather".to_owned(),
                source: "test.ini".to_owned(),
                section: "LIGHTING".to_owned(),
                values: FxConfig::from([("rain".to_owned(), FxValue::Float(0.75))]),
            }],
            ..crate::ext_config::BindingPlan::default()
        };
        let mut manager = FxManager::new();
        manager.build_with_plan(&FxProfile::default(), &plan);
        let mut ctx = FxContext::default();
        manager.update(&mut ctx);
        let weather = manager
            .report()
            .entries
            .iter()
            .find(|entry| entry.id == "vettura.fx.weather")
            .unwrap();
        assert_eq!(
            weather.snapshot.get("rain").and_then(FxValue::as_f32),
            Some(0.75)
        );
    }

    #[test]
    fn duplicate_registration_fails() {
        let mut manager = FxManager::new();
        let duplicate = FakeModule {
            meta: FxMeta::new(
                "vettura.fx.weather",
                FxScope::Scene,
                &[FxCapability::Environment],
                FxQuality::Low,
            ),
        };
        assert!(matches!(
            manager.register(Box::new(duplicate)),
            Err(FxError::Duplicate(_))
        ));
    }
}
