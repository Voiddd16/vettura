//! 解析行为的用例：成功路径、容错策略与各类损坏文件的报错。

use std::sync::Arc;

use super::*;
use crate::{
    BlendMode, DepthMode, Error, Kn5, MAGIC, MAX_NODE_DEPTH, MeshFlags, SharedBytes, SkinnedVertex,
};

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
