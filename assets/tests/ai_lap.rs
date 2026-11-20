//! Daytona roadcourse full-lap AI regression.
//!
//! Ignored by default: it loads the real track and F1 car, then runs a whole
//! lap of Jolt vehicle physics at 600 Hz with the AI driver called at 60 Hz —
//! the same cadence the Godot probe uses. Prints one line per simulated
//! second so a regression can be read like a telemetry trace.
//!
//! ```bash
//! cargo test -p vettura-assets --test ai_lap -- --ignored --nocapture
//! ```
//!
//! Works on **any** AC track mod: point `AI_LAP_TRACK` at another directory
//! under `mods/` and `AI_LAYOUT` (or `AI_LAP_LAYOUT`) at its layout. The AI
//! takes every number from the track line and the car spec, so a circuit it
//! has never seen is driven by the same rule.
//!
//! Environment:
//! - `AI_LAP_TRACK`: track directory under `mods/`, default `daytona_2017`.
//! - `AI_LAP_LAYOUT`: layout suffix of `models_*.ini`, default `roadcourse`.
//! - `AI_LAP_CAR`: car directory under `mods/`.
//! - `AI_LAP_SKILL`: AC AI level `0..=110`, default `100`.
//! - `AI_LAP_MAX_S`: simulation budget, default 200 s.
//! - `AI_LAP_MAX_TIME_S`: asserted lap-time ceiling, default 175 s.
//! - `AI_LAP_MAX_LATERAL_M`: asserted line deviation ceiling, default 12 m.

use std::path::PathBuf;

use vettura_assets::{CarFolder, TrackModule};
use vettura_world::{AiDriver, AiDriverTuning, DriveInput};

const STEP_S: f32 = 1.0 / 600.0;
/// Godot calls `WorldBody.ai_controls` once per physics frame.
const AI_PERIOD_S: f32 = 1.0 / 60.0;

fn env_text(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// Track directory under test, relative to `mods/`. Any AC track mod whose
/// directory has a `fast_lane.ai` works, so this regression is not tuned for
/// one circuit: point `AI_LAP_TRACK` at another mod to check it there.
fn track_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mods")
        .join(env_text("AI_LAP_TRACK", "daytona_2017"))
}

/// Car directory under test, relative to `mods/`.
fn car_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mods")
        .join(env_text(
            "AI_LAP_CAR",
            "F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari",
        ))
}

fn env_f32(name: &str, fallback: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|text| text.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

#[test]
#[ignore = "loads a real track and car and drives a full lap"]
fn ai_lap_stays_on_track() {
    let track_root = track_root();
    let car_root = car_root();
    let layout = env_text("AI_LAP_LAYOUT", "roadcourse");
    let models = track_root.join(format!("models_{layout}.ini"));
    if !models.is_file() || !car_root.is_dir() {
        eprintln!(
            "skipping: assets missing at {} or {}",
            models.display(),
            car_root.display()
        );
        return;
    }

    let budget_s = env_f32("AI_LAP_MAX_S", 200.0);
    let max_lap_s = env_f32("AI_LAP_MAX_TIME_S", 175.0);
    let max_lateral_limit_m = env_f32("AI_LAP_MAX_LATERAL_M", 12.0);
    // 难度允许 0，所以不走 `env_f32` 的「>0」过滤。
    let skill = std::env::var("AI_LAP_SKILL")
        .ok()
        .and_then(|text| text.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && (0.0..=110.0).contains(value))
        .unwrap_or(100.0);

    let track = TrackModule::load_layout(&track_root, &layout).expect("load track");
    let spec = CarFolder::open(&car_root)
        .expect("open car folder")
        .vehicle_spec()
        .expect("map car data");
    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    let spawn = track
        .spawn_poses(1, 8.0)
        .expect("select grid pose")
        .first()
        .copied()
        .unwrap_or_else(|| line.start_pose().expect("line start pose"));

    let mut world = track
        .create_world_without_ground()
        .expect("create Jolt world");
    let car = world
        .add_vehicle(spec.clone(), spawn)
        .expect("spawn car on the grid");
    // 比赛温度：AI 代表已经暖过胎的职业车手。不暖胎时实测圈速慢 20%+（冷机
    // ECU 限扭到 55% 功率、冷胎掉一档抓地，Jolt 的变速箱还会拒绝升挡）。
    world
        .warm_up_vehicle(car)
        .expect("warm the car up before the lap");
    let tuning = AiDriverTuning {
        skill,
        ..AiDriverTuning::default()
    };
    let mut driver = AiDriver::with_tuning(line.clone(), tuning);
    let mut timer = track
        .lap_timer()
        .expect("create lap timer")
        .expect("track ships sections.ini");

    println!(
        "[ai-lap-test] track={} layout={} skill={:.0} spec={} wheelbase={:.2} mass={:.0} lock={:.1} mu={:.2} ClA={:.2}",
        env_text("AI_LAP_TRACK", "daytona_2017"),
        layout,
        skill,
        spec.name,
        spec.wheelbase_m,
        spec.mass_kg,
        spec.max_steer_angle_deg,
        spec.tyres.peak_friction,
        spec.aero.downforce_coeff,
    );
    println!(
        "[ai-lap-test] lap={:.1} m, budget={:.0} s, target lap<{:.0} s",
        line.lap_length_m(),
        budget_s,
        max_lap_s,
    );

    let mut input = DriveInput::default();
    let mut elapsed_s = 0.0f32;
    let mut next_ai_s = 0.0f32;
    let mut max_lateral_m = 0.0f32;
    let mut max_speed_mps = 0.0f32;
    let mut max_airborne_s = 0.0f32;
    let mut airborne_s = 0.0f32;
    let mut max_lateral_g = 0.0f32;
    let mut max_lateral_g_speed = 0.0f32;
    let mut impact_count = 0usize;
    let mut max_impact_mps = 0.0f32;
    let mut last_print_s = -1.0f32;

    while elapsed_s < budget_s && timer.laps() == 0 {
        if elapsed_s + 1.0e-6 >= next_ai_s {
            input = {
                let view = world.vehicle(car).expect("vehicle view");
                driver.update_view(view)
            };
            // The Godot probe drives with ABS/TC on; keep the comparison fair.
            input.abs = true;
            input.tc = true;
            next_ai_s += AI_PERIOD_S;
        }
        world.set_input(car, input).expect("set AI input");
        world.step_exact(STEP_S).expect("step world");
        elapsed_s += STEP_S;

        for impact in world.drain_impacts() {
            if impact.a.vehicle().is_none() && impact.b.vehicle().is_none() {
                continue;
            }
            impact_count += 1;
            max_impact_mps = max_impact_mps.max(impact.impact_speed_mps);
            if impact.impact_speed_mps > 1.0 {
                println!(
                    "[ai-lap-test] t={:6.2} IMPACT {:.1} m/s severity={:.2} a={:?} b={:?} pos={:?}",
                    elapsed_s,
                    impact.impact_speed_mps,
                    impact.severity,
                    impact.a,
                    impact.b,
                    impact.position,
                );
            }
        }

        let snapshot = world.snapshot(car).expect("vehicle snapshot");
        let position = snapshot.reference_position;
        let contacts = snapshot.wheels.iter().filter(|wheel| wheel.contact).count();
        if contacts <= 1 {
            airborne_s += STEP_S;
            max_airborne_s = max_airborne_s.max(airborne_s);
        } else {
            airborne_s = 0.0;
        }
        max_speed_mps = max_speed_mps.max(snapshot.forward_speed_mps.max(0.0));
        let lateral_g = snapshot.angular_velocity.y.abs() * snapshot.forward_speed_mps.abs() / 9.81;
        if lateral_g > max_lateral_g {
            max_lateral_g = lateral_g;
            max_lateral_g_speed = snapshot.forward_speed_mps;
        }
        let projection = driver.last_projection();
        if let Some(projection) = projection {
            max_lateral_m = max_lateral_m.max(projection.lateral_m.abs());
        }
        timer.update(position, elapsed_s);

        if elapsed_s - last_print_s >= 1.0 {
            last_print_s = elapsed_s;
            let projection = projection.expect("first projection");
            let line_speed = line
                .sample_at_distance(projection.along_m)
                .map(|sample| sample.speed_mps.unwrap_or(f32::NAN))
                .unwrap_or(f32::NAN);
            println!(
                "[ai-lap-test] t={:6.1} lapT={:6.2} along={:7.1} lat={:6.2} linev={:5.1} \
                 y={:5.2}/{:5.2} spd={:6.1} thr={:.2} brk={:.2} steer={:+.2} latg={:4.2} ct={} lap={} tyreT={:.0} engMul={:.2} coolant={:.1}",
                elapsed_s,
                timer.lap_time_s(),
                projection.along_m,
                projection.lateral_m,
                line_speed,
                snapshot.reference_position.y,
                projection.position.y,
                snapshot.forward_speed_mps,
                input.throttle,
                input.brake,
                input.steering,
                lateral_g,
                contacts,
                timer.laps(),
                snapshot.tyre_temperatures_c.iter().sum::<f32>() / 4.0,
                snapshot.engine_power_multiplier,
                world.engine_temperatures_c(car).unwrap().0,
            );
        }
    }

    let lap_s = timer.last_lap_s().unwrap_or(f32::NAN);
    let best_s = timer.best_lap_s().unwrap_or(f32::NAN);
    let snapshot = world.snapshot(car).expect("final snapshot");
    println!(
        "[ai-lap-test] done t={:.1} laps={} lap={:.2} best={:.2} max_lat={:.2} m \
         max_speed={:.1} m/s max_lat_g={:.2} @ {:.1} m/s max_airborne={:.2} s impacts={} max_impact={:.1} m/s pos={:?}",
        elapsed_s,
        timer.laps(),
        lap_s,
        best_s,
        max_lateral_m,
        max_speed_mps,
        max_lateral_g,
        max_lateral_g_speed,
        max_airborne_s,
        impact_count,
        max_impact_mps,
        snapshot.reference_position,
    );

    assert_eq!(
        timer.laps(),
        1,
        "AI did not complete a lap in {budget_s} s (max lateral {max_lateral_m:.1} m)"
    );
    assert!(
        lap_s.is_finite() && lap_s <= max_lap_s,
        "lap time {lap_s:.2} s exceeds {max_lap_s:.0} s"
    );
    assert!(
        max_lateral_m <= max_lateral_limit_m,
        "AI strayed {max_lateral_m:.2} m off the racing line (limit {max_lateral_limit_m:.0} m)"
    );
    assert!(
        max_airborne_s < 2.0,
        "AI was airborne for {max_airborne_s:.2} s straight: it left the track"
    );
}
