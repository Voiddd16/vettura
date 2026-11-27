//! 真实 AI 线上的提示投影等价性测试。
//!
//! 需要 mods/ 下的 `fast_lane.ai`：
//!
//! ```bash
//! VETTURA_AI_LINE="$PWD/mods/fn_nurburgring/nordschleife/ai/fast_lane.ai" \
//!   cargo test -p vettura-track --test real_spline -- --ignored --nocapture
//! ```
//!
//! 逐帧模拟「沿整条线前进 + 横向偏移变化」的查询序列，断言
//! [`AiSpline::project_near`] 与全量 [`AiSpline::project`] 给出同一个段。
//! 这是提示窗口优化的正确性护栏：窗口只在能复现全量结果时才被采用，任何
//! 平局/自交/桥洞附近的分歧都会在这里暴露。

use std::path::PathBuf;

use glam::Vec3;
use vettura_track::AiSpline;

#[test]
#[ignore = "needs a real fast_lane.ai from mods/"]
fn hinted_projection_matches_full_scan_on_a_real_line() {
    let Ok(path) = std::env::var("VETTURA_AI_LINE") else {
        eprintln!("skipping: VETTURA_AI_LINE is not set");
        return;
    };
    let path = PathBuf::from(path);
    if !path.is_file() {
        eprintln!("skipping: {} is not available", path.display());
        return;
    }

    let line = AiSpline::from_path(&path).expect("parse AI line");
    let lap = line.lap_length_m();
    assert!(line.len() > 100, "line has only {} points", line.len());

    // 与 60 fps 的 AI 更新同量级的查询序列：整圈约 2 万次，每次前进约 1 m。
    let steps = 20_000usize;
    let mut hint = None::<usize>;
    let mut mismatches = 0usize;
    let mut max_distance = 0.0f32;
    for step in 0..steps {
        let along = lap * step as f32 / steps as f32;
        let Some(sample) = line.sample_at_distance(along) else {
            continue;
        };
        // 横向偏移按一个不共振的正弦变化，覆盖贴线/宽线两种走法。
        let offset = (step as f32 * 0.017).sin() * 6.0;
        let right = sample.tangent.cross(Vec3::Y);
        let position = sample.position + right * offset;

        let full = line.project(position).expect("full projection");
        let near = line
            .project_near(position, hint, AiSpline::DEFAULT_PROJECT_WINDOW)
            .expect("hinted projection");
        max_distance = max_distance.max(full.distance_m);

        if near.index != full.index
            || (near.along_m - full.along_m).abs() > 1.0e-2
            || (near.lateral_m - full.lateral_m).abs() > 1.0e-2
        {
            mismatches += 1;
            if mismatches <= 10 {
                eprintln!(
                    "step {step}: full idx {} along {:.3} lat {:.3} | near idx {} along {:.3} lat {:.3}",
                    full.index,
                    full.along_m,
                    full.lateral_m,
                    near.index,
                    near.along_m,
                    near.lateral_m
                );
            }
        }
        hint = Some(near.index);
    }

    eprintln!(
        "checked {steps} queries on {} points / {lap:.1} m (max |offset| {max_distance:.1} m)",
        line.len()
    );
    assert_eq!(
        mismatches, 0,
        "hinted projection diverged from the full scan"
    );
}
