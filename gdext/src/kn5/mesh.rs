//! KN5 网格 → Godot `ArrayMesh`。
//!
//! 轴镜像 `S`（`(x,y,z) → (-x,y,-z)`）分两半做：节点局部变换走
//! `S·M·S`（[`super::convert::kn5_transform_to_godot`]），顶点分量
//! （位置/法线/切线）乘 `S`（[`vertex_vec3`]）。链式复合后整体是
//! `S·T·v`，网格与 dummy 挂点（`WHEEL_LF`、`SUSP_LF`…）共用同一条转换
//! 路径，物理层拿到的位置与渲染结果严格一致。`S` 行列式 `+1`（绕 Y 转
//! 180°），轴镜像不影响三角形绕序；KN5 原始绕序与 Godot 正面相反是另一
//! 回事，见 [`set_indices`]。
//!
//! 参考：Content Manager 的 `Kn5RenderableObject`（顶点布局 `PNTG`）和
//! `Kn5SkinnedObject`（`PNTGW4B`）。

use std::sync::Arc;

use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, MeshInstance3D, Node, Node3D, Skeleton3D, Skin};
use godot::prelude::*;
use vettura_kn5::{Kn5, Mesh, MeshFlags, Node as Kn5Node, NodeKind, SkinnedMesh, Transform};

use super::cache::FileResources;
use super::convert::kn5_transform_to_godot;

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
        positions.push(vertex_vec3(vertex.position));
        normals.push(vertex_vec3(vertex.normal));
        uvs.push(super::convert::uv_to_godot(vertex.uv));
        let tangent = vertex_vec3(vertex.tangent);
        tangents.extend_from_slice(&[tangent.x, tangent.y, tangent.z, 1.0]);
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
        positions.push(vertex_vec3(vertex.position));
        normals.push(vertex_vec3(vertex.normal));
        uvs.push(Vector2::new(vertex.uv[0], vertex.uv[1]));
        let tangent = vertex_vec3(vertex.tangent);
        tangents.extend_from_slice(&[tangent.x, tangent.y, tangent.z, 1.0]);

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

/// dummy 节点 → 只带变换的 `Node3D`（挂点，例如 `WHEEL_LF`）。
pub fn dummy_instance(name: &str, transform: &Transform) -> Gd<Node3D> {
    let mut node = Node3D::new_alloc();
    node.set_transform(kn5_transform_to_godot(transform));
    node.set_name(&StringName::from(name));
    node
}

/// 静态网格节点 → 挂好网格与材质的 `MeshInstance3D`。
///
/// 返回 `None` 表示该节点不应实例化（几何数据为空）。
pub fn static_mesh_instance(
    name: &str,
    mesh: &Mesh,
    resources: &FileResources,
) -> Option<Gd<MeshInstance3D>> {
    let array_mesh = static_array_mesh(mesh, name)?;
    finish_instance(
        name,
        array_mesh,
        mesh.flags,
        mesh.material_index(),
        resources,
    )
}

/// 蒙皮网格节点 → 带 `BONES/WEIGHTS` 的 `MeshInstance3D`（无骨骼时退化为静态表面）。
pub fn skinned_mesh_instance(
    name: &str,
    mesh: &Arc<SkinnedMesh>,
    resources: &FileResources,
) -> Option<Gd<MeshInstance3D>> {
    let array_mesh = skinned_array_mesh(mesh, name)?;
    let mut instance = finish_instance(
        name,
        array_mesh,
        mesh.flags,
        mesh.material_index(),
        resources,
    )?;
    if !mesh.bones.is_empty() {
        attach_skeleton(&mut instance, mesh);
    }
    Some(instance)
}

/// 网格 + 材质 + 渲染标志 → 完成的 `MeshInstance3D`。
fn finish_instance(
    name: &str,
    mut array_mesh: Gd<ArrayMesh>,
    flags: MeshFlags,
    material_index: Option<usize>,
    resources: &FileResources,
) -> Option<Gd<MeshInstance3D>> {
    // 网格为空的节点在 `static_array_mesh` 里已被拒绝；这里只做挂载。
    let mut instance = MeshInstance3D::new_alloc();
    instance.set_name(&StringName::from(name));
    if let Some(index) = material_index {
        if resources.kn5().materials.get(index).is_some() {
            // 材质按 KN5 材质下标缓存：同一份模型里的网格共享一份材质。
            if let Some(material) = resources.material(index) {
                array_mesh.surface_set_material(0, &material);
            } else {
                godot_warn!("material {} could not be created for '{}'", index, name);
            }
        } else {
            godot_warn!(
                "material index {} is out of range (node '{}', {} materials)",
                index,
                name,
                resources.kn5().materials.len()
            );
        }
    }

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
    // KN5 原始绕序与 Godot 的正面方向相反。实测症状：从车外看到的是内壁
    // （车身发黑，内饰碳纤维），从驾驶位看穿车身、赞助商文字呈镜像。
    // 交换每个三角形后两个索引把正面翻回外侧；法线仍按 KN5 原值（朝外），
    // 光照不受影响。静态与蒙皮网格都经过这里，单点修复。
    let mut converted: Vec<i32> = Vec::with_capacity(indices.len());
    let mut chunks = indices.chunks_exact(3);
    for tri in &mut chunks {
        converted.push(tri[0] as i32);
        converted.push(tri[2] as i32);
        converted.push(tri[1] as i32);
    }
    for &index in chunks.remainder() {
        converted.push(index as i32);
    }
    arrays.set(
        ArrayType::INDEX.ord() as usize,
        &PackedInt32Array::from(converted).to_variant(),
    );
}

/// KN5 顶点分量（位置/法线/切线）→ Godot 空间。
///
/// 顶点按 AC 帧存放，这里乘与节点变换同一个轴镜像 `S`；转发给
/// [`super::convert::ac_to_godot_point`]，与物理侧的点转换共用一处定义。
fn vertex_vec3(value: [f32; 3]) -> Vector3 {
    super::convert::ac_to_godot_point(value[0], value[1], value[2])
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
