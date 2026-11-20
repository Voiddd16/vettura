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

/// 物理步长与 `World` 固定步一致；每个采样点之间必须真的走完这段时间，
/// 否则测到的是「一步之后的速度差」，既不是包络也不是稳态。
const DT: f32 = 1.0 / 600.0;

fn aids_enabled() -> bool {
    std::env::var("PROBE_AIDS").map_or(true, |value| value != "off")
}

/// 从静止开始全油门，每 1 s 记一行：速度、这一秒的平均加速度、挡位。
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
    let aids = aids_enabled();
    // 起步瞬间 TC 会压制油门，先跑 2 s 让车动起来，再开始记包络。
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
    println!("full throttle: from rest, 20 s (aids={aids})");
    let mut previous = world.snapshot(car).unwrap().forward_speed_mps;
    for second in 0..20 {
        for _ in 0..(1.0 / DT) as usize {
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
        let snapshot = world.snapshot(car).unwrap();
        let speed = snapshot.forward_speed_mps;
        println!(
            "t={:4.1} speed={:6.1} m/s ({:6.1} km/h) accel={:5.2} m/s^2 gear={} rpm={:6.0} coolant={:5.1}",
            3.0 + second as f32,
            speed,
            speed * 3.6,
            speed - previous,
            snapshot.gear,
            snapshot.engine_rpm,
            world.engine_temperatures_c(car).unwrap().0,
        );
        previous = speed;
    }
}

/// 直线全刹车（ABS 开）从各初速刹到 5 m/s，记录减速度包络。
#[test]
#[ignore]
fn straight_line_braking() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 不存在");
        return;
    };
    let aids = aids_enabled();
    for start in [25.0f32, 50.0, 75.0] {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let handle = world
            .add_vehicle(spec.clone(), vettura_physics::Pose::default())
            .unwrap();
        world.warm_up_vehicle(handle).expect("warm up");
        // 全油门推到目标速度，同样用固定步长，避免一次 50 ms 的巨步。
        let mut speed = 0.0f32;
        for _ in 0..(60.0 / DT) as usize {
            world
                .set_controls(
                    handle,
                    vettura_physics::Controls {
                        throttle: 1.0,
                        tc: aids,
                        abs: aids,
                        ..Default::default()
                    },
                )
                .unwrap();
            world.step_exact(DT).unwrap();
            speed = world.snapshot(handle).unwrap().forward_speed_mps;
            if speed >= start {
                break;
            }
        }
        let reached = speed;
        let mut samples: Vec<(f32, f32)> = Vec::new();
        let mut elapsed = 0.0f32;
        while elapsed < 10.0 {
            world
                .set_controls(
                    handle,
                    vettura_physics::Controls {
                        throttle: 0.0,
                        brake: 1.0,
                        tc: aids,
                        abs: aids,
                        ..Default::default()
                    },
                )
                .unwrap();
            world.step_exact(DT).unwrap();
            elapsed += DT;
            samples.push((elapsed, world.snapshot(handle).unwrap().forward_speed_mps));
            if samples.last().unwrap().1 < 5.0 {
                break;
            }
        }
        println!(
            "braking from {reached:.1} m/s (aids={aids}), {:.2} s to 5 m/s",
            elapsed
        );
        // 每 0.1 s 打一行：窗口平均减速度（窗口两端的速度差 / 时间差）。
        let window = 0.1f32;
        let mut anchor = 0usize;
        let mut report_at = window;
        while anchor + 1 < samples.len() {
            let Some(next) = (anchor..samples.len()).find(|&i| samples[i].0 >= report_at) else {
                break;
            };
            let (t0, v0) = samples[anchor];
            let (t1, v1) = samples[next];
            let decel = (v0 - v1) / (t1 - t0).max(1.0e-6);
            println!(
                "  t={:5.2} speed={:6.1} decel={:5.2} m/s^2 ({:.2} g)",
                t1,
                v1,
                decel,
                decel / 9.81
            );
            anchor = next;
            report_at += window;
        }
    }
}
