//! CSP `ext_config.ini` 的声明式子集解析与绑定计划。
//!
//! 这里**不执行** CSP 的脚本/动态 patch，也不解释任意 shader 代码：
//! - `[SHADER_REPLACEMENT_...]` / `[TEXTURE_OVERRIDE_...]` → 材质/贴图覆盖；
//! - `[LIGHTING]` / `[EXTRA_LIGHT_...]` / `[PARTICLES_...]` → FX 配置；
//! - `[INCLUDE]` → 递归加载；
//! - 未知 section → 兼容报告。
//!
//! 结果 [`BindingPlan`] 是可序列化、可缓存的“应用计划”，由 `gdext` 或工具消费。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use vettura_acdata::Ini;
use vettura_kn5::Material;

use crate::fx::{FxConfig, FxValue};
use crate::warning::CompatWarning;

#[derive(Debug, Error)]
pub enum ExtConfigError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtConfigKind {
    ShaderReplacement,
    TextureOverride,
    Material,
    Emissive,
    ModelReplacement,
    Driver3d,
    ScriptableDisplay,
    TyresFx,
    GrassFx,
    Weather,
    Dash,
    Camera,
    Lighting,
    ExtraLight,
    Particles,
    Animation,
    Condition,
    Include,
    Physics,
    Audio,
    Gui,
    Other,
    Unknown,
}

impl ExtConfigKind {
    pub fn from_name(name: &str) -> Self {
        let base = name
            .split(':')
            .next()
            .unwrap_or(name)
            .trim()
            .to_ascii_uppercase();
        if base.starts_with("SHADER_REPLACEMENT") {
            Self::ShaderReplacement
        } else if base.starts_with("TEXTURE_OVERRIDE") {
            Self::TextureOverride
        } else if base.starts_with("MODEL_REPLACEMENT") {
            Self::ModelReplacement
        } else if base.starts_with("DRIVER3D") {
            Self::Driver3d
        } else if base.starts_with("SCRIPTABLE_DISPLAY") || base.starts_with("DISPLAY") {
            Self::ScriptableDisplay
        } else if base.starts_with("TYRES_FX") {
            Self::TyresFx
        } else if base.starts_with("GRASS") {
            Self::GrassFx
        } else if base.starts_with("WEATHER")
            || base.starts_with("WIND")
            || base.starts_with("RAIN")
        {
            Self::Weather
        } else if base.starts_with("WIPER") || base.starts_with("WOBBLY") {
            Self::Animation
        } else if base.starts_with("DASH") {
            Self::Dash
        } else if base.starts_with("CAMERA") {
            Self::Camera
        } else if base.starts_with("EXTRA_LIGHT") {
            Self::ExtraLight
        } else if base.starts_with("PARTICLES") {
            Self::Particles
        } else if base.starts_with("ANIMATION") {
            Self::Animation
        } else if base.starts_with("CONDITION") {
            Self::Condition
        } else if base.starts_with("INCLUDE") {
            Self::Include
        } else if base.starts_with("LIGHTING") {
            Self::Lighting
        } else if base.starts_with("EMISSIVE") {
            Self::Emissive
        } else if base.starts_with("MATERIAL") {
            Self::Material
        } else if base.starts_with("PHYSICS") || base.starts_with("TYRES") {
            Self::Physics
        } else if base.starts_with("AUDIO") || base.starts_with("SOUND") {
            Self::Audio
        } else if base.starts_with("GUI") || base.starts_with("UI") {
            Self::Gui
        } else if base.is_empty() {
            Self::Unknown
        } else {
            Self::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ShaderReplacement => "shader_replacement",
            Self::TextureOverride => "texture_override",
            Self::Material => "material",
            Self::Emissive => "emissive",
            Self::ModelReplacement => "model_replacement",
            Self::Driver3d => "driver3d",
            Self::ScriptableDisplay => "scriptable_display",
            Self::TyresFx => "tyres_fx",
            Self::GrassFx => "grass_fx",
            Self::Weather => "weather",
            Self::Dash => "dash",
            Self::Camera => "camera",
            Self::Lighting => "lighting",
            Self::ExtraLight => "extra_light",
            Self::Particles => "particles",
            Self::Animation => "animation",
            Self::Condition => "condition",
            Self::Include => "include",
            Self::Physics => "physics",
            Self::Audio => "audio",
            Self::Gui => "gui",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }

    /// v1 是否已经有原生映射。
    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::ShaderReplacement
                | Self::TextureOverride
                | Self::Material
                | Self::Emissive
                | Self::Lighting
                | Self::ExtraLight
                | Self::Particles
                | Self::GrassFx
                | Self::Weather
                | Self::Include
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

impl From<&vettura_acdata::IniEntry> for ConfigEntry {
    fn from(entry: &vettura_acdata::IniEntry) -> Self {
        Self {
            key: entry.key.clone(),
            value: entry.value.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtConfigSection {
    pub kind: ExtConfigKind,
    pub name: String,
    pub source: String,
    pub entries: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtConfigOptions {
    pub max_files: usize,
    pub max_depth: usize,
}

impl Default for ExtConfigOptions {
    fn default() -> Self {
        Self {
            max_files: 128,
            max_depth: 16,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExtConfigDocument {
    pub files: Vec<String>,
    pub sections: Vec<ExtConfigSection>,
    pub warnings: Vec<CompatWarning>,
}

impl ExtConfigDocument {
    /// 解析一段文本（不处理 include；用于测试/单文件）。
    pub fn parse_text(path: impl Into<String>, text: &str) -> Self {
        let path = path.into();
        let ini = Ini::parse(text);
        let mut document = Self::default();
        document.files.push(path.clone());
        for section in ini.sections {
            document.push_section(&path, section);
        }
        document
    }

    /// 解析单个文件。
    pub fn parse_file(path: &Path) -> Result<Self, ExtConfigError> {
        let text = fs::read_to_string(path)?;
        Ok(Self::parse_text(path.display().to_string(), &text))
    }

    /// 递归扫描目录下的 `ext_config.ini`，并处理 `[INCLUDE]`。
    pub fn load_tree(root: &Path, options: ExtConfigOptions) -> Result<Self, ExtConfigError> {
        let mut document = Self::default();
        let mut visited = BTreeSet::new();
        let files = find_ext_config_files(root, options.max_files);
        if files.len() >= options.max_files {
            document.warnings.push(CompatWarning::warning(
                root.display().to_string(),
                "ext_config",
                format!("ext_config 文件数达到上限 {}", options.max_files),
            ));
        }
        for path in files {
            document.load_file(&path, 0, &mut visited, options)?;
        }
        Ok(document)
    }

    fn load_file(
        &mut self,
        path: &Path,
        depth: usize,
        visited: &mut BTreeSet<String>,
        options: ExtConfigOptions,
    ) -> Result<(), ExtConfigError> {
        if depth > options.max_depth {
            self.warnings.push(CompatWarning::warning(
                path.display().to_string(),
                "include",
                "include 深度超过上限",
            ));
            return Ok(());
        }

        let key = fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string();
        if !visited.insert(key) {
            self.warnings.push(CompatWarning::warning(
                path.display().to_string(),
                "include",
                "检测到 include 循环，已跳过",
            ));
            return Ok(());
        }

        let text = fs::read_to_string(path)?;
        let source = path.display().to_string();
        if !self.files.contains(&source) {
            self.files.push(source.clone());
        }

        let ini = Ini::parse(&text);
        for section in ini.sections {
            let kind = ExtConfigKind::from_name(&section.name);
            let entries = section
                .entries
                .iter()
                .map(ConfigEntry::from)
                .collect::<Vec<_>>();

            if kind == ExtConfigKind::Include {
                for include in include_paths(&section) {
                    let include_path = resolve_include(path, &include);
                    if include_path.is_file() {
                        self.load_file(&include_path, depth + 1, visited, options)?;
                    } else {
                        self.warnings.push(CompatWarning::warning(
                            source.clone(),
                            section.name.clone(),
                            format!("include 文件不存在：{}", include_path.display()),
                        ));
                    }
                }
            }

            self.sections.push(ExtConfigSection {
                kind,
                name: section.name,
                source: source.clone(),
                entries,
            });
        }
        Ok(())
    }

    fn push_section(&mut self, source: &str, section: vettura_acdata::IniSection) {
        let kind = ExtConfigKind::from_name(&section.name);
        self.sections.push(ExtConfigSection {
            kind,
            name: section.name,
            source: source.to_owned(),
            entries: section.entries.iter().map(ConfigEntry::from).collect(),
        });
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }
}

fn find_ext_config_files(root: &Path, limit: usize) -> Vec<PathBuf> {
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
                if matches!(
                    name.as_str(),
                    ".git" | "target" | "node_modules" | "skins" | "textures" | "texture" | "sfx"
                ) {
                    continue;
                }
                walk(&path, out, limit, depth + 1);
            } else if file_type.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("ext_config.ini"))
            {
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

fn include_paths(section: &vettura_acdata::IniSection) -> Vec<String> {
    let suffix = section
        .name
        .split_once(':')
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(suffix) = suffix {
        return vec![suffix];
    }

    let mut paths = Vec::new();
    for entry in &section.entries {
        let key = entry.key.to_ascii_uppercase();
        if matches!(
            key.as_str(),
            "FILE" | "PATH" | "INCLUDE" | "FILES" | "INCLUDES"
        ) || entry.value.to_ascii_lowercase().ends_with(".ini")
        {
            paths.extend(split_list(&entry.value));
        }
    }
    paths
}

fn resolve_include(source: &Path, include: &str) -> PathBuf {
    // AC 模组常见 Windows 反斜杠；macOS/Linux 的 Path 不会把 `\` 当分隔符。
    let include = include
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .replace('\\', "/");
    let candidate = Path::new(&include);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(candidate)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterialOverride {
    pub source: String,
    pub section: String,
    pub selectors: Vec<String>,
    pub matched: Vec<String>,
    pub shader: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub textures: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureOverride {
    pub source: String,
    pub section: String,
    pub selectors: Vec<String>,
    pub matched: Vec<String>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FxConfigPatch {
    pub module: String,
    pub source: String,
    pub section: String,
    pub values: FxConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct LightDefinition {
    pub name: String,
    pub source: String,
    pub section: String,
    pub values: FxConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParticleDefinition {
    pub name: String,
    pub source: String,
    pub section: String,
    pub values: FxConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationDefinition {
    pub name: String,
    pub source: String,
    pub section: String,
    pub values: FxConfig,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConditionBlock {
    pub name: String,
    pub source: String,
    pub section: String,
    pub values: BTreeMap<String, String>,
    pub supported: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BindingStats {
    pub sections: usize,
    pub supported_sections: usize,
    pub unsupported_sections: usize,
    pub material_overrides: usize,
    pub texture_overrides: usize,
    pub fx_patches: usize,
    pub lights: usize,
    pub particles: usize,
    pub animations: usize,
    pub conditions: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BindingPlan {
    pub material_overrides: Vec<MaterialOverride>,
    pub texture_overrides: Vec<TextureOverride>,
    pub fx_patches: Vec<FxConfigPatch>,
    pub lights: Vec<LightDefinition>,
    pub particles: Vec<ParticleDefinition>,
    pub animations: Vec<AnimationDefinition>,
    pub conditions: Vec<ConditionBlock>,
    pub warnings: Vec<CompatWarning>,
    pub stats: BindingStats,
}

/// 把 ext_config 变成可应用的覆盖/参数计划。
pub struct ExtConfigResolver<'a> {
    materials: &'a [Material],
}

impl<'a> ExtConfigResolver<'a> {
    pub fn new(materials: &'a [Material]) -> Self {
        Self { materials }
    }

    pub fn resolve(&self, document: &ExtConfigDocument) -> BindingPlan {
        let mut plan = BindingPlan::default();
        plan.warnings.extend(document.warnings.clone());
        plan.stats.sections = document.sections.len();

        for section in &document.sections {
            let entries = entry_map(&section.entries);
            let supported = section.kind.supported();
            if supported {
                plan.stats.supported_sections += 1;
            } else {
                plan.stats.unsupported_sections += 1;
            }

            match section.kind {
                ExtConfigKind::ShaderReplacement
                | ExtConfigKind::Material
                | ExtConfigKind::Emissive => {
                    let selectors = selectors(&entries);
                    let matched = self.match_materials(&selectors);
                    let shader = first(&entries, &["SHADER"]).or_else(|| {
                        (section.kind == ExtConfigKind::Emissive).then(|| "ksEmissive".to_owned())
                    });
                    let properties = properties(&entries);
                    plan.material_overrides.push(MaterialOverride {
                        source: section.source.clone(),
                        section: section.name.clone(),
                        selectors,
                        matched,
                        shader,
                        properties,
                        textures: BTreeMap::new(),
                    });
                    plan.stats.material_overrides += 1;
                }
                ExtConfigKind::TextureOverride => {
                    let selectors = selectors(&entries);
                    let matched = self.match_materials(&selectors);
                    let values = entries
                        .iter()
                        .filter(|(key, _)| !is_selector_key(key))
                        .map(|(key, values)| {
                            (key.clone(), values.first().cloned().unwrap_or_default())
                        })
                        .collect::<BTreeMap<_, _>>();
                    plan.texture_overrides.push(TextureOverride {
                        source: section.source.clone(),
                        section: section.name.clone(),
                        selectors,
                        matched,
                        values,
                    });
                    plan.stats.texture_overrides += 1;
                }
                ExtConfigKind::GrassFx => {
                    plan.fx_patches.push(FxConfigPatch {
                        module: "vettura.fx.grass".to_owned(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                    });
                    plan.stats.fx_patches += 1;
                }
                ExtConfigKind::Weather => {
                    plan.fx_patches.push(FxConfigPatch {
                        module: "vettura.fx.weather".to_owned(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                    });
                    plan.stats.fx_patches += 1;
                }
                ExtConfigKind::Lighting => {
                    plan.fx_patches.push(FxConfigPatch {
                        module: "vettura.fx.lighting".to_owned(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                    });
                    plan.stats.fx_patches += 1;
                }
                ExtConfigKind::ExtraLight => {
                    plan.lights.push(LightDefinition {
                        name: section.name.clone(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                    });
                    plan.stats.lights += 1;
                }
                ExtConfigKind::Particles => {
                    plan.particles.push(ParticleDefinition {
                        name: section.name.clone(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                    });
                    plan.stats.particles += 1;
                }
                ExtConfigKind::Animation => {
                    plan.animations.push(AnimationDefinition {
                        name: section.name.clone(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values: entries_to_config(&entries),
                        supported: false,
                    });
                    plan.stats.animations += 1;
                    plan.warnings.push(CompatWarning::warning(
                        section.source.clone(),
                        section.name.clone(),
                        "animation 已解析，但原生动画模块尚未接入",
                    ));
                }
                ExtConfigKind::Condition => {
                    let values = entries
                        .iter()
                        .map(|(key, values)| {
                            (key.clone(), values.first().cloned().unwrap_or_default())
                        })
                        .collect::<BTreeMap<_, _>>();
                    let supported = values.keys().all(|key| is_simple_condition(key));
                    plan.conditions.push(ConditionBlock {
                        name: section.name.clone(),
                        source: section.source.clone(),
                        section: section.name.clone(),
                        values,
                        supported,
                    });
                    plan.stats.conditions += 1;
                    if !supported {
                        plan.warnings.push(CompatWarning::warning(
                            section.source.clone(),
                            section.name.clone(),
                            "condition 含未支持表达式，按静态条件处理",
                        ));
                    }
                }
                ExtConfigKind::Include => {}
                ExtConfigKind::ModelReplacement
                | ExtConfigKind::Driver3d
                | ExtConfigKind::ScriptableDisplay
                | ExtConfigKind::TyresFx
                | ExtConfigKind::Dash
                | ExtConfigKind::Camera => {
                    plan.warnings.push(CompatWarning::warning(
                        section.source.clone(),
                        section.name.clone(),
                        format!(
                            "{} 已识别，但原生模块尚未接入（当前保留数据）",
                            section.kind.label()
                        ),
                    ));
                }
                ExtConfigKind::Physics | ExtConfigKind::Audio | ExtConfigKind::Gui => {
                    plan.warnings.push(CompatWarning::info(
                        section.source.clone(),
                        section.name.clone(),
                        format!("{} 已识别，但不在视觉兼容范围内", section.kind.label()),
                    ));
                }
                ExtConfigKind::Other => {
                    plan.warnings.push(CompatWarning::info(
                        section.source.clone(),
                        section.name.clone(),
                        "未识别的 ext_config section，已忽略",
                    ));
                }
                ExtConfigKind::Unknown => {
                    plan.warnings.push(CompatWarning::warning(
                        section.source.clone(),
                        section.name.clone(),
                        "空/未知 section 名，已忽略",
                    ));
                }
            }
        }

        plan
    }

    fn match_materials(&self, selectors: &[String]) -> Vec<String> {
        if selectors.is_empty() {
            return self
                .materials
                .iter()
                .map(|material| material.name.clone())
                .collect();
        }
        self.materials
            .iter()
            .filter(|material| {
                selectors
                    .iter()
                    .any(|selector| selector_matches(selector, &material.name))
            })
            .map(|material| material.name.clone())
            .collect()
    }
}

fn entry_map(entries: &[ConfigEntry]) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in entries {
        map.entry(entry.key.to_ascii_uppercase())
            .or_default()
            .push(entry.value.clone());
    }
    map
}

fn first(map: &BTreeMap<String, Vec<String>>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(values) = map.get(&key.to_ascii_uppercase()) {
            if let Some(value) = values.first() {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

fn selectors(map: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["MATERIALS", "MESHES"] {
        if let Some(values) = map.get(key) {
            for value in values {
                out.extend(split_list(value));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn properties(map: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, String> {
    map.iter()
        .filter(|(key, _)| {
            key.starts_with("PROP_")
                || matches!(
                    key.as_str(),
                    "BLEND_MODE"
                        | "DEPTH_MODE"
                        | "ALPHA_TESTED"
                        | "CAST_SHADOWS"
                        | "IS_TRANSPARENT"
                )
        })
        .map(|(key, values)| (key.clone(), values.join(",")))
        .collect()
}

fn is_selector_key(key: &str) -> bool {
    matches!(key, "MATERIALS" | "MESHES" | "PROP_..." | "FILE" | "PATH")
}

fn is_simple_condition(key: &str) -> bool {
    matches!(
        key,
        "TIME"
            | "WEATHER"
            | "LAPS"
            | "SESSION"
            | "CAR"
            | "TRACK"
            | "DISTANCE"
            | "SPEED"
            | "POSITION"
            | "WETNESS"
            | "RAIN"
    )
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(|character: char| character == ',' || character.is_whitespace())
        .map(|part| part.trim().trim_matches('"').trim_matches('\''))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn entries_to_config(entries: &BTreeMap<String, Vec<String>>) -> FxConfig {
    entries
        .iter()
        .filter(|(key, _)| !is_selector_key(key))
        .map(|(key, values)| {
            let value = if values.len() == 1 {
                parse_scalar(&values[0])
            } else {
                FxValue::List(values.iter().map(|value| parse_scalar(value)).collect())
            };
            (key.to_ascii_lowercase(), value)
        })
        .collect()
}

fn parse_scalar(value: &str) -> FxValue {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return FxValue::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return FxValue::Bool(false);
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return FxValue::Integer(integer);
    }
    if let Ok(float) = trimmed.parse::<f64>() {
        return FxValue::Float(float);
    }
    if trimmed.contains(',') {
        return FxValue::List(
            split_list(trimmed)
                .into_iter()
                .map(FxValue::String)
                .collect(),
        );
    }
    FxValue::String(trimmed.to_owned())
}

/// 简单 glob：支持 `*` / `?`，大小写不敏感；没有通配符时按“精确或包含”匹配。
pub fn selector_matches(selector: &str, name: &str) -> bool {
    let selector = selector.trim().to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    if selector.is_empty() {
        return false;
    }
    if selector.contains('*') || selector.contains('?') {
        wildcard_match(selector.as_bytes(), name.as_bytes())
    } else {
        name == selector || name.contains(&selector)
    }
}

fn wildcard_match(pattern: &[u8], text: &[u8]) -> bool {
    let mut p = 0;
    let mut t = 0;
    let mut star = None;
    let mut star_match = 0;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            star_match = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            star_match += 1;
            t = star_match;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_kn5::Material;

    fn material(name: &str) -> Material {
        Material {
            name: name.to_owned(),
            shader: "ksPerPixel".to_owned(),
            ..Material::default()
        }
    }

    #[test]
    fn classifies_sections() {
        assert_eq!(
            ExtConfigKind::from_name("SHADER_REPLACEMENT_..."),
            ExtConfigKind::ShaderReplacement
        );
        assert_eq!(
            ExtConfigKind::from_name("TEXTURE_OVERRIDE_0"),
            ExtConfigKind::TextureOverride
        );
        assert_eq!(
            ExtConfigKind::from_name("LIGHTING"),
            ExtConfigKind::Lighting
        );
        assert_eq!(
            ExtConfigKind::from_name("EXTRA_LIGHT_1"),
            ExtConfigKind::ExtraLight
        );
        assert_eq!(
            ExtConfigKind::from_name("PARTICLES_FX"),
            ExtConfigKind::Particles
        );
        assert_eq!(
            ExtConfigKind::from_name("SOMETHING_NEW"),
            ExtConfigKind::Other
        );
    }

    #[test]
    fn resolves_material_lighting_and_warnings() {
        let text = r#"
[SHADER_REPLACEMENT_0]
MATERIALS = body, glass
SHADER = ksPerPixelMultiMap
PROP_0 = ksAmbient, 0.4

[TEXTURE_OVERRIDE_0]
MATERIALS = body
PROP_0 = txDiffuse, red.dds

[LIGHTING]
TEMP = 12
STRENGTH = 0.8

[EXTRA_LIGHT_0]
NAME = headlight
COLOR = 1, 1, 1

[PARTICLES_FX]
SMOKE = 1

[UNKNOWN_THING]
FOO = bar
"#;
        let document = ExtConfigDocument::parse_text("test.ini", text);
        let materials = [material("body"), material("glass"), material("wheel")];
        let plan = ExtConfigResolver::new(&materials).resolve(&document);

        assert_eq!(plan.stats.sections, 6);
        assert_eq!(plan.material_overrides.len(), 1);
        assert_eq!(plan.material_overrides[0].matched, vec!["body", "glass"]);
        assert_eq!(plan.texture_overrides.len(), 1);
        assert_eq!(plan.fx_patches.len(), 1);
        assert_eq!(plan.lights.len(), 1);
        assert_eq!(plan.particles.len(), 1);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.section == "UNKNOWN_THING")
        );
    }

    #[test]
    fn selectors_match_glob_and_contains() {
        assert!(selector_matches("body", "body_red"));
        assert!(selector_matches("body*", "body_red"));
        assert!(selector_matches("body?", "body1"));
        assert!(!selector_matches("body*", "paint_body"));
    }

    #[test]
    fn condition_support_is_reported() {
        let text = r#"
[CONDITION_0]
TIME = 10..18
WEATHER = clear
[CONDITION_1]
COMPLEX_EXPRESSION = a && b
"#;
        let document = ExtConfigDocument::parse_text("test.ini", text);
        let plan = ExtConfigResolver::new(&[]).resolve(&document);
        assert_eq!(plan.conditions.len(), 2);
        assert!(plan.conditions[0].supported);
        assert!(!plan.conditions[1].supported);
    }

    #[test]
    fn parse_text_collects_sections_in_order() {
        let document =
            ExtConfigDocument::parse_text("test.ini", "[LIGHTING]\nA=1\n[PARTICLES_FX]\nB=2\n");
        assert_eq!(document.section_count(), 2);
        assert_eq!(document.sections[0].kind, ExtConfigKind::Lighting);
        assert_eq!(document.sections[1].kind, ExtConfigKind::Particles);
    }

    #[test]
    fn load_tree_follows_includes_and_skips_cycles() {
        let dir =
            std::env::temp_dir().join(format!("vettura-render-ext-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ext_config.ini"),
            "[INCLUDE]\nINCLUDE=common.ini\n[LIGHTING]\nA=1\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("common.ini"),
            "[TEXTURE_OVERRIDE_0]\nMATERIALS=body\n[INCLUDE]\nINCLUDE=ext_config.ini\n",
        )
        .unwrap();

        let document = ExtConfigDocument::load_tree(&dir, ExtConfigOptions::default()).unwrap();
        assert!(document.files.len() >= 2);
        assert!(
            document
                .sections
                .iter()
                .any(|section| section.kind == ExtConfigKind::TextureOverride)
        );
        assert!(
            document
                .warnings
                .iter()
                .any(|warning| warning.message.contains("循环"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
