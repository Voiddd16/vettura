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

use godot::classes::{RenderingServer, StandardMaterial3D, base_material_3d};
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

    // 介电高光强度（Godot 4.7 的真实属性名是 `metallic_specular`，旧名
    // `specular` 的 setter 在这版不写到它）。AC 的赛道反射类材质——
    // `ksPerPixelReflection`（护栏玻璃/看台墙/水面）和 `ksMultilayer_fresnel_nm`
    // （沥青/湿路面）——在 AC 里靠 envmap + fresnel 边缘反射表现，强度被精心
    // 控制；我们没有 AC 的 envmap，Godot 用默认天空 cubemap + 直射光近似，
    // 低粗糙度路面会把日光反射成一整片纯白高光（起跑区右侧路面发白就是这个）。
    // 对这类材质关掉介电高光，保留 albedo/环境色，路面恢复正常的漫反射灰。
    // 车漆（clearcoat）和其它普通材质不受影响。
    if params.specular_suppressed {
        set_property(godot_material, "metallic_specular", 0.0f32.to_variant());
        // 金属材质（reflective → metallic 0.85）的高光来自金属流程，
        // metallic_specular 管不到；它的粗糙度若被 AC 的高 ksSpecularEXP 压到
        // 0.09，会把天空反射成一整片纯白。给这类材质设粗糙度下限 0.3。
        if params.metallic > 0.5 && params.roughness < 0.3 {
            godot_material.set_roughness(0.3);
        }
    }

    // 车漆（`isAdditive == 2`）在 AC 里是单独的清漆层。带天空/云层的 IBL 之后
    // 清漆可以比旧版更明显：0.5 的 clearcoat + 0.06~0.1 的粗糙度，反射里
    // 能看到天光和云的结构，又不会把红色车漆刷成白车。
    if params.car_paint {
        godot_material.set_clearcoat(0.3);
        godot_material.set_clearcoat_roughness(params.roughness.max(0.12));
        godot_material.set_feature(base_material_3d::Feature::CLEARCOAT, true);
    }

    // 树叶/草/旗帜：单面卡片在 KN5 里很常见，双面渲染避免转视角时整片消失。
    if params.foliage {
        godot_material.set_cull_mode(base_material_3d::CullMode::DISABLED);
    }

    // 车窗/挡风玻璃：Forward+ 下开屏幕空间折射，玻璃后面的赛道会有轻微
    // 位移，不再是“一大块均匀 alpha”；Mobile 没有折射，保持纯半透明。
    if params.glass {
        godot_material.set_metallic(0.0);
        godot_material.set_roughness(params.roughness.min(0.12));
        if is_forward_plus() {
            godot_material.set_feature(base_material_3d::Feature::REFRACTION, true);
            godot_material.set_refraction(0.035);
        }
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

    // 自发光。`ksEmissive` 类 shader 会把 txDiffuse/txEmissive 接进 EMISSION，
    // 车灯/屏幕/招牌保留贴图细节；普通材质的 ksEmissive 仍然只是纯色发光。
    godot_material.set_emission(params.emission);
    godot_material.set_emission_energy_multiplier(params.emission_energy);
    godot_material.set_feature(base_material_3d::Feature::EMISSION, params.emission_enabled);
    if let Some(texture) = params
        .emission_texture
        .as_deref()
        .and_then(|name| cache.texture_by_name(name, TextureKind::Diffuse))
    {
        godot_material.set_texture(base_material_3d::TextureParam::EMISSION, &texture);
        godot_material.set_emission_operator(base_material_3d::EmissionOperator::MULTIPLY);
    }

    // 混合与裁剪。
    if params.uses_alpha() {
        if params.alpha_tested {
            // 树叶的 `ksAlphaRef` 常写 0.1（尼龙/松针保留更多细节），但 0.1 的
            // scissor 在 MetalFX Temporal 的抖动下会让细叶边缘持续“闪”。给
            // foliage 一个 0.4 的下限，牺牲少量针叶密度换稳定的画面。
            let threshold = if params.foliage {
                params.alpha_scissor.max(0.4)
            } else {
                params.alpha_scissor
            };
            godot_material.set_transparency(base_material_3d::Transparency::ALPHA_SCISSOR);
            godot_material.set_alpha_scissor_threshold(threshold);
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

/// 当前是否 Forward+ 桌面后端（折射/屏幕空间效果可用）。
fn is_forward_plus() -> bool {
    RenderingServer::singleton()
        .get_current_rendering_method()
        .to_string()
        == "forward_plus"
}
