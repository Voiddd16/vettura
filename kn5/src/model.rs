use std::fmt;
use std::ops::{Deref, Range};
use std::sync::Arc;

use glam::Vec3;

use crate::dds::{self, DdsError, DdsInfo};

/// 共享的 KN5 原始字节缓冲区。
///
/// 使用 `Arc<Vec<u8>>` 而不是 `Arc<[u8]>`：从 `Vec<u8>` 构造时不会复制数据，
/// 因此 `Kn5::from_path` 读入文件后可以直接被所有贴图切片共享。
pub type SharedBytes = Arc<Vec<u8>>;

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

/// 解析后的 KN5 模型容器。
///
/// 布局与 `AcTools.Kn5File` 一致：文件头、贴图表、材质表，以及一个拥有整棵
/// 层级树的隐式根节点。与 AcTools 不同的是，这里用扁平 `Vec` 和索引表达
/// 引用关系，因此模型易于克隆、序列化，也不依赖任何渲染器。
#[derive(Debug, Clone)]
pub struct Kn5 {
    /// KN5 容器版本，当前支持 5 和 6。
    pub version: u32,
    /// 版本 6 在版本号之后额外写入的一个 `u32`。含义仍未知，但保留下来可以
    /// 为将来的回写功能提供字节级还原能力。
    pub header_extra: Option<u32>,
    pub textures: Vec<Texture>,
    pub materials: Vec<Material>,
    /// 隐式根节点，拥有整棵节点层级。
    pub root: Node,
}

impl Kn5 {
    /// 返回节点总数（包含根节点）。
    pub fn node_count(&self) -> usize {
        self.root.node_count()
    }

    /// 返回可渲染节点数量，即静态网格与蒙皮网格之和，不包括 dummy。
    pub fn mesh_count(&self) -> usize {
        self.root.mesh_count()
    }

    /// 返回三角形总数。
    pub fn triangle_count(&self) -> usize {
        self.root.triangle_count()
    }

    /// 按名称查找节点，忽略 ASCII 大小写，深度优先。
    pub fn find_node(&self, name: &str) -> Option<&Node> {
        self.root.find_node(name)
    }

    /// 深度优先遍历层级树，回调参数为节点和所在深度。
    pub fn walk<'a>(&'a self, mut f: impl FnMut(&'a Node, usize)) {
        self.root.walk(&mut f, 0);
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    /// 贴图表中的 `active` 标志。AcTools 把 `1` 视为启用；真实文件中被引用
    /// 的贴图通常都是 `1`。
    pub active: bool,
    pub name: String,
    /// 指向共享文件缓冲区中的 DDS 载荷，不复制像素数据。
    pub data: TextureData,
}

impl Texture {
    /// 尝试读取内嵌 DDS 的尺寸与格式信息。
    ///
    /// 不是 DDS（或数据被截断）时返回 [`DdsError`]。
    pub fn dds_info(&self) -> Result<DdsInfo, DdsError> {
        dds::parse(self.data.as_slice())
    }
}

/// 指向共享 KN5 缓冲区中一段贴图数据的只读视图。
///
/// 克隆 `TextureData` 只会增加 `Arc` 引用计数，不会复制 DDS 字节。`Deref` 到
/// `[u8]`，因此可以继续按切片使用。
#[derive(Clone)]
pub struct TextureData {
    source: SharedBytes,
    range: Range<usize>,
}

impl TextureData {
    /// 创建一段共享视图。范围由解析器保证在源缓冲区之内。
    pub(crate) fn new(source: SharedBytes, range: Range<usize>) -> Self {
        debug_assert!(range.start <= range.end);
        debug_assert!(range.end <= source.len());
        Self { source, range }
    }

    /// 贴图数据切片。
    pub fn as_slice(&self) -> &[u8] {
        &self.source[self.range.clone()]
    }

    /// 在整个 KN5 文件中的字节范围。
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// 底层共享文件缓冲区。
    pub fn source(&self) -> &SharedBytes {
        &self.source
    }

    pub fn len(&self) -> usize {
        self.range.len()
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

impl Deref for TextureData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for TextureData {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl fmt::Debug for TextureData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextureData")
            .field("range", &self.range)
            .field("len", &self.len())
            .finish()
    }
}

/// 材质混合模式，在文件中占 `u8`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Opaque,
    AlphaBlend,
    AlphaToCoverage,
    /// 未知值不会导致解析失败，而是原样保留，行为与 AcTools 一致。
    Unknown(u8),
}

impl BlendMode {
    /// 从原始字节构造，未知值进入 [`BlendMode::Unknown`]。
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Opaque,
            1 => Self::AlphaBlend,
            2 => Self::AlphaToCoverage,
            other => Self::Unknown(other),
        }
    }

    /// 返回写回文件时应使用的原始字节。
    pub fn as_raw(self) -> u8 {
        match self {
            Self::Opaque => 0,
            Self::AlphaBlend => 1,
            Self::AlphaToCoverage => 2,
            Self::Unknown(value) => value,
        }
    }
}

/// 材质深度模式，在文件中占 `i32`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthMode {
    /// 正常深度测试并写入深度。
    Normal,
    /// 保留深度测试，但关闭深度写入（只读）。
    NoWrite,
    /// 关闭深度测试。
    ///
    /// 这也是 AcTools 遇到未知值时的回退值。
    #[default]
    Off,
    /// 未知值不会导致解析失败，而是原样保留，行为与 AcTools 一致。
    Unknown(i32),
}

impl DepthMode {
    /// 从原始整数构造，未知值进入 [`DepthMode::Unknown`]。
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::NoWrite,
            2 => Self::Off,
            other => Self::Unknown(other),
        }
    }

    /// 返回写回文件时应使用的原始整数。
    pub fn as_raw(self) -> i32 {
        match self {
            Self::Normal => 0,
            Self::NoWrite => 1,
            Self::Off => 2,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Material {
    pub name: String,
    /// shader 名称，例如 `ksPerPixelMultiMap`。
    pub shader: String,
    pub blend_mode: BlendMode,
    pub alpha_tested: bool,
    pub depth_mode: DepthMode,
    pub properties: Vec<MaterialProperty>,
    /// 贴图槽绑定。KN5 中称为 texture mapping。
    pub samples: Vec<TextureSample>,
}

impl Material {
    /// 返回指定 shader 属性的第一个标量分量。
    ///
    /// KN5 属性总是按四个分量存储，但大多数属性只使用 `ValueA`
    /// （例如 `ksAmbient`）。其余分量有意义时请使用
    /// [`Material::property_values`]。
    pub fn property(&self, name: &str) -> Option<f32> {
        self.property_values(name).map(|p| p.value_a)
    }

    /// 按名称查找完整属性值，忽略 ASCII 大小写。
    pub fn property_values(&self, name: &str) -> Option<&MaterialProperty> {
        self.properties
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// 返回某个贴图槽（例如 `txDiffuse`）绑定的贴图名。
    pub fn texture(&self, slot_name: &str) -> Option<&str> {
        self.texture_sample(slot_name).map(|s| s.texture.as_str())
    }

    /// 按名称查找贴图槽，忽略 ASCII 大小写。
    pub fn texture_sample(&self, slot_name: &str) -> Option<&TextureSample> {
        self.samples
            .iter()
            .find(|s| s.slot_name.eq_ignore_ascii_case(slot_name))
    }
}

/// 一个 shader 属性。KN5 固定写入 `f32`、`vec2`、`vec3`、`vec4` 四组值；
/// 多数材质属性只使用 `value_a`。
#[derive(Debug, Clone)]
pub struct MaterialProperty {
    pub name: String,
    pub value_a: f32,
    pub value_b: [f32; 2],
    pub value_c: [f32; 3],
    pub value_d: [f32; 4],
}

impl MaterialProperty {
    /// 返回常见 `ks*` 属性使用的标量值。
    pub fn value(&self) -> f32 {
        self.value_a
    }
}

#[derive(Debug, Clone)]
pub struct TextureSample {
    /// 槽名，例如 `txDiffuse`。
    pub slot_name: String,
    /// 与槽名一起存储的数字槽索引。
    pub slot: u32,
    /// 该槽引用的贴图名。
    pub texture: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_row_major_transform_to_column_major() {
        // 平移位于最后一行，与 AcTools 的 Mat4x4 一致。
        let transform = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [3.0, 4.0, 5.0, 1.0],
        ];

        let matrix = transform_to_mat4(&transform);
        assert_eq!(matrix.w_axis, glam::Vec4::new(3.0, 4.0, 5.0, 1.0));
        assert_eq!(
            matrix.transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(3.0, 4.0, 5.0)
        );
    }

    #[test]
    fn skinned_vertex_filters_unbound_bones() {
        let vertex = SkinnedVertex {
            position: [0.0; 3],
            normal: [0.0; 3],
            uv: [0.0; 2],
            tangent: [0.0; 3],
            weights: [0.5, 0.5, 0.0, 0.0],
            bone_indices: [2.0, 4.0, -1.0, -1.0],
        };

        let bones: Vec<_> = vertex.bones().collect();
        assert_eq!(bones, vec![(2, 0.5), (4, 0.5)]);
    }

    #[test]
    fn material_helpers_are_case_insensitive() {
        let material = Material {
            name: "body".to_owned(),
            shader: "ksPerPixelMultiMap".to_owned(),
            properties: vec![
                MaterialProperty {
                    name: "ksAmbient".to_owned(),
                    value_a: 0.5,
                    value_b: [0.0; 2],
                    value_c: [0.0; 3],
                    value_d: [0.0; 4],
                },
                MaterialProperty {
                    name: "ksDiffuse".to_owned(),
                    value_a: 0.3,
                    value_b: [0.0; 2],
                    value_c: [0.0; 3],
                    value_d: [0.0; 4],
                },
            ],
            samples: vec![TextureSample {
                slot_name: "txDiffuse".to_owned(),
                slot: 0,
                texture: "body.dds".to_owned(),
            }],
            ..Material::default()
        };

        assert_eq!(material.property("KSAMBIENT"), Some(0.5));
        assert_eq!(material.property("missing"), None);
        assert_eq!(material.texture("TXDIFFUSE"), Some("body.dds"));
        assert!(material.texture_sample("txnormal").is_none());
    }

    #[test]
    fn mesh_material_index_handles_sentinel() {
        let mut mesh = Mesh {
            flags: MeshFlags::default(),
            vertices: Vec::new(),
            indices: Vec::new(),
            material: 3,
            layer: 0,
            lod_in: 0.0,
            lod_out: 0.0,
            bounding_sphere_center: [0.0; 3],
            bounding_sphere_radius: 0.0,
            is_renderable: true,
        };

        assert_eq!(mesh.material_index(), Some(3));
        mesh.material = Mesh::NO_MATERIAL;
        assert_eq!(mesh.material_index(), None);
    }

    #[test]
    fn node_helpers_report_kind() {
        let dummy = Node {
            name: "WHEEL_LF".to_owned(),
            active: true,
            kind: NodeKind::Dummy {
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            },
            children: Vec::new(),
        };

        assert!(dummy.is_dummy());
        assert!(dummy.as_mesh().is_none());
        assert!(dummy.as_skinned_mesh().is_none());
        assert_eq!(dummy.node_count(), 1);
        assert_eq!(dummy.mesh_count(), 0);
    }
}
