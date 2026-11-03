//! End-to-end world tests: car ↔ track, car ↔ car, and weather.
//!
//! These drive the public API only; they do not reach into Jolt internals.

use glam::Vec3;
use vettura_physics::{Controls, Pose, VehicleSpec};
use vettura_world::{
    ContactEntity, SurfaceKind, SurfaceMap, VehicleHandle, VehicleTuning, Weather, World,
};

const DT: f32 = 1.0 / 240.0;

fn step(world: &mut World, steps: usize) {
    for _ in 0..steps {
        world.step_exact(DT).unwrap();
    }
}

fn settle(world: &mut World, car: VehicleHandle) {
    for _ in 0..240 {
        world.step_exact(DT).unwrap();
    }
    assert!(
        world
            .snapshot(car)
            .unwrap()
            .wheels
            .iter()
            .all(|wheel| wheel.contact),
        "car did not settle on its wheels"
    );
}

fn car() -> VehicleSpec {
    VehicleSpec::default()
}

fn lateral_speed(snapshot: &vettura_world::VehicleSnapshot) -> f32 {
    snapshot.linear_velocity.dot(snapshot.right())
}

#[test]
fn car_car_collision_reports_an_impact_between_two_vehicles() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let rear = world.add_vehicle(car(), Pose::default()).unwrap();
    let front = world
        .add_vehicle(
            car(),
            Pose {
                position: Vec3::new(0.0, 0.0, -8.0),
                orientation: glam::Quat::IDENTITY,
            },
        )
        .unwrap();
    settle(&mut world, rear);
    step(&mut world, 240);

    // Rear car approaches the stationary front car along physics forward (-Z).
    world
        .set_vehicle_velocity(rear, Vec3::new(0.0, 0.0, -15.0), Vec3::ZERO)
        .unwrap();
    step(&mut world, 180);

    let impacts = world.drain_impacts();
    let car_car: Vec<_> = impacts
        .iter()
        .filter(|impact| {
            matches!(impact.a, ContactEntity::Vehicle(_))
                && matches!(impact.b, ContactEntity::Vehicle(_))
                && impact.a != impact.b
        })
        .collect();
    assert!(
        !car_car.is_empty(),
        "expected a car-car impact, got {impacts:?}"
    );
    assert!(car_car.iter().any(|impact| impact.normal_impulse_ns > 0.0));
    assert!(car_car.iter().any(|impact| impact.impact_speed_mps > 0.5));

    let _ = front;
}

#[test]
fn wall_collision_reports_a_track_object_impact() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    let wall = world
        .add_wall(Vec3::new(0.0, 0.6, -15.0), Vec3::new(6.0, 0.6, 0.25), 0.0)
        .unwrap();
    settle(&mut world, car);

    world
        .set_vehicle_velocity(car, Vec3::new(0.0, 0.0, -20.0), Vec3::ZERO)
        .unwrap();
    step(&mut world, 240);

    let impacts = world.drain_impacts();
    let wall_hit = impacts.iter().any(|impact| {
        matches!(impact.a, ContactEntity::TrackObject(id) if id == wall)
            || matches!(impact.b, ContactEntity::TrackObject(id) if id == wall)
    });
    assert!(wall_hit, "expected a wall impact, got {impacts:?}");
    assert!(
        impacts
            .iter()
            .filter(|impact| {
                matches!(impact.a, ContactEntity::TrackObject(id) if id == wall)
                    || matches!(impact.b, ContactEntity::TrackObject(id) if id == wall)
            })
            .any(|impact| impact.severity > 0.0),
        "wall impact should have non-zero severity"
    );
}

#[test]
fn wet_track_reduces_turn_rate_at_the_same_speed() {
    let dry = turn_rate(SurfaceMap::new(), Weather::dry());
    let wet = turn_rate(SurfaceMap::new(), Weather::wet(1.0));

    assert!(
        dry > wet * 1.08,
        "dry yaw rate {dry} should be clearly higher than wet {wet}"
    );
}

#[test]
fn standing_water_on_the_surface_map_reduces_turn_rate() {
    let dry = turn_rate(SurfaceMap::new(), Weather::dry());

    let mut flooded = SurfaceMap::new();
    flooded.puddle_rect(-100.0, -100.0, 100.0, 100.0, 0.03);
    let puddle = turn_rate(flooded, Weather::dry());

    assert!(
        dry > puddle * 1.08,
        "dry yaw rate {dry} should exceed the puddle's {puddle}"
    );
}

fn turn_rate(surface_map: SurfaceMap, weather: Weather) -> f32 {
    let mut world = World::new(surface_map).unwrap();
    world.set_weather(weather);
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);

    // Put both worlds at exactly the same speed, then let the wheels spin up
    // so the only difference is the surface grip.
    world
        .set_vehicle_velocity(car, Vec3::new(0.0, 0.0, -20.0), Vec3::ZERO)
        .unwrap();
    step(&mut world, 60);

    world
        .set_controls(
            car,
            Controls {
                steering: 0.22,
                ..Controls::default()
            },
        )
        .unwrap();

    let mut max_yaw_rate = 0.0f32;
    let mut max_lateral = 0.0f32;
    for _ in 0..300 {
        world.step_exact(DT).unwrap();
        let snapshot = world.snapshot(car).unwrap();
        max_yaw_rate = max_yaw_rate.max(snapshot.angular_velocity.y.abs());
        max_lateral = max_lateral.max(lateral_speed(snapshot).abs());
    }
    // Diagnostic: lateral slip should also be larger on the wet track.
    assert!(max_yaw_rate.is_finite() && max_lateral.is_finite());
    max_yaw_rate
}

#[test]
fn grass_reduces_acceleration_and_top_speed() {
    let asphalt = speed_after_throttle(SurfaceMap::new());
    let grass = speed_after_throttle(SurfaceMap::all(SurfaceKind::Grass));

    assert!(
        asphalt > grass + 3.0,
        "asphalt speed {asphalt} should clearly exceed grass {grass}"
    );
}

fn speed_after_throttle(surface_map: SurfaceMap) -> f32 {
    let mut world = World::new(surface_map).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);
    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 3);
    world.snapshot(car).unwrap().forward_speed_mps
}

#[test]
fn normal_cornering_does_not_scrape_the_chassis_on_the_ground() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);
    world.clear_impacts();

    world
        .set_vehicle_velocity(car, Vec3::new(0.0, 0.0, -20.0), Vec3::ZERO)
        .unwrap();
    step(&mut world, 60);
    world
        .set_controls(
            car,
            Controls {
                steering: 0.22,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 300);

    let ground_impacts: Vec<_> = world
        .drain_impacts()
        .into_iter()
        .filter(|impact| {
            matches!(impact.a, ContactEntity::Ground) || matches!(impact.b, ContactEntity::Ground)
        })
        .collect();
    assert!(
        ground_impacts.is_empty(),
        "normal cornering should not bottom out: {ground_impacts:?}"
    );
}

#[test]
fn wind_pushes_a_car_on_low_grip_ice() {
    let mut world = World::new(SurfaceMap::all(SurfaceKind::Ice)).unwrap();
    world.set_weather(Weather::dry().with_wind(Vec3::new(80.0, 0.0, 0.0)));
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);

    step(&mut world, 240 * 2);

    let snapshot = world.snapshot(car).unwrap();
    assert!(
        snapshot.linear_velocity.x > 0.1,
        "wind should push the car sideways, velocity was {:?}",
        snapshot.linear_velocity
    );
}

#[test]
fn repeated_runs_are_bit_identical() {
    fn run() -> (Vec3, Vec3, f32, [f32; 4]) {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        world.set_weather(Weather::wet(0.7).with_wind(Vec3::new(3.0, 0.0, -2.0)));
        let car = world.add_vehicle(car(), Pose::default()).unwrap();
        settle(&mut world, car);

        for frame in 0..480 {
            let controls = Controls {
                throttle: if frame < 300 { 0.8 } else { 0.0 },
                brake: if frame >= 300 { 0.4 } else { 0.0 },
                steering: if (180..360).contains(&frame) {
                    0.3
                } else {
                    0.0
                },
                ..Controls::default()
            };
            world.set_controls(car, controls).unwrap();
            world.step_exact(DT).unwrap();
        }

        let snapshot = world.snapshot(car).unwrap();
        let loads = std::array::from_fn(|index| snapshot.wheels[index].load_n);
        (
            snapshot.position,
            snapshot.linear_velocity,
            snapshot.angular_velocity.y,
            loads,
        )
    }

    let first = run();
    let second = run();
    assert_eq!(first.0, second.0, "positions diverged");
    assert_eq!(first.1, second.1, "velocities diverged");
    assert_eq!(first.2.to_bits(), second.2.to_bits(), "yaw rate diverged");
    assert_eq!(first.3, second.3, "wheel loads diverged");
}

#[test]
fn custom_tuning_changes_the_car() {
    fn speed_after_throttle(tuning: VehicleTuning) -> f32 {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle_with(car(), Pose::default(), tuning)
            .unwrap();
        settle(&mut world, car);
        world
            .set_controls(
                car,
                Controls {
                    throttle: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 240 * 2);
        world.snapshot(car).unwrap().forward_speed_mps
    }

    let stock_speed = speed_after_throttle(VehicleTuning::default());
    let weak_speed = speed_after_throttle(VehicleTuning {
        engine_scale: 0.25,
        ..VehicleTuning::default()
    });
    assert!(
        stock_speed > weak_speed + 2.0,
        "engine tuning should change acceleration: stock {stock_speed}, weak {weak_speed}"
    );
}
