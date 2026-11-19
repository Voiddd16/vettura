//! 解析行为单元测试的共享辅助。
//!
//! 这里按 `docs/kn5-format.md` 的布局手工拼出最小合法字节流（`push_*` 系列），
//! 用例本身放在 [`parse`] 子模块。

use super::*;

mod parse;

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
fn push_node_header(bytes: &mut Vec<u8>, node_type: i32, name: &str, children: i32, active: bool) {
    bytes.extend_from_slice(&node_type.to_le_bytes());
    push_string(bytes, name);
    bytes.extend_from_slice(&children.to_le_bytes());
    bytes.push(u8::from(active));
}

/// 写一个完整的 dummy 节点（含变换）。
fn push_dummy(bytes: &mut Vec<u8>, name: &str, children: i32, active: bool, transform: &Transform) {
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
