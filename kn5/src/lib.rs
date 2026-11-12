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
mod reader;
#[doc(hidden)]
pub mod synth;

pub use dds::{
    DdsError, DdsFormat, DdsInfo, DdsLayout, fix_orphan_pitch_value, layout as dds_layout,
    strip_mip_levels,
};
pub use error::{Error, Result};
pub use model::{
    BlendMode, Bone, DepthMode, Kn5, Material, MaterialProperty, Mesh, MeshFlags, Node, NodeKind,
    SharedBytes, SkinnedMesh, SkinnedVertex, Texture, TextureData, TextureSample, Transform,
    Vertex, transform_to_mat4,
};

use std::sync::Arc;

use reader::Reader;

const MAGIC: &[u8; 6] = b"sc6969";
const MAX_NODE_DEPTH: usize = 256;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 写文件头（magic + 版本号）。
    fn header(version: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes
    }

    /// 写带 `u32` 长度前缀的字符串。
    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    /// 写一个 f32。
    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// 写行主序 4x4 矩阵。
    fn push_matrix(bytes: &mut Vec<u8>, matrix: &Transform) {
        for row in matrix {
            for value in row {
                push_f32(bytes, *value);
            }
        }
    }

    fn identity() -> Transform {
        let mut matrix = [[0.0; 4]; 4];
        for (index, row) in matrix.iter_mut().enumerate() {
            row[index] = 1.0;
        }
        matrix
    }

    /// 写节点公共头部。
    fn push_node_header(
        bytes: &mut Vec<u8>,
        node_type: i32,
        name: &str,
        children: i32,
        active: bool,
    ) {
        bytes.extend_from_slice(&node_type.to_le_bytes());
        push_string(bytes, name);
        bytes.extend_from_slice(&children.to_le_bytes());
        bytes.push(u8::from(active));
    }

    /// 写一个完整的 dummy 节点（含变换）。
    fn push_dummy(
        bytes: &mut Vec<u8>,
        name: &str,
        children: i32,
        active: bool,
        transform: &Transform,
    ) {
        push_node_header(bytes, 1, name, children, active);
        push_matrix(bytes, transform);
    }

    /// 写一个贴图条目。
    fn push_texture(bytes: &mut Vec<u8>, active: bool, name: &str, data: &[u8]) {
        bytes.extend_from_slice(&(if active { 1i32 } else { 0i32 }).to_le_bytes());
        push_string(bytes, name);
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
    }

    /// 写一个 shader 属性。
    fn push_property(
        bytes: &mut Vec<u8>,
        name: &str,
        value_a: f32,
        value_b: [f32; 2],
        value_c: [f32; 3],
        value_d: [f32; 4],
    ) {
        push_string(bytes, name);
        push_f32(bytes, value_a);
        for value in value_b {
            push_f32(bytes, value);
        }
        for value in value_c {
            push_f32(bytes, value);
        }
        for value in value_d {
            push_f32(bytes, value);
        }
    }

    /// 测试用 shader 属性元组：(名称, ValueA, ValueB, ValueC, ValueD)。
    type TestProperty<'a> = (&'a str, f32, [f32; 2], [f32; 3], [f32; 4]);

    /// 测试用贴图槽元组：(槽名, 槽索引, 贴图名)。
    type TestSample<'a> = (&'a str, u32, &'a str);

    /// 写一个材质条目。
    struct TestMaterial<'a> {
        name: &'a str,
        shader: &'a str,
        blend: u8,
        alpha_tested: bool,
        depth: i32,
        properties: &'a [TestProperty<'a>],
        samples: &'a [TestSample<'a>],
    }

    fn push_material(bytes: &mut Vec<u8>, material: &TestMaterial<'_>) {
        push_string(bytes, material.name);
        push_string(bytes, material.shader);
        bytes.push(material.blend);
        bytes.push(u8::from(material.alpha_tested));
        bytes.extend_from_slice(&material.depth.to_le_bytes());

        bytes.extend_from_slice(&(material.properties.len() as u32).to_le_bytes());
        for (name, a, b, c, d) in material.properties {
            push_property(bytes, name, *a, *b, *c, *d);
        }

        bytes.extend_from_slice(&(material.samples.len() as u32).to_le_bytes());
        for (slot_name, slot, texture) in material.samples {
            push_string(bytes, slot_name);
            bytes.extend_from_slice(&slot.to_le_bytes());
            push_string(bytes, texture);
        }
    }

    /// 写一个静态网格节点。
    #[allow(clippy::too_many_arguments)]
    fn push_mesh_node(
        bytes: &mut Vec<u8>,
        name: &str,
        children: i32,
        active: bool,
        flags: MeshFlags,
        vertices: &[Vertex],
        indices: &[u16],
        material: u32,
        layer: u32,
        lod_in: f32,
        lod_out: f32,
        bounding_sphere_center: [f32; 3],
        bounding_sphere_radius: f32,
        is_renderable: bool,
    ) {
        push_node_header(bytes, 2, name, children, active);
        bytes.push(u8::from(flags.cast_shadows));
        bytes.push(u8::from(flags.is_visible));
        bytes.push(u8::from(flags.is_transparent));

        bytes.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        for vertex in vertices {
            for value in vertex.position {
                push_f32(bytes, value);
            }
            for value in vertex.normal {
                push_f32(bytes, value);
            }
            for value in vertex.uv {
                push_f32(bytes, value);
            }
            for value in vertex.tangent {
                push_f32(bytes, value);
            }
        }

        bytes.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        for index in indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }

        bytes.extend_from_slice(&material.to_le_bytes());
        bytes.extend_from_slice(&layer.to_le_bytes());
        push_f32(bytes, lod_in);
        push_f32(bytes, lod_out);
        for value in bounding_sphere_center {
            push_f32(bytes, value);
        }
        push_f32(bytes, bounding_sphere_radius);
        bytes.push(u8::from(is_renderable));
    }

    /// 写一个蒙皮网格节点。
    #[allow(clippy::too_many_arguments)]
    fn push_skinned_node(
        bytes: &mut Vec<u8>,
        name: &str,
        children: i32,
        active: bool,
        flags: MeshFlags,
        bones: &[(&str, Transform)],
        vertices: &[SkinnedVertex],
        indices: &[u16],
        material: u32,
        layer: u32,
        lod_in: f32,
        lod_out: f32,
    ) {
        push_node_header(bytes, 3, name, children, active);
        bytes.push(u8::from(flags.cast_shadows));
        bytes.push(u8::from(flags.is_visible));
        bytes.push(u8::from(flags.is_transparent));

        bytes.extend_from_slice(&(bones.len() as u32).to_le_bytes());
        for (name, transform) in bones {
            push_string(bytes, name);
            push_matrix(bytes, transform);
        }

        bytes.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        for vertex in vertices {
            for value in vertex.position {
                push_f32(bytes, value);
            }
            for value in vertex.normal {
                push_f32(bytes, value);
            }
            for value in vertex.uv {
                push_f32(bytes, value);
            }
            for value in vertex.tangent {
                push_f32(bytes, value);
            }
            for value in vertex.weights {
                push_f32(bytes, value);
            }
            for value in vertex.bone_indices {
                push_f32(bytes, value);
            }
        }

        bytes.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        for index in indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }

        bytes.extend_from_slice(&material.to_le_bytes());
        bytes.extend_from_slice(&layer.to_le_bytes());
        push_f32(bytes, lod_in);
        push_f32(bytes, lod_out);
    }

    fn vertex(position: [f32; 3]) -> Vertex {
        Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let err = Kn5::parse(b"nope!!\0\0\0\0").unwrap_err();
        assert!(matches!(err, Error::InvalidMagic(_)));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&99u32.to_le_bytes());
        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::UnsupportedVersion(99)));
    }

    #[test]
    fn parses_empty_model_with_dummy_root() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量
        push_dummy(&mut bytes, "Root", 0, true, &identity());

        let kn5 = Kn5::parse(&bytes).expect("最小模型应当解析成功");
        assert_eq!(kn5.version, 5);
        assert_eq!(kn5.header_extra, None);
        assert_eq!(kn5.root.name, "Root");
        assert!(kn5.root.active);
        assert_eq!(kn5.root.children.len(), 0);
        assert_eq!(kn5.node_count(), 1);
        assert_eq!(kn5.mesh_count(), 0);
    }

    #[test]
    fn version_six_keeps_extra_header_word() {
        let mut bytes = header(6);
        bytes.extend_from_slice(&7u32.to_le_bytes()); // 额外头字段
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量
        push_dummy(&mut bytes, "Root", 0, true, &identity());

        let kn5 = Kn5::parse(&bytes).expect("版本 6 模型应当解析成功");
        assert_eq!(kn5.header_extra, Some(7));
    }

    #[test]
    fn parses_texture_table() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        push_texture(&mut bytes, true, "body.dds", b"DDS-body");
        push_texture(&mut bytes, false, "unused.dds", b"");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量
        push_dummy(&mut bytes, "Root", 0, true, &identity());

        let kn5 = Kn5::parse(&bytes).expect("贴图表应当解析成功");
        assert_eq!(kn5.textures.len(), 2);
        assert!(kn5.textures[0].active);
        assert_eq!(kn5.textures[0].name, "body.dds");
        assert_eq!(kn5.textures[0].data.as_slice(), b"DDS-body");
        assert!(!kn5.textures[1].active);
        assert_eq!(kn5.textures[1].name, "unused.dds");
        assert!(kn5.textures[1].data.is_empty());
    }

    #[test]
    fn textures_share_the_input_buffer() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        push_texture(&mut bytes, true, "a.dds", b"DDS-a");
        push_texture(&mut bytes, true, "b.dds", b"DDS-b");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量
        push_dummy(&mut bytes, "Root", 0, true, &identity());

        let source: SharedBytes = Arc::new(bytes);
        let kn5 = Kn5::parse_shared(Arc::clone(&source)).expect("共享输入应当解析成功");

        assert_eq!(kn5.textures.len(), 2);
        for texture in &kn5.textures {
            // 两个贴图必须指向同一个缓冲区，而不是各自的拷贝。
            assert!(Arc::ptr_eq(texture.data.source(), &source));
            let range = texture.data.range();
            assert_eq!(&source[range], texture.data.as_slice());
        }
    }

    #[test]
    fn parses_material_fields_and_unknown_enums() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&2u32.to_le_bytes());

        push_material(
            &mut bytes,
            &TestMaterial {
                name: "body",
                shader: "ksPerPixelMultiMap",
                blend: 1,
                alpha_tested: true,
                depth: 1,
                properties: &[(
                    "ksAmbient",
                    0.5,
                    [0.1, 0.2],
                    [0.3, 0.4, 0.5],
                    [1.0, 2.0, 3.0, 4.0],
                )],
                samples: &[("txDiffuse", 0, "body.dds")],
            },
        );
        push_material(
            &mut bytes,
            &TestMaterial {
                name: "mystery",
                shader: "custom",
                blend: 9,
                alpha_tested: false,
                depth: -7,
                properties: &[],
                samples: &[],
            },
        );

        push_dummy(&mut bytes, "Root", 0, true, &identity());

        let kn5 = Kn5::parse(&bytes).expect("材质表应当解析成功");
        let body = &kn5.materials[0];
        assert_eq!(body.name, "body");
        assert_eq!(body.shader, "ksPerPixelMultiMap");
        assert_eq!(body.blend_mode, BlendMode::AlphaBlend);
        assert!(body.alpha_tested);
        assert_eq!(body.depth_mode, DepthMode::NoWrite);
        assert_eq!(body.property("ksamBIENT"), Some(0.5));
        let property = body.property_values("ksAmbient").unwrap();
        assert_eq!(property.value_b, [0.1, 0.2]);
        assert_eq!(property.value_c, [0.3, 0.4, 0.5]);
        assert_eq!(property.value_d, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(body.texture("TXDIFFUSE"), Some("body.dds"));
        assert_eq!(body.samples[0].slot, 0);

        // 未知枚举值必须原样保留，不能被静默改成默认值。
        let mystery = &kn5.materials[1];
        assert_eq!(mystery.blend_mode, BlendMode::Unknown(9));
        assert_eq!(mystery.depth_mode, DepthMode::Unknown(-7));
    }

    #[test]
    fn parses_static_mesh_node_fields() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量

        let vertices = [
            vertex([0.0, 0.0, 0.0]),
            vertex([1.0, 0.0, 0.0]),
            vertex([0.0, 0.0, 1.0]),
        ];
        let flags = MeshFlags {
            cast_shadows: true,
            is_visible: false,
            is_transparent: true,
        };
        push_mesh_node(
            &mut bytes,
            "body",
            0,
            true,
            flags,
            &vertices,
            &[0, 1, 2],
            3,
            5,
            10.0,
            20.0,
            [1.0, 2.0, 3.0],
            4.5,
            true,
        );

        let kn5 = Kn5::parse(&bytes).expect("静态网格应当解析成功");
        let mesh = kn5.root.as_mesh().expect("根节点应当是静态网格");
        assert_eq!(kn5.root.name, "body");
        assert!(kn5.root.active);
        assert_eq!(mesh.flags, flags);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.material, 3);
        assert_eq!(mesh.material_index(), Some(3));
        assert_eq!(mesh.layer, 5);
        assert_eq!(mesh.lod_in, 10.0);
        assert_eq!(mesh.lod_out, 20.0);
        assert_eq!(mesh.bounding_sphere_center, [1.0, 2.0, 3.0]);
        assert_eq!(mesh.bounding_sphere_radius, 4.5);
        assert!(mesh.is_renderable);
        assert_eq!(kn5.mesh_count(), 1);
        assert_eq!(kn5.triangle_count(), 1);
    }

    #[test]
    fn parses_skinned_mesh_node_fields() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量

        let bone_transform = identity();
        let flags = MeshFlags {
            cast_shadows: false,
            is_visible: true,
            is_transparent: false,
        };
        let vertices = [
            SkinnedVertex {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: [1.0, 0.0, 0.0],
                weights: [0.75, 0.25, 0.0, 0.0],
                bone_indices: [0.0, -1.0, -1.0, -1.0],
            },
            SkinnedVertex {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 0.0],
                tangent: [1.0, 0.0, 0.0],
                weights: [1.0, 0.0, 0.0, 0.0],
                bone_indices: [0.0, -1.0, -1.0, -1.0],
            },
        ];
        push_skinned_node(
            &mut bytes,
            "driver",
            0,
            false,
            flags,
            &[("Bone", bone_transform)],
            &vertices,
            &[0, 1, 0],
            2,
            4,
            1.0,
            100.0,
        );

        let kn5 = Kn5::parse(&bytes).expect("蒙皮网格应当解析成功");
        let mesh = kn5.root.as_skinned_mesh().expect("根节点应当是蒙皮网格");
        assert!(!kn5.root.active);
        assert_eq!(mesh.flags, flags);
        assert_eq!(mesh.bones.len(), 1);
        assert_eq!(mesh.bones[0].name, "Bone");
        assert_eq!(mesh.vertices.len(), 2);
        assert_eq!(mesh.vertices[0].weights, [0.75, 0.25, 0.0, 0.0]);
        assert_eq!(mesh.vertices[0].bone_indices, [0.0, -1.0, -1.0, -1.0]);
        assert_eq!(mesh.indices, [0, 1, 0]);
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.material, 2);
        assert_eq!(mesh.layer, 4);
        assert_eq!(mesh.lod_in, 1.0);
        assert_eq!(mesh.lod_out, 100.0);
    }

    #[test]
    fn parses_nested_hierarchy_and_finds_nodes() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 贴图数量
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 材质数量

        // 根 dummy 有两个子节点：一个车轮 dummy 和一个空网格。
        push_dummy(&mut bytes, "Root", 2, true, &identity());
        push_dummy(&mut bytes, "WHEEL_LF", 0, true, &identity());
        push_mesh_node(
            &mut bytes,
            "body",
            0,
            false,
            MeshFlags::default(),
            &[],
            &[],
            0,
            0,
            0.0,
            0.0,
            [0.0; 3],
            0.0,
            true,
        );

        let kn5 = Kn5::parse(&bytes).expect("层级树应当解析成功");
        assert_eq!(kn5.node_count(), 3);
        assert_eq!(kn5.mesh_count(), 1);
        assert!(kn5.find_node("wheel_lf").is_some());
        assert!(kn5.find_node("WHEEL_RF").is_none());

        let mut visited = Vec::new();
        kn5.walk(|node, depth| visited.push((node.name.clone(), depth)));
        assert_eq!(
            visited,
            vec![
                ("Root".to_owned(), 0),
                ("WHEEL_LF".to_owned(), 1),
                ("body".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn accepts_zero_padding_after_root() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_dummy(&mut bytes, "Root", 0, true, &identity());
        bytes.extend_from_slice(&[0u8; 16]);

        assert!(Kn5::parse(&bytes).is_ok());
    }

    #[test]
    fn rejects_non_zero_trailing_data() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_dummy(&mut bytes, "Root", 0, true, &identity());
        bytes.push(1);

        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::TrailingData(_)));
    }

    #[test]
    fn rejects_unknown_node_type() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_node_header(&mut bytes, 99, "Root", 0, true);

        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidNodeType(99, _)));
    }

    #[test]
    fn rejects_negative_child_count() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        push_string(&mut bytes, "Root");
        bytes.extend_from_slice(&(-1i32).to_le_bytes());

        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidCount {
                what: "child node",
                ..
            }
        ));
    }

    #[test]
    fn rejects_truncated_vertex_data() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        push_node_header(&mut bytes, 2, "body", 0, true);
        bytes.extend_from_slice(&[1, 1, 1]); // 三个网格标志
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 声称有 1 个顶点，但没有数据

        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidCount { .. } | Error::Eof { .. }
        ));
    }

    #[test]
    fn rejects_deeply_nested_nodes() {
        let mut bytes = header(5);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        // 每个节点声明一个子节点；最后一层为叶子。
        for index in 0..=MAX_NODE_DEPTH + 1 {
            let children = if index < MAX_NODE_DEPTH + 1 { 1 } else { 0 };
            push_dummy(
                &mut bytes,
                &format!("n{index}"),
                children,
                true,
                &identity(),
            );
        }

        let err = Kn5::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::DepthLimit(MAX_NODE_DEPTH)));
    }

    #[test]
    fn env_sample_parses_when_present() {
        let Ok(path) = std::env::var("VETTURA_KN5_SAMPLE") else {
            return;
        };
        let kn5 = Kn5::from_path(path).expect("环境变量指定的样例应当解析成功");
        assert!(kn5.node_count() > 0);
        assert!(!kn5.textures.is_empty());
    }
}
