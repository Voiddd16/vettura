use godot::classes::{ArrayMesh, Material, Node, Node3D, Resource};
use godot::prelude::*;
use vettura_kn5::{Kn5, NodeKind, transform_to_mat4};

use crate::kn5::convert;
use crate::kn5::mesh;
use crate::kn5::scene::{self, SceneOptions};
use crate::kn5::texture::TextureLibrary;

/// KN5 模型资源。
///
/// 把 Assetto Corsa 的 `.kn5` 容器解析后暴露给 Godot：既能查统计信息，也能直接
/// 在场景里实例化（网格、贴图、材质都会自动生成）。
///
/// 用法示例：
///
/// ```gdscript
/// var kn5 := Kn5File.new()
/// if kn5.load("res://cars/car.kn5"):
///     print(kn5.node_count())
///     var root := kn5.instantiate(self)
///     print(root.get_node("WHEEL_LF").position)
/// ```
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct Kn5File {
    base: Base<Resource>,

    #[init(val = GString::new())]
    path: GString,

    model: Option<Kn5>,
}

#[godot_api]
impl Kn5File {
    /// 加载一个 `.kn5` 文件。
    ///
    /// `path` 可以是绝对路径，也可以是 Godot 资源路径。加载成功返回
    /// `true`；失败会打印错误并返回 `false`，此时旧模型会被清空。
    #[func]
    pub fn load(&mut self, path: GString) -> bool {
        match Kn5::from_path(crate::paths::native_path(&path)) {
            Ok(model) => {
                self.path = path;
                self.model = Some(model);
                true
            }
            Err(err) => {
                godot_error!("KN5 load failed: {}", err);
                self.model = None;
                false
            }
        }
    }

    /// 当前是否已成功加载模型。
    #[func]
    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    /// 节点总数，包含根节点。
    #[func]
    pub fn node_count(&self) -> i64 {
        self.model.as_ref().map_or(0, |kn5| kn5.node_count() as i64)
    }

    /// 可渲染网格数量（静态网格 + 蒙皮网格，不含 dummy）。
    #[func]
    pub fn mesh_count(&self) -> i64 {
        self.model.as_ref().map_or(0, |kn5| kn5.mesh_count() as i64)
    }

    /// 三角形总数。
    #[func]
    pub fn triangle_count(&self) -> i64 {
        self.model
            .as_ref()
            .map_or(0, |kn5| kn5.triangle_count() as i64)
    }

    /// 内嵌贴图数量。
    #[func]
    pub fn texture_count(&self) -> i64 {
        self.model
            .as_ref()
            .map_or(0, |kn5| kn5.textures.len() as i64)
    }

    /// 材质数量。
    #[func]
    pub fn material_count(&self) -> i64 {
        self.model
            .as_ref()
            .map_or(0, |kn5| kn5.materials.len() as i64)
    }

    /// 最近一次成功加载的文件路径；未加载时为空字符串。
    #[func]
    pub fn model_path(&self) -> GString {
        self.path.clone()
    }

    /// 在 `parent` 下实例化整个模型，返回创建的根节点。
    ///
    /// dummy 节点（车轮/悬挂挂点）保留为 `Node3D` 并按原名命名，网格节点变成带
    /// `ArrayMesh` 与材质的 `MeshInstance3D`。KN5 里标记为不可见的节点及其子树
    /// 会被跳过。加载失败或父节点为 null 时返回 `null`。
    #[func]
    pub fn instantiate(&mut self, parent: Gd<Node>) -> Option<Gd<Node3D>> {
        self.instantiate_with_options(parent, true, GString::new())
    }

    /// 与 `instantiate` 相同，但可以选择保留不可见节点，并指定根节点名字。
    ///
    /// `root_name` 为空字符串时使用 KN5 根节点名。
    #[func]
    pub fn instantiate_with_options(
        &mut self,
        mut parent: Gd<Node>,
        skip_hidden: bool,
        root_name: GString,
    ) -> Option<Gd<Node3D>> {
        let name = if root_name.is_empty() {
            None
        } else {
            Some(root_name.to_string())
        };
        let name = name.as_deref();

        scene::instantiate(
            self.model.as_ref()?,
            &mut parent,
            name,
            SceneOptions { skip_hidden },
        )
    }

    /// 按节点名生成独立的 `ArrayMesh`（不创建节点树）。
    ///
    /// 适合只需要网格数据的场景，例如把轮胎网格挂到物理车轮上。
    #[func]
    pub fn build_array_mesh(&mut self, node_name: GString) -> Option<Gd<ArrayMesh>> {
        mesh::array_mesh_for(self.model.as_ref()?, &node_name.to_string())
    }

    /// 按索引创建独立材质（`StandardMaterial3D`）。
    #[func]
    pub fn build_material(&mut self, index: i64) -> Option<Gd<Material>> {
        let model = self.model.as_ref()?;
        let index = usize::try_from(index).ok()?;
        let kn5_material = model.materials.get(index)?;
        let mut textures = TextureLibrary::new(model);
        Some(crate::kn5::material::create(kn5_material, &mut textures).upcast::<Material>())
    }

    /// 查 dummy 节点的位置（Godot 帧，米）。
    ///
    /// 找不到或不是 dummy 时返回 `(0, 0, 0)`；用于把物理挂点对齐到模型，
    /// 例如 `WHEEL_LF`、`SUSP_LF`。
    #[func]
    pub fn dummy_position(&mut self, node_name: GString) -> Vector3 {
        let Some(model) = self.model.as_ref() else {
            godot_warn!("dummy_position() 在加载模型前被调用");
            return Vector3::ZERO;
        };
        let Some(node) = model.find_node(&node_name.to_string()) else {
            godot_warn!("node '{}' not found in {}", node_name, self.path);
            return Vector3::ZERO;
        };
        let NodeKind::Dummy { transform } = &node.kind else {
            godot_warn!("node '{}' is not a dummy", node_name);
            return Vector3::ZERO;
        };

        let matrix = transform_to_mat4(transform);
        convert::ac_to_godot_point(matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z)
    }

    /// 访问解析后的模型，给未来的网格/贴图转换层使用。
    ///
    /// 当前没有暴露为 `#[func]`，因为 `Kn5` 不是 Godot 类型。
    #[allow(dead_code)]
    pub fn model(&self) -> Option<&Kn5> {
        self.model.as_ref()
    }
}
