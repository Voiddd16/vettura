use godot::classes::Resource;
use godot::prelude::*;
use vettura_kn5::Kn5;

/// KN5 模型资源。
///
/// 把 Assetto Corsa 的 `.kn5` 容器解析后暴露给 Godot。当前只提供加载和统计，
/// 不生成 `ArrayMesh` / 贴图；网格与材质转换属于后续层，这样解析器可以继续被
/// CLI、测试和未来的渲染器复用。
///
/// 用法示例：
///
/// ```gdscript
/// var kn5 := Kn5File.new()
/// if kn5.load("res://cars/car.kn5"):
///     print(kn5.node_count())
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
        match Kn5::from_path(path.to_string()) {
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

    /// 访问解析后的模型，给未来的网格/贴图转换层使用。
    ///
    /// 当前没有暴露为 `#[func]`，因为 `Kn5` 不是 Godot 类型。
    #[allow(dead_code)]
    pub fn model(&self) -> Option<&Kn5> {
        self.model.as_ref()
    }
}
