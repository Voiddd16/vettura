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

use godot::classes::geometry_instance_3d::ShadowCastingSetting;
use godot::classes::{MeshInstance3D, Node, Node3D};
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
        let lower = entry.file.to_ascii_lowercase();
        // 物理模型不渲染：`physics.kn5` 里是路面/护墙的碰撞网格（很多是
        // 未贴图的灰模/白模），画出来就是两侧巨大的橙/白墙。
        // `spawns_*.kn5` / `sounds.kn5` 是发车点与音源标记，同样不该出现在画面里。
        if entry.is_collider()
            || lower.contains("physics")
            || lower.contains("spawns")
            || lower.contains("sounds")
        {
            continue;
        }
        // `acstobjects.kn5` 是 AC 编辑器放的实例标记（起终点/维修区 digital
        // marker），共用一张 node.dds 图集，里面有红色禁止通行符号。AC 自己
        // 在比赛视图里不显示它们；我们这边会渲染成终点线附近一排巨大的
        // 黑底红杠牌子。跳过这个文件，其它视觉模型照常加载。
        if entry.file.eq_ignore_ascii_case("acstobjects.kn5") {
            continue;
        }
        let mut path = track.root().join(&entry.file);
        if !path.is_file() {
            // mod 常见的文件名笔误（`models.ini` 写 `marshals.kn5`，目录里是
            // `marshal.kn5`）。尝试单复数互换，找到就用——比整块模型跳过强。
            if let Some(fixed) = singular_plural_variant(&path) {
                godot_warn!(
                    "WorldBody: track model '{}' not found; using '{}' instead",
                    entry.file,
                    fixed.file_name().unwrap_or_default().to_string_lossy()
                );
                path = fixed;
            } else {
                godot_warn!(
                    "WorldBody: track model '{}' not found; skipping visual",
                    entry.file
                );
                skipped += 1;
                continue;
            }
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
            disable_shadow_casting(&wrapper.clone().upcast::<Node>());
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
    // 跨文件贴图去重统计：Nürburgring 这种把同一张图嵌进多个 KN5 的模组，
    // 这里能看到实际共享了多少张（省下的显存）。
    crate::kn5::cache::log_texture_dedup_stats();

    // 按材质合并静态网格：几千个 `MeshInstance3D` → 几百个 draw call，
    // 解决高速行驶时 CPU 主线程被护栏等小网格的绘制提交拖慢的问题。
    // `VETTURA_NO_MERGE=1` 可关闭合并，用于 A/B 排查合并引入的渲染问题。
    let merge_disabled = std::env::var("VETTURA_NO_MERGE")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false);
    if !merge_disabled {
        let merge_stats = crate::kn5::merge::merge_static_children(&mut container);
        godot_print!(
            "WorldBody: merged {} static meshes into {} material batches",
            merge_stats.merged_nodes,
            merge_stats.groups
        );
    }
    parent.add_child(&container.clone().upcast::<Node>());
    Some(container)
}

/// 文件不存在时尝试单复数互换（`marshals.kn5` ↔ `marshal.kn5`）。
fn singular_plural_variant(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let stem = path.file_stem()?.to_string_lossy();
    let extension = path.extension().unwrap_or_default();
    let candidates = if let Some(base) = stem.strip_suffix('s') {
        vec![base.to_string()]
    } else {
        vec![format!("{stem}s")]
    };
    candidates
        .into_iter()
        .map(|name| path.with_file_name(format!("{name}.{}", extension.to_string_lossy())))
        .find(|candidate| candidate.is_file())
}

/// 赛道视觉网格只接收阴影、不投射阴影。
///
/// 赛道路面是跨度数公里的近共面大网格（Daytona 的 `daytonamesh.kn5` 单文件
/// 44 张路面），定向光阴影图的深度精度在这么大的范围里不够，路面会投到
/// 自己身上形成细密的自遮挡条纹（shadow acne），起跑区尤其明显。单纯抬高
/// bias/normal_bias 要么条纹消不干净，要么把车体阴影推离地面（peter-panning）。
/// 赛道不需要在路面上投出自己的影子，关掉投射即可根治；车辆仍照常投射。
fn disable_shadow_casting(node: &Gd<Node>) {
    if let Ok(mut mesh) = node.clone().try_cast::<MeshInstance3D>() {
        mesh.set_cast_shadows_setting(ShadowCastingSetting::OFF);
    }
    for child in node.get_children().iter_shared() {
        disable_shadow_casting(&child);
    }
}
