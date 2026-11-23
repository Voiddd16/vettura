//! Vettura 的 Godot 4 GDExtension 绑定层。
//!
//! 把 Rust 侧的数据结构桥接到 Godot 节点与资源上：
//! KN5 模型实例化、贴图加载与材质创建、AC 车目录访问、车身物理挂点。
//!
//! # 模块结构
//!
//! - [`ac_car`](self::ac_car)：AC 车目录资源（`AcCar`），打开 `data.acd` /
//!   `data/`，映射物理参数，并把主模型实例化到场景树；
//! - [`kn5`](self::kn5)：KN5 → Godot 桥接层，按职责分五层（convert 纯转换 /
//!   texture 贴图解码缓存 / material / mesh / scene），见 `kn5/mod.rs`；
//! - [`kn5_file`](self::kn5_file)：`.kn5` 文件资源（`Kn5File`），供编辑器与
//!   脚本查询统计、按需构建独立 `ArrayMesh` / 材质；
//! - [`paths`](self::paths)：把 Godot 的 `res://` 路径解析成 `std::fs` 能用的
//!   文件系统路径，避免上层脚本自己调 `globalize_path`；
//! - [`logging`](self::logging)：全系统日志（`user://log.cfg` 驱动的环形缓冲 +
//!   控制台 + 滚动文件 subscriber，`VetturaLogger` 供 GDScript 运行期调级别 /
//!   读最近日志；崩溃时 panic hook 写 `user://logs/last-crash.log`），业务
//!   crate 只发 `tracing` 事件，安装与落盘都在这里；
//! - [`vehicle_body`](self::vehicle_body)：车身刚体节点，与物理层对接；
//! - [`world`](self::world)：Jolt 世界节点（`WorldBody`），负责赛道加载、多车
//!   调度、圈速计时与车-道-AI 同步，是主项目进入赛车模拟的入口。

// `#[derive(GodotClass)]` generates `base: base` style initializers that this
// lint flags at the field declaration site.
#![allow(clippy::redundant_field_names)]

use godot::prelude::*;

mod ac_car;
mod fmod_system;
mod kn5;
mod kn5_file;
mod logging;
mod paths;
mod vehicle_body;
mod visual_compat;
mod world;

struct VetturaNative;

#[gdextension]
unsafe impl ExtensionLibrary for VetturaNative {
    fn on_stage_init(stage: InitStage) {
        // 日志必须早于任何业务场景事件装上，否则早期事件仍可能丢失。
        // 选 Scene 而不是更早的 Core/Servers：业务日志都发生在场景加载之后，
        // 且保持默认 min_level = Scene 不动扩展的加载级别。
        if stage == InitStage::Scene {
            logging::init_from_user_config();
        }
    }

    // 每帧低频轮询（内部 0.5s 节流）：日志配置热重载 + warn 聚合 flush。
    // 净成本是每帧一次 Instant 比较，600 Hz 主循环无感。
    // 注意不要加 `#[cfg(since_api)]`——那是 godot-ffi 内部 build.rs 的 cfg，
    // 不会传播到本 crate，加了会被静默裁掉。
    fn on_main_loop_frame() {
        logging::poll_config_reload();
    }
}
