//! 视觉兼容报告：扫描一个 mod 目录，输出材质覆盖率、ext_config 绑定和 FX 模块状态。
//!
//! 这是“先看再改”的入口：它不修改渲染，只告诉你哪些能自动适配、哪些会降级。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::ext_config::{BindingPlan, ExtConfigDocument, ExtConfigOptions, ExtConfigResolver};
use crate::fx::{FxContext, FxManager, FxProfile, FxReport};
use crate::material::{MaterialCoverage, MaterialRegistry};
use crate::warning::CompatWarning;

#[derive(Debug, Error)]
pub enum InspectError {
    #[error("path does not exist: {0}")]
    Missing(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ext_config error: {0}")]
    ExtConfig(#[from] crate::ext_config::ExtConfigError),
}

#[derive(Debug, Clone, Copy)]
pub struct InspectOptions {
    /// 最多解析多少个 KN5；大车包/赛道包可能有很多。
    pub max_kn5: usize,
    pub ext_config: ExtConfigOptions,
}

impl Default for InspectOptions {
    fn default() -> Self {
        Self {
            max_kn5: 8,
            ext_config: ExtConfigOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualReport {
    pub path: String,
    pub kind: String,
    pub kn5_files: usize,
    pub kn5_parsed: usize,
    pub materials: MaterialCoverage,
    pub ext_config: BindingPlan,
    pub fx: FxReport,
    pub warnings: Vec<CompatWarning>,
}

impl VisualReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|error| {
            format!("{{\"error\":\"failed to serialize report: {error}\"}}")
        })
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("path: {}\n", self.path));
        out.push_str(&format!("kind: {}\n", self.kind));
        out.push_str(&format!(
            "kn5:   {}/{} parsed\n",
            self.kn5_parsed, self.kn5_files
        ));
        out.push_str(&format!(
            "materials: {}/{} mapped ({:.1}%), {} unknown\n",
            self.materials.mapped,
            self.materials.total,
            self.materials.ratio() * 100.0,
            self.materials.unknown
        ));
        if !self.materials.by_family.is_empty() {
            out.push_str("  by family:\n");
            for (family, count) in &self.materials.by_family {
                out.push_str(&format!("    {family}: {count}\n"));
            }
        }
        if !self.materials.unknown_shaders.is_empty() {
            out.push_str("  unknown shaders:\n");
            for (shader, count) in &self.materials.unknown_shaders {
                out.push_str(&format!("    {shader}: {count}\n"));
            }
        }
        out.push_str(&format!(
            "ext_config: {} sections, {}/{} supported\n",
            self.ext_config.stats.sections,
            self.ext_config.stats.supported_sections,
            self.ext_config.stats.sections
        ));
        out.push_str(&format!(
            "  material overrides: {}, texture overrides: {}, fx patches: {}, lights: {}, particles: {}, records: {}\n",
            self.ext_config.stats.material_overrides,
            self.ext_config.stats.texture_overrides,
            self.ext_config.stats.fx_patches,
            self.ext_config.stats.lights,
            self.ext_config.stats.particles,
            self.ext_config.stats.records
        ));
        if !self.ext_config.records.is_empty() {
            let mut by_module = std::collections::BTreeMap::new();
            for record in &self.ext_config.records {
                *by_module.entry(record.module.as_str()).or_insert(0usize) += 1;
            }
            out.push_str("  fx records by module:\n");
            for (module, count) in by_module {
                out.push_str(&format!("    {module}: {count}\n"));
            }
        }
        out.push_str(&format!(
            "fx: {} enabled, {} disabled (quality {})\n",
            self.fx.enabled_count(),
            self.fx.disabled_count(),
            self.fx.quality.label()
        ));
        for entry in &self.fx.entries {
            if entry.enabled {
                out.push_str(&format!(
                    "  + {} ({})\n",
                    entry.id,
                    entry
                        .capabilities
                        .iter()
                        .map(|capability| capability.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            } else if let Some(reason) = &entry.reason {
                out.push_str(&format!("  - {}: {reason}\n", entry.id));
            }
        }

        if !self.warnings.is_empty() {
            out.push_str(&format!("warnings: {}\n", self.warnings.len()));
            for warning in self.warnings.iter().take(40) {
                out.push_str(&format!(
                    "  [{}] {} :: {} — {}\n",
                    warning.severity_label(),
                    warning.section,
                    warning.source,
                    warning.message
                ));
            }
            if self.warnings.len() > 40 {
                out.push_str(&format!("  ... {} more\n", self.warnings.len() - 40));
            }
        }
        out
    }
}

pub fn inspect_path(
    path: impl AsRef<Path>,
    options: InspectOptions,
) -> Result<VisualReport, InspectError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(InspectError::Missing(path.display().to_string()));
    }

    let all_files = collect_files(path, 4_000);
    let kn5_files = all_files
        .iter()
        .filter(|file| {
            file.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("kn5"))
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut warnings = Vec::new();
    let mut materials = Vec::new();
    let mut parsed = 0;
    for file in kn5_files.iter().take(options.max_kn5) {
        match vettura_kn5::Kn5::from_path(file) {
            Ok(kn5) => {
                parsed += 1;
                materials.extend(kn5.materials);
            }
            Err(error) => warnings.push(CompatWarning::warning(
                file.display().to_string(),
                "kn5",
                format!("解析失败：{error}"),
            )),
        }
    }
    if kn5_files.len() > options.max_kn5 {
        warnings.push(CompatWarning::info(
            path.display().to_string(),
            "kn5",
            format!(
                "只解析了前 {} 个 KN5（共 {} 个）；用 --max-kn5 调整",
                options.max_kn5,
                kn5_files.len()
            ),
        ));
    }

    let registry = MaterialRegistry::builtin();
    let coverage = registry.coverage(materials.iter());

    let ext_config_document = ExtConfigDocument::load_tree(path, options.ext_config)?;
    let binding = ExtConfigResolver::new(&materials).resolve(&ext_config_document);

    let mut fx_manager = FxManager::new();
    fx_manager.build_with_plan(&FxProfile::default(), &binding);
    let mut fx_context = FxContext::default();
    fx_manager.update(&mut fx_context);
    let fx_report = fx_manager.report().clone();

    warnings.extend(ext_config_document.warnings.clone());
    warnings.extend(binding.warnings.clone());
    warnings.extend(fx_report.warnings.clone());
    dedupe_warnings(&mut warnings);

    Ok(VisualReport {
        path: path.display().to_string(),
        kind: detect_kind(path, &all_files),
        kn5_files: kn5_files.len(),
        kn5_parsed: parsed,
        materials: coverage,
        ext_config: binding,
        fx: fx_report,
        warnings,
    })
}

fn detect_kind(root: &Path, files: &[PathBuf]) -> String {
    let has = |name: &str| {
        files.iter().any(|file| {
            file.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
        })
    };
    let any_contains = |needle: &str| {
        files.iter().any(|file| {
            file.to_string_lossy()
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
    };

    if has("data.acd") || has("car.ini") {
        "car".to_owned()
    } else if has("surfaces.ini") || has("fast_lane.ai") || any_contains("ui_track.json") {
        "track".to_owned()
    } else if any_contains("apps/lua") || any_contains("apps\\lua") {
        "lua_app".to_owned()
    } else if any_contains("ppfilters") {
        "ppfilter".to_owned()
    } else if any_contains("weather") {
        "weather".to_owned()
    } else if any_contains("skins") || any_contains("skin.ini") {
        "skins".to_owned()
    } else if !files.is_empty() {
        "visual".to_owned()
    } else {
        let _ = root;
        "unknown".to_owned()
    }
}

fn dedupe_warnings(warnings: &mut Vec<CompatWarning>) {
    let mut seen = BTreeSet::new();
    warnings.retain(|warning| {
        seen.insert((
            warning.source.clone(),
            warning.section.clone(),
            warning.message.clone(),
        ))
    });
}

fn collect_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, limit: usize, depth: usize) {
        if out.len() >= limit || depth > 24 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if matches!(name.as_str(), ".git" | "target" | "node_modules") {
                    continue;
                }
                walk(&path, out, limit, depth + 1);
            } else if file_type.is_file() {
                out.push(path);
                if out.len() >= limit {
                    return;
                }
            }
        }
    }

    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
    } else {
        walk(root, &mut out, limit, 0);
    }
    out
}

trait SeverityLabel {
    fn severity_label(&self) -> &'static str;
}

impl SeverityLabel for CompatWarning {
    fn severity_label(&self) -> &'static str {
        match self.severity {
            crate::warning::WarningSeverity::Info => "info",
            crate::warning::WarningSeverity::Warning => "warn",
            crate::warning::WarningSeverity::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_is_reported() {
        let error = inspect_path(
            "/definitely/not/a/real/vettura/path",
            InspectOptions::default(),
        );
        assert!(matches!(error, Err(InspectError::Missing(_))));
    }
}
