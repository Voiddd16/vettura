//! 转向能力诊断：真实车辆在平地上全油门、恒定方向盘输入下的稳态转弯包络。
//!
//! AI 速度规划的横向极限（[`vettura_world::ai`] 里的
//! `PLAN_LATERAL_GRIP_SHARE` / `PLAN_LATERAL_BASE_MAX_G` /
//! `PLAN_LATERAL_DOWNFORCE_SHARE`）就是从这里测得的：每个舵角从静止全油门
//! 加速，让车在圆上自己找平衡点（推力 = 阻力 + 侧向饱和），输出稳态速度、
//! 半径、横向 g、四轮载荷与侧向力。
//!
//! 这个探针必须跑在**修好辅助弹簧载荷**之后：车身重量有一半由 heave/渐进
//! 弹簧以外力形式承担，若轮胎回调看不到这部分载荷，测出来的包络只有真实
//! 值的一半。
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
    let car = std::env::var("PROBE_CAR").unwrap_or_else(|_| {
        "F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari".into()
    });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../mods")
        .join(car);
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

const DT: f32 = 1.0 / 600.0;

/// 侧翻边界：固定目标速度，逐渐增大舵角，记录稳态侧倾角与内侧轮离地。
///
/// 准静态侧翻阈值 `a = g·track/(2·CG)`；车在阈值之前内侧轮就会离地
/// （载荷转移到 0），此后是否继续滚到侧翻取决于侧倾刚度。这里直接把
/// roll 角、接触轮数和四轮载荷打出来，判断是「正常负载转移」还是
/// 「滚到搁浅/翻车」。
#[test]
#[ignore]
fn rollover_envelope() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    let track = spec.wheels[0].position.x.abs() * 2.0;
    let threshold_g = track / (2.0 * spec.cg_height_m);
    println!(
        "spec: mass={:.0} cg={:.3} track={:.3} static_rollover_threshold={:.2} g",
        spec.mass_kg, spec.cg_height_m, track, threshold_g
    );
    for target in [15.0f32, 20.0, 25.0, 30.0, 40.0] {
        for steer in [0.2f32, 0.4, 0.6, 0.8, 1.0] {
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
            world.warm_up_vehicle(car).expect("warm up");
            // 直接把车设到目标速度（物理帧 -Z 为前），不要全油门蓄速。
            world
                .set_vehicle_velocity(car, Vec3::NEG_Z * target, Vec3::ZERO)
                .unwrap();
            for _ in 0..(1.0 / DT) as usize {
                world.step_exact(DT).unwrap();
            }
            let mut roll_sum = 0.0f32;
            let mut lat_sum = 0.0f32;
            let mut contact_min = 4usize;
            let mut samples = 0u32;
            for step in 0..(8.0 / DT) as usize {
                let snap = world.snapshot(car).unwrap();
                let speed = snap.forward_speed_mps;
                let err = target - speed;
                let throttle = (err * 0.5 + 0.3).clamp(0.0, 1.0);
                let brake = if err < -1.5 {
                    (-err * 0.25).clamp(0.0, 1.0)
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
                if step as f32 * DT >= 5.0 {
                    let snap = world.snapshot(car).unwrap();
                    let right = snap.orientation * Vec3::X;
                    roll_sum += right.y.asin().to_degrees().abs();
                    lat_sum += snap.angular_velocity.y.abs() * snap.forward_speed_mps.abs();
                    contact_min = contact_min.min(snap.wheels.iter().filter(|w| w.contact).count());
                    samples += 1;
                }
            }
            let snap = world.snapshot(car).unwrap();
            let loads: Vec<String> = snap
                .wheels
                .iter()
                .map(|w| format!("{:.0}", w.load_n))
                .collect();
            let y = snap.reference_position.y;
            println!(
                "v*={:4.0} steer={:.1} v={:5.1} roll={:5.1}deg lat_g={:4.2} min_ct={} y={:6.3} loads=[{}]",
                target,
                steer,
                snap.forward_speed_mps,
                roll_sum / samples.max(1) as f32,
                lat_sum / samples.max(1) as f32 / 9.81,
                contact_min,
                y,
                loads.join(" "),
            );
        }
    }
}

#[test]
#[ignore]
fn sustained_circle_envelope() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };
    println!(
        "spec: wheelbase={:.2} mass={:.0} lock={:.1} peak_friction={:.2} ClA={:.2} cg={:.3}",
        spec.wheelbase_m,
        spec.mass_kg,
        spec.max_steer_angle_deg,
        spec.tyres.peak_friction,
        spec.aero.downforce_coeff,
        spec.cg_height_m,
    );
    // 小舵角 = 大半径高速弯，大舵角 = 低速发夹。每个组合从静止全油门跑
    // 25 s，最后 5 s 当作稳态。
    for steer in [0.04f32, 0.07, 0.10, 0.15, 0.22, 0.32, 0.48] {
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
        world.warm_up_vehicle(car).expect("warm up");
        let mut steady_sum = 0.0f32;
        let mut steady_samples = 0u32;
        for step in 0..(25.0 / DT) as usize {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 1.0,
                        brake: 0.0,
                        steering: steer,
                        abs: true,
                        tc: true,
                        ..Controls::default()
                    },
                )
                .unwrap();
            world.step_exact(DT).unwrap();
            if step as f32 * DT >= 20.0 {
                let snap = world.snapshot(car).unwrap();
                steady_sum += snap.angular_velocity.y.abs() * snap.forward_speed_mps.abs();
                steady_samples += 1;
            }
        }
        let snap = world.snapshot(car).unwrap();
        let speed = snap.forward_speed_mps;
        let yaw_rate = snap.angular_velocity.y.abs();
        let radius = if yaw_rate > 1.0e-4 {
            speed / yaw_rate
        } else {
            f32::INFINITY
        };
        let steady_lat = steady_sum / steady_samples.max(1) as f32;
        let contacts = snap.wheels.iter().filter(|wheel| wheel.contact).count();
        let load: f32 = snap.wheels.iter().map(|wheel| wheel.load_n).sum();
        let lat_force: f32 = snap
            .wheels
            .iter()
            .map(|wheel| wheel.lateral_force_n.abs())
            .sum();
        println!(
            "steer={:.2} wheel={:5.1}deg v={:5.1} m/s R={:6.1} m lat_g={:4.2} steady_g={:4.2} \
             mu={:.2} contacts={} load={:.0}N",
            steer,
            snap.wheels[0].steer_angle.to_degrees(),
            speed,
            radius,
            speed * yaw_rate / 9.81,
            steady_lat / 9.81,
            lat_force / load.max(1.0),
            contacts,
            load,
        );
    }
}
