//! KN5 容器各区块的解析实现。
//!
//! 按文件顺序推进字节游标：文件头（[`Kn5::parse_shared`]）、贴图表、材质表、
//! 节点树。这里只做字节游标推进与结构组装，数据模型定义见 [`crate::model`]。

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::model::{
    BlendMode, Bone, DepthMode, Kn5, Material, MaterialProperty, Mesh, MeshFlags, Node, NodeKind,
    SharedBytes, SkinnedMesh, SkinnedVertex, Texture, TextureData, TextureSample, Vertex,
};
use crate::reader::Reader;
use crate::{MAGIC, MAX_NODE_DEPTH};

/// 把一段连续的 4 字节小端数据批量转换为 `f32` 数组。
///
/// 调用方保证 `bytes.len() == N * 4`。一次切片读取再顺序转换，比为每个字段
/// 单独调用 `Reader` 少做很多边界检查。
fn f32s_from_bytes<const N: usize>(bytes: &[u8]) -> [f32; N] {
    debug_assert_eq!(bytes.len(), N * 4);
    let mut values = [0.0; N];
    for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*chunk);
    }
    values
}

impl Kn5 {
    /// 从文件路径读取并解析完整的 KN5。
    ///
    /// 文件只读取一次，贴图直接指向同一块内存，不会为每张贴图单独复制。
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::parse_owned(bytes)
    }

    /// 从内存中的完整字节解析 KN5。
    ///
    /// 这是最方便的入口，但传入的是借用切片，内部必须把整份数据复制一次
    /// （`Vec<u8>` → `Arc<Vec<u8>>`）。追求零拷贝时请使用
    /// [`Kn5::parse_owned`] 或 [`Kn5::parse_shared`]。
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_owned(bytes.to_vec())
    }

    /// 从已有的 `Vec<u8>` 解析，转换到共享缓冲区时不复制文件内容。
    pub fn parse_owned(bytes: Vec<u8>) -> Result<Self> {
        Self::parse_shared(Arc::new(bytes))
    }

    /// 从共享缓冲区解析。所有贴图共享同一个 `Arc`，只保存字节范围。
    ///
    /// 各区块顺序固定：文件头、贴图表、材质表、节点树。
    pub fn parse_shared(source: SharedBytes) -> Result<Self> {
        let mut reader = Reader::new(source.as_slice());

        let magic: [u8; 6] = reader.take(6)?.try_into().unwrap();
        if &magic != MAGIC {
            return Err(Error::InvalidMagic(magic));
        }

        let version = reader.u32()?;
        if !(5..=6).contains(&version) {
            return Err(Error::UnsupportedVersion(version));
        }

        // 版本 6 在版本号后额外写入一个字段。AcTools 会读取但从不使用它；
        // 这里保留下来，方便以后实现字节级回写。
        let header_extra = if version > 5 {
            Some(reader.u32()?)
        } else {
            None
        };

        let textures = parse_textures(&mut reader, &source)?;
        let materials = parse_materials(&mut reader)?;

        let root = parse_node(&mut reader, 0)?;

        let trailing_len = reader.remaining();
        if trailing_len > 0 {
            // 少数导出器会在文件末尾补零；其它非零尾随数据视为损坏。
            let trailing_offset = reader.position();
            let trailing = reader.take(trailing_len)?;
            if trailing.iter().any(|&b| b != 0) {
                return Err(Error::TrailingData(trailing_offset));
            }
        }

        tracing::debug!(
            version,
            textures = textures.len(),
            materials = materials.len(),
            "KN5 parsed"
        );

        Ok(Self {
            version,
            header_extra,
            textures,
            materials,
            root,
        })
    }
}

/// 读取贴图表。
///
/// 每一项依次是 `active: i32`、带长度前缀的名称、`u32` 数据长度和原始 DDS
/// 字节。AcTools 会丢弃长度为 0 的贴图；这里保留全部条目，便于完整检查文件。
///
/// 贴图数据不复制：只记录它在共享缓冲区中的 `Range`，因此 100 MB 的车模不会
/// 因为贴图解析再产生一份等量内存。
fn parse_textures(reader: &mut Reader<'_>, source: &SharedBytes) -> Result<Vec<Texture>> {
    let count = reader.u32()? as usize;
    // 每项至少包含 active、名称长度字段和数据长度字段。
    reader.check_count("texture", count, 12)?;

    let mut textures = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let active = reader.i32()? == 1;
        let name = reader.string()?;
        let size = reader.u32()? as usize;

        let start = reader.position();
        reader.take(size)?;
        let range = start..start + size;

        textures.push(Texture {
            active,
            name,
            data: TextureData::new(Arc::clone(source), range),
        });
    }
    Ok(textures)
}

/// 读取材质表。
///
/// 单个材质的字段顺序为：名称、shader、混合模式（`u8`）、alpha test（`u8`）、
/// 深度模式（`i32`）、shader 属性列表、贴图槽列表。
fn parse_materials(reader: &mut Reader<'_>) -> Result<Vec<Material>> {
    let count = reader.u32()? as usize;
    // 名称长度 + shader 长度 + 混合模式字节 + alpha 字节 + 深度模式 i32 +
    // 属性数量。
    reader.check_count("material", count, 18)?;

    let mut materials = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let name = reader.string()?;
        let shader = reader.string()?;
        let blend_mode = BlendMode::from_raw(reader.u8()?);
        let alpha_tested = reader.bool()?;
        let depth_mode = DepthMode::from_raw(reader.i32()?);

        let property_count = reader.u32()? as usize;
        // 名称长度 + f32 + vec2 + vec3 + vec4。
        reader.check_count("material property", property_count, 44)?;
        let mut properties = Vec::with_capacity(property_count.min(256));
        for _ in 0..property_count {
            let name = reader.string()?;
            // 一次性读入 ValueA/B/C/D 共 40 字节。
            let values = f32s_from_bytes::<10>(reader.take(40)?);
            properties.push(MaterialProperty {
                name,
                value_a: values[0],
                value_b: [values[1], values[2]],
                value_c: [values[3], values[4], values[5]],
                value_d: [values[6], values[7], values[8], values[9]],
            });
        }

        let sample_count = reader.u32()? as usize;
        // 槽名长度 + 槽索引 + 贴图名长度。
        reader.check_count("texture sample", sample_count, 12)?;
        let mut samples = Vec::with_capacity(sample_count.min(64));
        for _ in 0..sample_count {
            samples.push(TextureSample {
                slot_name: reader.string()?,
                slot: reader.u32()?,
                texture: reader.string()?,
            });
        }

        materials.push(Material {
            name,
            shader,
            blend_mode,
            alpha_tested,
            depth_mode,
            properties,
            samples,
        });
    }
    Ok(materials)
}

/// 解析一个节点，并递归解析它的所有子节点。
///
/// 所有节点共享同一段头部：
/// `node_type: i32`、名称、`children_count: i32`、`active: u8`。
fn parse_node(reader: &mut Reader<'_>, depth: usize) -> Result<Node> {
    if depth > MAX_NODE_DEPTH {
        return Err(Error::DepthLimit(MAX_NODE_DEPTH));
    }

    let node_type_offset = reader.position();
    let node_type = reader.i32()?;
    let name = reader.string()?;
    let children_count = reader.i32()?;
    if children_count < 0 {
        return Err(Error::InvalidCount {
            what: "child node",
            count: children_count as usize,
            offset: reader.position(),
            remaining: reader.remaining(),
        });
    }
    let children_count = children_count as usize;
    // 每个子节点至少需要完整的公共头部。
    reader.check_count("child node", children_count, 13)?;

    let active = reader.bool()?;

    let kind = match node_type {
        1 => parse_dummy(reader)?,
        2 => NodeKind::Mesh(Arc::new(parse_mesh(reader)?)),
        3 => NodeKind::SkinnedMesh(Arc::new(parse_skinned_mesh(reader)?)),
        other => return Err(Error::InvalidNodeType(other, node_type_offset)),
    };

    let mut children = Vec::with_capacity(children_count.min(1024));
    for _ in 0..children_count {
        children.push(parse_node(reader, depth + 1)?);
    }

    Ok(Node {
        name,
        active,
        kind,
        children,
    })
}

/// 类型 1：只包含变换的节点，例如车轮、悬挂枢轴和相机锚点。
fn parse_dummy(reader: &mut Reader<'_>) -> Result<NodeKind> {
    Ok(NodeKind::Dummy {
        transform: reader.matrix()?,
    })
}

/// 读取静态网格和蒙皮网格共用的三个渲染标志：
/// 投影阴影、可见、透明。
fn read_mesh_flags(reader: &mut Reader<'_>) -> Result<MeshFlags> {
    Ok(MeshFlags {
        cast_shadows: reader.bool()?,
        is_visible: reader.bool()?,
        is_transparent: reader.bool()?,
    })
}

/// 类型 2：静态网格。
fn parse_mesh(reader: &mut Reader<'_>) -> Result<Mesh> {
    let flags = read_mesh_flags(reader)?;

    let vertex_count = reader.u32()? as usize;
    // position + normal + uv + tangent。
    reader.check_count("mesh vertex", vertex_count, 44)?;
    let raw_vertices = reader.take_array(vertex_count, 44)?;
    let mut vertices = Vec::with_capacity(vertex_count);
    for bytes in raw_vertices.as_chunks::<44>().0 {
        let values = f32s_from_bytes::<11>(bytes);
        vertices.push(Vertex {
            position: [values[0], values[1], values[2]],
            normal: [values[3], values[4], values[5]],
            uv: [values[6], values[7]],
            tangent: [values[8], values[9], values[10]],
        });
    }

    let index_count = reader.u32()? as usize;
    reader.check_count("mesh index", index_count, 2)?;
    let indices = reader.u16_vec(index_count)?;

    Ok(Mesh {
        flags,
        vertices,
        indices,
        material: reader.u32()?,
        layer: reader.u32()?,
        lod_in: reader.f32()?,
        lod_out: reader.f32()?,
        bounding_sphere_center: reader.f32_array::<3>()?,
        bounding_sphere_radius: reader.f32()?,
        is_renderable: reader.bool()?,
    })
}

/// 类型 3：蒙皮网格。
///
/// 布局与静态网格不同：骨骼数据位于顶点之前，每个顶点多带 32 字节蒙皮数据，
/// 尾部只包含 LOD，没有包围球和 `is_renderable` 标志。
fn parse_skinned_mesh(reader: &mut Reader<'_>) -> Result<SkinnedMesh> {
    let flags = read_mesh_flags(reader)?;

    let bone_count = reader.u32()? as usize;
    // 骨骼名长度字段 + 4x4 f32 矩阵。
    reader.check_count("bone", bone_count, 68)?;
    let mut bones = Vec::with_capacity(bone_count.min(1024));
    for _ in 0..bone_count {
        bones.push(Bone {
            name: reader.string()?,
            transform: reader.matrix()?,
        });
    }

    let vertex_count = reader.u32()? as usize;
    // position + normal + uv + tangent + weights + bone indices。
    reader.check_count("skinned vertex", vertex_count, 76)?;
    let raw_vertices = reader.take_array(vertex_count, 76)?;
    let mut vertices = Vec::with_capacity(vertex_count);
    for bytes in raw_vertices.as_chunks::<76>().0 {
        let values = f32s_from_bytes::<19>(bytes);
        vertices.push(SkinnedVertex {
            position: [values[0], values[1], values[2]],
            normal: [values[3], values[4], values[5]],
            uv: [values[6], values[7]],
            tangent: [values[8], values[9], values[10]],
            weights: [values[11], values[12], values[13], values[14]],
            bone_indices: [values[15], values[16], values[17], values[18]],
        });
    }

    let index_count = reader.u32()? as usize;
    reader.check_count("mesh index", index_count, 2)?;
    let indices = reader.u16_vec(index_count)?;

    Ok(SkinnedMesh {
        flags,
        bones,
        vertices,
        indices,
        material: reader.u32()?,
        layer: reader.u32()?,
        lod_in: reader.f32()?,
        lod_out: reader.f32()?,
    })
}
