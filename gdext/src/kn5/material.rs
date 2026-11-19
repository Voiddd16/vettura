//! AC 材质 → Godot `StandardMaterial3D`。
//!
//! 语义参考 Content Manager 的 `Kn5MaterialDark` / `Kn5MaterialDarkMaps`
//! （`AcTools.Render/Kn5SpecificForwardDark/Materials`）：
//!
//! - `txDiffuse` → 漫反射贴图，`txNormal` → 法线，`txMaps` → AO/mask，
//!   `txDetail` / `txNormalDetail` → AC 的细节层（乘进漫反射，**车漆的红色
//!   涂装就来自这里**，倍率 `detailUVMultiplier` 写进 `uv2_scale`）；
//! - `ksDiffuse`·`ksAmbient` 合并成 `albedo_color` 的乘法系数；
//! - `ksSpecularEXP` → `roughness = sqrt(2/(exp+2))`；
//! - `alpha_tested` → alpha scissor（阈值 `ksAlphaRef`，默认 0.5）；
//! - `BlendMode::AlphaBlend` 本身**不**代表半透明（见 `is_alpha_blended`），
//!   `isAdditive == 2`（非轮胎）→ 清漆层。
//!
//! 细节层必须走 `UV2`：Godot 的 detail 采样第二套 UV，而 AC 的倍率换算成
//! `uv2_scale`；`super::mesh` 会给带细节材质的模型补一份 `TEX_UV2`（= UV1）。
//!
//! 刻意不映射的：`isAdditive == 1`（AC 只在镜面通道做叠加；整个表面改成 ADD
//! 混合会把沥青看台、碳纤维这类深色 albedo 的画面搅成黑色剪影并互相闪烁）。
//!
//! AC 专有 shader（树、草、玻璃、发光贴花）暂不特殊处理，统一映射到
//! 标准 PBR 参数上，让 mod 至少能完整显示。

use godot::classes::{StandardMaterial3D, base_material_3d};
use godot::prelude::*;
use vettura_kn5::Material as Kn5Material;

use super::cache::FileResources;
use super::convert::MaterialParams;
use super::texture::TextureKind;

/// 动态写一个材质属性（gdext 绑定里没有的 4.7 新属性走这里）。
///
/// 本仓库的 gdext 还没生成 `detail_blend_mode` / `detail_uv_layer`，但引擎
/// （4.7）认这两个属性；写不进去时 Godot 只在控制台留一条错误、材质退回默认
/// （Mix 混合、UV2），比编译不过去强。
fn set_property(godot_material: &mut Gd<StandardMaterial3D>, name: &str, value: Variant) {
    let name = StringName::from(name);
    godot_material.set(&name, &value);
}

/// 由 KN5 材质创建 Godot 材质。
///
/// 材质会经 [`FileResources::material`] 按材质下标缓存，所以同一份模型里的
/// 所有网格节点共享一份 `StandardMaterial3D`——这正是 Godot 常规用法
/// （导入的资源本来就是共享的），既省内存也省掉重复的属性写入。
pub fn create(material: &Kn5Material, cache: &FileResources) -> Gd<StandardMaterial3D> {
    let params = MaterialParams::from_material(material);
    let mut godot_material = StandardMaterial3D::new_gd();
    // 保留 KN5 材质名：编辑器/调试里能看出是哪张材质出了问题。
    godot_material.set_name(&GString::from(material.name.as_str()));
    apply(&mut godot_material, &params, cache);
    godot_material
}

/// 把 [`MaterialParams`] 写到 Godot 材质上（便于测试与复用）。
pub fn apply(
    godot_material: &mut Gd<StandardMaterial3D>,
    params: &MaterialParams,
    cache: &FileResources,
) {
    godot_material.set_albedo(params.albedo);
    godot_material.set_roughness(params.roughness);
    godot_material.set_metallic(params.metallic);
    godot_material.set_specular(params.specular);
    godot_material.set_normal_scale(params.normal_scale);

    // 车漆（`isAdditive == 2`）在 AC 里是单独的清漆层。不要用 1.0：清漆的
    // 高光会把环境光的反射整片刷成白色，红色车漆在大白天直接变白车。
    // 0.35 + 0.1 粗糙度≈一层薄蜡，够亮而不抢走底色。
    if params.car_paint {
        godot_material.set_clearcoat(0.35);
        godot_material.set_clearcoat_roughness(params.roughness.max(0.1));
        godot_material.set_feature(base_material_3d::Feature::CLEARCOAT, true);
    }

    // 法线贴图。
    if let Some(texture) = params
        .textures
        .normal
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::Normal))
    {
        godot_material.set_feature(base_material_3d::Feature::NORMAL_MAPPING, true);
        godot_material.set_texture(base_material_3d::TextureParam::NORMAL, &texture);
    }

    // 细节层：AC 是 `albedo *= detail(uv * detailUVMultiplier)`，Godot 的
    // MULTIPLY 混合 + `uv2_scale` 正好等价。没有 UV2 的模型（`needs_uv2` 为
    // false）接上去只会把整张贴图糊成一个颜色，所以由 `detail_uv_scale > 0`
    // 与网格侧是否生成 TEX_UV2 共同把关。
    //
    // `detail_blend_mode` / `detail_uv_layer` 这两个属性 Godot 4.7 有，但本仓库
    // 的 gdext 绑定版本还没生成它们，所以走动态属性写入（不存在的属性会返回
    // Error，这里显式忽略：老引擎没有 MULTIPLY 时退回默认 Mix 也只是细节层略亮）。
    let detail_albedo = params
        .textures
        .detail
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::Detail));
    let detail_normal = params
        .textures
        .detail_normal
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::DetailNormal));

    if params.detail_uv_scale > 0.0 && (detail_albedo.is_some() || detail_normal.is_some()) {
        godot_material.set_feature(base_material_3d::Feature::DETAIL, true);
        set_property(godot_material, "detail_blend_mode", 3i32.to_variant());
        set_property(godot_material, "detail_uv_layer", 0i32.to_variant());
        set_property(
            godot_material,
            "uv2_scale",
            Vector2::new(params.detail_uv_scale, params.detail_uv_scale).to_variant(),
        );
        if let Some(texture) = detail_albedo {
            godot_material.set_texture(base_material_3d::TextureParam::DETAIL_ALBEDO, &texture);
        }
        if let Some(texture) = detail_normal {
            godot_material.set_texture(base_material_3d::TextureParam::DETAIL_NORMAL, &texture);
        }
    }

    if let Some(texture) = params
        .textures
        .diffuse
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::Diffuse))
    {
        godot_material.set_texture(base_material_3d::TextureParam::ALBEDO, &texture);
    }
    if let Some(texture) = params
        .textures
        .maps
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::Mask))
    {
        godot_material.set_feature(base_material_3d::Feature::AMBIENT_OCCLUSION, true);
        godot_material.set_texture(base_material_3d::TextureParam::AMBIENT_OCCLUSION, &texture);
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
            // AC 的半透明材质多数不写深度（人群、树冠），跟着关掉深度写入，
            // 否则同一网格的多个面会互相排序闪烁。
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA);
            godot_material.set_depth_draw_mode(base_material_3d::DepthDrawMode::DISABLED);
        } else {
            // `BlendMode::AlphaToCoverage`
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA_HASH);
        }
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
