//! KN5 节点树 → Godot 场景。
//!
//! 规则（与 Content Manager 的 `Kn5RenderableObject` 一致）：
//!
//! - 只有 `active && is_visible && is_renderable` 的网格节点才会渲染；
//! - dummy 节点一律创建为 `Node3D`，保留原名字（`WHEEL_LF`、`SUSP_LF`…），
//!   这样物理层可以按名字找挂点；
//! - 网格节点创建为 `MeshInstance3D`，局部变换沿用父节点的帧（KN5 网格自己
//!   不带变换），父 dummy 的变换转成 Godot 帧；
//! - 兄弟节点重名时追加 `_2`、`_3` 后缀，避免 `get_node()` 歧义；
//! - 不可见节点默认连同子树一起跳过；`SceneOptions::skip_hidden = false` 时
//!   保留它们（`visible = false`），挂点依然可查。
//!
//! 实现细节见 `docs/godot.md` 的“KN5 → 场景的实现要点”。
//!
//! 实际的节点创建在 [`super::builder`] 里：这里是“一次做完”的入口，需要分帧
//! 加载（进度条）的调用方请直接使用 `Kn5SceneBuilder`。

use std::rc::Rc;

use godot::classes::Node;
use godot::prelude::*;

use super::builder::Build;
use super::cache::FileResources;

/// 实例化选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneOptions {
    /// 跳过 KN5 标记为不可见的节点（含整个子树）。默认 `true`。
    ///
    /// 关掉之后仍会创建这些节点，只是 `visible = false`；当需要保留被隐藏的
    /// 挂点（例如某些 mod 把轮子 dummy 关掉）时有用。
    pub skip_hidden: bool,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self { skip_hidden: true }
    }
}

/// 把整个 KN5 实例化到 `parent` 下，返回创建的根节点。
///
/// `root_name` 为根 `Node3D` 的名字；`None` 时用 KN5 根节点名（做字符清理）。
/// 这等价于把 [`super::builder::Build`] 的所有任务一次做完。
pub fn instantiate(
    resources: &Rc<FileResources>,
    parent: &mut Gd<Node>,
    root_name: Option<&str>,
    options: SceneOptions,
) -> Option<Gd<Node3D>> {
    let mut build = Build::new(Rc::clone(resources), options);
    if let Some(name) = root_name {
        build = build.with_root_name(Some(name.to_owned()));
    }
    build.step(0, parent.clone());
    build.root()
}
