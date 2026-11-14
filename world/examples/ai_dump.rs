//! 赛道 AI 线诊断：把 `fast_lane.ai` 的目标速度 / 宽度沿里程打印出来。
//!
//! 用法：
//!
//! ```bash
//! cargo run -p vettura-world --example ai_dump -- \
//!     mods/daytona_2017/roadcourse/ai/fast_lane.ai --step 50
//! ```
//!
//! 输出列：里程(m) / 目标速度(m/s, km/h) / 路宽左右(m)。用于排查 AI
//! 刹车点与速度规划，不依赖 Godot。

use vettura_world::AiSpline;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: ai_dump <fast_lane.ai> [--step 米]");
        std::process::exit(2);
    };
    let mut step_m = 50.0f32;
    while let Some(arg) = args.next() {
        if arg == "--step" {
            step_m = args.next().and_then(|v| v.parse().ok()).unwrap_or(50.0);
        }
    }

    let line = match AiSpline::from_path(&path) {
        Ok(line) => line,
        Err(err) => {
            eprintln!("解析失败: {err}");
            std::process::exit(1);
        }
    };

    let length = line.lap_length_m();
    println!(
        "path={} points={} lap_length={:.1} m",
        path,
        line.points().len(),
        length
    );
    println!(
        "{:>9} {:>9} {:>9} {:>9} {:>9}",
        "d_m", "v_mps", "v_kmh", "left_m", "right_m"
    );

    let mut distance = 0.0f32;
    while distance < length {
        match line.sample_at_distance(distance) {
            Some(sample) => {
                let speed = sample.speed_mps.unwrap_or(f32::NAN);
                // 宽度存在 extras 里，这里用最近点近似：sample 没带宽度，
                // 打印速度即可；宽度列留空占位。
                println!(
                    "{:>9.1} {:>9.2} {:>9.1} {:>9} {:>9}",
                    distance,
                    speed,
                    speed * 3.6,
                    "-",
                    "-"
                );
            }
            None => println!("{distance:.1}: no sample"),
        }
        distance += step_m;
    }
}
