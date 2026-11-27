//! `AiSpline` 查询基准（不依赖 criterion）。
//!
//! `AiDriver` 每帧把车辆位置投影到 AI 线上。真实赛道（Nürburgring）的
//! `fast_lane.ai` 有一万多个点；全量扫描的墙钟开销决定了多车 AI 比赛每帧的
//! CPU 预算。这里用真实线（`VETTURA_AI_LINE`）或等规模合成圆环测量：
//!
//! - `project`：全量最近点扫描（老路径）；
//! - `project_near`：带上一帧索引提示的窗口扫描（新路径，窗口外的结果会
//!   自动退回全量扫描）。
//!
//! ```bash
//! VETTURA_AI_LINE=mods/fn_nurburgring/nordschleife/ai/fast_lane.ai \
//!   cargo bench -p vettura-track --bench spline
//! ```

use std::path::PathBuf;
use std::time::Instant;

use glam::Vec3;
use vettura_track::AiSpline;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// 合成 AI 线：一个半径 `radius` 的圆，每个 `spacing` 米一个点。
fn synthetic_circle(radius: f32, spacing: f32) -> AiSpline {
    let lap = std::f32::consts::TAU * radius;
    let count = (lap / spacing).round().max(8.0) as usize;
    // 用真实格式的字节流构造，避免给测试开后门：先算点，再按 v7 布局打包。
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&7i32.to_le_bytes());
    bytes.extend_from_slice(&(count as i32).to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&(count as i32).to_le_bytes());
    let mut length = 0.0f32;
    let mut previous = Vec3::new(radius, 0.0, 0.0);
    for index in 0..count {
        let angle = index as f32 / count as f32 * std::f32::consts::TAU;
        // AC 帧 → Godot 帧：x 不变，z 取反。
        let position = Vec3::new(angle.cos() * radius, 0.0, -angle.sin() * radius);
        if index > 0 {
            length += position.distance(previous);
        }
        previous = position;
        // 位置（AC 帧：解析器会做 (-x, y, -z) 的 Godot 转换，这里先取反）。
        bytes.extend_from_slice(&(-position.x).to_le_bytes());
        bytes.extend_from_slice(&position.y.to_le_bytes());
        bytes.extend_from_slice(&(-position.z).to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&(index as i32).to_le_bytes());
    }
    // 每点一个 extra 块（18 个 f32 = 72 B）：速度 30 m/s，其余字段为 0。
    bytes.extend_from_slice(&(count as i32).to_le_bytes());
    for _ in 0..count {
        bytes.extend_from_slice(&30.0f32.to_le_bytes());
        for _ in 0..17 {
            bytes.extend_from_slice(&0.0f32.to_le_bytes());
        }
    }
    AiSpline::parse(&bytes).expect("synthetic AI spline")
}

fn load_line() -> AiSpline {
    if let Ok(path) = std::env::var("VETTURA_AI_LINE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return AiSpline::from_path(&path).expect("parse AI line");
        }
    }
    synthetic_circle(400.0, 1.5)
}

fn main() {
    let line = load_line();
    let samples = env_usize("VETTURA_SPLINE_SAMPLES", 2_000);
    let hint_window = env_usize("VETTURA_SPLINE_WINDOW", 64);
    println!(
        "spline: points={} lap={:.1}m samples={samples} window={hint_window}",
        line.len(),
        line.lap_length_m()
    );

    // 沿整圈均匀取位置，模拟 60 fps 下连续多帧的查询序列。
    let lap = line.lap_length_m();
    let positions: Vec<Vec3> = (0..samples)
        .filter_map(|index| {
            let distance = lap * index as f32 / samples as f32;
            line.sample_at_distance(distance)
                .map(|sample| sample.position)
        })
        .collect();
    if positions.len() < samples {
        eprintln!("warning: only {} of {samples} positions", positions.len());
    }

    let iterations = env_usize("VETTURA_SPLINE_ITERS", 20);

    // 全量扫描。
    let mut checksum = 0.0f64;
    let start = Instant::now();
    for _ in 0..iterations {
        for position in &positions {
            if let Some(projection) = line.project(*position) {
                checksum += projection.distance_m as f64;
            }
        }
    }
    let full = start.elapsed();
    let full_calls = operations(iterations, positions.len());
    println!(
        "project       : {:>9.3} ms total, {:>9.0} ns/call  ({:.0} calls/s)",
        full.as_secs_f64() * 1e3,
        full.as_secs_f64() * 1e9 / full_calls as f64,
        full_calls as f64 / full.as_secs_f64(),
    );

    // 带提示：上一帧的索引，像真实 AI 循环一样逐帧推进。
    let mut hint = None;
    let start = Instant::now();
    for _ in 0..iterations {
        for position in &positions {
            match line.project_near(*position, hint, hint_window) {
                Some(projection) => {
                    hint = Some(projection.index);
                    checksum += projection.distance_m as f64;
                }
                None => hint = None,
            }
        }
    }
    let hinted = start.elapsed();
    println!(
        "project_near  : {:>9.3} ms total, {:>9.0} ns/call  ({:.1}× faster)",
        hinted.as_secs_f64() * 1e3,
        hinted.as_secs_f64() * 1e9 / full_calls as f64,
        full.as_secs_f64() / hinted.as_secs_f64().max(1e-9),
    );
    println!("checksum {checksum:.3}");
}

fn operations(iterations: usize, samples: usize) -> usize {
    iterations * samples
}
