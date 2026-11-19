//! 转向能力诊断：真实车辆在平地上恒定方向盘输入下的稳态转弯半径。
//!
//! AI 速度规划的横向极限（[`vettura_world::ai`] 里的
//! `PLAN_LATERAL_GRIP_SHARE` / `PLAN_LATERAL_BASE_MAX_MPS2`）就是从这里测得
//! 的：每个「目标速度 × 方向盘比例」组合跑 8 秒，输出稳态半径、侧向 g、四轮
//! 载荷与侧向力。调 AI 弯速前先跑这个确认抓地包络。
//!
//! ```bash
//! cargo test -p vettura-world --test steer_probe -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use glam::Vec3;
use vettura_assets::CarFolder;
use vettura_physics::{Controls, Pose, VehicleSpec};
use vettura_world::{SurfaceMap, World};

fn real_car_spec() -> Option<VehicleSpec> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari");
    if !root.is_dir() {
        return None;
    }
    Some(
        CarFolder::open(&root)
            .expect("real car folder should open")
            .vehicle_spec()
            .expect("real car data should map"),
    )
}

#[test]
#[ignore]
fn constant_steer_circle() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    println!(
        "spec: wheelbase={:.2} mass={:.0} lock={:.1} peak_friction={:.2}",
        spec.wheelbase_m, spec.mass_kg, spec.max_steer_angle_deg, spec.tyres.peak_friction
    );
    const DT: f32 = 1.0 / 600.0;
    for target_speed in [20.0f32, 30.0, 40.0, 50.0, 60.0, 70.0] {
        for steer in [0.2f32, 0.4, 0.6, 0.8] {
            let mut world = World::new(SurfaceMap::new()).unwrap();
            let car = world
                .add_vehicle(
                    spec.clone(),
                    Pose {
                        position: Vec3::Y * 0.05,
                        ..Pose::default()
                    },
                )
                .unwrap();
            for _ in 0..(0.5 / DT) as usize {
                world.step_exact(DT).unwrap();
            }
            {
                let snap = world.snapshot(car).unwrap();
                let loads: Vec<f32> = snap.wheels.iter().map(|w| w.load_n).collect();
                let latf: Vec<f32> = snap.wheels.iter().map(|w| w.lateral_force_n).collect();
                println!(
                    "static loads={loads:?} lateral_forces={latf:?} mass={} weight={:.0}",
                    spec.mass_kg,
                    spec.mass_kg * 9.81
                );
            }
            let mut lat_accel_max = 0.0f32;
            let mut speed = 0.0f32;
            let mut steady_g = 0.0f32;
            let mut elapsed = 0.0f32;
            let mut steady_accel_sum = 0.0f32;
            let mut steady_samples = 0u32;
            for _ in 0..(8.0 / DT) as usize {
                let elapsed_last_3s = elapsed >= 5.0;
                let snap = world.snapshot(car).unwrap();
                speed = snap.forward_speed_mps;
                let err = target_speed - speed;
                let throttle = (err * 0.5 + 0.3).clamp(0.0, 1.0);
                let brake = if err < -1.0 {
                    (-err * 0.2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                world
                    .set_controls(
                        car,
                        Controls {
                            throttle,
                            brake,
                            steering: steer,
                            abs: true,
                            tc: true,
                            ..Controls::default()
                        },
                    )
                    .unwrap();
                world.step_exact(DT).unwrap();
                let snap = world.snapshot(car).unwrap();
                let lateral_accel = snap.angular_velocity.y.abs() * snap.forward_speed_mps.abs();
                // 最后 3 秒的均值就是稳态横向加速度，比全程最大值更接近规划器要的数。
                if elapsed_last_3s {
                    steady_accel_sum += lateral_accel;
                    steady_samples += 1;
                }
                if snap.forward_speed_mps > 1.0 {
                    lat_accel_max = lat_accel_max.max(lateral_accel);
                }
            }
            let snap = world.snapshot(car).unwrap();
            let wheel_angle_deg = snap.wheels[0].steer_angle.to_degrees();
            let yaw_rate = snap.angular_velocity.y;
            let radius = if yaw_rate.abs() > 1.0e-4 {
                snap.forward_speed_mps / yaw_rate.abs()
            } else {
                f32::INFINITY
            };
            let load: f32 = snap.wheels.iter().map(|w| w.load_n).sum();
            let lat_force: f32 = snap.wheels.iter().map(|w| w.lateral_force_n).sum();
            println!(
                "target={:4.0} steer={:.2}  wheel={:6.1}deg speed={:6.1} yaw_rate={:7.3} radius={:7.1} lat_g={:5.2} steady_g={:5.2} slipF={:.2}/{:.2} slipR={:.2}/{:.2} load={:6.0} latF={:7.0} mu={:5.2}\n  wheels load={:?}\n  wheels lat={:?}",
                target_speed,
                steer,
                wheel_angle_deg,
                snap.forward_speed_mps,
                yaw_rate,
                radius,
                lat_accel_max / 9.81,
                steady_g,
                snap.wheels[0].slip_angle,
                snap.wheels[1].slip_angle,
                snap.wheels[2].slip_angle,
                snap.wheels[3].slip_angle,
                load,
                lat_force,
                lat_force / load.max(1.0),
                snap.wheels.iter().map(|w| w.load_n).collect::<Vec<_>>(),
                snap.wheels
                    .iter()
                    .map(|w| w.lateral_force_n)
                    .collect::<Vec<_>>(),
            );
        }
    }
}
