//! 赛道视觉模型：把 `models.ini` 里的非碰撞 KN5 实例化成 Godot 节点。
//!
//! 物理层（`TrackModule`）只吃碰撞几何，渲染层则需要 `*mesh*.kn5` 这类带贴图
//! 的视觉模型。这里按 `models.ini` 的放置表逐个加载：跳过 `collision*.kn5`
//! （物理已经装好）与 `acstobjects.kn5`（AC 编辑器用的起终点/维修区标记，
//! 比赛视图里不显示），文件缺失或解析失败只给 warning 并跳过——赛道目录里
//! 引用不存在的灯效模型（如 Daytona 的 `daytonalightson.kn5`）很常见，
//! 不应该让整条赛道加载失败。
//!
//! 放置矩阵直接用 [`vettura_assets::TrackModule`] 给出的 Godot 帧变换
//! （`euler_deg_to_mat4` 已做坐标系共轭），与物理共用同一份数学，保证路面
//! 与碰撞体贴合。一个模型失败不影响其余模型；全部失败时调用方回退到
//! [`super::track_mesh`] 的低模路面。

use std::sync::Arc;
use std::time::Instant;

use godot::classes::{Node, Node3D};
use godot::prelude::*;
use vettura_assets::TrackModule;

use crate::kn5::convert::matrix_to_transform3d;

/// 把 `track.models()` 里所有可视模型实例化到 `parent` 下，返回容器节点。
///
/// 返回 `None` 表示一个模型都没加载成功（全部缺失/损坏），调用方应退回低模。
pub(crate) fn visual_models_instance(
    track: &TrackModule,
    parent: &mut Gd<Node>,
) -> Option<Gd<Node3D>> {
    let started = Instant::now();
    let mut container = Node3D::new_alloc();
    container.set_name("TrackModels");

    let mut loaded = 0usize;
    let mut skipped = 0usize;
    for entry in track.models() {
        if entry.is_collider() {
            continue;
        }
        // `acstobjects.kn5` 是 AC 编辑器放的实例标记（起终点/维修区 digital
        // marker），共用一张 node.dds 图集，里面有红色禁止通行符号。AC 自己
        // 在比赛视图里不显示它们；我们这边会渲染成终点线附近一排巨大的
        // 黑底红杠牌子。跳过这个文件，其它视觉模型照常加载。
        if entry.file.eq_ignore_ascii_case("acstobjects.kn5") {
            continue;
        }
        let path = track.root().join(&entry.file);
        if !path.is_file() {
            godot_warn!(
                "WorldBody: track model '{}' not found; skipping visual",
                entry.file
            );
            skipped += 1;
            continue;
        }

        // 与车模同一份按文件路径的缓存：同一文件只解析一次、贴图只解码一次。
        let Some(resources) = crate::kn5::cache::acquire(&path, || {
            match vettura_kn5::Kn5::parse_owned(std::fs::read(&path).ok()?) {
                Ok(model) => Some(Arc::new(model)),
                Err(err) => {
                    godot_warn!(
                        "WorldBody: track model '{}' parse failed: {}; skipping visual",
                        entry.file,
                        err
                    );
                    None
                }
            }
        }) else {
            skipped += 1;
            continue;
        };

        // 放在单独的 wrapper 节点下：wrapper 只带 `models.ini` 的放置变换，
        // KN5 根节点自带的变换原样保留在里面，两层相乘正好等于
        // `放置 · KN5 局部`，与物理侧 `放置 · 顶点` 一致。
        let mut wrapper = Node3D::new_alloc();
        let stem = path
            .file_stem()
            .map(|raw| crate::kn5::convert::sanitize_node_name(&raw.to_string_lossy()))
            .unwrap_or_else(|| entry.file.clone());
        wrapper.set_name(&StringName::from(stem.as_str()));
        wrapper.set_transform(matrix_to_transform3d(entry.transform()));
        container.add_child(&wrapper.clone().upcast::<Node>());

        let options = crate::kn5::scene::SceneOptions { skip_hidden: true };
        let mut host = wrapper.clone().upcast::<Node>();
        if crate::kn5::scene::instantiate(&resources, &mut host, None, options).is_some() {
            loaded += 1;
        } else {
            // 一个节点都没建出来（空模型 / 根任务失败）：撤掉空 wrapper。
            godot_warn!(
                "WorldBody: track model '{}' produced no nodes; skipping visual",
                entry.file
            );
            container.remove_child(&wrapper.clone().upcast::<Node>());
            wrapper.free();
            skipped += 1;
        }
    }

    if loaded == 0 {
        container.free();
        return None;
    }

    godot_print!(
        "WorldBody: {} track visual models loaded ({} skipped) in {:.1}s",
        loaded,
        skipped,
        started.elapsed().as_secs_f32()
    );
    parent.add_child(&container.clone().upcast::<Node>());
    Some(container)
}
