//! 纵向能力诊断：真实车辆的直线加速 / 刹车减速度 vs 速度。
//!
//! AI 速度规划里的 `planning_decel_mps2` 与油门模型都必须是这两个包络的
//! 保守估计，否则要么刹不停（撞墙）要么白丢直线速度。跑这个探针就是拿实测
//! 包络去校准规划器，而不是拍脑袋给常数。
//!
//! ```bash
//! cargo test -p vettura-world --test longitudinal_probe -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use glam::Vec3;

use vettura_assets::CarFolder;
use vettura_world::{SurfaceMap, World};

fn real_car_spec() -> Option<vettura_physics::VehicleSpec> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari");
    if !root.is_dir() {
        return None;
    }
    CarFolder::open(&root).ok()?.vehicle_spec().ok()
}

const DT: f32 = 1.0 / 600.0;

/// 从静止开始全油门，每 0.5 s 记一行：速度、加速度、挡位。
#[test]
#[ignore]
fn full_throttle_acceleration() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(
            spec.clone(),
            vettura_physics::Pose {
                position: Vec3::Y * 0.05,
                ..Default::default()
            },
        )
        .unwrap();
    world
        .warm_up_vehicle(car)
        .expect("warm up the car before measuring");
    let aids = std::env::var("PROBE_AIDS").map_or(true, |v| v != "off");
    for _ in 0..(2.0 / DT) as usize {
        world
            .set_controls(
                car,
                vettura_physics::Controls {
                    throttle: 1.0,
                    tc: aids,
                    abs: aids,
                    ..Default::default()
                },
            )
            .unwrap();
        world.step_exact(DT).unwrap();
    }
    println!("full throttle: from rest, 10 s");
    let mut previous = 0.0f32;
    for step in 0..20 {
        world
            .set_controls(
                car,
                vettura_physics::Controls {
                    throttle: 1.0,
                    tc: aids,
                    abs: aids,
                    ..Default::default()
                },
            )
            .unwrap();
        world.step_exact(DT).unwrap();
        let snapshot = world.snapshot(car).unwrap();
        let speed = snapshot.forward_speed_mps;
        if step % 2 == 1 {
            let accel = (speed - previous) / 1.0;
            println!(
                "t={:5.1} speed={:6.1} m/s ({:6.1} km/h) accel={:5.2} m/s^2 gear={} rpm={:6.0} coolant={:5.1}",
                2.0 + 0.5 * step as f32,
                speed,
                speed * 3.6,
                accel,
                snapshot.gear,
                snapshot.engine_rpm,
                world.engine_temperatures_c(car).unwrap().0,
            );
            previous = speed;
        }
    }
}

/// 直线全刹车（ABS 开）从各初速刹到 5 m/s，记录减速。
#[test]
#[ignore]
fn straight_line_braking() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 不存在");
        return;
    };
    for start in [30.0f32, 50.0, 70.0] {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        // 先把车推到目标速度，再开始测刹车。
        let handle = world
            .add_vehicle(spec.clone(), vettura_physics::Pose::default())
            .unwrap();
        let mut speed = 0.0f32;
        let mut elapsed = 0.0f32;
        while elapsed < 40.0 {
            let controls = vettura_physics::Controls {
                throttle: if speed < start { 1.0 } else { 0.0 },
                brake: if speed > start + 1.0 { 1.0 } else { 0.0 },
                tc: true,
                abs: true,
                ..Default::default()
            };
            world.set_controls(handle, controls).unwrap();
            world.step_exact(0.05).unwrap();
            speed = world.snapshot(handle).unwrap().forward_speed_mps;
            elapsed += 0.05;
            if speed >= start {
                break;
            }
        }
        world.step_exact(1.0 / 60.0).unwrap();
        let mut samples = Vec::new();
        let mut elapsed = 0.0;
        while elapsed < 8.0 {
            world
                .set_controls(
                    handle,
                    vettura_physics::Controls {
                        throttle: 0.0,
                        brake: 1.0,
                        tc: true,
                        abs: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            world.step_exact(DT).unwrap();
            samples.push((elapsed, world.snapshot(handle).unwrap().forward_speed_mps));
            elapsed += DT;
            if samples.last().unwrap().1 < 5.0 {
                break;
            }
        }
        println!(
            "braking from {start:.0} m/s (ABS on), {} samples",
            samples.len()
        );
        // 每 0.05 s 打一行：窗口瞬时减速度就是 (t0-t1) 之间的平均。
        let mut printed = -1.0;
        for pair in samples.windows(2) {
            let (t0, v0) = pair[0];
            let (t1, v1) = pair[1];
            if t0 - printed < 0.049 {
                continue;
            }
            printed = t0;
            let decel = (v0 - v1) / (t1 - t0).max(1.0e-6);
            println!(
                "  t={:5.2} speed={:6.1} decel={:5.2} m/s^2 ({:.2} g)",
                t0,
                v0,
                decel,
                decel / 9.81
            );
        }
    }
}
