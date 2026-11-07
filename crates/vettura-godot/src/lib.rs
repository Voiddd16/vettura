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
//! - [`vehicle_body`](self::vehicle_body)：车身刚体节点，与物理层对接；
//! - [`world`](self::world)：Jolt 世界节点（`WorldBody`），负责赛道加载、多车
//!   调度、圈速计时与车-道-AI 同步，是主项目进入赛车模拟的入口。

// `#[derive(GodotClass)]` generates `base: base` style initializers that this
// lint flags at the field declaration site.
#![allow(clippy::redundant_field_names)]

use godot::prelude::*;

mod ac_car;
mod kn5;
mod kn5_file;
mod paths;
mod vehicle_body;
mod world;

struct VetturaNative;

#[gdextension]
unsafe impl ExtensionLibrary for VetturaNative {}
