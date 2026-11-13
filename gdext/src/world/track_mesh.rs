//! 赛道碰撞体 → Godot `MeshInstance3D`。
//!
//! 主项目在拿到 `WorldBody.load_track()` 之后，第一件事就是“看见路”。真实赛道
//! 的视觉 KN5 动辄 100–200 MB（Daytona `daytona_2017.kn5` 是 193 MB / 806k 三角
//! 形），首屏加载与显存都吃不消；而 `TrackModule` 为了做物理已经解析过的
//! 碰撞三角网，本身就是一张足够在开发 baseline 里辨认赛道的低模路面。
//!
//! 这里把碰撞网直接转成一张 `ArrayMesh`：物理与渲染共用同一份几何，加载时间和
//! 显存都由 `TrackModule` 的规格决定，不引入第二个解析器。这是**回退方案**：
//! 正常路径优先加载真实视觉 KN5（见 [`super::track_models`]），只有视觉模型
//! 全部缺失/损坏时才落到这里。

use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, MeshInstance3D, StandardMaterial3D};
use godot::prelude::*;
use vettura_assets::TrackModule;

use crate::kn5::mesh;

/// 用碰撞体三角网生成一个 `MeshInstance3D`；没有碰撞体时返回 `None`。
pub(crate) fn collider_mesh_instance(track: &TrackModule) -> Option<Gd<MeshInstance3D>> {
    let (vertices, triangles) = track.collider_mesh()?;
    if vertices.is_empty() || triangles.is_empty() {
        return None;
    }

    let normals = vertex_normals(vertices, triangles);
    let packed_normals: PackedVector3Array = normals.iter().copied().collect();
    let mut array_mesh = array_mesh(vertices, &packed_normals, triangles);

    let mut material = StandardMaterial3D::new_gd();
    material.set_albedo(Color::from_rgb(0.16, 0.16, 0.18));
    material.set_roughness(0.95);
    material.set_metallic(0.0);
    array_mesh.surface_set_material(0, &material);

    let mut instance = MeshInstance3D::new_alloc();
    instance.set_mesh(&array_mesh);
    instance.set_name("TrackCollider");
    Some(instance)
}

/// 把顶点/法线/索引打包成 `ArrayMesh`，槽位填法与 `kn5::mesh` 保持一致。
fn array_mesh(
    vertices: &[glam::Vec3],
    normals: &PackedVector3Array,
    triangles: &[[u32; 3]],
) -> Gd<ArrayMesh> {
    let mut arrays = mesh::empty_surface_arrays();
    let positions: PackedVector3Array = vertices
        .iter()
        .map(|vertex| Vector3::new(vertex.x, vertex.y, vertex.z))
        .collect();
    let indices: PackedInt32Array = triangles
        .iter()
        .flat_map(|triangle| triangle.iter().map(|index| *index as i32))
        .collect();

    mesh::set(&mut arrays, ArrayType::VERTEX, positions);
    mesh::set(&mut arrays, ArrayType::NORMAL, normals.clone());
    mesh::set(&mut arrays, ArrayType::INDEX, indices);

    let mut array_mesh = ArrayMesh::new_gd();
    array_mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
    array_mesh
}

/// 逐顶点累加所在面的法线，得到平滑顶点法线。
///
/// 碰撞网里的同一个顶点通常被多条相邻三角形共享，累加能避免把路画成一片小平面；
/// 归一化前的零向量（退化三角形）留给调用方处理。
pub(crate) fn vertex_normals(vertices: &[glam::Vec3], triangles: &[[u32; 3]]) -> Vec<Vector3> {
    let mut normals = vec![Vector3::ZERO; vertices.len()];
    let point = |index: u32| -> Vector3 {
        let vertex = vertices[index as usize];
        Vector3::new(vertex.x, vertex.y, vertex.z)
    };

    for triangle in triangles {
        let a = point(triangle[0]);
        let b = point(triangle[1]);
        let c = point(triangle[2]);
        // `Vector3::cross` 返回右手系下的面积法向，方向与 AC/物理帧一致。
        let normal = (b - a).cross(c - a);
        if !normal.is_finite() {
            continue;
        }
        for &index in triangle {
            normals[index as usize] += normal;
        }
    }

    for normal in &mut normals {
        if normal.is_finite() && normal.length_squared() > 1.0e-12 {
            *normal = normal.normalized();
        } else {
            *normal = Vector3::UP;
        }
    }

    normals
}

#[cfg(test)]
mod tests {
    use super::{Vector3, vertex_normals};
    use glam::Vec3;

    /// 平面网格：四个顶点、两个三角形，法线都应该朝上。
    #[test]
    fn computes_flat_normals_for_a_quad() {
        let vertices = [
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ];
        let triangles = [[0u32, 2, 1], [0, 3, 2]];

        let normals = vertex_normals(&vertices, &triangles);
        for normal in normals {
            assert!(
                normal.dot(Vector3::UP) > 0.99,
                "normal {normal} did not stay up"
            );
        }
    }

    /// 退化三角形（共线顶点）会让面积为零，法线必须回退而不是塞 NaN 进渲染层。
    #[test]
    fn degenerate_triangles_do_not_produce_nan() {
        let vertices = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        let normals = vertex_normals(&vertices, &[[0u32, 1, 2]]);

        for normal in normals {
            assert!(normal.is_finite());
            assert_eq!(normal, Vector3::UP);
        }
    }
}
