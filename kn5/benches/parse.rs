//! KN5 解析性能测试。
//!
//! 这个 bench 刻意不依赖 criterion 等第三方框架，直接使用
//! `std::time::Instant`，保证 `cargo bench` 在干净环境中也能运行。
//!
//! 用法：
//!
//! ```bash
//! # 使用内置的合成模型
//! cargo bench -p vettura-kn5
//!
//! # 测试真实文件（也可以设置 VETTURA_KN5_SAMPLE）
//! cargo bench -p vettura-kn5 -- "/path/to/car.kn5"
//! ```
//!
//! 可通过环境变量调整参数：
//!
//! - `VETTURA_BENCH_ITERS`：正式迭代次数，默认 20
//! - `VETTURA_BENCH_WARMUP`：预热次数，默认 3
//! - `VETTURA_BENCH_MESHES`：合成模型网格数，默认 128
//! - `VETTURA_BENCH_VERTS`：每个网格顶点数，默认 1024
//! - `VETTURA_BENCH_TEXTURES`：贴图数量，默认 32
//! - `VETTURA_BENCH_TEXTURE_KB`：每张贴图大小，默认 64 KiB

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vettura_kn5::{Kn5, SharedBytes};

fn main() {
    let iterations = env_usize("VETTURA_BENCH_ITERS", 20).max(1);
    let warmup = env_usize("VETTURA_BENCH_WARMUP", 3);
    let path = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("VETTURA_KN5_SAMPLE").map(PathBuf::from));

    let (label, bytes) = match &path {
        Some(path) => {
            let bytes = std::fs::read(path).expect("无法读取指定的 KN5 文件");
            (format!("文件 {}", path.display()), bytes)
        }
        None => {
            let meshes = env_usize("VETTURA_BENCH_MESHES", 128).max(1);
            let vertices = env_usize("VETTURA_BENCH_VERTS", 1024).clamp(3, 65_535);
            let textures = env_usize("VETTURA_BENCH_TEXTURES", 32);
            let texture_kb = env_usize("VETTURA_BENCH_TEXTURE_KB", 64);
            let bytes = synthetic_model(meshes, vertices, textures, texture_kb);
            (
                format!(
                    "合成模型（{meshes} 网格 × {vertices} 顶点，{textures} 贴图 × {texture_kb} KiB）"
                ),
                bytes,
            )
        }
    };

    // 贴图零拷贝路径：所有贴图共享同一块输入缓冲区。
    let source: SharedBytes = Arc::new(bytes);
    let byte_count = source.len();

    // 先完整解析一次，取得统计信息，同时确保输入本身是合法的。
    let model = Kn5::parse_shared(Arc::clone(&source)).expect("基准输入必须是合法的 KN5");
    let triangles = model.triangle_count();
    let node_count = model.node_count();

    println!("KN5 性能测试");
    println!("输入:       {label}");
    println!(
        "大小:       {:.2} MiB",
        byte_count as f64 / (1024.0 * 1024.0)
    );
    println!(
        "结构:       {node_count} 节点，{} 网格，{triangles} 三角形，{} 贴图，{} 材质",
        model.mesh_count(),
        model.textures.len(),
        model.materials.len()
    );
    println!("预热:       {warmup} 次");
    println!("迭代:       {iterations} 次");
    println!();

    let shared_samples = measure(warmup, iterations, || {
        Kn5::parse_shared(black_box(Arc::clone(&source))).expect("解析失败")
    });
    report(
        "解析（零拷贝共享输入）",
        &shared_samples,
        byte_count,
        triangles,
    );

    // 方便对比：`parse(&[u8])` 每次都要复制一份输入。
    let copy_samples = measure(warmup, iterations, || {
        Kn5::parse(black_box(source.as_slice())).expect("解析失败")
    });
    report("解析（含输入复制）", &copy_samples, byte_count, triangles);

    // 遍历只做少量工作，所以用更多迭代来获得稳定结果。
    let walk_iterations = iterations.saturating_mul(100).max(100);
    let walk_samples = measure(warmup, walk_iterations, || {
        let mut visited = 0usize;
        model.walk(|_, _| visited += 1);
        visited
    });
    report_nodes("遍历节点树", &walk_samples, node_count);

    // 真实文件额外测一次完整路径：文件读取 + 零拷贝解析。
    if let Some(path) = &path {
        let file_samples = measure(warmup.min(1), iterations, || {
            Kn5::from_path(path).expect("从文件解析失败")
        });
        report("解析（含文件读取）", &file_samples, byte_count, triangles);
    }
}

/// 计时 `f`，返回从快到慢排序后的每次耗时。
fn measure<R>(warmup: usize, iterations: usize, mut f: impl FnMut() -> R) -> Vec<Duration> {
    for _ in 0..warmup {
        black_box(f());
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let result = f();
        let elapsed = start.elapsed();
        black_box(result);
        samples.push(elapsed);
    }

    samples.sort_unstable();
    samples
}

fn report(label: &str, samples: &[Duration], bytes: usize, triangles: usize) {
    let median = percentile(samples, 0.50);
    let seconds = median.as_secs_f64();
    println!("{label}");
    println!("  最快 / 中位 / 平均 / p95 / 最慢:");
    println!(
        "    {} / {} / {} / {} / {}",
        format_duration(samples[0]),
        format_duration(median),
        format_duration(mean(samples)),
        format_duration(percentile(samples, 0.95)),
        format_duration(samples[samples.len() - 1]),
    );
    if seconds > 0.0 {
        println!(
            "  吞吐（按中位数）: {:.1} MiB/s，{:.2} M 三角形/s",
            bytes as f64 / seconds / (1024.0 * 1024.0),
            triangles as f64 / seconds / 1_000_000.0
        );
    }
    println!();
}

fn report_nodes(label: &str, samples: &[Duration], nodes: usize) {
    let median = percentile(samples, 0.50);
    let seconds = median.as_secs_f64();
    println!("{label}");
    println!("  中位 / 平均 / p95:");
    println!(
        "    {} / {} / {}",
        format_duration(median),
        format_duration(mean(samples)),
        format_duration(percentile(samples, 0.95)),
    );
    if seconds > 0.0 {
        println!(
            "  速率（按中位数）: {:.1} M 节点/s",
            nodes as f64 / seconds / 1_000_000.0
        );
    }
    println!();
}

fn percentile(samples: &[Duration], percentile: f64) -> Duration {
    let index = ((samples.len() - 1) as f64 * percentile).round() as usize;
    samples[index.min(samples.len() - 1)]
}

fn mean(samples: &[Duration]) -> Duration {
    let total: Duration = samples.iter().sum();
    total / samples.len() as u32
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds >= 1.0 {
        format!("{seconds:.3} s")
    } else if seconds >= 0.001 {
        format!("{:.3} ms", seconds * 1_000.0)
    } else if seconds >= 0.000_001 {
        format!("{:.1} µs", seconds * 1_000_000.0)
    } else {
        format!("{:.0} ns", seconds * 1_000_000_000.0)
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    // 上限护栏：这些值直接决定合成模型要分配多少内存，
    // 一个手滑多打两个零的 VETTURA_BENCH_MESHES 不应该把机器拖死。
    let limit = match name {
        "VETTURA_BENCH_WARMUP" => 1_000,
        "VETTURA_BENCH_VERTS" => 200_000,
        "VETTURA_BENCH_TEXTURE_KB" => 10_000,
        _ => 1_000,
    };

    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|parsed: usize| parsed.min(limit))
        .unwrap_or(default)
}

/// 生成一个结构合法、规模接近真实车模的 KN5。
///
/// 每个网格拥有 `vertices` 个顶点和 `vertices` 个三角形，因此可以直接
/// 用三角形数量衡量解析吞吐。
fn synthetic_model(
    mesh_count: usize,
    vertices_per_mesh: usize,
    texture_count: usize,
    texture_kb: usize,
) -> Vec<u8> {
    let texture_bytes = texture_count
        .saturating_mul(texture_kb)
        .saturating_mul(1024);
    let geometry_bytes = mesh_count
        .saturating_mul(vertices_per_mesh)
        .saturating_mul(44 + 6);
    let mut bytes = Vec::with_capacity(texture_bytes + geometry_bytes + 4096);

    // 文件头：version 5 没有额外字段。
    bytes.extend_from_slice(b"sc6969");
    bytes.extend_from_slice(&5u32.to_le_bytes());

    // 贴图表。
    bytes.extend_from_slice(&(texture_count as u32).to_le_bytes());
    for index in 0..texture_count {
        bytes.extend_from_slice(&1i32.to_le_bytes());
        push_string(&mut bytes, &format!("tex{index}.dds"));
        bytes.extend_from_slice(&((texture_kb * 1024) as u32).to_le_bytes());
        let new_len = bytes.len() + texture_kb * 1024;
        bytes.resize(new_len, 0);
    }

    // 材质表：复用少量材质，避免材质数量影响解析主路径。
    let material_count = mesh_count.clamp(1, 16);
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
        push_string(&mut bytes, &format!("tex{index}.dds"));
        push_string(&mut bytes, "txNormal");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        push_string(&mut bytes, &format!("tex{}.dds", index + 1));
    }

    // 根节点 + 若干静态网格子节点。
    push_dummy_header(&mut bytes, "Root", mesh_count as i32);
    for mesh in 0..mesh_count {
        push_node_header(&mut bytes, 2, &format!("mesh{mesh}"), 0);
        bytes.push(1); // cast shadows
        bytes.push(1); // visible
        bytes.push(0); // transparent

        bytes.extend_from_slice(&(vertices_per_mesh as u32).to_le_bytes());
        for vertex in 0..vertices_per_mesh {
            let x = vertex as f32 * 0.01;
            push_f32(&mut bytes, x);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.0);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 0.0);
        }

        let index_count = vertices_per_mesh.saturating_mul(3);
        bytes.extend_from_slice(&(index_count as u32).to_le_bytes());
        for vertex in 0..vertices_per_mesh {
            let next = ((vertex + 1) % vertices_per_mesh) as u16;
            let after_next = ((vertex + 2) % vertices_per_mesh) as u16;
            bytes.extend_from_slice(&(vertex as u16).to_le_bytes());
            bytes.extend_from_slice(&next.to_le_bytes());
            bytes.extend_from_slice(&after_next.to_le_bytes());
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

fn push_dummy_header(bytes: &mut Vec<u8>, name: &str, children: i32) {
    push_node_header(bytes, 1, name, children);
    // 单位矩阵，行主序。
    for row in 0..4 {
        for column in 0..4 {
            push_f32(bytes, if row == column { 1.0 } else { 0.0 });
        }
    }
}
