mod material;
mod mesh;
mod node;
mod texture;

pub use material::{BlendMode, DepthMode, Material, MaterialProperty, TextureSample};
pub use mesh::{Bone, Mesh, MeshFlags, SkinnedMesh, SkinnedVertex, Vertex};
pub use node::{Node, NodeKind, Transform, transform_to_mat4};
pub use texture::{SharedBytes, Texture, TextureData};

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
