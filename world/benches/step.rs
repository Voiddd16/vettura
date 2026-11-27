//! 物理世界步进的轻量基准（不依赖 criterion）。
//!
//! 目的：给 `World::step_exact` 一个可重复的墙钟基线，改动热点路径前后对比
//! 用同一台机器跑同一份数据即可。默认用程序生成的平地，保证 CI/无 mods 环境
//! 也能跑；设置 `VETTURA_TRACK_ROOT` 指向真实 AC 赛道目录时会改用真实碰撞
//! 三角网与 AI 线（更接近游戏内的采样开销）。
//!
//! ```bash
//! cargo bench -p vettura-world --bench step
//! VETTURA_TRACK_ROOT=mods/daytona_2017 cargo bench -p vettura-world --bench step
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use vettura_physics::{Pose, VehicleSpec};
use vettura_world::{SurfaceMap, World};

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// 程序生成的方格三角网：每格 2 个三角形，边长 `cell`。
fn synthetic_track(half_extent: f32, cell: f32) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let steps = (2.0 * half_extent / cell).ceil() as usize;
    let mut vertices = Vec::with_capacity((steps + 1) * (steps + 1));
    for z in 0..=steps {
        for x in 0..=steps {
            vertices.push(Vec3::new(
                -half_extent + x as f32 * cell,
                0.0,
                -half_extent + z as f32 * cell,
            ));
        }
    }
    let row = steps + 1;
    let mut triangles = Vec::with_capacity(steps * steps * 2);
    for z in 0..steps {
        for x in 0..steps {
            let i = (z * row + x) as u32;
            triangles.push([i, i + row as u32, i + 1]);
            triangles.push([i + 1, i + row as u32, i + row as u32 + 1]);
        }
    }
    (vertices, triangles)
}

fn weekday_world(track_root: Option<&PathBuf>, spec: &VehicleSpec, cars: usize) -> World {
    if let Some(root) = track_root {
        let surfaces = root.join("data/surfaces.ini");
        if surfaces.is_file() {
            let track = vettura_assets::TrackModule::load(root).expect("load track");
            let mut world = track.create_world().expect("create world from track");
            let poses = track.spawn_poses(cars.max(1), 8.0).unwrap_or_default();
            let fallback = track.start_pose().ok().flatten().unwrap_or(Pose {
                position: Vec3::new(0.0, 0.5, 0.0),
                orientation: Quat::IDENTITY,
            });
            for index in 0..cars.max(1) {
                let pose = poses.get(index).copied().unwrap_or(fallback);
                let _ = world.add_vehicle(
                    spec.clone(),
                    Pose {
                        position: pose.position + Vec3::Y * 0.05,
                        orientation: pose.orientation,
                    },
                );
            }
            return world;
        }
    }

    let (vertices, triangles) = synthetic_track(200.0, 5.0);
    let mut world = World::without_ground(SurfaceMap::new()).expect("world");
    world
        .add_static_mesh(&vertices, &triangles, 1.0, 0.0)
        .expect("static mesh");
    for index in 0..cars.max(1) {
        world
            .add_vehicle(
                spec.clone(),
                Pose {
                    position: Vec3::new(index as f32 * 4.0, 0.6, 0.0),
                    orientation: Quat::IDENTITY,
                },
            )
            .expect("vehicle");
    }
    world
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn main() {
    let track_root = std::env::var("VETTURA_TRACK_ROOT")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    let car_root = std::env::var("VETTURA_CAR_ROOT")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    let dt = env_f32("VETTURA_STEP_DT", 1.0 / 600.0);
    let warmup = env_usize("VETTURA_STEP_WARMUP", 600);
    let iters = env_usize("VETTURA_STEP_ITERS", 6_000);
    let runs = env_usize("VETTURA_STEP_RUNS", 5);
    let cars = env_usize("VETTURA_CARS", 1);

    let spec = match &car_root {
        Some(root) => vettura_assets::CarFolder::open(root)
            .and_then(|car| car.vehicle_spec())
            .unwrap_or_else(|error| {
                eprintln!(
                    "car {} unusable ({error}); using the default spec",
                    root.display()
                );
                VehicleSpec::default()
            }),
        None => VehicleSpec::default(),
    };

    let mut world = weekday_world(track_root.as_ref(), &spec, cars);
    println!(
        "world: vehicles={} objects={} dt={:.6}s cars={cars} mass={:.0}kg",
        world.vehicle_count(),
        world.track_object_count(),
        dt,
        spec.mass_kg,
    );

    for _ in 0..warmup {
        world.step_exact(dt).expect("warmup step");
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(runs);
    for run in 0..runs {
        let start = Instant::now();
        for _ in 0..iters {
            world.step_exact(dt).expect("step");
        }
        let elapsed = start.elapsed();
        samples.push(elapsed);
        println!(
            "run {run}: {:>8.3} ms  ({:>9.1} steps/s, {:>8.3} ms/step)",
            elapsed.as_secs_f64() * 1e3,
            iters as f64 / elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1e3 / iters as f64,
        );
    }

    samples.sort();
    let best = samples[0];
    let median = percentile(&samples, 0.5);
    let mean: Duration = samples.iter().sum::<Duration>() / samples.len() as u32;
    println!(
        "best {:.3} ms | median {:.3} ms | mean {:.3} ms  →  {:.1} steps/s (median)",
        best.as_secs_f64() * 1e3,
        median.as_secs_f64() * 1e3,
        mean.as_secs_f64() * 1e3,
        iters as f64 / median.as_secs_f64(),
    );

    // 60 fps + 600 Hz 固定步长：一帧最多 10 个物理子步，但默认上限是 8。
    let per_frame = median.as_secs_f64() * 1e3 / iters as f64 * 8.0;
    println!("8 substeps: {per_frame:.3} ms/frame");
}
