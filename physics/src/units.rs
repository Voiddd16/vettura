//! SI 单位约定与通用常数。
//!
//! Vettura 内部一律 SI：长度 m、质量 kg、时间 s、力 N、力矩 N·m、角度 rad。
//! AC 数据混用 km/h / 度 / 0..1 归一值——**转换发生在资产边界**
//! (`vettura-assets`)，物理与世界的代码只吃 SI。

/// ISO 80000-3 标准重力加速度，m/s²。
pub const GRAVITY: f32 = 9.806_65;
