//! 分步实例化：把一次加载切成多帧可执行的小块。
//!
//! 车模加载的耗时集中在单帧里（几十 MB 的几何 + 贴图解压 + 上百个节点），
//! 直接 `instantiate()` 会让游戏卡住几百毫秒到几秒。[`Build`] 先把整棵树
//! **扁平化**成任务列表（顺序 = 深度优先前序，父任务永远排在子任务前面），
//! 之后每次只处理 `max_nodes` 个任务，进度随时可读。
//!
//! [`super::scene::instantiate`] 就是“一次做完所有任务”的包装——这样常规
//! 用法和分步用法走同一条代码路径，不会出现两套行为不同的实现。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use glam::Vec3;
use godot::classes::Node;
use godot::prelude::*;
use vettura_kn5::{Kn5, Mesh, Node as Kn5Node, NodeKind, SkinnedMesh, Transform};

use super::cache::FileResources;
use super::convert::{sanitize_node_name, unique_name};
use super::mesh;
use super::scene::SceneOptions;

/// 扁平化后的一个待建节点。
struct Task {
    name: String,
    active: bool,
    /// 网格自身的可见性（KN5 `is_visible && is_renderable`）；dummy 恒为 true。
    visible: bool,
    /// 父任务的下标；`None` 表示这是根任务。
    parent: Option<usize>,
    kind: TaskKind,
}

/// 任务的几何负载。
///
/// 网格句柄是 `Arc` 克隆（引用计数 +1），所以任务列表不需要自引用树，
/// 也不会为了分步加载而复制顶点。
enum TaskKind {
    Dummy(Transform),
    Static(Arc<Mesh>),
    Skinned(Arc<SkinnedMesh>),
}

/// 一次（可分步的）实例化过程。
pub struct Build {
    resources: Rc<FileResources>,
    tasks: Vec<Task>,
    /// 下一个待处理任务的下标。
    next: usize,
    /// 任务下标 → 已创建的节点；父节点被跳过时空着，子树随之跳过。
    slots: Vec<Option<Gd<Node3D>>>,
    /// 按父任务分组的兄弟名去重集合。
    taken: HashMap<Option<usize>, HashSet<String>>,
    root: Option<Gd<Node3D>>,
    /// 根节点挂到哪个父节点下（第一次 `step` 时记下）。
    host: Option<Gd<Node>>,
    /// 根任务的自定义名字（`None` 用 KN5 根节点名）。
    root_name: Option<String>,
    finished: bool,
}

impl Build {
    pub fn new(resources: Rc<FileResources>, options: SceneOptions) -> Self {
        let tasks = flatten(resources.kn5(), options);
        let slots = vec![None; tasks.len()];
        Self {
            resources,
            tasks,
            next: 0,
            slots,
            taken: HashMap::new(),
            root: None,
            host: None,
            root_name: None,
            finished: false,
        }
    }

    /// 覆盖根节点名字（空表示用 KN5 根节点名）。
    pub fn with_root_name(mut self, name: Option<String>) -> Self {
        self.root_name = name;
        self
    }

    /// 已处理进度，`0.0..=1.0`。
    pub fn progress(&self) -> f32 {
        if self.tasks.is_empty() {
            return 1.0;
        }
        (self.next.min(self.tasks.len())) as f32 / self.tasks.len() as f32
    }

    /// 是否已全部完成。
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// 处理最多 `max_nodes` 个任务；返回是否已全部完成。
    ///
    /// `parent` 只在第一次调用时生效（根节点的宿主）；`max_nodes` 为 `0` 时
    /// 表示“不限量”，一次做完——[`super::scene::instantiate`] 用的就是这个。
    pub fn step(&mut self, max_nodes: usize, parent: Gd<Node>) -> bool {
        if self.finished {
            return true;
        }
        if self.host.is_none() {
            self.host = Some(parent);
        }

        let mut budget = if max_nodes == 0 {
            usize::MAX
        } else {
            max_nodes
        };
        while self.next < self.tasks.len() && budget > 0 {
            let index = self.next;
            self.next += 1;
            budget -= 1;
            self.build_task(index);
        }

        self.finished = self.next >= self.tasks.len();
        self.finished
    }

    /// 建好的根节点（还没建完或根任务失败时为 `None`）。
    pub fn root(&self) -> Option<Gd<Node3D>> {
        self.root.clone()
    }

    /// 建第 `index` 个任务对应的节点。
    fn build_task(&mut self, index: usize) {
        // 逐个字段解构，避免“整块借用 self”与“改 taken”互相打架。
        let Self {
            resources,
            tasks,
            slots,
            taken,
            root,
            host,
            root_name,
            ..
        } = self;

        let Some(task) = tasks.get(index) else {
            return;
        };
        let parent_slot = task.parent;

        // 父任务没有建成节点（不可见、几何为空或构建失败）：整棵子树跳过。
        let parent_node =
            parent_slot.and_then(|parent| slots.get(parent).and_then(Option::as_ref).cloned());
        if parent_slot.is_some() && parent_node.is_none() {
            return;
        }

        let raw = if index == 0 {
            root_name.clone().unwrap_or_else(|| task.name.clone())
        } else {
            task.name.clone()
        };
        let base = sanitize_node_name(&raw);
        let mut siblings = taken.get(&parent_slot).cloned().unwrap_or_default();
        let name = unique_name(&base, &mut siblings);
        taken.insert(parent_slot, siblings);

        let node: Option<Gd<Node3D>> = match &task.kind {
            TaskKind::Dummy(transform) => Some(mesh::dummy_instance(&name, transform).upcast()),
            TaskKind::Static(mesh) => {
                mesh::static_mesh_instance(&name, mesh, resources).map(Gd::upcast)
            }
            TaskKind::Skinned(mesh) => {
                mesh::skinned_mesh_instance(&name, mesh, resources).map(Gd::upcast)
            }
        };

        let Some(mut node) = node else {
            return;
        };
        // 湿滑路面层（`_WET`）在 `finish_instance` 里已默认隐藏，不要被 KN5
        // 的 active 标志重新打开——它由天气系统按湿度切换。
        if !node.has_meta("vettura_wet_layer") {
            node.set_visible(task.active && task.visible);
        }

        if let Some(mut parent_node) = parent_node {
            parent_node.add_child(&node.clone().upcast::<Node>());
        } else if let Some(mut host) = host.as_ref().cloned() {
            host.add_child(&node.clone().upcast::<Node>());
        }

        if index == 0 {
            *root = Some(node.clone());
        }
        slots[index] = Some(node);
    }
}

/// 深度优先前序展开整棵树。
fn flatten(kn5: &Kn5, options: SceneOptions) -> Vec<Task> {
    // 部分 AC 车模同时带 `WINDOWS`（车外玻璃）和 `WIND_IN`（车内挡风玻璃
    // 覆盖层）。后者常与前者几何完全重复，两层都按半透明 + 折射渲染时，
    // 从座舱里看出去会出现明显的双影/重影（992 GT3 R 实测 3708 顶点 /
    // 6992 三角面完全一致）。我们没有 AC/CSP 那套车内雨滴/脏污 shader，
    // 重复层没有额外信息，直接在构建期跳过；只有几何确实不同才保留。
    let exterior_windows = kn5
        .root
        .find_node("WINDOWS")
        .and_then(|node| node.as_mesh())
        .filter(|mesh| mesh.flags.is_visible && mesh.is_renderable)
        .map(|mesh| (mesh.vertices.len(), mesh.indices.len()));

    let mut tasks = Vec::with_capacity(kn5.node_count());
    let mut stack: Vec<(&Kn5Node, Option<usize>)> = vec![(&kn5.root, None)];

    // KN5 里每一级 LOD 常常是独立节点（`*_lodHI*` / `*_lodLOW*`、`*_LOW`），
    // 且都标着 visible/renderable。我们不做运行时 LOD 替换，过去会把所有级
    // 一起画——同一位置的两份几何互相 z-fighting，赛道边旗帜/帐篷营地会闪烁。
    // 这里按“量化 AABB → 最高三角面数”记下主 LOD，低 LOD 若与主 LOD 的
    // AABB 重合就跳过。只处理 `lod_in > 0` 的节点，不碰车模/普通网格。
    let high_lod = collect_high_lod_aabbs(kn5);
    let mut skipped_low_lod = 0usize;

    while let Some((node, parent)) = stack.pop() {
        // 与 `scene.rs` 的文档一致：只有 active && is_visible && is_renderable
        // 的网格才渲染。`is_renderable=false` 是 AC 的物理/辅助网格标记
        // （例如 Nordschleife 的 `nords.kn5` 护墙），画出来会是大块未贴图墙面。
        let (kind, visible) = match &node.kind {
            NodeKind::Dummy { transform } => (TaskKind::Dummy(*transform), true),
            NodeKind::Mesh(mesh) => {
                let duplicate_windscreen = node.name.eq_ignore_ascii_case("WIND_IN")
                    && exterior_windows.is_some_and(|(vertices, indices)| {
                        mesh.vertices.len() == vertices && mesh.indices.len() == indices
                    });
                let duplicate_lod = mesh.lod_in > 0.0
                    && mesh_aabb_key(mesh).is_some_and(|key| {
                        high_lod
                            .get(&key)
                            .is_some_and(|&triangles| triangles >= mesh.triangle_count())
                    });
                if duplicate_lod {
                    skipped_low_lod += 1;
                }
                (
                    TaskKind::Static(Arc::clone(mesh)),
                    mesh.flags.is_visible
                        && mesh.is_renderable
                        && !duplicate_windscreen
                        && !duplicate_lod,
                )
            }
            // SkinnedMesh 没有 is_renderable 字段，只尊重 is_visible。
            NodeKind::SkinnedMesh(mesh) => {
                (TaskKind::Skinned(Arc::clone(mesh)), mesh.flags.is_visible)
            }
        };
        // 根节点无条件保留；其余节点在 `skip_hidden` 下过滤不可见网格。
        if parent.is_some() && options.skip_hidden && (!node.active || !visible) {
            continue;
        }

        let index = tasks.len();
        tasks.push(Task {
            name: node.name.clone(),
            active: node.active,
            visible,
            parent,
            kind,
        });

        // 反序入栈，保证弹出顺序 = 文件里的兄弟顺序。
        for child in node.children.iter().rev() {
            stack.push((child, Some(index)));
        }
    }

    if skipped_low_lod > 0 {
        godot_print!(
            "[kn5] skipped {} duplicate low-LOD mesh(es) in '{}'",
            skipped_low_lod,
            kn5.root.name
        );
    }
    tasks
}

/// 网格 AABB 的 1 m 量化键；同一 LOD 的不同导出通常只差几厘米到几十厘米。
type AabbKey = (i32, i32, i32, i32, i32, i32);

fn mesh_aabb_key(mesh: &Mesh) -> Option<AabbKey> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for point in mesh.positions() {
        min = min.min(point);
        max = max.max(point);
    }
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    let quantize = |value: f32| value.round() as i32;
    Some((
        quantize(min.x),
        quantize(min.y),
        quantize(min.z),
        quantize(max.x),
        quantize(max.y),
        quantize(max.z),
    ))
}

/// 主 LOD（`lod_in == 0`）的 AABB → 三角面数上限。
fn collect_high_lod_aabbs(kn5: &Kn5) -> HashMap<AabbKey, usize> {
    let mut out: HashMap<AabbKey, usize> = HashMap::new();
    kn5.root.walk(
        &mut |node, _depth| {
            let NodeKind::Mesh(mesh) = &node.kind else {
                return;
            };
            if mesh.lod_in > 0.0 {
                return;
            }
            let Some(key) = mesh_aabb_key(mesh) else {
                return;
            };
            let triangles = mesh.triangle_count();
            out.entry(key)
                .and_modify(|value| *value = (*value).max(triangles))
                .or_insert(triangles);
        },
        0,
    );
    out
}

/// 每帧默认预算：约 64 个节点 ≈ 一帧几毫秒，进度条又能平滑推进。
pub const DEFAULT_STEP_BUDGET: usize = 64;
/// 分帧加载器：把一次 `instantiate()` 切成多帧小块。
///
/// 用法（GDScript）：
///
/// ```gdscript
/// var builder := Kn5SceneBuilder.new()
/// if builder.start("res://cars/cim_2008_ferrari/cim_2008_ferrari.kn5"):
///     while not builder.step(Kn5SceneBuilder.STEP_BUDGET, self):
///         loading_bar.value = builder.progress()
///         await get_tree().process_frame
///     var car := builder.root()
/// ```
///
/// `start()` 只做“展开任务列表”（读文件 + 解析，几毫秒），贴图与几何在
/// `step()` 里按需生成，所以首帧就能拿到进度，且不会出现长卡顿。
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub struct Kn5SceneBuilder {
    base: Base<RefCounted>,

    build: Option<Build>,
    /// 缓存的错误信息；GDScript 侧读 `last_error()` 决定是否重试。
    error: GString,
}

#[godot_api]
impl Kn5SceneBuilder {
    /// 每帧默认处理多少个节点（`DEFAULT_STEP_BUDGET` 的脚本侧常量）。
    #[constant]
    const STEP_BUDGET: i64 = DEFAULT_STEP_BUDGET as i64;

    /// 展开一个模型的加载任务；成功返回 `true`。
    #[func]
    fn start(&mut self, path: GString) -> bool {
        self.build = None;
        self.error = GString::new();

        let native = crate::paths::native_path(&path);
        let options = SceneOptions::default();
        let resources = super::cache::acquire(std::path::Path::new(&native), || {
            match vettura_kn5::Kn5::parse_owned(std::fs::read(&native).ok()?) {
                Ok(model) => Some(std::sync::Arc::new(model)),
                Err(err) => {
                    godot_error!("KN5 load failed: {}", err);
                    None
                }
            }
        });

        let Some(resources) = resources else {
            self.error = GString::from(format!("cannot open {native}").as_str());
            return false;
        };

        self.build = Some(Build::new(resources, options));
        true
    }

    /// 处理最多 `max_nodes` 个任务，返回是否已全部完成。
    ///
    /// `0` 表示一次做完（等价于 `scene::instantiate`）。`parent` 只在第一次
    /// 调用时生效；根节点会挂到它下面。
    #[func]
    fn step(&mut self, max_nodes: i64, parent: Gd<Node>) -> bool {
        let budget = usize::try_from(max_nodes).unwrap_or(0);
        self.build
            .as_mut()
            .map(|build| build.step(budget, parent))
            .unwrap_or(true)
    }

    /// `0.0..=1.0`；未开始或已取消时为 `0.0`。
    #[func]
    fn progress(&self) -> f32 {
        self.build.as_ref().map_or(0.0, Build::progress)
    }

    /// 是否已全部完成。
    #[func]
    fn is_finished(&self) -> bool {
        self.build.as_ref().is_none_or(Build::is_finished)
    }

    /// 建好的根节点；没建完/失败时返回 `null`。
    #[func]
    fn root(&self) -> Option<Gd<Node3D>> {
        self.build.as_ref().and_then(Build::root)
    }

    /// 已解码的贴图数量——直接观察缓存是否真的命中。
    #[func]
    fn decoded_texture_count(&self) -> i64 {
        self.build
            .as_ref()
            .map_or(0, |build| build.resource_texture_count() as i64)
    }

    /// 剩余任务数（含跳过逻辑后实际会创建的节点数）。
    #[func]
    fn remaining_tasks(&self) -> i64 {
        self.build
            .as_ref()
            .map_or(0, |build| build.remaining_tasks() as i64)
    }

    /// 放弃这次加载；已创建的节点保留在场景树里。
    #[func]
    fn cancel(&mut self) {
        self.build = None;
    }

    /// 最近一次失败的原因（没有失败时为空字符串）。
    #[func]
    fn last_error(&self) -> GString {
        self.error.clone()
    }
}

impl Build {
    /// 诊断用：当前缓存里已解码的贴图数。
    fn resource_texture_count(&self) -> usize {
        self.resources.decoded_texture_count()
    }

    /// 还没处理的任务数。
    fn remaining_tasks(&self) -> usize {
        self.tasks.len().saturating_sub(self.next)
    }
}
