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

use vettura_kn5::{Kn5, SharedBytes, synth::SynthOptions, synth::synthetic_kn5};

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
            let bytes = synthetic_kn5(SynthOptions {
                meshes,
                vertices_per_mesh: vertices,
                textures,
                texture_bytes: texture_kb * 1024,
                ..SynthOptions::default()
            });
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
