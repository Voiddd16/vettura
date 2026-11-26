//! Assetto Corsa `.kn5` 模型容器的读取器。
//!
//! 二进制布局参考社区广泛使用的 `AcTools.Kn5File`（Content Manager 背后的
//! 解析器）。Rust 版本保持相同的字段顺序和容错策略，但用普通结构体暴露数据，
//! 而不是 C# 的字典和类层级。
//!
//! 格式说明见 `docs/kn5-format.md`。解析器对结构刻意严格（损坏文件应当明确
//! 报错），但对文本编码、未知枚举值和文件尾补零保持宽容。
//!
//! ```no_run
//! let kn5 = vettura_kn5::Kn5::from_path("car.kn5")?;
//! println!("{} nodes, {} textures", kn5.node_count(), kn5.textures.len());
//! # Ok::<(), vettura_kn5::Error>(())
//! ```

mod dds;
mod error;
mod model;
mod parse;
mod reader;
#[doc(hidden)]
pub mod synth;
#[cfg(test)]
mod tests;

pub use dds::{
    DdsError, DdsFormat, DdsInfo, DdsLayout, DecodedBase, decode_base_rgba, fix_orphan_pitch_value,
    layout as dds_layout, strip_mip_levels, truncate_to_base_level, truncate_to_declared_mips,
};
pub use error::{Error, Result};
pub use model::{
    BlendMode, Bone, DepthMode, Kn5, Material, MaterialProperty, Mesh, MeshFlags, Node, NodeKind,
    SharedBytes, SkinnedMesh, SkinnedVertex, Texture, TextureData, TextureSample, Transform,
    Vertex, transform_to_mat4,
};

/// 文件头魔数：`sc6969`。
const MAGIC: &[u8; 6] = b"sc6969";
/// 节点树的最大递归深度，超过视为损坏文件。
const MAX_NODE_DEPTH: usize = 256;
