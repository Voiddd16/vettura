//! 节点与层级树数据模型：dummy / 静态网格 / 蒙皮网格三类节点。

use std::sync::Arc;

use super::mesh::{Mesh, SkinnedMesh};

/// KN5 dummy 节点中按原样存储的行主序 4x4 变换。
///
/// KN5 使用行向量，因此 `transform[row][column]` 与文件中的布局一致。
/// 在参与 `glam` 等列向量数学运算前，先用 [`transform_to_mat4`] 转换。
pub type Transform = [[f32; 4]; 4];

/// 把 KN5 的行主序变换转换为 `glam` 的列主序矩阵。
///
/// KN5（以及 AcTools 使用的 DirectX/SlimDX）采用行向量，平移位于最后一行。
/// `glam` 采用列向量，所以这里把原来的行当作列写入；得到的矩阵对列向量
/// 施加的仿射变换与 KN5 完全等价。
pub fn transform_to_mat4(transform: &Transform) -> glam::Mat4 {
    glam::Mat4::from_cols_array_2d(transform)
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// 子节点数量之后读取的 `active` 标志。AcTools 中对应
    /// `Kn5Node.Active`；具体渲染语义尚不明确，因此只保留，不做解释。
    pub active: bool,
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

impl Node {
    /// 深度优先遍历以当前节点为根的子树。
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node, usize), depth: usize) {
        f(self, depth);
        for child in &self.children {
            child.walk(f, depth + 1);
        }
    }

    /// 以当前节点为根的子树节点总数。
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(Node::node_count).sum::<usize>()
    }

    /// 以当前节点为根的子树中可渲染节点数量。
    pub fn mesh_count(&self) -> usize {
        let own = usize::from(!matches!(self.kind, NodeKind::Dummy { .. }));
        own + self.children.iter().map(Node::mesh_count).sum::<usize>()
    }

    /// 以当前节点为根的子树三角形总数。
    pub fn triangle_count(&self) -> usize {
        let own = match &self.kind {
            NodeKind::Dummy { .. } => 0,
            NodeKind::Mesh(mesh) => mesh.triangle_count(),
            NodeKind::SkinnedMesh(mesh) => mesh.triangle_count(),
        };
        own + self
            .children
            .iter()
            .map(Node::triangle_count)
            .sum::<usize>()
    }

    /// 按名称查找节点，忽略 ASCII 大小写。
    pub fn find_node(&self, name: &str) -> Option<&Node> {
        if self.name.eq_ignore_ascii_case(name) {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find_node(name))
    }

    /// 是否为只包含变换的 dummy 节点。
    pub fn is_dummy(&self) -> bool {
        matches!(self.kind, NodeKind::Dummy { .. })
    }

    /// 如果是静态网格，返回其网格数据。
    ///
    /// 几何存在 [`Arc`] 里：解析出来的网格可以被场景构建器等上层结构共享，
    /// 不必为了再实例化一次而复制一份顶点。
    pub fn as_mesh(&self) -> Option<&Mesh> {
        match &self.kind {
            NodeKind::Mesh(mesh) => Some(mesh),
            _ => None,
        }
    }

    /// 蒙皮网格的同名访问器；网格句柄便于把骨骼数据转交 Godot `Skeleton3D`。
    pub fn as_skinned_mesh_handle(&self) -> Option<&Arc<SkinnedMesh>> {
        match &self.kind {
            NodeKind::SkinnedMesh(mesh) => Some(mesh),
            _ => None,
        }
    }

    /// 如果是蒙皮网格，返回其网格数据。
    pub fn as_skinned_mesh(&self) -> Option<&SkinnedMesh> {
        self.as_skinned_mesh_handle().map(Arc::as_ref)
    }
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    /// 只包含变换的节点。车轮、悬挂枢轴和相机锚点通常是 dummy。
    Dummy {
        /// 文件中按原样存储的行主序局部变换。
        transform: Transform,
    },
    Mesh(Arc<Mesh>),
    SkinnedMesh(Arc<SkinnedMesh>),
}
