//! KN5 → Godot 的桥接层。
//!
//! 分层：
//!
//! - [`convert`]：纯数据转换（坐标、节点名、材质参数），不调用引擎，可单元测试；
//! - [`cache`]：按文件共享的贴图 / 材质 / 解析结果缓存（加载性能的关键）；
//! - [`texture`]：内嵌 DDS 的解码（mip 修补路径），缓存命中时不再重复解码；
//! - [`material`]：AC 材质 → `StandardMaterial3D`；
//! - [`mesh`]：网格顶点 → `ArrayMesh`，以及 dummy / 网格节点实例化；
//! - [`builder`]：把节点树扁平化成任务列表，支持分帧切片加载（进度反馈）；
//! - [`scene`]：`instantiate()` 入口——“一次做完”的包装，内部走 [`builder`]。

pub mod builder;
pub mod cache;
pub mod convert;
pub mod material;
pub mod mesh;
pub mod scene;
pub mod texture;
