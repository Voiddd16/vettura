//! 材质配方：把 AC 的 shader 名 / 材质名 / 贴图槽映射成原生“材质家族”。
//!
//! 这个模块刻意不依赖 Godot：只输出可测试的 [`ResolvedMaterial`]。
//! `gdext` 负责把配方写成 `StandardMaterial3D` / `ShaderMaterial`，CLI 负责
//! 统计覆盖率与未知 shader。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vettura_kn5::{DepthMode, Material};

/// 原生材质家族。不是完整的 shader 枚举，而是“行为相近”的分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialFamily {
    #[default]
    Pbr,
    MultiMap,
    Reflective,
    Glass,
    Emissive,
    Foliage,
    Transparent,
    CarPaint,
    BrakeDisc,
    Decal,
    Unknown,
}

impl MaterialFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pbr => "pbr",
            Self::MultiMap => "multi_map",
            Self::Reflective => "reflective",
            Self::Glass => "glass",
            Self::Emissive => "emissive",
            Self::Foliage => "foliage",
            Self::Transparent => "transparent",
            Self::CarPaint => "car_paint",
            Self::BrakeDisc => "brake_disc",
            Self::Decal => "decal",
            Self::Unknown => "unknown",
        }
    }
}

/// 一条配方的匹配条件。每个字段内部是 OR，字段之间是 AND。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MaterialMatch {
    #[serde(default)]
    pub shader: Vec<String>,
    #[serde(default)]
    pub shader_contains: Vec<String>,
    #[serde(default)]
    pub material_contains: Vec<String>,
    #[serde(default)]
    pub material_starts_with: Vec<String>,
    #[serde(default)]
    pub has_texture: Vec<String>,
    #[serde(default)]
    pub has_property: Vec<String>,
}

impl MaterialMatch {
    fn matches(&self, material: &Material) -> bool {
        if !self.shader.is_empty()
            && !self
                .shader
                .iter()
                .any(|want| material.shader.eq_ignore_ascii_case(want))
        {
            return false;
        }

        let shader_lower = material.shader.to_ascii_lowercase();
        if !self.shader_contains.is_empty()
            && !self
                .shader_contains
                .iter()
                .any(|want| shader_lower.contains(&want.to_ascii_lowercase()))
        {
            return false;
        }

        let name_lower = material.name.to_ascii_lowercase();
        if !self.material_contains.is_empty()
            && !self
                .material_contains
                .iter()
                .any(|want| name_lower.contains(&want.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.material_starts_with.is_empty()
            && !self
                .material_starts_with
                .iter()
                .any(|want| name_lower.starts_with(&want.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.has_texture.is_empty()
            && !self
                .has_texture
                .iter()
                .any(|slot| material.texture(slot).is_some())
        {
            return false;
        }

        if !self.has_property.is_empty()
            && !self
                .has_property
                .iter()
                .any(|property| material.property(property).is_some())
        {
            return false;
        }

        true
    }
}

/// 一条数据驱动的材质配方。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MaterialRecipe {
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub family: MaterialFamily,
    #[serde(default, rename = "match")]
    pub matcher: MaterialMatch,
    #[serde(default)]
    pub textures: BTreeMap<String, String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, f32>,
    /// 兜底配方：只在没有其它配方命中时使用。
    #[serde(default)]
    pub fallback: bool,
}

impl MaterialRecipe {
    fn matches(&self, material: &Material) -> bool {
        !self.fallback && self.matcher.matches(material)
    }
}

/// 单条材质的解析结果。
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedMaterial {
    pub material: String,
    pub shader: String,
    pub recipe_id: String,
    pub family: MaterialFamily,
    /// `false` 表示命中了 fallback（未知 shader/材质）。
    pub known: bool,
    pub blend_alpha: bool,
    pub alpha_tested: bool,
    pub depth_off: bool,
    pub textures: BTreeMap<String, String>,
    pub parameters: BTreeMap<String, f32>,
    pub warnings: Vec<String>,
}

/// 一个模型/目录的材质覆盖率统计。
#[derive(Debug, Clone, Default, Serialize)]
pub struct MaterialCoverage {
    pub total: usize,
    /// 命中非 fallback 配方的数量。
    pub mapped: usize,
    /// 命中 fallback 的数量。
    pub unknown: usize,
    pub by_family: BTreeMap<String, usize>,
    pub unknown_shaders: BTreeMap<String, usize>,
    pub unknown_materials: Vec<String>,
}

impl MaterialCoverage {
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.mapped as f32 / self.total as f32
        }
    }

    fn record(&mut self, resolved: &ResolvedMaterial) {
        self.total += 1;
        *self
            .by_family
            .entry(resolved.family.label().to_owned())
            .or_default() += 1;
        if resolved.known {
            self.mapped += 1;
        } else {
            self.unknown += 1;
            *self
                .unknown_shaders
                .entry(resolved.shader.clone())
                .or_default() += 1;
            self.unknown_materials.push(resolved.material.clone());
        }
    }
}

/// 内置配方库。
#[derive(Debug, Clone)]
pub struct MaterialRegistry {
    recipes: Vec<MaterialRecipe>,
    fallback: MaterialRecipe,
}

const BUILTIN_RECIPES: &str = include_str!("../data/recipes.toml");

impl Default for MaterialRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl MaterialRegistry {
    /// 使用内置 `data/recipes.toml`。
    pub fn builtin() -> Self {
        #[derive(Deserialize)]
        struct RecipeFile {
            recipe: Vec<MaterialRecipe>,
        }

        let file: RecipeFile =
            toml::from_str(BUILTIN_RECIPES).expect("render/data/recipes.toml 必须是合法 TOML");
        Self::from_recipes(file.recipe)
    }

    /// 从一组配方构造；自动排序并提取 fallback。
    pub fn from_recipes(mut recipes: Vec<MaterialRecipe>) -> Self {
        recipes.sort_by(|a, b| b.priority.cmp(&a.priority));
        let fallback = recipes
            .iter()
            .find(|recipe| recipe.fallback)
            .cloned()
            .unwrap_or_else(|| MaterialRecipe {
                id: "vettura.core.fallback".to_owned(),
                priority: i32::MIN,
                family: MaterialFamily::Unknown,
                matcher: MaterialMatch::default(),
                textures: BTreeMap::new(),
                parameters: BTreeMap::new(),
                fallback: true,
            });
        Self { recipes, fallback }
    }

    pub fn recipes(&self) -> &[MaterialRecipe] {
        &self.recipes
    }

    /// 解析一条 KN5 材质。
    pub fn resolve(&self, material: &Material) -> ResolvedMaterial {
        let recipe = self
            .recipes
            .iter()
            .find(|recipe| recipe.matches(material))
            .unwrap_or(&self.fallback);

        let mut warnings = Vec::new();
        if recipe.fallback {
            warnings.push(format!(
                "未知 shader '{}'，使用 fallback 配方",
                material.shader
            ));
        }

        ResolvedMaterial {
            material: material.name.clone(),
            shader: material.shader.clone(),
            recipe_id: recipe.id.clone(),
            family: recipe.family,
            known: !recipe.fallback,
            blend_alpha: material.blend_mode == vettura_kn5::BlendMode::AlphaBlend,
            alpha_tested: material.alpha_tested,
            depth_off: matches!(material.depth_mode, DepthMode::Off | DepthMode::Unknown(_)),
            textures: recipe.textures.clone(),
            parameters: recipe.parameters.clone(),
            warnings,
        }
    }

    /// 批量统计。
    pub fn coverage<'a>(
        &self,
        materials: impl IntoIterator<Item = &'a Material>,
    ) -> MaterialCoverage {
        let mut coverage = MaterialCoverage::default();
        for material in materials {
            let resolved = self.resolve(material);
            coverage.record(&resolved);
        }
        coverage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_kn5::{MaterialProperty, TextureSample};

    fn material(shader: &str, name: &str) -> Material {
        Material {
            name: name.to_owned(),
            shader: shader.to_owned(),
            ..Material::default()
        }
    }

    #[test]
    fn builtin_registry_parses() {
        let registry = MaterialRegistry::builtin();
        assert!(registry.recipes().len() >= 10);
        assert!(registry.recipes().iter().any(|recipe| recipe.fallback));
    }

    #[test]
    fn resolves_known_and_unknown() {
        let registry = MaterialRegistry::builtin();
        let known = registry.resolve(&material("ksPerPixelMultiMap", "body"));
        assert!(known.known);
        assert_eq!(known.family, MaterialFamily::MultiMap);
        assert_eq!(known.recipe_id, "vettura.core.pbr_multimap");

        let unknown = registry.resolve(&material("ksTotallyCustom", "mystery"));
        assert!(!unknown.known);
        assert_eq!(unknown.recipe_id, "vettura.core.fallback");
        assert_eq!(unknown.family, MaterialFamily::Unknown);
    }

    #[test]
    fn car_paint_requires_is_additive_property() {
        let mut paint = material("ksPerPixel", "paint");
        paint.properties.push(MaterialProperty {
            name: "isAdditive".to_owned(),
            value_a: 2.0,
            value_b: [0.0; 2],
            value_c: [0.0; 3],
            value_d: [0.0; 4],
        });
        let registry = MaterialRegistry::builtin();
        assert_eq!(registry.resolve(&paint).family, MaterialFamily::CarPaint);
        assert_eq!(
            registry.resolve(&material("ksPerPixel", "plain")).family,
            MaterialFamily::Pbr
        );
    }

    #[test]
    fn texture_slot_matcher_works() {
        let mut mat = material("ksPerPixel", "detail_only");
        mat.samples.push(TextureSample {
            slot_name: "txDetail".to_owned(),
            slot: 0,
            texture: "detail.dds".to_owned(),
        });
        let mut recipes = MaterialRegistry::builtin()
            .recipes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        recipes.push(MaterialRecipe {
            id: "test.detail".to_owned(),
            priority: 999,
            family: MaterialFamily::CarPaint,
            matcher: MaterialMatch {
                has_texture: vec!["txDetail".to_owned()],
                ..MaterialMatch::default()
            },
            textures: BTreeMap::new(),
            parameters: BTreeMap::new(),
            fallback: false,
        });
        let registry = MaterialRegistry::from_recipes(recipes);
        assert_eq!(registry.resolve(&mat).recipe_id, "test.detail");
    }

    #[test]
    fn coverage_counts_unknown_shaders() {
        let registry = MaterialRegistry::builtin();
        let materials = [
            material("ksPerPixel", "a"),
            material("ksPerPixelMultiMap", "b"),
            material("ksUnknownOne", "c"),
            material("ksUnknownOne", "d"),
        ];
        let coverage = registry.coverage(materials.iter());
        assert_eq!(coverage.total, 4);
        assert_eq!(coverage.mapped, 2);
        assert_eq!(coverage.unknown, 2);
        assert_eq!(coverage.unknown_shaders.get("ksUnknownOne"), Some(&2));
    }
}
