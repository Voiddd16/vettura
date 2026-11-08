//! KN5 网格 → Godot `ArrayMesh`。
//!
//! 顶点数据保持 KN5 原样（AC 帧），坐标转换统一在节点变换上做
//! （[`super::convert::kn5_transform_to_godot`]）。这样做的好处是 dummy 挂点
//! （`WHEEL_LF`、`SUSP_LF`…）与网格共用同一条转换路径，物理层拿到的位置与渲染
//! 结果严格一致；因为 AC 帧 → Godot 帧等价于绕 Y 旋转 180°，三角形绕序不需要
//! 像 Content Manager 那样额外倒转。
//!
//! 参考：Content Manager 的 `Kn5RenderableObject`（顶点布局 `PNTG`）和
//! `Kn5SkinnedObject`（`PNTGW4B`）。

use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, MeshInstance3D, Node, Skeleton3D, Skin};
use godot::prelude::*;
use vettura_kn5::{Kn5, Mesh, Node as Kn5Node, NodeKind, SkinnedMesh};

use super::convert::kn5_transform_to_godot;
use super::material;
use super::texture::TextureLibrary;

/// surface 数组的槽位数量（`Mesh::ArrayType::MAX`）。
const ARRAY_TYPE_COUNT: usize = 13;

/// 静态网格 → 单 surface 的 `ArrayMesh`。
///
/// 切线按 Godot 约定写成 4 个 float（`xyz` + 副切线符号）；KN5 只存 3 个分量，
/// 符号取 `+1.0`。
pub fn static_array_mesh(mesh: &Mesh, name: &str) -> Option<Gd<ArrayMesh>> {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        godot_warn!("skipping empty mesh node '{}'", name);
        return None;
    }

    let mut positions = Vec::with_capacity(mesh.vertices.len());
    let mut normals = Vec::with_capacity(mesh.vertices.len());
    let mut uvs = Vec::with_capacity(mesh.vertices.len());
    let mut tangents = Vec::with_capacity(mesh.vertices.len() * 4);
    for vertex in &mesh.vertices {
        positions.push(vec3(vertex.position));
        normals.push(vec3(vertex.normal));
        uvs.push(super::convert::uv_to_godot(vertex.uv));
        tangents.extend_from_slice(&[vertex.tangent[0], vertex.tangent[1], vertex.tangent[2], 1.0]);
    }

    let mut arrays = empty_surface_arrays();
    set(
        &mut arrays,
        ArrayType::VERTEX,
        PackedVector3Array::from(positions),
    );
    set(
        &mut arrays,
        ArrayType::NORMAL,
        PackedVector3Array::from(normals),
    );
    set(
        &mut arrays,
        ArrayType::TANGENT,
        PackedFloat32Array::from(tangents),
    );
    set(
        &mut arrays,
        ArrayType::TEX_UV,
        PackedVector2Array::from(uvs),
    );
    set_indices(&mut arrays, &mesh.indices);

    Some(surface_from_arrays(arrays))
}

/// 蒙皮网格 → 带 `BONES/WEIGHTS` 的 `ArrayMesh`（无骨骼时退化为静态表面）。
pub fn skinned_array_mesh(mesh: &SkinnedMesh, name: &str) -> Option<Gd<ArrayMesh>> {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        godot_warn!("skipping empty skinned mesh node '{}'", name);
        return None;
    }

    let mut positions = Vec::with_capacity(mesh.vertices.len());
    let mut normals = Vec::with_capacity(mesh.vertices.len());
    let mut uvs = Vec::with_capacity(mesh.vertices.len());
    let mut tangents = Vec::with_capacity(mesh.vertices.len() * 4);
    let mut bones = Vec::with_capacity(mesh.vertices.len() * 4);
    let mut weights = Vec::with_capacity(mesh.vertices.len() * 4);
    for vertex in &mesh.vertices {
        positions.push(vec3(vertex.position));
        normals.push(vec3(vertex.normal));
        uvs.push(Vector2::new(vertex.uv[0], vertex.uv[1]));
        tangents.extend_from_slice(&[vertex.tangent[0], vertex.tangent[1], vertex.tangent[2], 1.0]);

        // KN5 的骨骼索引是 f32，`-1` 表示未绑定；Godot 需要固定 4 个 i32。
        let mut mapped = [0i32; 4];
        for (slot, (bone_index, weight)) in mapped.iter_mut().zip(vertex.bones()) {
            *slot = bone_index as i32;
            weights.push(weight);
        }
        bones.extend_from_slice(&mapped);
    }

    let mut arrays = empty_surface_arrays();
    set(
        &mut arrays,
        ArrayType::VERTEX,
        PackedVector3Array::from(positions),
    );
    set(
        &mut arrays,
        ArrayType::NORMAL,
        PackedVector3Array::from(normals),
    );
    set(
        &mut arrays,
        ArrayType::TANGENT,
        PackedFloat32Array::from(tangents),
    );
    set(
        &mut arrays,
        ArrayType::TEX_UV,
        PackedVector2Array::from(uvs),
    );
    if !mesh.bones.is_empty() {
        set(&mut arrays, ArrayType::BONES, PackedInt32Array::from(bones));
        set(
            &mut arrays,
            ArrayType::WEIGHTS,
            PackedFloat32Array::from(weights),
        );
    } else {
        godot_warn!(
            "skinned mesh '{}' has no bones; rendering it as a static surface",
            name
        );
    }
    set_indices(&mut arrays, &mesh.indices);

    Some(surface_from_arrays(arrays))
}

/// 网格节点 → 挂好网格与材质的 `MeshInstance3D`。
///
/// 返回 `None` 表示该节点不应实例化（dummy，或几何数据为空）。
pub fn mesh_instance(
    node: &Kn5Node,
    kn5: &Kn5,
    textures: &mut TextureLibrary<'_>,
) -> Option<Gd<MeshInstance3D>> {
    let mut instance = MeshInstance3D::new_alloc();

    let (array_mesh, flags) = match &node.kind {
        NodeKind::Mesh(mesh) => {
            let array_mesh = static_array_mesh(mesh, &node.name)?;
            (array_mesh, mesh.flags)
        }
        NodeKind::SkinnedMesh(mesh) => {
            let array_mesh = skinned_array_mesh(mesh, &node.name)?;
            attach_skeleton(&mut instance, mesh);
            (array_mesh, mesh.flags)
        }
        NodeKind::Dummy { .. } => return None,
    };

    let mut array_mesh = array_mesh;
    let material_index = node.as_mesh().map_or_else(
        || node.as_skinned_mesh().and_then(SkinnedMesh::material_index),
        |mesh| mesh.material_index(),
    );
    apply_material(&mut array_mesh, kn5, material_index, textures);
    instance.set_mesh(&array_mesh);
    instance.set_visible(flags.is_visible);
    instance.set_cast_shadows_setting(shadow_setting(flags.cast_shadows));

    Some(instance)
}

/// 蒙皮网格：创建 `Skeleton3D` + `Skin`，并挂到该 `MeshInstance3D` 下。
///
/// AC 的蒙皮是 `顶点位置 × (bind矩阵 · 动画矩阵)`；把 bind 矩阵同时作为骨骼的
/// rest pose 和 Skin 的 bind pose，可以在动画为空时复现 AC 的姿态。
fn attach_skeleton(instance: &mut MeshInstance3D, mesh: &SkinnedMesh) {
    let mut skeleton = Skeleton3D::new_alloc();
    let mut skin = Skin::new_gd();
    for bone in &mesh.bones {
        let rest = kn5_transform_to_godot(&bone.transform);
        let index = skeleton.add_bone(&GString::from(bone.name.as_str()));
        skeleton.set_bone_rest(index, rest);
        skin.add_named_bind(&GString::from(bone.name.as_str()), rest);
    }

    instance.set_skin(&skin);

    let mut child = skeleton;
    child.set_name(&StringName::from("SKN"));
    instance.add_child(&child.upcast::<Node>());
    instance.set_skeleton_path(&NodePath::from("SKN"));
}

fn apply_material(
    array_mesh: &mut Gd<ArrayMesh>,
    kn5: &Kn5,
    material_index: Option<usize>,
    textures: &mut TextureLibrary<'_>,
) {
    // `ArrayMesh` 是引用计数资源：这里的 `&mut` 只是为了 API 语义，set 之后
    // 挂到 MeshInstance3D 上的同一份资源会立即看到材质。
    let Some(index) = material_index else {
        return;
    };
    let Some(kn5_material) = kn5.materials.get(index) else {
        godot_warn!("material index {} is out of range", index);
        return;
    };

    let material = material::create(kn5_material, textures);
    array_mesh.surface_set_material(0, &material);
}

fn shadow_setting(
    cast_shadows: bool,
) -> godot::classes::geometry_instance_3d::ShadowCastingSetting {
    use godot::classes::geometry_instance_3d::ShadowCastingSetting;
    if cast_shadows {
        ShadowCastingSetting::ON
    } else {
        ShadowCastingSetting::OFF
    }
}

fn surface_from_arrays(arrays: Array<Variant>) -> Gd<ArrayMesh> {
    let mut array_mesh = ArrayMesh::new_gd();
    array_mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
    array_mesh
}

/// 只填 `Variant::nil()` 的 surface 数组；Godot 要求 13 个槽位都写。
pub(crate) fn empty_surface_arrays() -> Array<Variant> {
    let mut arrays = Array::new();
    for _ in 0..ARRAY_TYPE_COUNT {
        arrays.push(&Variant::nil());
    }
    arrays
}

/// 写入一个 surface 槽位（`Mesh::ArrayType` 的槽位顺序）。
pub(crate) fn set<T: godot::meta::ToGodot>(arrays: &mut Array<Variant>, slot: ArrayType, value: T) {
    arrays.set(slot.ord() as usize, &value.to_variant());
}

fn set_indices(arrays: &mut Array<Variant>, indices: &[u16]) {
    let converted: PackedInt32Array = indices.iter().map(|&index| index as i32).collect();
    arrays.set(ArrayType::INDEX.ord() as usize, &converted.to_variant());
}

fn vec3(value: [f32; 3]) -> Vector3 {
    Vector3::new(value[0], value[1], value[2])
}

/// 按节点名生成 `ArrayMesh`（`Kn5File.build_array_mesh` 用）。
///
/// KN5 里同一个名字可能对应多个节点（dummy 父节点 + 同名网格子节点很常见），
/// 所以遍历整棵树、优先取第一个网格节点。
pub fn array_mesh_for(kn5: &Kn5, node_name: &str) -> Option<Gd<ArrayMesh>> {
    let mut found: Option<&Kn5Node> = None;
    kn5.walk(|node, _| {
        if node.name.eq_ignore_ascii_case(node_name) && !node.is_dummy() {
            found.get_or_insert(node);
        }
    });

    let Some(node) = found else {
        godot_warn!("KN5 mesh node '{}' not found", node_name);
        return None;
    };

    match &node.kind {
        NodeKind::Mesh(mesh) => static_array_mesh(mesh, &node.name),
        NodeKind::SkinnedMesh(mesh) => skinned_array_mesh(mesh, &node.name),
        NodeKind::Dummy { .. } => None,
    }
}
