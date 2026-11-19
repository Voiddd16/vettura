//! 网格与顶点数据模型：静态网格、蒙皮网格、骨骼与顶点布局。

use super::node::Transform;
// 文档链接 `[Kn5::materials`]` 指向容器结构；仅在 rustdoc 构建时需要该名字在作用域内。
#[cfg(doc)]
use super::Kn5;

use glam::Vec3;

/// 几何数据之前连续存储的三个渲染标志。
///
/// AcTools 分别命名为 `CastShadows`、`IsVisible`、`IsTransparent`。
/// 每个标志用一个字节表示布尔值（`0` 或 `1`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshFlags {
    pub cast_shadows: bool,
    pub is_visible: bool,
    pub is_transparent: bool,
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub flags: MeshFlags,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    /// [`Kn5::materials`] 的下标。部分导出器用 `u32::MAX` 表示“无材质”；
    /// 需要安全判断时请使用 [`Mesh::material_index`]。
    pub material: u32,
    /// 渲染层，原样保留。
    pub layer: u32,
    /// LOD 范围，单位为米。常驻可见的网格常见值为 `0.0`/`1e9`。
    pub lod_in: f32,
    pub lod_out: f32,
    /// 局部空间包围球中心，用于渲染裁剪。
    pub bounding_sphere_center: [f32; 3],
    pub bounding_sphere_radius: f32,
    /// 是否应提交给渲染器绘制。
    pub is_renderable: bool,
}

impl Mesh {
    /// 表示“无材质”的哨兵值。
    pub const NO_MATERIAL: u32 = u32::MAX;

    /// 三角形数量。
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// 材质下标；无材质时返回 `None`。
    pub fn material_index(&self) -> Option<usize> {
        (self.material != Self::NO_MATERIAL).then_some(self.material as usize)
    }

    /// 遍历所有顶点位置。
    pub fn positions(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.vertices.iter().map(|v| Vec3::from_array(v.position))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct SkinnedMesh {
    pub flags: MeshFlags,
    pub bones: Vec<Bone>,
    pub vertices: Vec<SkinnedVertex>,
    pub indices: Vec<u16>,
    /// [`Kn5::materials`] 的下标。
    pub material: u32,
    /// 渲染层，原样保留。
    pub layer: u32,
    /// LOD 范围，单位为米。
    pub lod_in: f32,
    pub lod_out: f32,
}

impl SkinnedMesh {
    /// 表示“无材质”的哨兵值。
    pub const NO_MATERIAL: u32 = u32::MAX;

    /// 三角形数量。
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// 材质下标；无材质时返回 `None`。
    pub fn material_index(&self) -> Option<usize> {
        (self.material != Self::NO_MATERIAL).then_some(self.material as usize)
    }
}

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    /// 行主序 4x4 绑定变换。
    pub transform: Transform,
}

/// 一个蒙皮顶点。KN5 把权重和骨骼索引存成两个 `vec4`；索引虽然概念上是
/// 整数，但文件中确实以浮点数存储。
#[derive(Debug, Clone, Copy)]
pub struct SkinnedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 3],
    /// 每个骨骼的权重，通常会归一化到总和 `1.0`。
    pub weights: [f32; 4],
    /// [`SkinnedMesh::bones`] 的下标。`-1.0` 表示未绑定。
    pub bone_indices: [f32; 4],
}

impl SkinnedVertex {
    /// 把骨骼索引转换为 `u32`，同时过滤掉未绑定（`-1`）和零权重的项。
    pub fn bones(&self) -> impl Iterator<Item = (u32, f32)> + '_ {
        self.bone_indices
            .iter()
            .zip(self.weights)
            .filter(|(index, weight)| **index >= 0.0 && *weight > 0.0)
            .map(|(index, weight)| (*index as u32, weight))
    }
}
