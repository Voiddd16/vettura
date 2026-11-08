//! KN5 → Godot 的桥接层。
//!
//! 分层：
//!
//! - [`convert`]：纯数据转换（坐标、节点名、材质参数），不调用引擎，可单元测试；
//! - [`texture`]：内嵌 DDS 的惰性解码与缓存；
//! - [`material`]：AC 材质 → `StandardMaterial3D`；
//! - [`mesh`]：网格顶点 → `ArrayMesh`；
//! - [`scene`]：节点树 → `Node3D` / `MeshInstance3D` 层级。

pub mod convert;
pub mod material;
pub mod mesh;
pub mod scene;
pub mod texture;
