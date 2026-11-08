//! AC 材质 → Godot `StandardMaterial3D`。
//!
//! 语义参考 Content Manager 的 `Kn5MaterialDark` / `Kn5MaterialDarkMaps`
//! （`AcTools.Render/Kn5SpecificForwardDark/Materials`）：
//!
//! - `txDiffuse` → 漫反射贴图，`txNormal` → 法线，`txMaps` → AO/mask，
//!   `txDetail` / `txNormalDetail` → 细节层；
//! - `ksDiffuse`·`ksAmbient` 合并成 `albedo_color` 的乘法系数；
//! - `ksSpecularEXP` → `roughness = sqrt(2/(exp+2))`；
//! - `alpha_tested` → alpha scissor（阈值 `ksAlphaRef`，默认 0.5）；
//! - `BlendMode::AlphaBlend` → 半透明；`isAdditive == 1` → 叠加混合。
//!
//! AC 专有 shader（树、草、玻璃、发光贴花）暂不特殊处理，统一映射到
//! 标准 PBR 参数上，让 mod 至少能完整显示。

use godot::classes::{StandardMaterial3D, base_material_3d};
use godot::prelude::*;
use vettura_kn5::Material as Kn5Material;

use super::convert::MaterialParams;
use super::texture::{TextureKind, TextureLibrary};

/// 由 KN5 材质创建 Godot 材质。
pub fn create(material: &Kn5Material, textures: &mut TextureLibrary<'_>) -> Gd<StandardMaterial3D> {
    let params = MaterialParams::from_material(material);
    let mut godot_material = StandardMaterial3D::new_gd();
    apply(&mut godot_material, &params, textures);
    godot_material
}

/// 把 [`MaterialParams`] 写到 Godot 材质上（便于测试与复用）。
pub fn apply(
    godot_material: &mut Gd<StandardMaterial3D>,
    params: &MaterialParams,
    textures: &mut TextureLibrary<'_>,
) {
    godot_material.set_albedo(params.albedo);
    godot_material.set_roughness(params.roughness);
    godot_material.set_metallic(params.metallic);
    godot_material.set_specular(params.specular);
    godot_material.set_normal_scale(params.normal_scale);

    // 车漆（`isAdditive == 2`）在 AC 里是单独的清漆层。
    if params.car_paint {
        godot_material.set_clearcoat(1.0);
        godot_material.set_clearcoat_roughness(params.roughness * 0.5);
        godot_material.set_feature(base_material_3d::Feature::CLEARCOAT, true);
    }

    // 法线贴图。
    if let Some(texture) = params
        .textures
        .normal
        .as_deref()
        .and_then(|name| textures.get(name, TextureKind::Normal))
    {
        godot_material.set_feature(base_material_3d::Feature::NORMAL_MAPPING, true);
        godot_material.set_texture(base_material_3d::TextureParam::NORMAL, &texture);
    }
    if let Some(texture) = params
        .textures
        .detail_normal
        .as_deref()
        .and_then(|name| textures.get(name, TextureKind::DetailNormal))
    {
        godot_material.set_feature(base_material_3d::Feature::DETAIL, true);
        godot_material.set_texture(base_material_3d::TextureParam::DETAIL_NORMAL, &texture);
    }

    if let Some(texture) = params
        .textures
        .diffuse
        .as_deref()
        .and_then(|name| textures.get(name, TextureKind::Diffuse))
    {
        godot_material.set_texture(base_material_3d::TextureParam::ALBEDO, &texture);
    }
    if let Some(texture) = params
        .textures
        .maps
        .as_deref()
        .and_then(|name| textures.get(name, TextureKind::Mask))
    {
        godot_material.set_feature(base_material_3d::Feature::AMBIENT_OCCLUSION, true);
        godot_material.set_texture(base_material_3d::TextureParam::AMBIENT_OCCLUSION, &texture);
    }
    if let Some(texture) = params
        .textures
        .detail
        .as_deref()
        .and_then(|name| textures.get(name, TextureKind::Detail))
    {
        godot_material.set_feature(base_material_3d::Feature::DETAIL, true);
        godot_material.set_texture(base_material_3d::TextureParam::DETAIL_ALBEDO, &texture);
    }

    // 自发光。
    godot_material.set_emission(params.emission);
    godot_material.set_feature(base_material_3d::Feature::EMISSION, params.emission_enabled);

    // 混合与裁剪。
    if params.uses_alpha() {
        if params.alpha_tested {
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA_SCISSOR);
            godot_material.set_alpha_scissor_threshold(params.alpha_scissor);
        } else if params.alpha_blended {
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA);
        } else {
            // `BlendMode::AlphaToCoverage`
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA_HASH);
        }
    }

    if params.additive {
        godot_material.set_blend_mode(base_material_3d::BlendMode::ADD);
    }

    // AC 的 depth mode：Off → 关闭深度测试；NoWrite → 只在非透明阶段写深度。
    godot_material.set_flag(
        base_material_3d::Flags::DISABLE_DEPTH_TEST,
        params.depth_test_disabled,
    );
    if params.depth_opaque_only {
        godot_material.set_depth_draw_mode(base_material_3d::DepthDrawMode::OPAQUE_ONLY);
    }
}
