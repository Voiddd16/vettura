//! Track-module tests.
//!
//! The asset test is ignored by default: it parses the Daytona models, builds
//! the Jolt static mesh, and drives the AI line with real vehicle physics.
//! Run it with:
//!
//! ```bash
//! cargo test -p vettura-assets --test track_module -- --ignored
//! ```

use std::path::PathBuf;

use glam::{Quat, Vec3};
use vettura_assets::{TrackModule, parse_models_ini, parse_surfaces_ini};
use vettura_physics::{Pose, VehicleSpec};
use vettura_world::{AiDriver, SurfaceMap, SurfaceShape};

fn daytona_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../vettura/mods/daytona_2017")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("../vettura/mods/daytona_2017"))
}

#[test]
fn surface_map_uses_mesh_triangles_before_the_default() {
    let mut map = SurfaceMap::new();
    let grass = map.table().standard_id(vettura_world::SurfaceKind::Grass);
    let triangles = [vettura_world::ColliderTriangle {
        vertices: [
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, -10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ],
        surface: grass,
    }];
    map.set_mesh_index(vettura_world::MeshSurfaceIndex::new(&triangles));

    assert_eq!(map.surface_at(Vec3::new(0.0, 0.0, 0.0)), grass);
    assert_eq!(
        map.surface_at(Vec3::new(100.0, 0.0, 100.0)),
        vettura_world::SurfaceId::ASPHALT
    );

    // Explicit patches still win over the mesh.
    map.patch(
        SurfaceShape::circle(0.0, 0.0, 1.0),
        vettura_world::SurfaceId::GRAVEL,
    );
    assert_eq!(map.surface_at(Vec3::ZERO), vettura_world::SurfaceId::GRAVEL);
}

#[test]
fn car_settles_on_a_static_triangle_mesh() {
    let vertices = [
        Vec3::new(-20.0, 0.5, -20.0),
        Vec3::new(20.0, 0.5, -20.0),
        Vec3::new(20.0, 0.5, 20.0),
        Vec3::new(-20.0, 0.5, 20.0),
    ];
    // Counter-clockwise when viewed from above, so the front face points +Y.
    let triangles = [[0, 2, 1], [0, 3, 2]];
    let mut world = vettura_world::World::without_ground(SurfaceMap::new()).unwrap();
    world
        .add_static_mesh(&vertices, &triangles, 1.0, 0.0)
        .unwrap();
    let car = world
        .add_vehicle(
            VehicleSpec::default(),
            Pose {
                position: Vec3::new(0.0, 0.55, 0.0),
                orientation: Quat::IDENTITY,
            },
        )
        .unwrap();
    for _ in 0..240 {
        world.step_exact(1.0 / 240.0).unwrap();
    }
    let snapshot = world.snapshot(car).unwrap();
    assert!(
        snapshot.wheels.iter().all(|wheel| wheel.contact),
        "car did not settle on mesh: {snapshot:?}"
    );
    assert!((snapshot.reference_position.y - 0.5).abs() < 0.1);
}

#[test]
fn car_settles_on_an_offset_static_triangle_mesh() {
    let offset = Vec3::new(-521.0, 1.4, 276.0);
    let vertices = [
        offset + Vec3::new(-20.0, 0.0, -20.0),
        offset + Vec3::new(20.0, 0.0, -20.0),
        offset + Vec3::new(20.0, 0.0, 20.0),
        offset + Vec3::new(-20.0, 0.0, 20.0),
    ];
    let triangles = [[0, 2, 1], [0, 3, 2]];
    let mut world = vettura_world::World::without_ground(SurfaceMap::new()).unwrap();
    world
        .add_static_mesh(&vertices, &triangles, 1.0, 0.0)
        .unwrap();
    let car = world
        .add_vehicle(
            VehicleSpec::default(),
            Pose {
                position: offset + Vec3::Y * 0.05,
                orientation: Quat::IDENTITY,
            },
        )
        .unwrap();
    for _ in 0..240 {
        world.step_exact(1.0 / 240.0).unwrap();
    }
    let snapshot = world.snapshot(car).unwrap();
    assert!(
        snapshot.wheels.iter().all(|wheel| wheel.contact),
        "car did not settle on offset mesh: {snapshot:?}"
    );
}

#[test]
fn models_and_surfaces_parse_without_touching_disk() {
    let models = parse_models_ini("[MODEL_0]\nFILE=collision.kn5\n");
    assert_eq!(models.len(), 1);
    let surfaces = parse_surfaces_ini("[SURFACE_1]\nKEY=ROAD\nFRICTION=1\n");
    assert_eq!(surfaces.len(), 1);
    assert!(
        models[0]
            .transform()
            .abs_diff_eq(glam::Mat4::IDENTITY, 1.0e-6)
    );
}

#[test]
#[ignore = "loads the real Daytona models and builds Jolt static meshes"]
fn daytona_roadcourse_loads_into_a_world() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let surfaces_text =
        std::fs::read_to_string(root.join("roadcourse/data/surfaces.ini")).expect("read surfaces");
    assert_eq!(parse_surfaces_ini(&surfaces_text).len(), 5);
    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");
    assert_eq!(track.layout(), Some("roadcourse"));
    assert_eq!(track.models().len(), 8);
    assert_eq!(track.surfaces().len(), 5);
    assert!(track.collider_triangle_count() > 50_000);
    let index = track.surface_index().expect("mesh surface index");
    assert!(index.len() > 10_000);

    let map = track.surface_map();
    assert!(map.mesh_index().is_some());
    let road = track.surface_id("ROAD").expect("ROAD surface");
    assert_eq!(map.default_surface(), road);

    let mut world = track.create_world().expect("create Jolt world");
    assert_eq!(world.track_object_count(), 1);
    assert_eq!(world.vehicle_count(), 0);

    // Find a reasonably flat, sizable triangle and drop a car on it.
    let (vertices, triangles) = track.collider_mesh().expect("collider mesh");
    let road = track.surface_id("ROAD").expect("ROAD surface");
    let spawn = triangles
        .iter()
        .filter_map(|triangle| {
            let points = triangle.map(|index| vertices[index as usize]);
            let normal = (points[1] - points[0]).cross(points[2] - points[0]);
            let area = 0.5 * normal.length();
            let centroid = (points[0] + points[1] + points[2]) / 3.0;
            if area <= 4.0 || normal.normalize().y <= 0.8 {
                return None;
            }
            if track
                .surface_map()
                .mesh_index()?
                .sample(centroid + Vec3::Y * 0.1)
                != Some(road)
            {
                return None;
            }
            // The car footprint must be clear: no bridge, wall or raised curb
            // just above the candidate. O(n) per candidate is fine in a test.
            let blocked = triangles.iter().any(|other| {
                other
                    .map(|index| vertices[index as usize])
                    .iter()
                    .any(|vertex| {
                        let dx = vertex.x - centroid.x;
                        let dz = vertex.z - centroid.z;
                        dx * dx + dz * dz < 9.0 && vertex.y > centroid.y + 0.12
                    })
            });
            (!blocked).then_some(centroid)
        })
        .next()
        .expect("open flat road triangle");

    let car = world
        .add_vehicle(
            VehicleSpec::default(),
            Pose {
                position: spawn + Vec3::Y * 0.05,
                orientation: Quat::IDENTITY,
            },
        )
        .expect("spawn car on track");
    for _ in 0..240 {
        world.step_exact(1.0 / 240.0).unwrap();
    }
    let snapshot = world.snapshot(car).unwrap().clone();
    let impacts = world.drain_impacts();
    assert!(
        snapshot.wheels.iter().all(|wheel| wheel.contact),
        "car fell through the track collider: {snapshot:?}"
    );
    assert!(!impacts.is_empty() || snapshot.wheels[0].load_n > 0.0);
}

#[test]
#[ignore = "loads the real Daytona AI line and times two updates at the start line"]
fn daytona_lap_timer_survives_starting_on_the_line() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");
    let start = track.start_pose().expect("spawn pose").expect("start pose");
    let mut timer = track
        .lap_timer()
        .expect("create lap timer")
        .expect("lap timer");

    // The AI line is closed, so the start point can project to the *end* of the
    // lap. The boundary walk has to consume the start/finish line in that case;
    // otherwise it used to spin forever.
    timer.update(start.position, 0.0);
    timer.update(start.position, 1.0 / 60.0);

    assert!(
        timer.sector_count() > 0,
        "a track with sections.ini must have sectors"
    );
}

#[test]
#[ignore = "loads the real Daytona track, collision mesh and AI line"]
fn daytona_spawn_poses_are_clear_and_settle() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");
    let road = track.surface_id("ROAD").expect("ROAD surface");
    let poses = track.spawn_poses(6, 8.0).expect("select spawn poses");
    assert_eq!(poses.len(), 6, "not enough clear grid slots: {poses:?}");

    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    let line_start = line.points()[0].position;
    let delta = poses[0].position - line_start;
    assert!(
        (delta.x * delta.x + delta.z * delta.z).sqrt() <= vettura_assets::SPAWN_SNAP_RADIUS_M + 0.5,
        "first grid slot is far from the AI-line start: {poses:?}"
    );

    // Daytona's start line is banked; the spawn must follow the road normal
    // instead of standing the car on the flat Y axis.
    let up = poses[0].orientation * Vec3::Y;
    assert!(
        (0.8..0.99).contains(&up.y),
        "first grid slot ignored the road banking: up={up:?}"
    );

    for pose in &poses {
        assert_eq!(
            track.surface_map().surface_at(pose.position),
            road,
            "grid slot is off the road at {pose:?}"
        );
        assert!(pose.position.is_finite());
    }
    for (index, pose) in poses.iter().enumerate() {
        for other in &poses[index + 1..] {
            let delta = pose.position - other.position;
            assert!(
                delta.x * delta.x + delta.z * delta.z >= 7.0 * 7.0,
                "grid slots are too close: {pose:?} vs {other:?}"
            );
        }
    }

    // The first slot is the one a race would use; make sure a car can settle
    // there without touching overhead geometry.
    let mut world = track
        .create_world_without_ground()
        .expect("create Jolt world");
    let car = world
        .add_vehicle(VehicleSpec::default(), poses[0])
        .expect("spawn car at first grid slot");
    for _ in 0..240 {
        world.step_exact(1.0 / 240.0).unwrap();
    }
    let snapshot = world.snapshot(car).unwrap();
    assert!(
        snapshot.wheels.iter().all(|wheel| wheel.contact),
        "car did not settle at the first grid slot: {snapshot:?}"
    );
    assert!(
        snapshot.reference_position.y > poses[0].position.y - 0.2,
        "car fell through the track at the first grid slot: {snapshot:?}"
    );
}

#[test]
#[ignore = "loads the real Daytona track and AI line"]
fn daytona_fast_lane_starts_on_the_road() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");
    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    assert!(line.len() > 1_000);
    assert_eq!(line.extras().len(), line.len());
    assert!(line.extras().iter().any(|extra| extra.width_m() > 4.0));

    let spawn = track
        .start_pose()
        .expect("load start pose")
        .expect("AI line start");
    let start = line.points()[0].position;
    assert!((spawn.position.y - start.y - vettura_world::SPAWN_CLEARANCE_M).abs() < 1.0e-5);

    let road = track.surface_id("ROAD").expect("ROAD surface");
    assert_eq!(
        track.surface_map().surface_at(start),
        road,
        "AI line starts off the road at {start:?}"
    );
    let tangent = line.points()[1].position - line.points()[0].position;
    let tangent = Vec3::new(tangent.x, 0.0, tangent.z).normalize();
    assert!(
        (spawn.forward() - tangent).length() < 1.0e-4,
        "spawn heading {:?} does not match the line tangent {tangent:?}",
        spawn.forward()
    );
}

#[test]
#[ignore = "drives the real Daytona track with Jolt vehicle physics"]
fn daytona_ai_driver_makes_progress() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");
    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    let poses = track.spawn_poses(1, 8.0).expect("select spawn pose");
    let spawn = poses
        .first()
        .copied()
        .unwrap_or_else(|| line.start_pose().unwrap());

    let mut world = track
        .create_world_without_ground()
        .expect("create Jolt world");
    let car = world
        .add_vehicle(VehicleSpec::default(), spawn)
        .expect("spawn car");
    let mut driver = AiDriver::new(line);
    let start = world.snapshot(car).unwrap().reference_position;

    for _ in 0..(240 * 5) {
        let input = {
            let view = world.vehicle(car).unwrap();
            driver.update_view(view)
        };
        world.set_input(car, input).unwrap();
        world.step_exact(1.0 / 240.0).unwrap();
    }

    let snapshot = world.snapshot(car).unwrap();
    let traveled = (snapshot.reference_position - start).length();
    let contacts = snapshot.wheels.iter().filter(|wheel| wheel.contact).count();
    assert!(
        traveled > 30.0,
        "AI car only travelled {traveled:.1} m: {snapshot:?}"
    );
    assert!(
        contacts >= 2,
        "AI car left the track ({contacts} wheels in contact): {snapshot:?}"
    );
    assert!(
        snapshot.reference_position.y > start.y - 5.0,
        "AI car fell below the track: {snapshot:?}"
    );
}

#[test]
#[ignore = "loads the real Daytona pit lane, sections and timing gates"]
fn daytona_pit_lane_sections_and_timing() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");

    // `roadcourse/data/sections.ini` names the seven timed sectors of the lap.
    let sections = track.sections();
    assert_eq!(sections.len(), 7, "unexpected sections: {sections:?}");
    assert_eq!(sections[0].name, "International Horseshoe");
    assert_eq!(sections.last().expect("sections").name, "Start/Finish");
    assert!((sections[0].out_fraction - 0.183).abs() < 1.0e-4);
    for sections_pair in sections.windows(2) {
        assert!(
            sections_pair[1].in_fraction >= sections_pair[0].out_fraction - 1.0e-4,
            "sections overlap in lap order: {sections_pair:?}"
        );
    }

    // The pit lane is a real AI spline that can drive a car.
    assert!(track.has_pit_lane());
    let pit = track
        .load_pit_lane()
        .expect("parse pit_lane.ai")
        .expect("track ships pit_lane.ai");
    assert_eq!(pit.len(), 898);
    assert!(pit.lap_length_m() > 200.0);

    let mut world = track
        .create_world_without_ground()
        .expect("create Jolt world");
    let spawn = pit.start_pose().expect("pit lane start");
    let car = world
        .add_vehicle(VehicleSpec::default(), spawn)
        .expect("spawn car in the pit lane");
    let mut driver = track.pit_lane_driver().expect("pit driver").unwrap();
    let start = world.snapshot(car).unwrap().reference_position;
    for _ in 0..(240 * 5) {
        let input = {
            let view = world.vehicle(car).unwrap();
            driver.update_view(view)
        };
        world.set_input(car, input).unwrap();
        world.step_exact(1.0 / 240.0).unwrap();
    }
    let snapshot = world.snapshot(car).unwrap();
    let traveled = (snapshot.reference_position - start).length();
    assert!(traveled > 10.0, "pit car did not move: {snapshot:?}");
    assert!(
        snapshot.wheels.iter().filter(|wheel| wheel.contact).count() >= 2,
        "pit car left the pit lane: {snapshot:?}"
    );

    // Timing the racing line needs no physics: sweep the line and check that
    // the sector gates land on it and one lap is counted.
    let line = track
        .load_fast_lane()
        .expect("parse fast_lane.ai")
        .expect("track ships fast_lane.ai");
    let mut timer = track.lap_timer().expect("create lap timer").unwrap();
    // Seven authored boundaries plus the start/finish line at 1.0.
    assert_eq!(timer.sector_count(), 8);
    assert_eq!(timer.last_sector_times_s().len(), 0);
    let lap_length = line.lap_length_m();
    let speed_mps = 60.0;
    let steps = 800;
    let step_m = 4.0 * lap_length / steps as f32;
    for step in 0..=steps {
        let position = line
            .sample_at_distance(step as f32 * step_m)
            .expect("sample the line")
            .position;
        timer.update(position, step as f32 * step_m / speed_mps);
    }
    assert_eq!(timer.laps(), 3, "three full laps were swept");
    let lap = timer.last_lap_s().expect("recorded lap time");
    assert!(
        (lap - lap_length / speed_mps).abs() < 0.5,
        "lap time {lap} s does not match {lap_length} m at {speed_mps} m/s"
    );
    assert!(timer.was_last_lap_valid());
    assert_eq!(
        timer.last_sector_times_s().len(),
        timer.sector_count(),
        "one timed split per sector: {:?}",
        timer.last_sector_times_s()
    );
    let total: f32 = timer.last_sector_times_s().iter().sum();
    assert!(
        (total - lap).abs() < 0.1,
        "sector splits {total} s do not add up to the lap time {lap} s"
    );
    assert!(timer.best_lap_s().is_some_and(|best| best <= lap + 0.1));
}

#[test]
#[ignore = "loads the real Daytona track with its groove.ini"]
fn daytona_groove_map_covers_the_road() {
    let root = daytona_root();
    if !root.join("models_roadcourse.ini").is_file() {
        eprintln!("skipping: no Daytona assets at {}", root.display());
        return;
    }

    let track = TrackModule::load_layout(&root, "roadcourse").expect("load track");

    // Daytona ships 78 authored grooves: cold tracks have no bonus and a
    // rubbed-in line gains up to the clamped maximum.
    assert!(track.has_groove(), "Daytona should ship groove.ini");
    let settings = track.groove_settings().expect("groove settings");
    assert_eq!(settings.cold_bonus, 0.0);
    assert!(settings.mult > 0.0);
    assert!(settings.multiplier(1.0) > 1.0);

    let map = track.groove_map().expect("groove map");
    assert!(map.cell_count() > 1_000, "{} cells", map.cell_count());

    // The grid covers the whole circuit: the start straight, the far end of
    // the lap and the infield all sit inside it.
    for (x, y, z) in [(-24.0, 0.0, -2.0), (-817.0, 0.0, -800.0), (0.0, 0.0, 0.0)] {
        let progress = map.progress(Vec3::new(x, y, z));
        assert!(
            (0.0..=1.0).contains(&progress),
            "position ({x}, {y}, {z}) is off the groove grid: {progress}"
        );
    }

    // Nothing is deposited yet, so the whole track starts cold.
    assert_eq!(map.total_progress(), 0.0);
    assert_eq!(map.coverage(), 0.0);
}
