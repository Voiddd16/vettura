//! 起步/静载诊断：落地高度、静载、自由落体、全油门起步。
//!
//! 这些数字是车辆物理的地基：静载必须等于车重（轮胎回调看到的是总载荷，
//! 不只是 Jolt 线性弹簧的那一份），自由落体必须接近 9.81 m/s²。跑法：
//!
//! ```bash
//! cargo test -p vettura-world --test launch_probe -- --ignored --nocapture
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

/// 落地后的四轮载荷总和必须等于车重，且与初始高度无关。
#[test]
#[ignore]
fn static_load() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    println!(
        "cg={:.3} radii={:?} travel={:?} springs={:?}",
        spec.cg_height_m,
        spec.wheels.iter().map(|w| w.radius_m).collect::<Vec<_>>(),
        spec.wheels
            .iter()
            .map(|w| w.suspension.travel_m)
            .collect::<Vec<_>>(),
        spec.wheels
            .iter()
            .map(|w| w.suspension.spring_rate_n_per_m)
            .collect::<Vec<_>>(),
    );
    for start_y in [0.05f32, 0.2, 0.4, 0.8] {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(
                spec.clone(),
                vettura_physics::Pose {
                    position: Vec3::Y * start_y,
                    ..Default::default()
                },
            )
            .unwrap();
        for _ in 0..(3.0 / DT) as usize {
            world.step_exact(DT).unwrap();
        }
        let snap = world.snapshot(car).unwrap();
        let total: f32 = snap.wheels.iter().map(|wheel| wheel.load_n).sum();
        let loads: Vec<String> = snap
            .wheels
            .iter()
            .map(|wheel| {
                format!(
                    "{:.0}N/{:.3}m{}",
                    wheel.load_n,
                    wheel.suspension_length_m,
                    if wheel.hit_hard_point { "!" } else { "" }
                )
            })
            .collect();
        println!(
            "start_y={start_y:.2} y={:.3} total={total:.0}N (weight {:.0}N) | {}",
            snap.position.y,
            spec.mass_kg * 9.81,
            loads.join(" ")
        );
    }
}

#[test]
#[ignore]
fn free_fall_gravity() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(
            spec,
            vettura_physics::Pose {
                position: Vec3::Y * 5.0,
                ..Default::default()
            },
        )
        .unwrap();
    let start = world.snapshot(car).unwrap().position;
    for _ in 0..(0.5 / DT) as usize {
        world.step_exact(DT).unwrap();
    }
    let end = world.snapshot(car).unwrap().position;
    let drop_m = start.y - end.y;
    println!(
        "free fall 0.5 s: drop={drop_m:.3} m → g={:.2} m/s²",
        2.0 * drop_m / 0.25
    );
}

#[test]
#[ignore]
fn launch_from_rest() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    let tc = std::env::var("PROBE_TC").map_or(true, |value| value != "off");
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(
            spec,
            vettura_physics::Pose {
                position: Vec3::Y * 0.05,
                ..Default::default()
            },
        )
        .unwrap();
    world.warm_up_vehicle(car).expect("warm up");
    println!("launch: tc={tc}");
    for step in 0..(8.0 / DT) as usize {
        world
            .set_controls(
                car,
                vettura_physics::Controls {
                    throttle: 1.0,
                    tc,
                    abs: true,
                    ..Default::default()
                },
            )
            .unwrap();
        world.step_exact(DT).unwrap();
        if step % (0.5 / DT) as usize != 0 {
            continue;
        }
        let snap = world.snapshot(car).unwrap();
        println!(
            "t={:4.2} v={:6.2} gear={} rpm={:5.0} slips={:?}",
            step as f32 * DT,
            snap.forward_speed_mps,
            snap.gear,
            snap.engine_rpm,
            snap.wheels
                .iter()
                .map(|wheel| (wheel.slip_ratio * 100.0).round() / 100.0)
                .collect::<Vec<_>>(),
        );
    }
}
