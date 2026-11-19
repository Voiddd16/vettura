//! Daytona `fast_lane.ai` 剖面诊断：打印每个 4 m 区间的曲率、半径、线速和
//! 线本身隐含的横向加速度，用来核对 AI 速度规划器看到的曲率与线速。
//!
//! `FROM_M` / `TO_M` 限定区间（米），省略则整圈。
//!
//! ```bash
//! FROM_M=900 TO_M=1000 cargo test -p vettura-assets --test line_profile -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use vettura_assets::TrackModule;

fn env_f32(name: &str, fallback: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|text| text.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(fallback)
}

#[test]
#[ignore]
fn print_fast_lane_profile() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mods/daytona_2017");
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: assets missing");
        return;
    }
    let layout = std::env::var("AI_LAP_LAYOUT").unwrap_or_else(|_| "roadcourse".to_owned());
    let track = TrackModule::load_layout(&root, &layout).expect("load track");
    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    println!(
        "lap={:.1} m points={} extras={}",
        line.lap_length_m(),
        line.len(),
        line.extras().len()
    );

    let from = env_f32("FROM_M", 0.0);
    let to = env_f32("TO_M", f32::INFINITY);
    let step = 4.0f32;
    let mut distance = 0.0f32;
    let mut prev: Option<vettura_world::AiSample> = None;
    let mut min_radius = f32::INFINITY;
    let mut min_at = 0.0f32;
    while distance <= line.lap_length_m() {
        let Some(sample) = line.sample_at_distance(distance) else {
            break;
        };
        if let Some(prev) = prev {
            let curvature = (sample.tangent - prev.tangent).length() / step;
            if curvature > 1.0e-4 {
                let radius = 1.0 / curvature;
                if radius < min_radius {
                    min_radius = radius;
                    min_at = distance;
                }
                if distance >= from && distance <= to {
                    let speed = sample.speed_mps.unwrap_or(f32::NAN);
                    let authored = line
                        .extra(sample.index)
                        .map(|e| e.radius_m)
                        .unwrap_or(f32::NAN);
                    let line_lat = speed * speed / radius;
                    println!(
                        "d={:7.1} R={:7.1} authoredR={:8.1} lineV={:6.1} lineA={:6.2} lat={:?}",
                        distance, radius, authored, speed, line_lat, sample.position
                    );
                }
            }
        }
        prev = Some(sample);
        distance += step;
    }
    println!("min radius {:.1} m at d={:.1}", min_radius, min_at);
}
