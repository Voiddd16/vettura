//! 合成 KN5 / DDS 模型生成器——给基准测试、工具和集成测试共用。
//!
//! 真实 mod 车模有几十 MB，不可能进版本库（见 README「本地测试资源」），
//! 但解析与转换路径的性能必须有可复现的测量输入。这里按 KN5 / DDS 的规范
//! 现场拼出结构完全合法的文件：
//!
//! - 网格几何按真实车模的密度生成（顶点 44 字节、索引 u16）；
//! - 贴图是**合法 DDS**（DXT1、完整或故意截断的 mip 链），Godot 侧能真正走
//!   解码路径，而不是拿到一段无头字节直接失败；
//! - 规模由 [`SynthOptions`] 控制，一致输入 → 一致输出（不掺随机数），
//!   方便把两次测量的差异归因到代码改动而不是输入。
//!
//! ```no_run
//! use vettura_kn5::synth::{synthetic_kn5, SynthOptions};
//!
//! let bytes = synthetic_kn5(SynthOptions::default());
//! # let _ = vettura_kn5::Kn5::parse_owned(bytes);
//! ```

/// 合成模型的规模参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynthOptions {
    /// 网格数量（每个都是独立的静态网格节点）。
    pub meshes: usize,
    /// 每个网格的顶点数；索引按每顶点一个三角形生成。
    pub vertices_per_mesh: usize,
    /// 贴图数量。
    pub textures: usize,
    /// 每张贴图的基础层尺寸（`width_kb` × 1024 字节近似）。
    pub texture_bytes: usize,
    /// 贴图声明的 mip 级数。
    pub texture_mip_levels: u32,
    /// 是否把贴图写成 KN5 导出器常见的“截断 mip 链”（声明 N 级、只给 1 级数据）。
    pub truncated_texture_mips: bool,
}

impl Default for SynthOptions {
    fn default() -> Self {
        Self {
            meshes: 128,
            vertices_per_mesh: 1024,
            textures: 32,
            texture_bytes: 64 * 1024,
            texture_mip_levels: 8,
            truncated_texture_mips: true,
        }
    }
}

/// 按 `options` 生成一段合法的 KN5 字节。
///
/// 生成的模型有一个 dummy 根节点，`meshes` 个静态网格子节点，
/// `textures` 张贴图和 `min(meshes, 16)` 个材质（材质会复用贴图，让材质解析
/// 与贴图解码都按真实比例发生）。
pub fn synthetic_kn5(options: SynthOptions) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(estimated_size(options));
    write_header(&mut bytes, 5);

    // 贴图表。
    bytes.extend_from_slice(&(options.textures as u32).to_le_bytes());
    for index in 0..options.textures {
        bytes.extend_from_slice(&1i32.to_le_bytes()); // active
        push_string(&mut bytes, &format!("tex{index}.dds"));
        let dds = synthetic_dds(
            options.texture_bytes,
            options.texture_mip_levels,
            options.truncated_texture_mips,
        );
        bytes.extend_from_slice(&(dds.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&dds);
    }

    // 材质表。
    let material_count = options.meshes.clamp(1, 16);
    bytes.extend_from_slice(&(material_count as u32).to_le_bytes());
    for index in 0..material_count {
        push_string(&mut bytes, &format!("mat{index}"));
        push_string(&mut bytes, "ksPerPixelMultiMap");
        bytes.push(0); // Opaque
        bytes.push(0); // 非 alpha test
        bytes.extend_from_slice(&0i32.to_le_bytes()); // Normal depth

        bytes.extend_from_slice(&8u32.to_le_bytes());
        for property in 0..8 {
            push_string(&mut bytes, &format!("prop{property}"));
            push_f32(&mut bytes, property as f32);
            for value in 0..2 {
                push_f32(&mut bytes, value as f32);
            }
            for value in 0..3 {
                push_f32(&mut bytes, value as f32);
            }
            for value in 0..4 {
                push_f32(&mut bytes, value as f32);
            }
        }

        bytes.extend_from_slice(&2u32.to_le_bytes());
        push_string(&mut bytes, "txDiffuse");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_string(
            &mut bytes,
            &format!("tex{}.dds", index % options.textures.max(1)),
        );
        push_string(&mut bytes, "txNormal");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        push_string(
            &mut bytes,
            &format!("tex{}.dds", (index + 1) % options.textures.max(1)),
        );
    }

    // 根 dummy + 每个网格一个静态网格子节点。
    push_node_header(&mut bytes, 1, "Root", options.meshes as i32);
    for row in 0..4 {
        for column in 0..4 {
            push_f32(&mut bytes, if row == column { 1.0 } else { 0.0 });
        }
    }

    let vertices = options.vertices_per_mesh.clamp(3, 65_535);
    for mesh in 0..options.meshes {
        push_node_header(&mut bytes, 2, &format!("mesh{mesh}"), 0);
        bytes.push(1); // cast shadows
        bytes.push(1); // visible
        bytes.push(0); // transparent

        bytes.extend_from_slice(&(vertices as u32).to_le_bytes());
        for vertex in 0..vertices {
            // 位置沿 +X 铺开，法线统一朝上，UV 与切线取确定值。
            push_f32(&mut bytes, vertex as f32 * 0.01);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, vertex as f32 / vertices as f32);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
        }

        let index_count = vertices * 3;
        bytes.extend_from_slice(&(index_count as u32).to_le_bytes());
        for vertex in 0..vertices {
            let current = vertex as u16;
            let next = ((vertex + 1) % vertices) as u16;
            let after = ((vertex + 2) % vertices) as u16;
            bytes.extend_from_slice(&current.to_le_bytes());
            bytes.extend_from_slice(&next.to_le_bytes());
            bytes.extend_from_slice(&after.to_le_bytes());
        }

        bytes.extend_from_slice(&((mesh % material_count) as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // layer
        push_f32(&mut bytes, 0.0); // LOD in
        push_f32(&mut bytes, 1.0e9); // LOD out
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 1.0); // 包围球半径
        bytes.push(1); // is renderable
    }

    bytes
}

/// 生成一段合法的 DDS 字节（DXT1）。
///
/// `base_bytes` 决定基础层 payload 的近似大小（真实 DXT1 是 `w*h/2`，这里按
/// 请求的字节数取整到 4x4 块的倍数）。`declared_levels` 是头部声明的 mip
/// 级数；`truncated` 为 `true` 时只写入一级数据——这正好复现 KN5 导出器裁剪
/// mip 链的行为，用来测 Godot 侧的 mip 修补路径。
pub fn synthetic_dds(base_bytes: usize, declared_levels: u32, truncated: bool) -> Vec<u8> {
    // 用 2 的幂尺寸：DXT1 的基础层是 `side²/2` 字节，这样按“每级 1/4 像素”
    // 推算出的 mip 链与 `DdsLayout::full_chain_size()` 的账严格对齐。
    let side = side_for_base_bytes(base_bytes);
    let level_zero = (side as usize * side as usize) / 2;
    let levels = if truncated { 1 } else { levels_for(side, side) };

    let mut payload = level_zero;
    for level in 1..levels {
        // 与 `crate::dds_layout` 的 `level_bytes` 同一算法，账才对得上。
        let dim = (side >> level).max(1);
        payload += dim.div_ceil(4) as usize * dim.div_ceil(4) as usize * 8;
    }

    let mut bytes = vec![0u8; 128 + payload];
    bytes[..4].copy_from_slice(b"DDS ");
    bytes[4..8].copy_from_slice(&124u32.to_le_bytes());

    const DDSD_CAPS: u32 = 0x1;
    const DDSD_HEIGHT: u32 = 0x2;
    const DDSD_WIDTH: u32 = 0x4;
    /// 压缩格式（DXT/BC）用这个而不是 `DDSD_PITCH`：Godot 会校验
    /// “未声明 LINEARSize 却给了非零 pitch”并整张贴图拒掉。
    const DDSD_LINEARSIZE: u32 = 0x8_0000;
    const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
    let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_LINEARSIZE;
    if levels > 1 {
        flags |= DDSD_MIPMAPCOUNT;
    }
    bytes[8..12].copy_from_slice(&flags.to_le_bytes());

    // 用一个接近方形的尺寸，让 `full_chain_size` 的账对得上。
    let side = side_for_base_bytes(base_bytes);
    bytes[12..16].copy_from_slice(&side.to_le_bytes()); // height
    bytes[16..20].copy_from_slice(&side.to_le_bytes()); // width
    bytes[20..24].copy_from_slice(&(level_zero as u32).to_le_bytes()); // pitch / linear size
    // 非截断时声明真实级数；截断时保留“撒谎”的级数——这正是 KN5 导出器的行为，
    // Godot 侧要靠它触发 mip 修补。
    let declared = if truncated {
        declared_levels.max(1)
    } else {
        levels
    };
    bytes[28..32].copy_from_slice(&declared.to_le_bytes());

    // DDS_PIXELFORMAT：只有 FourCC（DXT1）。
    bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&0x4u32.to_le_bytes()); // DDPF_FOURCC
    bytes[84..88].copy_from_slice(b"DXT1");

    const DDSCAPS_TEXTURE: u32 = 0x1000;
    const DDSCAPS_MIPMAP: u32 = 0x40_0000;
    const DDSCAPS_COMPLEX: u32 = 0x8;
    let mut caps = DDSCAPS_TEXTURE;
    if levels > 1 {
        caps |= DDSCAPS_MIPMAP | DDSCAPS_COMPLEX;
    }
    bytes[108..112].copy_from_slice(&caps.to_le_bytes());

    // 像素填确定性图案：全零也能解码，但非零数据更容易暴露对齐错误。
    for (index, byte) in bytes[128..].iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }

    bytes
}

/// KN5 文件头：magic + 版本号（版本 5 没有额外字段）。
fn write_header(bytes: &mut Vec<u8>, version: u32) {
    bytes.extend_from_slice(b"sc6969");
    bytes.extend_from_slice(&version.to_le_bytes());
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_node_header(bytes: &mut Vec<u8>, node_type: i32, name: &str, children: i32) {
    bytes.extend_from_slice(&node_type.to_le_bytes());
    push_string(bytes, name);
    bytes.extend_from_slice(&children.to_le_bytes());
    bytes.push(1); // active
}

/// 取一个 2 的幂边长，使 DXT1 基础层（`side²/2`）不小于 `base_bytes`。
fn side_for_base_bytes(base_bytes: usize) -> u32 {
    let needed = base_bytes.max(8) as f64;
    let side = (2.0 * needed).sqrt().ceil().max(4.0) as u32;
    side.next_power_of_two()
}

/// 维度上最多可以有多少级 mip（含基础层）；与 [`crate::dds_layout`] 的口径一致。
fn levels_for(width: u32, height: u32) -> u32 {
    32 - width.max(height).leading_zeros()
}

fn estimated_size(options: SynthOptions) -> usize {
    let geometry = options
        .meshes
        .saturating_mul(options.vertices_per_mesh.min(65_535))
        .saturating_mul(44 + 6);
    let textures = options
        .textures
        .saturating_mul(options.texture_bytes)
        .saturating_mul(2);
    geometry + textures + 4096
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kn5, dds_layout};

    #[test]
    fn synthetic_model_parses_with_the_expected_shape() {
        let options = SynthOptions {
            meshes: 8,
            vertices_per_mesh: 64,
            textures: 4,
            texture_bytes: 8 * 1024,
            ..SynthOptions::default()
        };
        let bytes = synthetic_kn5(options);

        let kn5 = Kn5::parse(&bytes).expect("合成模型应当是合法 KN5");
        assert_eq!(kn5.version, 5);
        assert_eq!(kn5.textures.len(), options.textures);
        assert_eq!(kn5.materials.len(), options.meshes);
        assert_eq!(kn5.mesh_count(), options.meshes);
        assert_eq!(kn5.node_count(), options.meshes + 1);
        assert_eq!(
            kn5.triangle_count(),
            options.meshes * options.vertices_per_mesh
        );
        assert_eq!(kn5.root.children.len(), options.meshes);
    }

    #[test]
    fn synthetic_textures_are_valid_dds() {
        let dds = synthetic_dds(64 * 1024, 8, false);

        let layout = dds_layout(&dds).expect("合成贴图应当是合法 DDS");
        assert_eq!(layout.format, crate::DdsFormat::Bc1);
        assert_eq!(layout.bytes_per_block, Some(8));
        assert_eq!(layout.declared_levels, layout.max_levels());
        assert_eq!(layout.payload_len, layout.full_chain_size());
    }

    #[test]
    fn truncated_textures_look_like_kn5_exports() {
        let dds = synthetic_dds(64 * 1024, 8, true);

        let layout = dds_layout(&dds).expect("截断 mip 链的贴图也应当是合法 DDS");
        assert_eq!(layout.declared_levels, 8);
        assert!(layout.payload_len < layout.full_chain_size());
    }

    #[test]
    fn truncated_model_parses_and_points_at_short_chains() {
        let options = SynthOptions {
            meshes: 2,
            vertices_per_mesh: 32,
            textures: 3,
            texture_bytes: 16 * 1024,
            truncated_texture_mips: true,
            ..SynthOptions::default()
        };
        let kn5 = Kn5::parse_owned(synthetic_kn5(options)).expect("合法 KN5");

        for texture in &kn5.textures {
            let layout = dds_layout(texture.data.as_slice()).expect("合法 DDS 头");
            assert!(layout.payload_len < layout.full_chain_size());
        }
    }

    #[test]
    fn synthetic_model_is_deterministic() {
        let options = SynthOptions {
            meshes: 4,
            vertices_per_mesh: 16,
            textures: 2,
            texture_bytes: 4 * 1024,
            ..SynthOptions::default()
        };
        assert_eq!(synthetic_kn5(options), synthetic_kn5(options));
    }
}
