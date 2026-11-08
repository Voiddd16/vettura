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

use std::collections::HashSet;

use godot::classes::Node;
use godot::prelude::*;
use vettura_kn5::{Kn5, Node as Kn5Node, NodeKind};

use super::mesh;
use super::texture::TextureLibrary;

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
pub fn instantiate(
    kn5: &Kn5,
    parent: &mut Gd<Node>,
    root_name: Option<&str>,
    options: SceneOptions,
) -> Option<Gd<Node3D>> {
    let mut textures = TextureLibrary::new(kn5);
    let mut taken = HashSet::new();

    let root = build_node(
        kn5,
        &kn5.root,
        root_name.unwrap_or(kn5.root.name.as_str()),
        0,
        &mut textures,
        &mut taken,
        options,
    )?;
    parent.add_child(&root.clone().upcast::<Node>());
    Some(root)
}

/// 递归创建一个节点；返回 `None` 表示该节点被跳过。
fn build_node(
    kn5: &Kn5,
    node: &Kn5Node,
    name: &str,
    depth: usize,
    textures: &mut TextureLibrary<'_>,
    taken: &mut HashSet<String>,
    options: SceneOptions,
) -> Option<Gd<Node3D>> {
    if depth > 0 && options.skip_hidden && !node.active {
        return None;
    }

    let mut godot_node: Gd<Node3D> = match &node.kind {
        NodeKind::Dummy { transform } => {
            let mut dummy = Node3D::new_alloc();
            dummy.set_transform(super::convert::kn5_transform_to_godot(transform));
            dummy
        }
        // 网格节点自己就是 `MeshInstance3D`（几何数据沿用父节点的帧），它的子节点
        // 仍然挂到它下面，所以这里不提前返回。
        NodeKind::Mesh(_) | NodeKind::SkinnedMesh(_) => {
            mesh::mesh_instance(node, kn5, textures)?.upcast::<Node3D>()
        }
    };

    // KN5 里父级 dummy 与子网格同名很常见，这里保证兄弟节点唯一。
    let unique = super::convert::unique_name(&super::convert::sanitize_node_name(name), taken);
    godot_node.set_name(&StringName::from(unique.as_str()));
    godot_node.set_visible(node.active);

    for child in &node.children {
        if let Some(child_node) = build_node(
            kn5,
            child,
            child.name.as_str(),
            depth + 1,
            textures,
            taken,
            options,
        ) {
            godot_node.add_child(&child_node.upcast::<Node>());
        }
    }

    Some(godot_node)
}
