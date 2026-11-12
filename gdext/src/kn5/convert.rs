//! KN5 → Godot 的纯数据转换。
//!
//! 这个文件刻意只包含不依赖引擎运行时的逻辑：`glam` 运算、`Basis` /
//! `Transform3D` / `Color` / `Vector3` 都是普通 `#[repr(C)]` 结构体，可以像
//! POD 一样直接构造。因此这里的所有函数都能在 `cargo test -p vettura-godot`
//! 里验证，不需要启动 Godot。
//!
//! 真正调用引擎的代码（`ArrayMesh`、贴图解码、节点创建）放在
//! [`super::mesh`]、[`super::material`]、[`super::scene`]。

use std::collections::HashSet;

use glam::{Mat4, Vec4 as GlamVec4};
use godot::builtin::{Basis, Color, Transform3D, Vector2, Vector3};
use vettura_kn5::{BlendMode, DepthMode, Material, Transform, transform_to_mat4};

/// AC 帧 → Godot 帧的轴镜像：`(x, y, z) → (-x, y, -z)`。
///
/// 等价于绕 Y 轴旋转 180°，是**正常旋转**（行列式 `+1`）：对顶点/法线/切线
/// 只需乘这一个 `S`（见 [`super::mesh`]），不需要翻转绕序或手性；和
/// `vettura_core::coords::ac_to_godot` 对 vec3 的定义保持一致。
///
/// 注意这与 AcTools 的做法不同：Content Manager 渲染 KN5 时把 DirectX 的
/// 左手空间当作目标，只镜像 X（行列式 `-1`），所以必须用 `ToIndicesFixX()`
/// 倒转每个三角形的绕序。Godot 是右手系，我们同时镜像 X 和 Z，绕序保持原样。
pub const AC_TO_GODOT: Mat4 = Mat4::from_diagonal(GlamVec4::new(-1.0, 1.0, -1.0, 1.0));

/// 把整个 4x4 变换从 AC 帧转换到 Godot 帧：`S · M · S`（`S` 为轴镜像）。
pub fn ac_to_godot_matrix(matrix: &Mat4) -> Mat4 {
    AC_TO_GODOT * *matrix * AC_TO_GODOT
}

/// KN5 行主序局部变换 → Godot `Transform3D`。
pub fn kn5_transform_to_godot(transform: &Transform) -> Transform3D {
    matrix_to_transform3d(ac_to_godot_matrix(&transform_to_mat4(transform)))
}

/// `glam::Mat4` → Godot `Transform3D`（前三列为基向量，第四列为原点）。
pub fn matrix_to_transform3d(matrix: Mat4) -> Transform3D {
    let column = |index: usize| {
        let value = matrix.col(index);
        Vector3::new(value.x, value.y, value.z)
    };

    Transform3D {
        basis: Basis::from_cols(column(0), column(1), column(2)),
        origin: column(3),
    }
}

/// KN5 点/方向 → Godot 空间，用于把 `WHEEL_LF` 之类 dummy 的位置送去物理层。
pub fn ac_to_godot_point(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3::new(-x, y, -z)
}

/// Godot 节点名不允许出现路径分隔符和其它保留字符（`:` 和 `.` 会被引擎自己换成
/// `_`，`@name` 是编辑器里的“场景唯一名”约定），这里统一替换掉，保证拿到手的就是
/// 最终名字。
pub fn sanitize_node_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if matches!(
                c,
                '/' | '\\' | ':' | '.' | '%' | '@' | '\0' | '\n' | '\r' | '\t'
            ) {
                '_'
            } else {
                c
            }
        })
        .collect();

    if cleaned.trim().is_empty() {
        "node".to_owned()
    } else {
        cleaned
    }
}

/// 在兄弟节点集合里给 `base` 找一个唯一名字。
///
/// KN5 里父级 dummy 与子网格同名很常见（`F2008_BBWING_L` / `F2008_BBWING_L`），
/// 直接塞进 Godot 会让 `get_node()` 产生歧义，所以追加 `_2`、`_3` 后缀。
pub fn unique_name(base: &str, taken: &mut HashSet<String>) -> String {
    if taken.insert(base.to_owned()) {
        return base.to_owned();
    }

    let mut suffix = 2;
    loop {
        let candidate = format!("{base}_{suffix}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

/// KN5 材质 → “后端无关” 的中间表示。
///
/// 之所以再拆一层，是为了把 AC shader 的语义（`ksDiffuse`、`ksSpecularEXP` …）
/// 与 Godot 属性写调用分开，前者可以在普通单元测试里断言，后者见
/// [`super::material`]。
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParams {
    /// 乘法色调；AC 的 `ksDiffuse` 是标量，Godot 的 `albedo_color` 会再乘一次贴图。
    pub albedo: Color,
    /// 由 `ksSpecularEXP` 派生，`roughness = sqrt(2 / (exp + 2))`。
    pub roughness: f32,
    pub metallic: f32,
    pub specular: f32,
    pub normal_scale: f32,
    /// `ksEmissive` 的 ValueC（RGB）。
    pub emission: Color,
    pub emission_enabled: bool,
    /// `ksAlphaRef`；`alpha_tested` 时 Godot 走 alpha scissor。
    pub alpha_scissor: f32,
    pub alpha_tested: bool,
    /// `BlendMode::AlphaBlend` → 半透明混合。
    pub alpha_blended: bool,
    /// `BlendMode::AlphaToCoverage`。
    pub alpha_to_coverage: bool,
    /// `isAdditive == 1` → 叠加混合；`isAdditive == 2` 在 AC 里表示车漆。
    pub additive: bool,
    pub car_paint: bool,
    /// `DepthMode::Off` / `NoWrite`。
    pub depth_test_disabled: bool,
    pub depth_opaque_only: bool,
    pub textures: MaterialTextures,
}

/// 材质用到的贴图槽（存贴图名，解码在 [`super::texture`]）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MaterialTextures {
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    /// `txMaps`：AC 把 AO / mask 打包在一起，这里按环境光遮蔽使用。
    pub maps: Option<String>,
    pub detail: Option<String>,
    pub detail_normal: Option<String>,
}

impl MaterialParams {
    /// 从 KN5 材质推导参数。
    ///
    /// 缺失的属性一律回退到“合理默认 + `None` 槽”，让缺字段的 mod 仍能显示。
    /// 语义参考 Content Manager 的 `Kn5MaterialDark` / `Kn5MaterialDarkMaps`
    /// （`AcTools.Render/Kn5SpecificForwardDark/Materials`）。
    pub fn from_material(material: &Material) -> Self {
        let diffuse = material
            .property("ksDiffuse")
            .unwrap_or(1.0)
            .clamp(0.0, 4.0);
        let ambient = material.property("ksAmbient").unwrap_or(0.0);
        // AC 的最终结果是 `diffuse·ambient·texture`；Godot 只有一路乘法，
        // 把两者合并进 albedo_color。
        let albedo_scale = (diffuse * (ambient.max(0.0) + 1.0) * 0.5).clamp(0.0, 4.0);

        let opacity = material.property("ksOpacity");
        let emission_values = material.property_values("ksEmissive");
        let emission_rgb = emission_values
            .map(|value| value.value_c)
            .unwrap_or([0.0; 3]);

        let alpha_scissor = material
            .property("ksAlphaRef")
            .filter(|value| *value > 0.0)
            // AcTools 的 AlphaTest 用固定 0.5。
            .unwrap_or(0.5);

        Self {
            albedo: Color::from_rgba(
                albedo_scale,
                albedo_scale,
                albedo_scale,
                opacity.unwrap_or(1.0).clamp(0.0, 1.0),
            ),
            roughness: roughness_from_specular_exp(material.property("ksSpecularEXP")),
            metallic: 0.0,
            specular: material.property("ksSpecular").unwrap_or(0.5),
            normal_scale: material.property("normalMult").unwrap_or(1.0).max(0.0),
            emission: Color::from_rgb(emission_rgb[0], emission_rgb[1], emission_rgb[2]),
            emission_enabled: emission_rgb.iter().any(|channel| *channel > 0.0),
            alpha_scissor,
            alpha_tested: material.alpha_tested,
            alpha_blended: material.blend_mode == BlendMode::AlphaBlend,
            alpha_to_coverage: material.blend_mode == BlendMode::AlphaToCoverage,
            additive: property_is_additive(material),
            car_paint: material.property("isAdditive") == Some(2.0),
            depth_test_disabled: material.depth_mode == DepthMode::Off,
            depth_opaque_only: material.depth_mode == DepthMode::NoWrite,
            textures: MaterialTextures {
                diffuse: texture_slot(material, "txDiffuse"),
                normal: texture_slot(material, "txNormal"),
                maps: texture_slot(material, "txMaps"),
                detail: texture_slot(material, "txDetail"),
                detail_normal: texture_slot(material, "txNormalDetail"),
            },
        }
    }

    /// 是否需要 alpha 通道参与混合（透明）或裁剪（alpha test）。
    pub fn uses_alpha(&self) -> bool {
        self.alpha_blended || self.alpha_to_coverage || self.alpha_tested
    }
}

/// AC 的 `isAdditive`：0/缺失 = 普通，1 = 叠加，2 = 车漆。
fn property_is_additive(material: &Material) -> bool {
    material
        .property("isAdditive")
        .is_some_and(|value| value != 0.0 && value != 2.0)
}

/// `ksSpecularEXP` 越大约镜面越锐利；用常见的 `roughness = sqrt(2/(exp+2))`
/// 近似换算，并夹到 `[0.03, 1.0]` 避免除零和过硬的材质。
pub fn roughness_from_specular_exp(exp: Option<f32>) -> f32 {
    let Some(exp) = exp.filter(|value| value.is_finite()) else {
        return 0.5;
    };
    if exp <= 1.0 {
        return 1.0;
    }

    (2.0 / (exp + 2.0)).sqrt().clamp(0.03, 1.0)
}

fn texture_slot(material: &Material, slot: &str) -> Option<String> {
    material
        .texture(slot)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
}

/// 从 KN5 顶点 UV 生成 Godot 的 `Vector2`。
pub fn uv_to_godot(uv: [f32; 2]) -> Vector2 {
    Vector2::new(uv[0], uv[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_kn5::{MaterialProperty, TextureSample};

    fn identity() -> Transform {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// KN5 行主序变换的平移在最后一行。
    fn translation(x: f32, y: f32, z: f32) -> Transform {
        let mut transform = identity();
        transform[3][0] = x;
        transform[3][1] = y;
        transform[3][2] = z;
        transform
    }

    /// F1 2008 试样里 `WHEEL_LF` 的位置，见 `docs/kn5-format.md`。
    #[test]
    fn converts_wheel_dummy_position() {
        let godot = kn5_transform_to_godot(&translation(0.711, 0.301, 1.526));
        // +X 左 → -X 左；+Z 前 → -Z 前。
        assert_eq!(godot.origin, Vector3::new(-0.711, 0.301, -1.526));
        assert_eq!(godot.basis, Basis::IDENTITY);
    }

    #[test]
    fn mirror_matrix_is_a_proper_rotation() {
        // 镜像 X 与 Z 等价于绕 Y 旋转 180°，行列式 +1：绕序和法线不必翻转。
        assert!((AC_TO_GODOT.determinant() - 1.0).abs() < 1e-5);

        // 对仿射矩阵，S·M·S 的效果就是把平移镜像：S·t。
        let mirrored = ac_to_godot_matrix(&transform_to_mat4(&translation(0.711, 0.301, 1.526)));
        assert_eq!(mirrored.w_axis, GlamVec4::new(-0.711, 0.301, -1.526, 1.0));
        // 镜像自逆：再转一次回到原值。
        assert_eq!(
            ac_to_godot_matrix(&mirrored),
            transform_to_mat4(&translation(0.711, 0.301, 1.526))
        );
    }

    #[test]
    fn point_conversion_matches_vettura_core() {
        let converted = ac_to_godot_point(1.0, 2.0, 3.0);
        assert_eq!(converted, Vector3::new(-1.0, 2.0, -3.0));
    }

    #[test]
    fn node_names_are_sanitized_and_unique() {
        let mut taken = HashSet::new();

        assert_eq!(sanitize_node_name("F2008_BBWING/left"), "F2008_BBWING_left");
        assert_eq!(sanitize_node_name("  "), "node");
        assert_eq!(sanitize_node_name("FBX: untitled.fbx"), "FBX_ untitled_fbx");

        assert_eq!(unique_name("body", &mut taken), "body");
        assert_eq!(unique_name("body", &mut taken), "body_2");
        assert_eq!(unique_name("body", &mut taken), "body_3");
        assert_eq!(unique_name("wing", &mut taken), "wing");
    }

    #[test]
    fn specular_exp_maps_to_a_sane_roughness() {
        assert_eq!(roughness_from_specular_exp(None), 0.5);
        assert_eq!(roughness_from_specular_exp(Some(0.0)), 1.0);

        let shiny = roughness_from_specular_exp(Some(2000.0));
        let matte = roughness_from_specular_exp(Some(90.0));
        assert!(shiny < matte);
        assert!((0.03..=1.0).contains(&shiny));
        assert!((0.03..=1.0).contains(&matte));
    }

    fn material_with(properties: &[(&str, f32)], samples: &[(&str, &str)]) -> Material {
        Material {
            name: "test".to_owned(),
            shader: "ksPerPixelMultiMap".to_owned(),
            blend_mode: BlendMode::Opaque,
            alpha_tested: false,
            depth_mode: DepthMode::Normal,
            properties: properties
                .iter()
                .map(|(name, value)| MaterialProperty {
                    name: (*name).to_owned(),
                    value_a: *value,
                    value_b: [0.0; 2],
                    value_c: [0.0; 3],
                    value_d: [0.0; 4],
                })
                .collect(),
            samples: samples
                .iter()
                .map(|(slot, texture)| TextureSample {
                    slot_name: (*slot).to_owned(),
                    slot: 0,
                    texture: (*texture).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn material_params_map_ac_shader_properties() {
        // 与 F1 2008 试样里的 `Suspcar` 材质一致。
        let material = material_with(
            &[
                ("ksAmbient", 0.4),
                ("ksDiffuse", 0.4),
                ("ksSpecular", 0.4),
                ("ksSpecularEXP", 200.0),
                ("ksAlphaRef", 0.0),
                ("isAdditive", 2.0),
            ],
            &[
                ("txDiffuse", "f2008_suspcar_diffuse.dds"),
                ("txNormal", "suspcar_nm.dds"),
                ("txMaps", "f2008_suspcar_ao.dds"),
                ("txDetail", "f2008_suspcar.dds"),
            ],
        );

        let params = MaterialParams::from_material(&material);
        assert!((params.albedo.r - 0.28).abs() < 1e-6);
        assert_eq!(params.albedo.a, 1.0);
        assert_eq!(params.specular, 0.4);
        assert!((params.roughness - (2.0f32 / 202.0).sqrt()).abs() < 1e-6);
        // isAdditive == 2 → 车漆，不是叠加混合。
        assert!(params.car_paint);
        assert!(!params.additive);
        assert!(!params.uses_alpha());
        assert!(!params.depth_opaque_only);
        assert_eq!(
            params.textures.diffuse.as_deref(),
            Some("f2008_suspcar_diffuse.dds")
        );
        assert_eq!(params.textures.normal.as_deref(), Some("suspcar_nm.dds"));
        assert_eq!(
            params.textures.maps.as_deref(),
            Some("f2008_suspcar_ao.dds")
        );
    }

    #[test]
    fn alpha_blend_and_additive_are_detected() {
        let mut material = material_with(&[("ksSpecularEXP", 90.0)], &[]);
        material.blend_mode = BlendMode::AlphaBlend;
        let params = MaterialParams::from_material(&material);
        assert!(params.alpha_blended);
        assert!(params.uses_alpha());

        let mut material = material_with(&[("isAdditive", 1.0)], &[]);
        material.depth_mode = DepthMode::NoWrite;
        let params = MaterialParams::from_material(&material);
        assert!(params.additive);
        assert!(params.depth_opaque_only);
    }

    #[test]
    fn default_materials_stay_sane() {
        let params = MaterialParams::from_material(&material_with(&[], &[]));

        assert!(params.albedo.r > 0.0);
        assert_eq!(params.roughness, 0.5);
        assert_eq!(params.specular, 0.5);
        assert!(params.textures == MaterialTextures::default());
    }
}
