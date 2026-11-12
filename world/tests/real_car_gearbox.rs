//! 真车（F1 2008）TC/变速箱回归：需要本地 `mods/` 真车目录，`--ignored` 运行。
//!
//! 背景（桌面探针发现的两个 bug，见 CHANGELOG）：
//! - AC `INERTIA=0.012` 引擎惯性极小，TC 放行的两成油门就够把引擎钉在
//!   19000 限制器上；Jolt 的升挡门槛（驱动轮 slip > 0.1）与 TC 目标互相
//!   卡死，变速箱永不升挡。
//! - 静止刹停后 coast 的负 forward 推力把车缓慢倒推。
//!
//! 运行：cargo test -p vettura-world --test real_car_gearbox -- --ignored --nocapture

use std::path::PathBuf;

use vettura_assets::CarFolder;
use vettura_physics::{Controls, Pose, VehicleSpec};
use vettura_world::{SurfaceMap, World};

const DT: f32 = 1.0 / 600.0;

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

/// 全油门 12 s：应挣脱起步打滑正常升挡，而不是把引擎钉死在限制器上。
#[test]
#[ignore]
fn full_throttle_upshifts_and_accelerates() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };

    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(spec.clone(), Pose::default())
        .expect("spawn");
    // 前 0.4 s 让悬挂沉降到静态平衡，再开始计时。
    for _ in 0..(0.4 / DT) as usize {
        world.step_exact(DT).unwrap();
    }
    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                abs: true,
                tc: true,
                ..Controls::default()
            },
        )
        .unwrap();

    let mut speed_10s = 0.0;
    for frame in 0..(12.0 / DT) as usize {
        world.step_exact(DT).unwrap();
        if frame % 600 == 0 {
            let snap = world.snapshot(car).unwrap();
            println!(
                "t={:5.2} speed={:6.1} km/h gear={:2} rpm={:7.0} slipRL={:5.2}",
                frame as f32 * DT,
                snap.forward_speed_mps * 3.6,
                snap.gear,
                snap.engine_rpm,
                snap.wheels[2].slip_ratio,
            );
        }
        if frame == (10.0 / DT) as usize {
            speed_10s = world.snapshot(car).unwrap().forward_speed_mps;
        }
    }

    let snap = world.snapshot(car).unwrap();
    assert!(
        speed_10s >= 25.0,
        "10 s 全油门只到 {:.1} m/s（应 ≥ 25）：升挡/TC 回归",
        speed_10s
    );
    assert!(
        snap.gear >= 2,
        "12 s 全油门后仍在一挡（gear={}）：升挡封锁回归",
        snap.gear
    );
}

/// 全油门 5 s 起步，再满刹车 6 s：刹停后应稳稳停住，不许被 coast 推力倒车。
#[test]
#[ignore]
fn standstill_full_brake_holds_without_reverse() {
    let Some(spec) = real_car_spec() else {
        eprintln!("skipping: mods/ 车目录不存在");
        return;
    };

    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(spec.clone(), Pose::default())
        .expect("spawn");
    for _ in 0..(0.4 / DT) as usize {
        world.step_exact(DT).unwrap();
    }
    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                abs: true,
                tc: true,
                ..Controls::default()
            },
        )
        .unwrap();
    for _ in 0..(5.0 / DT) as usize {
        world.step_exact(DT).unwrap();
    }
    let speed_at_brake = world.snapshot(car).unwrap().forward_speed_mps;
    println!("刹车起点 speed={:.1} m/s", speed_at_brake);
    assert!(
        speed_at_brake > 8.0,
        "5 s 全油门后只有 {:.1} m/s：起步加速回归",
        speed_at_brake
    );

    world
        .set_controls(
            car,
            Controls {
                brake: 1.0,
                abs: true,
                tc: true,
                ..Controls::default()
            },
        )
        .unwrap();
    let mut min_speed = f32::INFINITY;
    for frame in 0..(6.0 / DT) as usize {
        world.step_exact(DT).unwrap();
        if frame % 100 == 0 {
            min_speed = min_speed.min(world.snapshot(car).unwrap().forward_speed_mps);
        }
        if frame % 600 == 0 {
            let snap = world.snapshot(car).unwrap();
            println!(
                "t={:5.2} speed={:6.2} m/s gear={:2} rpm={:7.0}",
                frame as f32 * DT,
                snap.forward_speed_mps,
                snap.gear,
                snap.engine_rpm,
            );
        }
    }

    let final_speed = world.snapshot(car).unwrap().forward_speed_mps;
    assert!(
        final_speed.abs() < 0.5,
        "刹停 6 s 后还在动（{:.2} m/s）：静止 hold 回归",
        final_speed
    );
    assert!(
        min_speed > -1.0,
        "刹车期间出现倒车（最低 {:.2} m/s）：coast 门槛回归",
        min_speed
    );
}
