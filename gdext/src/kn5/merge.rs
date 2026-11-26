//! 静态网格合并：按材质把大量小 `MeshInstance3D` 合成少数大网格。
//!
//! **为什么**：Daytona 赛道视觉模型有 3000+ 个网格，每个 `MeshInstance3D` 都是
//! 一次 draw call（实测全速行驶时 828~1876）。高速时更多护栏进入视野，CPU 主
//! 线程逼近 16.7 ms 的 60 fps 预算，表现为“护栏渲染跟不上”。按材质合并后
//! draw call 降到几百，CPU 余量充足。
//!
//! **不合并**：
//! - 湿路面层（`vettura_wet_layer`）：天气系统需要按节点切换可见性；
//! - 蒙皮网格（有 `skin` 或 `SKN` 子节点）：骨骼动画无法烘焙成静态顶点；
//! - 半透明材质：合并后透明排序粒度变粗，反而更容易闪；
//! - 不可见节点：保留原来的隐藏状态。
//!
//! 合并时把每个节点的相对变换烘焙进顶点/法线/切线，索引加偏移，UV 原样拼接。
//! 法线用 `basis.inverse().transposed()` 变换（赛道放置矩阵偶尔带旋转）。

use std::collections::HashMap;

use godot::classes::geometry_instance_3d::ShadowCastingSetting;
use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, Material, Mesh, MeshInstance3D, Node, Node3D};
use godot::prelude::*;

use super::mesh;

/// 合并结果统计。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct MergeStats {
    pub groups: usize,
    pub merged_nodes: usize,
}

/// 把 `root` 子树里可合并的静态网格按材质合并，返回统计。
pub(crate) fn merge_static_children(root: &mut Gd<Node3D>) -> MergeStats {
    let mut instances: Vec<Gd<MeshInstance3D>> = Vec::new();
    collect_meshes(&root.clone().upcast::<Node>(), &mut instances);

    let root_inverse = root.get_global_transform().affine_inverse();
    // 分组键：材质实例 ID + 阴影投射设置 + 空间块（200 m）。
    //
    // 纯按材质合并会让一个组覆盖整条赛道，视锥剔除粒度变粗、每帧绘制的
    // 三角形翻倍（Daytona 实测 1.2M → 2.3M）。按 200 m 空间块再切一刀：
    // draw call 仍比逐网格低一个数量级，同时保留“不在视野里的块被剔掉”。
    const SPATIAL_BUCKET_M: f32 = 200.0;
    let mut groups: HashMap<(i64, i32, i32, i32), MergeGroup> = HashMap::new();
    let mut removable: Vec<Gd<MeshInstance3D>> = Vec::new();

    for instance in &instances {
        if !instance.is_visible_in_tree() || instance.has_meta("vettura_wet_layer") {
            continue;
        }
        let Some(mesh_resource) = instance.get_mesh() else {
            continue;
        };
        if mesh_resource.get_surface_count() != 1 {
            continue;
        }
        // 蒙皮网格：有 skin 或挂了 SKN 骨骼子节点。
        if instance.get_skin().is_some() || instance.has_node(&NodePath::from("SKN")) {
            continue;
        }
        let Some(material) = mesh_resource.surface_get_material(0) else {
            continue;
        };
        // 只合并完全不透明的材质（StandardMaterial3D.transparency == 0）。
        // 用动态属性读取，避免依赖具体材质子类的绑定。
        let transparency_value = material.get(&StringName::from("transparency"));
        let transparency = if transparency_value.get_type() == VariantType::NIL {
            0
        } else {
            transparency_value.to::<i32>()
        };
        if transparency != 0 {
            continue;
        }

        let transform = root_inverse * instance.get_global_transform();
        let cast_shadows = instance.get_cast_shadows_setting();
        // 用网格 AABB 中心的世界坐标决定空间块。
        let aabb = instance.get_aabb();
        let center = instance.get_global_transform() * aabb.center();
        let bucket_x = (center.x / SPATIAL_BUCKET_M).floor() as i32;
        let bucket_z = (center.z / SPATIAL_BUCKET_M).floor() as i32;
        let key = (
            material.instance_id().to_i64(),
            cast_shadows.ord(),
            bucket_x,
            bucket_z,
        );
        let entry = groups.entry(key).or_insert_with(|| MergeGroup {
            material: material.clone(),
            cast_shadows,
            vertices: PackedVector3Array::new(),
            normals: PackedVector3Array::new(),
            tangents: PackedFloat32Array::new(),
            uvs: PackedVector2Array::new(),
            uv2s: PackedVector2Array::new(),
            has_uv2: false,
            indices: PackedInt32Array::new(),
        });
        if entry.push(&mesh_resource, transform) {
            removable.push(instance.clone());
        }
    }

    let mut stats = MergeStats::default();
    for (_, group) in groups {
        let cast_shadows = group.cast_shadows;
        let Some(array_mesh) = group.build() else {
            continue;
        };
        let mut merged = MeshInstance3D::new_alloc();
        merged.set_name(&StringName::from("merged"));
        merged.set_mesh(&array_mesh);
        merged.set_cast_shadows_setting(cast_shadows);
        root.add_child(&merged.clone().upcast::<Node>());
        stats.groups += 1;
    }
    for instance in removable.iter().rev() {
        // `free()` 会删除整个子树。KN5 的节点树里 mesh 可能带子节点（子 mesh、
        // dummy、骨骼挂点），其中没被合并的（湿路面层、蒙皮、半透明…）必须
        // 提升到父级保留下来——否则会随父网格一起消失（停车场等区域“浮空/
        // 缺块”就是这么来的）。
        let children: Vec<Gd<Node>> = instance.get_children().iter_shared().collect();
        if let Some(mut parent) = instance.get_parent() {
            for child in children {
                let keep_transform = child.clone().try_cast::<Node3D>().ok().map(|node| {
                    let global = node.get_global_transform();
                    Some((node, global))
                });
                parent.add_child(&child);
                if let Some(Some((mut node, global))) = keep_transform {
                    node.set_global_transform(global);
                }
            }
            parent.remove_child(&instance.clone().upcast::<Node>());
        }
        stats.merged_nodes += 1;
        instance.clone().free();
    }
    stats
}

fn collect_meshes(node: &Gd<Node>, out: &mut Vec<Gd<MeshInstance3D>>) {
    for child in node.get_children().iter_shared() {
        match child.clone().try_cast::<MeshInstance3D>() {
            Ok(mesh) => out.push(mesh),
            Err(other) => collect_meshes(&other, out),
        }
    }
}

/// 一组同材质网格的累积几何。
struct MergeGroup {
    material: Gd<Material>,
    cast_shadows: ShadowCastingSetting,
    vertices: PackedVector3Array,
    normals: PackedVector3Array,
    tangents: PackedFloat32Array,
    uvs: PackedVector2Array,
    uv2s: PackedVector2Array,
    has_uv2: bool,
    indices: PackedInt32Array,
}

impl MergeGroup {
    fn push(&mut self, mesh: &Gd<Mesh>, transform: Transform3D) -> bool {
        let arrays = mesh.surface_get_arrays(0);
        let Some(vertex_variant) = arrays.get(ArrayType::VERTEX.ord() as usize) else {
            return false;
        };
        let vertices = vertex_variant.to::<PackedVector3Array>();
        let count = vertices.len();
        if count == 0 {
            return false;
        }

        let basis = transform.basis;
        // 轨道放置矩阵理论上可逆；零缩放/退化矩阵的 inverse 会产生 inf/NaN，
        // 直接用单位基，避免把 NaN 写进法线。
        let normal_basis = if basis.determinant().abs() > 1.0e-9 {
            basis.inverse().transposed()
        } else {
            Basis::IDENTITY
        };
        let base_index = self.vertices.len() as i32;
        for index in 0..count {
            let vertex = vertices.get(index).unwrap_or(Vector3::ZERO);
            self.vertices.push(transform * vertex);
        }

        let normals = arrays
            .get(ArrayType::NORMAL.ord() as usize)
            .map(|value| value.to::<PackedVector3Array>());
        for index in 0..count {
            let normal = normals
                .as_ref()
                .and_then(|list| list.get(index))
                .unwrap_or(Vector3::UP);
            let transformed = (normal_basis * normal).normalized();
            self.normals.push(transformed);
        }

        let tangents = arrays
            .get(ArrayType::TANGENT.ord() as usize)
            .map(|value| value.to::<PackedFloat32Array>());
        for index in 0..count {
            let (direction, sign) = match tangents.as_ref() {
                Some(list) => {
                    let offset = index * 4;
                    (
                        Vector3::new(
                            list.get(offset).unwrap_or(1.0),
                            list.get(offset + 1).unwrap_or(0.0),
                            list.get(offset + 2).unwrap_or(0.0),
                        ),
                        list.get(offset + 3).unwrap_or(1.0),
                    )
                }
                None => (Vector3::RIGHT, 1.0),
            };
            let world = (basis * direction).normalized();
            self.tangents.push(world.x);
            self.tangents.push(world.y);
            self.tangents.push(world.z);
            self.tangents.push(sign);
        }

        let uvs = arrays
            .get(ArrayType::TEX_UV.ord() as usize)
            .map(|value| value.to::<PackedVector2Array>());
        for index in 0..count {
            self.uvs.push(
                uvs.as_ref()
                    .and_then(|list| list.get(index))
                    .unwrap_or(Vector2::ZERO),
            );
        }

        let uv2s = arrays
            .get(ArrayType::TEX_UV2.ord() as usize)
            .map(|value| value.to::<PackedVector2Array>());
        if uv2s.is_some() {
            self.has_uv2 = true;
        }
        for index in 0..count {
            self.uv2s.push(
                uv2s.as_ref()
                    .and_then(|list| list.get(index))
                    .unwrap_or(Vector2::ZERO),
            );
        }

        let indices = arrays
            .get(ArrayType::INDEX.ord() as usize)
            .map(|value| value.to::<PackedInt32Array>());
        match indices {
            Some(list) => {
                for index in 0..list.len() {
                    self.indices.push(list.get(index).unwrap_or(0) + base_index);
                }
            }
            None => {
                for index in 0..count as i32 {
                    self.indices.push(base_index + index);
                }
            }
        }
        true
    }

    fn build(self) -> Option<Gd<ArrayMesh>> {
        if self.vertices.is_empty() || self.indices.is_empty() {
            return None;
        }
        let mut arrays = mesh::empty_surface_arrays();
        mesh::set(&mut arrays, ArrayType::VERTEX, self.vertices);
        mesh::set(&mut arrays, ArrayType::NORMAL, self.normals);
        mesh::set(&mut arrays, ArrayType::TANGENT, self.tangents);
        mesh::set(&mut arrays, ArrayType::TEX_UV, self.uvs);
        if self.has_uv2 {
            mesh::set(&mut arrays, ArrayType::TEX_UV2, self.uv2s);
        }
        mesh::set(&mut arrays, ArrayType::INDEX, self.indices);

        let mut array_mesh = ArrayMesh::new_gd();
        array_mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
        array_mesh.surface_set_material(0, &self.material);
        Some(array_mesh)
    }
}
