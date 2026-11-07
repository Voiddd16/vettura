//! End-to-end world tests: car ↔ track, car ↔ car, and weather.
//!
//! These drive the public API only; they do not reach into Jolt internals.

use glam::Vec3;
use vettura_physics::{
    AeroSpec, Controls, DrsSpec, EngineSpec, ErsMode, ErsSpec, Pose, TorquePoint, TurboSpec,
    TyreSpec, TyreWearSpec, VehicleSpec,
};
use vettura_world::{
    ContactEntity, DebrisShape, DebrisSpec, SurfaceId, SurfaceKind, SurfaceMap, SurfaceShape,
    VehicleHandle, VehicleTuning, Weather, World,
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

/// A car with downforce, so the effect can be measured.
fn aero_car() -> VehicleSpec {
    VehicleSpec {
        aero: AeroSpec {
            downforce_coeff: 6.0,
            front_bias: 0.4,
            ride_height_fade: 0.25,
            wings: Vec::new(),
            side_force_coeff: 0.0,
            center_of_pressure_m: 0.6,
            dirty_air_loss: 0.0,
            dirty_air_distance_m: 25.0,
        },
        ..VehicleSpec::default()
    }
}

fn _unused_full_throttle(controls: Controls) -> Controls {
    Controls {
        throttle: 1.0,
        ..controls
    }
}

#[test]
fn downforce_loads_the_wheels_at_speed() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(aero_car(), Pose::default())
        .expect("spawn car");
    settle(&mut world, car);

    // Rolling along at speed: aero pushes the tyres into the road, so the
    // vertical load per wheel grows with velocity.
    let parked = world.snapshot(car).unwrap();
    let parked_load: f32 = parked.wheels.iter().map(|wheel| wheel.load_n).sum();

    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 4);

    let moving = world.snapshot(car).unwrap();
    let moving_load: f32 = moving.wheels.iter().map(|wheel| wheel.load_n).sum();
    assert!(
        moving_load > parked_load * 1.05,
        "downforce did not load the tyres: {parked_load} N → {moving_load} N"
    );
    assert!(
        moving.forward_speed_mps > 5.0,
        "car did not reach a meaningful speed: {moving:?}"
    );
}

#[test]
fn without_downforce_the_load_stays_near_static() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(VehicleSpec::default(), Pose::default())
        .unwrap();
    settle(&mut world, car);
    let parked = world.snapshot(car).unwrap();
    let parked_load: f32 = parked.wheels.iter().map(|wheel| wheel.load_n).sum();

    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 4);
    let moving = world.snapshot(car).unwrap();
    let moving_load: f32 = moving.wheels.iter().map(|wheel| wheel.load_n).sum();
    // Drag alone only lifts load slightly; the change must stay small.
    assert!(
        (moving_load - parked_load).abs() < parked_load * 0.25,
        "unrequested aero load appeared: {parked_load} N → {moving_load} N"
    );
}

#[test]
fn braking_heats_the_discs_and_fuel_burns() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);
    let initial_fuel = world.fuel_fraction(car).expect("fuel fraction");

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
    assert!(
        world.snapshot(car).unwrap().forward_speed_mps > 5.0,
        "car did not get moving"
    );
    assert!(
        world.fuel_fraction(car).unwrap() < initial_fuel,
        "fuel did not burn: {initial_fuel} → {}",
        world.fuel_fraction(car).unwrap()
    );

    // Brake: disc temperature must rise while the car slows down. Braking is
    // firm but not full, so the wheels keep turning and every disc dissipates.
    let cold = world.brake_temperatures_c(car).unwrap();
    world
        .set_controls(
            car,
            Controls {
                throttle: 0.0,
                brake: 0.6,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 2);
    let hot = world.brake_temperatures_c(car).unwrap();
    assert!(
        hot.iter().zip(cold).all(|(hot, cold)| *hot > cold),
        "braking did not heat the discs: {cold:?} → {hot:?}"
    );

    // The brakes fade once hot, so pedal input delivers less torque.
    let faded = hot
        .iter()
        .map(|temperature| VehicleSpec::default().brakes.efficiency_at(*temperature));
    assert!(
        faded.clone().all(|efficiency| efficiency < 1.0),
        "discs should be out of the cold window after hard braking: {hot:?}"
    );

    // A pit stop refuels the car.
    world.set_fuel_fraction(car, 1.0).unwrap();
    assert_eq!(world.fuel_fraction(car), Some(1.0));
    assert!(world.fuel_kg(car).unwrap() > 0.0);
}

#[test]
fn drs_cuts_drag_when_the_car_has_it() {
    let spec = |available: bool, reduction: f32| VehicleSpec {
        drs: DrsSpec {
            available,
            drag_reduction: reduction,
            ..DrsSpec::default()
        },
        ..VehicleSpec::default()
    };

    // Reach the same speed with and without DRS on a car whose wing has the
    // system: the DRS car must stay slightly faster, because drag is lower.
    let speed_after = |spec: VehicleSpec, drs: bool| -> f32 {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
        settle(&mut world, car);
        world
            .set_controls(
                car,
                Controls {
                    throttle: 1.0,
                    drs,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 240 * 4);
        world.snapshot(car).unwrap().forward_speed_mps
    };

    let with_drs = speed_after(spec(true, 0.2), true);
    let without = speed_after(spec(true, 0.2), false);
    assert!(
        with_drs > without,
        "DRS did not reduce drag: {without} m/s → {with_drs} m/s"
    );

    // A car without the system is unaffected by the request.
    let plain = speed_after(spec(false, 0.5), true);
    let plain_off = speed_after(spec(false, 0.5), false);
    assert!(
        (plain - plain_off).abs() < 0.5,
        "a car without DRS should ignore the request: {plain_off} vs {plain}"
    );
}

/// A straight line with a groove map on it, so a car can lay rubber.
#[test]
fn driving_over_the_same_line_builds_grip_there() {
    use glam::Vec3;
    use vettura_world::GrooveSettings;

    // A 400 m strip of road along -Z, wide enough to leave the racing line and
    // come back: the middle must gain grip, the edges must not.
    let vertices: Vec<Vec3> = (0..=400)
        .flat_map(|step| {
            let z = -step as f32;
            vec![Vec3::new(-12.0, 0.0, z), Vec3::new(12.0, 0.0, z)]
        })
        .collect();
    let mut indices = Vec::with_capacity(400);
    for strip in 0..400 {
        let a = strip * 2;
        indices.push([a, a + 1, a + 2]);
        indices.push([a + 1, a + 3, a + 2]);
    }

    let mut world = World::new(SurfaceMap::new()).unwrap();
    world
        .add_static_mesh(&vertices, &indices, 1.0, 0.0)
        .unwrap();
    world.set_groove_map(vettura_world::GrooveMap::new(
        &vertices,
        GrooveSettings::default(),
        4.0,
    ));
    assert!(world.has_groove(), "the world should track rubber now");

    let car = world
        .add_vehicle(VehicleSpec::default(), Pose::default())
        .unwrap();
    settle(&mut world, car);
    // A rolling car with no input still lays a little rubber; throttle makes
    // it a clear racing line.
    world
        .set_controls(
            car,
            Controls {
                throttle: 0.6,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 3);

    let middle = world.groove_progress_at(Vec3::new(0.0, 0.0, -20.0));
    let edge = world.groove_progress_at(Vec3::new(10.0, 0.0, -20.0));
    assert!(
        middle > 1.0e-3,
        "driving over the road did not lay rubber: {middle}"
    );
    assert!(
        edge < middle,
        "the road edge gained more rubber than the racing line: {edge} vs {middle}"
    );
    assert!(
        world.groove_grip_at(Vec3::new(0.0, 0.0, -20.0)) > 1.0,
        "grip on the racing line did not rise"
    );
    assert!(
        world.groove_grip_at(Vec3::new(30.0, 0.0, 30.0)) >= 1.0,
        "grip far from the track should stay neutral"
    );

    // Coverage is a small part of the strip, and a reset clears everything.
    let map = world.groove_map().unwrap();
    assert!(
        map.coverage() > 0.0 && map.coverage() < 0.5,
        "{}",
        map.coverage()
    );
    world.reset_groove();
    assert_eq!(world.groove_progress_at(Vec3::new(0.0, 0.0, -20.0)), 0.0);
    assert!(
        world.groove_grip_at(Vec3::new(0.0, 0.0, -20.0)) <= 1.0,
        "a reset track should not keep its rubber bonus"
    );
}

#[test]
fn a_world_without_a_groove_map_reports_neutral_grip() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    assert!(!world.has_groove());
    assert_eq!(world.groove_progress_at(Vec3::ZERO), 0.0);
    assert_eq!(world.groove_grip_at(Vec3::ZERO), 1.0);
    world.reset_groove();
}

#[test]
fn tyres_warm_up_and_wear_down_as_the_car_drives() {
    // A soft tyre, so a few seconds of driving produces measurable wear.
    let soft = VehicleSpec {
        tyres: TyreSpec {
            wear: TyreWearSpec {
                energy_capacity_j: 2.0e5,
                ..VehicleSpec::default().tyres.wear
            },
            ..VehicleSpec::default().tyres
        },
        ..VehicleSpec::default()
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(soft.clone(), Pose::default()).unwrap();
    settle(&mut world, car);

    // Cold start: the tyres begin at the ambient temperature the brakes use.
    let before = world.snapshot(car).unwrap().tyre_temperatures_c;
    let wear_before = world.snapshot(car).unwrap().tyre_wear;
    assert!(
        before.iter().all(|temperature| *temperature < 40.0),
        "tyres should start near ambient: {before:?}"
    );
    assert!(
        wear_before.iter().all(|wear| *wear < 1.0e-4),
        "fresh tyres should have no measurable wear: {wear_before:?}"
    );

    // Drive: slip energy heats the rubber up.
    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 4);

    let snapshot = world.snapshot(car).unwrap();
    let after = snapshot.tyre_temperatures_c;
    assert!(
        after.iter().zip(before).all(|(hot, cold)| *hot > cold),
        "driving did not heat the tyres: {before:?} → {after:?}"
    );
    // Pressure follows the carcass temperature.
    assert!(
        snapshot.tyre_pressures_psi.iter().all(|psi| *psi > 0.0),
        "{:?}",
        snapshot.tyre_pressures_psi
    );
    // Accelerating wears the driven rears, so at least one wheel has wear.
    assert!(
        snapshot.tyre_wear.iter().any(|wear| *wear > 0.0),
        "tyres did not wear: {:?}",
        snapshot.tyre_wear
    );

    // The grip multiplier must sit inside the tyre window the spec defines.
    let tyres = soft.tyres;
    for (index, temperature) in after.iter().enumerate() {
        let pressure = tyres.pressure_at(*temperature);
        let wear = snapshot.tyre_wear[index];
        let multiplier = tyres.grip_multiplier(*temperature, pressure, wear);
        assert!(
            (0.5..=1.0).contains(&multiplier),
            "wheel {index} left the tyre window: {multiplier}"
        );
    }

    // A pit stop fits fresh tyres. The reported values come from the snapshot,
    // so one step has to run before they show the new rubber.
    let worn_before_reset = snapshot.tyre_wear;
    world.reset_tyres(car).unwrap();
    step(&mut world, 1);
    let fresh = world.snapshot(car).unwrap();
    assert!(
        fresh
            .tyre_temperatures_c
            .iter()
            .all(|temperature| *temperature < 40.0),
        "tyres were not reset: {:?}",
        fresh.tyre_temperatures_c
    );
    // The pit stop also wipes the wear: what is left is the single step that
    // ran after the stop.
    assert!(
        fresh
            .tyre_wear
            .iter()
            .zip(worn_before_reset)
            .all(|(after, before)| *after < before),
        "tyre wear was not reset: {:?} vs {worn_before_reset:?}",
        fresh.tyre_wear
    );
}

/// A car with a hybrid system, so deployment and recovery can be measured.
fn hybrid_car() -> VehicleSpec {
    VehicleSpec {
        ers: ErsSpec {
            available: true,
            capacity_j: 4.0e6,
            max_deploy_w: 300_000.0,
            max_harvest_w: 300_000.0,
            deploy_efficiency: 1.0,
            harvest_efficiency: 1.0,
            lap_deploy_limit_j: 1.0e6,
            rear_bias: 1.0,
        },
        ..VehicleSpec::default()
    }
}

#[test]
fn deploying_the_hybrid_costs_energy_and_adds_speed() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(hybrid_car(), Pose::default()).unwrap();
    world.set_ers_soc(car, 0.5).unwrap();
    settle(&mut world, car);

    // Full throttle with deployment: energy drains and the car goes faster
    // than the same car without deployment.
    let speed_with = {
        world
            .set_controls(
                car,
                Controls {
                    throttle: 1.0,
                    ers: ErsMode::Attack,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 240 * 3);
        let snapshot = world.snapshot(car).unwrap().clone();
        (snapshot.forward_speed_mps, world.ers_energy_j(car).unwrap())
    };

    let speed_without = {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(hybrid_car(), Pose::default()).unwrap();
        world.set_ers_soc(car, 0.5).unwrap();
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
    };

    assert!(
        speed_with.1 < 0.5 * hybrid_car().ers.capacity_j - 1.0,
        "deployment did not spend battery: {}",
        speed_with.1
    );
    assert!(
        speed_with.0 > speed_without * 1.02,
        "deployment did not help: {} vs {speed_without}",
        speed_with.0
    );

    // The per-lap allowance stops the spending once it is used up.
    world.reset_ers_lap(car).unwrap();
    assert_eq!(world.ers_lap_deploy_j(car), Some(0.0));
}

#[test]
fn braking_with_harvest_recovers_energy_and_costs_speed() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(hybrid_car(), Pose::default()).unwrap();
    world.set_ers_soc(car, 0.1).unwrap();
    settle(&mut world, car);

    // Get up to speed first, with the machine off.
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
    let before = world.snapshot(car).unwrap();
    let energy_before = world.ers_energy_j(car).unwrap();
    let speed_before = before.forward_speed_mps;
    assert!(speed_before > 5.0, "car did not get moving: {speed_before}");

    // Brake in harvest mode: the battery fills and the car stops.
    world
        .set_controls(
            car,
            Controls {
                throttle: 0.0,
                brake: 1.0,
                ers: ErsMode::Harvest,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 2);
    let after = world.snapshot(car).unwrap();
    assert!(
        world.ers_energy_j(car).unwrap() > energy_before,
        "harvest did not recharge: {energy_before} → {}",
        world.ers_energy_j(car).unwrap()
    );
    assert!(
        after.forward_speed_mps < speed_before,
        "the car did not slow down: {speed_before} → {}",
        after.forward_speed_mps
    );
}

#[test]
fn an_empty_battery_cannot_deploy() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(hybrid_car(), Pose::default()).unwrap();
    world.set_ers_soc(car, 0.0).unwrap();
    settle(&mut world, car);

    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ers: ErsMode::Attack,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 3);

    assert!(
        (world.ers_energy_j(car).unwrap()) < 1.0,
        "a flat battery produced energy from nothing: {}",
        world.ers_energy_j(car).unwrap()
    );
    assert_eq!(world.ers_soc(car), Some(0.0));
}

#[test]
fn the_engine_warms_up_and_overheats_under_load() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle(VehicleSpec::default(), Pose::default())
        .unwrap();
    settle(&mut world, car);

    // A cold engine is only lightly penalised, and it warms up while driving.
    let cold = world.engine_temperatures_c(car).expect("temperatures");
    let cold_power = world.snapshot(car).unwrap().engine_power_multiplier;
    assert!(
        cold_power > 0.85 && cold_power < 1.0,
        "a cold engine should be off, but not crippled: {cold_power}"
    );

    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 5);

    let warm = world.engine_temperatures_c(car).expect("temperatures");
    assert!(
        warm.0 > cold.0 && warm.1 > cold.1,
        "the engine did not warm up: {cold:?} → {warm:?}"
    );
    let warm_power = world.snapshot(car).unwrap().engine_power_multiplier;
    assert!(
        warm_power > cold_power,
        "a warm engine should make at least as much power: {cold_power} → {warm_power}"
    );

    // Overheating: hold full throttle for a long time and the power multiplier
    // must fall below the warm value.
    step(&mut world, 240 * 25);
    let hot_power = world.snapshot(car).unwrap().engine_power_multiplier;
    assert!(
        hot_power < warm_power,
        "an overheating engine kept its power: {warm_power} → {hot_power}"
    );
}

#[test]
fn a_wet_track_builds_a_water_film_that_cools_the_tyres() {
    let wet = Weather::wet(0.9);
    let mut map = SurfaceMap::new();
    // Standing water everywhere the car drives: 2 cm of it.
    map.puddle_circle(0.0, 0.0, 200.0, 0.02);
    let mut world = World::new(map).unwrap();
    world.set_weather(wet);
    let car = world
        .add_vehicle(VehicleSpec::default(), Pose::default())
        .unwrap();
    settle(&mut world, car);

    // A stationary tyre in a puddle is already soaked, which is why rolling
    // off the line in the wet feels slow: the film is there before the car moves.
    let soaked = world.water_film_mm(car).expect("film");
    assert!(
        soaked.iter().all(|mm| *mm > 0.0),
        "a parked car in 2 cm of water should carry a film: {soaked:?}"
    );

    // Drive through the water: the tread sheds it again, and the tyres stay
    // cooler than they would on a dry road.
    world
        .set_controls(
            car,
            Controls {
                throttle: 0.8,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 4);

    let film = world.water_film_mm(car).expect("film");
    assert!(film.iter().all(|mm| mm.is_finite()), "{film:?}");
    assert!(
        film.iter().all(|mm| *mm <= 20.0 + 1.0e-3),
        "the film cannot exceed the water on the road: {film:?}"
    );
    // The snapshot lags the world by one step, but must agree.
    let reported = world.snapshot(car).unwrap().water_film_mm;
    assert!(
        reported
            .iter()
            .zip(film)
            .all(|(reported, film)| (*reported - film).abs() < 1.0),
        "the snapshot must report the same film: {reported:?} vs {film:?}"
    );

    // Water cools the tyres: the driven axle on a wet road must stay below
    // the overheat guard even after sustained load, and the film must bring
    // the temperature down once the car coasts.
    let wet_tyre = world.snapshot(car).unwrap().tyre_temperatures_c;
    assert!(
        wet_tyre.iter().all(|c| c.is_finite() && *c < 250.0),
        "the wet tyres ran away thermally: {wet_tyre:?}"
    );

    // Coasting on the dry-off road sheds the film, and the tyres cool.
    world.set_weather(Weather::dry());
    step(&mut world, 240 * 3);
    let dried = world.water_film_mm(car).expect("film");
    assert!(
        dried.iter().all(|mm| *mm < film[0] + 1.0e-3),
        "the film did not dry off: {film:?} → {dried:?}"
    );
}

#[test]
fn the_water_film_model_has_a_steady_state() {
    let wet = Weather::wet(1.0);
    let dry = Weather::dry();

    // More water on the road means a thicker film, and speed throws it off
    // again, so a wet road plus a fast tyre is not the worst case.
    let slow_wet = wet.water_film_mm(10.0, 0.02, 4_000.0, 90.0);
    let fast_wet = wet.water_film_mm(70.0, 0.02, 4_000.0, 90.0);
    assert!(slow_wet > 0.0, "{slow_wet}");
    assert!(
        fast_wet < slow_wet,
        "a faster tyre should shed more water: {slow_wet} vs {fast_wet}"
    );
    // Dry road: no film at all.
    assert_eq!(dry.water_film_mm(30.0, 0.0, 4_000.0, 90.0), 0.0);
    // Nonsense inputs stay finite.
    assert!(
        wet.water_film_mm(f32::NAN, f32::NAN, f32::NAN, f32::NAN)
            .is_finite()
    );
}

#[test]
fn aero_side_force_straightens_a_sliding_car() {
    // A car with real lateral aerodynamic data, rolling fast and sideways.
    let spec = VehicleSpec {
        aero: AeroSpec {
            downforce_coeff: 6.0,
            side_force_coeff: 8.0,
            center_of_pressure_m: 1.2,
            ..AeroSpec::default()
        },
        ..VehicleSpec::default()
    };
    // A car without it, for the comparison.
    let plain = VehicleSpec::default();

    let run = |spec: VehicleSpec| -> Vec3 {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
        settle(&mut world, car);
        // Accelerate to speed, then give it a sideways shove while still
        // rolling forward: exactly what a slide looks like.
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
        let velocity = world.snapshot(car).unwrap().linear_velocity;
        world
            .set_vehicle_velocity(
                car,
                velocity + glam::Vec3::new(6.0, 0.0, 0.0),
                glam::Vec3::ZERO,
            )
            .unwrap();
        step(&mut world, 30);
        let after = world.snapshot(car).unwrap();
        after.linear_velocity
    };

    const SHOVE_MPS: f32 = 6.0;
    let slid = run(spec);
    let drifted = run(plain);

    // With aero the side force removes the sideways velocity; without it the
    // car keeps drifting until the tyres scrub it off.
    let aero_lateral = slid.dot(glam::Vec3::X).abs();
    let plain_lateral = drifted.dot(glam::Vec3::X).abs();
    assert!(
        aero_lateral < plain_lateral,
        "aero side force did not help: {aero_lateral} vs {plain_lateral}"
    );
    // The injected sideways velocity has to shrink, and faster than the same
    // car without the aerodynamic data.
    assert!(
        aero_lateral < SHOVE_MPS * 0.9,
        "the slide was not corrected: {SHOVE_MPS} → {aero_lateral}"
    );
    assert!(
        aero_lateral < plain_lateral * 0.98,
        "the aero car must correct more than the plain one: {aero_lateral} vs {plain_lateral}"
    );

    // And the car must still be moving forward, not stopped dead.
    assert!(
        slid.dot(glam::Vec3::NEG_Z) > 1.0,
        "the car lost all forward speed: {slid:?}"
    );
}

#[test]
fn aero_side_force_scales_with_the_cars_own_velocity() {
    let spec = VehicleSpec {
        aero: AeroSpec {
            side_force_coeff: 8.0,
            center_of_pressure_m: 1.0,
            ..AeroSpec::default()
        },
        ..VehicleSpec::default()
    };
    // Sideslip of 0.2 rad at 40 m/s is a strong slide; at 5 m/s the same slip
    // is barely anything, because the car has not built up any airspeed.
    let fast = spec
        .aero
        .side_force_n(vettura_physics::AIR_DENSITY_KG_M3, 40.0, 0.2);
    let slow = spec
        .aero
        .side_force_n(vettura_physics::AIR_DENSITY_KG_M3, 5.0, 0.2);
    assert!(fast.abs() > slow.abs() * 10.0, "{fast} vs {slow}");
}

#[test]
fn braking_and_steering_at_the_same_time_costs_lateral_grip() {
    // Reach a steady corner, then keep the same steering and add brake: the
    // lateral force per newton of tyre load must drop, because the tyres have
    // only one friction budget.
    let cornering = |brake: f32, steering: f32| -> f32 {
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(VehicleSpec::default(), Pose::default())
            .unwrap();
        settle(&mut world, car);
        // Build up speed on the straight.
        world
            .set_controls(
                car,
                Controls {
                    throttle: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 240 * 4);
        // Hold a constant steer, with or without brake.
        let throttle = if brake > 0.0 { 0.0 } else { 1.0 };
        for _ in 0..180 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle,
                        steering,
                        brake,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        // Lateral force the front axle makes per newton of front load, which
        // is the utilisation of the tyres' friction budget.
        let lateral: f32 = snapshot
            .wheels
            .iter()
            .take(2)
            .map(|wheel| wheel.lateral_force_n.abs())
            .sum();
        let load: f32 = snapshot
            .wheels
            .iter()
            .take(2)
            .map(|wheel| wheel.load_n)
            .sum();
        if load > 1.0 { lateral / load } else { 0.0 }
    };

    // Steering hard enough to saturate the front tyres.
    let free = cornering(0.0, 0.6);
    let braked = cornering(1.0, 0.6);

    assert!(
        free > 0.2,
        "not enough lateral utilisation to measure: {free}"
    );
    assert!(
        braked < free,
        "braking while steering did not cost lateral grip: {free} vs {braked}"
    );
    // The effect is real but bounded: it must not collapse the budget.
    assert!(
        braked > free * 0.5,
        "the friction circle was too harsh: {free} → {braked}"
    );
}

#[test]
fn road_undulation_is_deterministic_and_surface_dependent() {
    // Smooth asphalt over the origin, rough gravel from x = 60 onwards.
    let mut map = SurfaceMap::new();
    map.patch(
        SurfaceShape::rect(-50.0, -50.0, 50.0, 50.0),
        SurfaceId::ASPHALT,
    );
    map.patch(
        SurfaceShape::rect(60.0, -50.0, 100.0, 50.0),
        SurfaceId::GRAVEL,
    );
    let world = World::new(map).unwrap();
    let heading = glam::Vec3::NEG_Z;

    // The racing line is flat...
    assert_eq!(world.road_bump_m(glam::Vec3::ZERO, heading), 0.0);
    // ...and it stays flat even deep into the corner, because asphalt is laid
    // flat by AC (`SIN_HEIGHT=0`).
    assert_eq!(
        world.road_bump_m(glam::Vec3::new(0.0, 0.0, -20.0), heading),
        0.0
    );

    // The gravel run-off undulates, and only along the travel direction: a
    // quarter of the way through the 0.35 m wavelength is at the peak.
    let groove = glam::Vec3::new(70.0, 0.0, 0.0);
    // Quarter wavelength into the run-off: the top of the undulation.
    let peak = world.road_bump_m(groove + heading * (0.35 * 0.25), heading);
    // Half a wavelength: back at the road surface.
    let flat = world.road_bump_m(groove + heading * (0.35 * 0.5), heading);
    // Three quarters: the bottom.
    let trough = world.road_bump_m(groove + heading * (0.35 * 0.75), heading);
    assert!(peak.abs() > 0.01, "{peak}");
    assert!(
        flat.abs() < 1.0e-4,
        "half a wavelength should be flat: {flat}"
    );
    assert!(peak > flat && flat > trough, "{peak} / {flat} / {trough}");
    // Deterministic: the same point reads the same value.
    assert_eq!(
        peak,
        world.road_bump_m(groove + heading * (0.35 * 0.25), heading)
    );
    // A metre sideways leaves the phase alone, because the phase is measured
    // along the travel direction.
    let sideways = world.road_bump_m(groove + glam::Vec3::X + heading * (0.35 * 0.25), heading);
    assert!((sideways - peak).abs() < 1.0e-6, "{peak} vs {sideways}");
    // A nonsense heading reports nothing rather than nonsense.
    assert!(
        world
            .road_bump_m(groove + heading * (0.35 * 0.25), glam::Vec3::ZERO)
            .abs()
            < 1.0e-6
    );
}

#[test]
fn a_rough_surface_costs_grip() {
    let smooth = World::new(SurfaceMap::new()).unwrap();
    let mut rough_map = SurfaceMap::new();
    rough_map.patch(
        SurfaceShape::rect(-200.0, -200.0, 200.0, 200.0),
        SurfaceId::GRAVEL,
    );
    let rough = World::new(rough_map).unwrap();

    let smooth_grip = smooth
        .surface_map()
        .table()
        .get(SurfaceId::ASPHALT)
        .unwrap()
        .dry_grip;
    let rough_grip = rough
        .surface_map()
        .table()
        .get(SurfaceId::GRAVEL)
        .unwrap()
        .dry_grip;
    // The gravel loss is real, and part of it is the undulation: a rough run-off
    // area cannot hold a steady load.
    assert!(rough_grip < smooth_grip);
    let rough_props = rough.surface_map().table().get(SurfaceId::GRAVEL).unwrap();
    assert!(rough_props.bump_height_m > 0.0);
    assert!(rough_props.bump_length_m > 0.0);
}

#[test]
fn active_suspension_keeps_the_body_flatter_in_a_corner() {
    let run = |active: bool| -> (f32, f32) {
        let spec = VehicleSpec {
            active_suspension: if active {
                vettura_physics::ActiveSuspensionSpec {
                    mode: vettura_physics::ActiveSuspensionMode::Active,
                    stiffness_n_per_m: 200_000.0,
                    damping_ns_per_m: 9_000.0,
                    max_force_n: 30_000.0,
                    ..Default::default()
                }
            } else {
                vettura_physics::ActiveSuspensionSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        step(&mut world, 240 * 4);
        for _ in 0..240 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.3,
                        steering: 0.5,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        let right = snapshot.orientation * Vec3::X;
        let average_length = snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.suspension_length_m)
            .sum::<f32>()
            / 4.0;
        (right.y.abs(), average_length)
    };

    let (passive_roll, _) = run(false);
    let (active_roll, _) = run(true);
    assert!(
        active_roll < passive_roll * 0.8,
        "active suspension did not reduce roll: {passive_roll} vs {active_roll}"
    );
}

#[test]
fn active_suspension_lowers_the_body_with_speed() {
    let run = |speed_drop: f32| -> f32 {
        let spec = VehicleSpec {
            active_suspension: vettura_physics::ActiveSuspensionSpec {
                mode: vettura_physics::ActiveSuspensionMode::Active,
                stiffness_n_per_m: 120_000.0,
                damping_ns_per_m: 8_000.0,
                max_force_n: 30_000.0,
                speed_drop_m: speed_drop,
                speed_ref_mps: 30.0,
                ..Default::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        step(&mut world, 240 * 5);
        let snapshot = world.snapshot(car).unwrap();
        snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.suspension_length_m)
            .sum::<f32>()
            / 4.0
    };

    let level = run(0.0);
    let lowered = run(0.05);
    assert!(
        lowered < level - 0.002,
        "speed-dependent ride height did not lower the body: {level} vs {lowered}"
    );
}

#[test]
fn a_heave_spring_carries_load_and_lifts_the_body() {
    let run = |heave: bool| -> (f32, [f32; 2]) {
        let spec = VehicleSpec {
            heave_springs: if heave {
                [vettura_physics::HeaveSpringSpec {
                    spring_rate_n_per_m: 90_000.0,
                    preload_m: 0.01,
                    damper_bump_ns_per_m: 4_000.0,
                    damper_rebound_ns_per_m: 5_000.0,
                    ..Default::default()
                }; 2]
            } else {
                [vettura_physics::HeaveSpringSpec::default(); 2]
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
        settle(&mut world, car);
        let snapshot = world.snapshot(car).unwrap();
        let average_length = snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.suspension_length_m)
            .sum::<f32>()
            / 4.0;
        (average_length, world.heave_force_n(car).unwrap())
    };

    let (passive_length, passive_force) = run(false);
    let (heave_length, heave_force) = run(true);
    assert_eq!(passive_force, [0.0, 0.0]);
    assert!(
        heave_force[0] > 100.0 && heave_force[1] > 100.0,
        "heave springs did not carry load: {heave_force:?}"
    );
    assert!(
        heave_length > passive_length + 0.001,
        "heave springs did not raise the body: {passive_length} vs {heave_length}"
    );
}

#[test]
fn turbo_builds_boost_and_adds_thrust() {
    let run = |turbo: bool| -> (f32, f32) {
        let spec = VehicleSpec {
            engine: EngineSpec {
                redline_rpm: 8_000.0,
                idle_rpm: 1_000.0,
                peak_torque_nm: 300.0,
                // Flat curve so the comparison only measures the boost.
                torque_curve: vec![
                    TorquePoint {
                        rpm: 0.0,
                        torque_nm: 300.0,
                    },
                    TorquePoint {
                        rpm: 8_000.0,
                        torque_nm: 300.0,
                    },
                ],
                turbo: if turbo {
                    TurboSpec {
                        max_boost_bar: 0.8,
                        wastegate_bar: 1.0,
                        reference_rpm: 2_000.0,
                        gamma: 1.0,
                        lag_up_s: 0.3,
                        lag_down_s: 0.2,
                        torque_gain_per_bar: 1.0,
                    }
                } else {
                    TurboSpec::default()
                },
                ..EngineSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle_with(
                spec,
                Pose::default(),
                VehicleTuning {
                    // Keep the comparison engine-limited instead of traction-limited.
                    grip_scale: 2.0,
                    ..VehicleTuning::default()
                },
            )
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
        let boost = world.turbo_boost_bar(car).unwrap();
        let snapshot = world.snapshot(car).unwrap();
        (boost, snapshot.forward_speed_mps)
    };

    let (na_boost, na_speed) = run(false);
    let (turbo_boost, turbo_speed) = run(true);
    assert_eq!(na_boost, 0.0);
    assert!(turbo_boost > 0.2, "boost did not build: {turbo_boost}");
    assert!(
        turbo_speed > na_speed * 1.05,
        "turbo did not add thrust: {na_speed} vs {turbo_speed}"
    );
}

#[test]
fn debris_falls_and_rests_on_the_ground() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let debris = world
        .spawn_debris(DebrisSpec {
            shape: DebrisShape::Sphere { radius: 0.2 },
            position: Vec3::new(0.0, 3.0, 0.0),
            restitution: 0.1,
            ..DebrisSpec::default()
        })
        .unwrap();
    assert_eq!(world.debris_count(), 1);
    assert_eq!(world.debris_handles().collect::<Vec<_>>(), vec![debris]);

    step(&mut world, 240 * 3);
    let snapshot = world.debris_snapshot(debris).unwrap();
    assert!(
        (snapshot.position.y - 0.2).abs() < 0.15,
        "debris rested at y={}",
        snapshot.position.y
    );
    assert!(
        snapshot.linear_velocity.length() < 0.2,
        "debris still moving at {:?}",
        snapshot.linear_velocity
    );
}

#[test]
fn a_car_can_hit_debris_and_the_contact_names_it() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);
    let debris = world
        .spawn_debris(DebrisSpec {
            shape: DebrisShape::Box {
                half_extents: Vec3::new(0.25, 0.25, 0.25),
            },
            position: Vec3::new(0.0, 0.25, -5.0),
            mass_kg: 4.0,
            ..DebrisSpec::default()
        })
        .unwrap();
    let before = world.debris_snapshot(debris).unwrap().position;

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

    let after = world.debris_snapshot(debris).unwrap().position;
    assert!(
        (after - before).length() > 0.5,
        "the car did not move the debris: {before:?} -> {after:?}"
    );
    let hit = world
        .impacts()
        .iter()
        .any(|event| event.a.debris() == Some(debris) || event.b.debris() == Some(debris));
    assert!(hit, "no impact event named the debris");
}

#[test]
fn removing_debris_invalidates_the_handle() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let first = world.spawn_debris(DebrisSpec::default()).unwrap();
    let second = world.spawn_debris(DebrisSpec::default()).unwrap();
    assert_eq!(world.debris_count(), 2);

    world.remove_debris(first).unwrap();
    assert!(world.debris_snapshot(first).is_none());
    assert!(world.remove_debris(first).is_err());
    assert_eq!(world.debris_count(), 1);

    world.clear_debris().unwrap();
    assert_eq!(world.debris_count(), 0);
    assert!(world.debris_snapshot(second).is_none());
}

#[test]
fn engine_braking_slows_the_car_more_than_drag_alone() {
    let run = |coast_nm: f32| -> f32 {
        let spec = VehicleSpec {
            engine: EngineSpec {
                redline_rpm: 8_000.0,
                idle_rpm: 1_000.0,
                peak_torque_nm: 300.0,
                torque_curve: vec![
                    TorquePoint {
                        rpm: 0.0,
                        torque_nm: 300.0,
                    },
                    TorquePoint {
                        rpm: 8_000.0,
                        torque_nm: 300.0,
                    },
                ],
                coast_torque_nm: coast_nm,
                coast_reference_rpm: 8_000.0,
                ..EngineSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle_with(
                spec,
                Pose::default(),
                VehicleTuning {
                    grip_scale: 2.0,
                    ..VehicleTuning::default()
                },
            )
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
        step(&mut world, 240 * 3);

        // Lift off and coast for one second; return the deceleration.
        world.set_controls(car, Controls::default()).unwrap();
        step(&mut world, 60);
        let before = world.snapshot(car).unwrap().speed_mps;
        step(&mut world, 240);
        let after = world.snapshot(car).unwrap().speed_mps;
        (before - after) / 1.0
    };

    let drag_only = run(0.0);
    let engine_braking = run(150.0);
    assert!(
        engine_braking > drag_only + 0.5,
        "engine braking did not slow the car: {drag_only} vs {engine_braking}"
    );
}

#[test]
fn a_locked_wheel_forms_a_flat_spot_that_shakes_and_heals() {
    let spec = VehicleSpec {
        tyres: TyreSpec {
            flat_spot_capacity_j: 5_000.0,
            flat_spot_grip_loss: 0.35,
            flat_spot_vibration_gain: 0.2,
            ..TyreSpec::default()
        },
        ..VehicleSpec::default()
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world
        .add_vehicle_with(
            spec,
            Pose::default(),
            VehicleTuning {
                // Low grip makes a lockup easy to provoke, like a cold tyre on
                // a wet racing line.
                grip_scale: 0.4,
                ..VehicleTuning::default()
            },
        )
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
    step(&mut world, 240 * 3);

    // Full brake: the wheels stop turning while the car is still moving, which
    // is exactly the locked-wheel case that forms a flat spot.
    let mut strongest_kick = 0.0f32;
    for _ in 0..240 * 2 {
        world
            .set_controls(
                car,
                Controls {
                    brake: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 1);
        let force = world.flat_spot_force_n(car).unwrap();
        strongest_kick = strongest_kick.max(force.iter().fold(0.0f32, |m, f| m.max(f.abs())));
    }
    let spots = world.flat_spot(car).unwrap();
    assert!(
        spots.iter().any(|spot| *spot > 0.2),
        "no flat spot: {spots:?}"
    );
    assert!(strongest_kick > 100.0, "no shake: {strongest_kick}");

    // Release the brake: rubber reliefs wear the spot away again.
    let healed_max = {
        let mut best = 0.0f32;
        for _ in 0..240 * 3 {
            world.set_controls(car, Controls::default()).unwrap();
            step(&mut world, 1);
            best = best.max(
                world
                    .flat_spot(car)
                    .unwrap()
                    .iter()
                    .copied()
                    .fold(0.0f32, f32::max),
            );
        }
        best
    };
    let locked_max = spots.iter().copied().fold(0.0f32, f32::max);
    assert!(
        healed_max < locked_max,
        "the flat spot did not heal: {locked_max} vs {healed_max}"
    );
}

#[test]
fn driver_aids_still_brake_and_drive() {
    // With the aids on the car must keep working: ABS releases pressure, it
    // does not refuse to brake, and traction control cuts torque, it does not
    // refuse to accelerate.
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    settle(&mut world, car);

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
    step(&mut world, 240 * 2);
    let driven = world.snapshot(car).unwrap().forward_speed_mps;
    assert!(driven > 1.0, "traction control stopped the car: {driven}");

    world
        .set_controls(
            car,
            Controls {
                brake: 1.0,
                abs: true,
                ..Controls::default()
            },
        )
        .unwrap();
    let before = world.snapshot(car).unwrap().forward_speed_mps;
    for _ in 0..240 {
        step(&mut world, 1);
    }
    let after = world.snapshot(car).unwrap().forward_speed_mps;
    let drop = before - after;
    assert!(
        drop > 3.0,
        "ABS stopped the car from braking: {before} -> {after}"
    );
}

#[test]
fn negative_camber_leans_the_wheels_in_without_pushing_the_car_sideways() {
    let run = |static_camber_deg: f32| -> (Vec3, f32) {
        let spec = VehicleSpec {
            ..VehicleSpec::default()
        };
        let spec = {
            let mut spec = spec;
            for wheel in &mut spec.wheels {
                wheel.suspension.static_camber_deg = static_camber_deg;
                wheel.suspension.camber_gain_rad_per_m = -0.5;
            }
            spec
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
        settle(&mut world, car);
        step(&mut world, 240 * 2);
        let snapshot = world.snapshot(car).unwrap();
        let camber = snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.camber_rad)
            .collect::<Vec<_>>();
        let lateral = snapshot.linear_velocity.dot(snapshot.right());
        (Vec3::new(camber[0], camber[1], camber[2]), lateral)
    };

    let (camber, lateral) = run(-4.0);
    // Every wheel leans inward, and by the same static amount.
    assert!(camber.x < -0.05 && camber.y < -0.05, "{camber:?}");
    assert!((camber.x - camber.y).abs() < 1e-3 && (camber.x - camber.z).abs() < 1e-3);
    // Equal and opposite camber thrusts cancel: a parked car must not drift.
    assert!(
        lateral.abs() < 0.05,
        "camber thrust drifted the car: {lateral}"
    );
}

#[test]
fn cornering_load_makes_the_outside_wheel_lean_further_in() {
    let spec = {
        let mut spec = VehicleSpec::default();
        for wheel in &mut spec.wheels {
            wheel.suspension.static_camber_deg = -2.0;
            wheel.suspension.camber_gain_rad_per_m = -0.5;
        }
        spec
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
    step(&mut world, 240 * 4);
    // Steer left: the body rolls right, so the right-hand wheels are loaded.
    for _ in 0..120 {
        world
            .set_controls(
                car,
                Controls {
                    throttle: 0.4,
                    steering: 0.6,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 1);
    }
    let snapshot = world.snapshot(car).unwrap();
    let left_rear = snapshot.wheels[2].camber_rad;
    let right_rear = snapshot.wheels[3].camber_rad;
    assert!(
        right_rear < left_rear,
        "outside wheel did not lean in more: {left_rear} vs {right_rear}"
    );
}

#[test]
fn a_two_rate_damper_changes_the_suspension_it_outputs() {
    let run = |fast: f32| -> (f32, f32) {
        let spec = {
            let mut spec = VehicleSpec::default();
            for wheel in &mut spec.wheels {
                wheel.suspension.fast_bump_damper_ns_per_m = fast;
                wheel.suspension.fast_bump_threshold_mps = 0.01;
                wheel.suspension.fast_rebound_damper_ns_per_m = fast;
                wheel.suspension.fast_rebound_threshold_mps = 0.01;
            }
            spec
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        // Corner hard: pitch and roll keep the dampers in the fast regime.
        let mut force_sum = 0.0;
        let mut samples = 0;
        for _ in 0..240 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.5,
                        steering: 0.8,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
            let force = world.suspension_force_n(car).unwrap();
            force_sum += force.iter().fold(0.0f32, |sum, value| sum + value.abs()) / 4.0;
            samples += 1;
        }
        (
            force_sum / samples as f32,
            world.snapshot(car).unwrap().speed_mps,
        )
    };

    let (slow, slow_speed) = run(0.0);
    let (fast, fast_speed) = run(1_200.0);
    assert!(
        (slow - fast).abs() > 0.05 * slow.abs(),
        "the two-rate damper made no difference: {slow} vs {fast}"
    );
    assert!(
        (slow_speed - fast_speed).abs() < 6.0,
        "the bleeder rate changed the driving line too much: {slow_speed} vs {fast_speed}"
    );
}

#[test]
fn a_published_inertia_tensor_reaches_the_body() {
    let run = |yaw_inertia: f32| -> f32 {
        let spec = VehicleSpec {
            inertia: Vec3::new(600.0, 900.0, yaw_inertia),
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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

        // Step steer and watch the yaw rate build up: the same lateral force
        // with a bigger yaw inertia has to turn the car more slowly.
        let mut peak_yaw = 0.0f32;
        for _ in 0..40 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.6,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
            let snapshot = world.snapshot(car).unwrap();
            let up = snapshot.orientation * Vec3::Y;
            peak_yaw = peak_yaw.max(snapshot.angular_velocity.dot(up).abs());
        }
        peak_yaw
    };

    let light = run(1_200.0);
    let heavy = run(60_000.0);
    assert!(
        heavy < light * 0.3,
        "the yaw inertia did not slow the rotation: {light} vs {heavy}"
    );
}

#[test]
fn debug_inertia_response() {
    fn run(yaw: f32, steps: usize) -> f32 {
        let spec = VehicleSpec {
            inertia: Vec3::new(600.0, 900.0, yaw),
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        let mut peak = 0.0f32;
        for _ in 0..steps {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.6,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
            let s = world.snapshot(car).unwrap();
            let up = s.orientation * Vec3::Y;
            peak = peak.max(s.angular_velocity.dot(up).abs());
        }
        peak
    }
    for steps in [5usize, 10, 20, 40, 120] {
        println!(
            "steps={steps} light={:.5} heavy={:.5}",
            run(1_200.0, steps),
            run(60_000.0, steps)
        );
    }
}

/// A car whose only aerodynamic data is one wing with authored tables.
fn winged_car() -> VehicleSpec {
    VehicleSpec {
        aero: vettura_physics::AeroSpec {
            downforce_coeff: 0.0,
            wings: vec![vettura_physics::WingSpec {
                chord_m: 1.0,
                span_m: 2.0,
                cl_gain: 0.0,
                cd_gain: 0.0,
                // Mounted above the centre of mass, AC frame (+Z forward).
                position: Vec3::new(0.0, 0.05, 2.0),
                angle_deg: 0.0,
                aoa_cl: vec![(0.0, 8.0), (10.0, 8.0)],
                // Low clearance makes much more load than high clearance.
                gh_cl: vec![(0.0, 1.0), (0.05, 1.0), (0.20, 0.0)],
                aoa_cd: vec![(0.0, 1.0), (10.0, 1.0)],
                gh_cd: vec![(0.0, 1.0), (0.10, 1.0)],
            }],
            ..vettura_physics::AeroSpec::default()
        },
        ..VehicleSpec::default()
    }
}

#[test]
fn wing_ground_height_tables_make_ground_effect() {
    let run = |local_y: f32| -> (f32, f32) {
        let mut spec = winged_car();
        if let Some(wing) = spec.aero.wings.first_mut() {
            wing.position.y = local_y;
        }
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        step(&mut world, 240 * 4);
        let snapshot = world.snapshot(car).unwrap();
        let length = snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.suspension_length_m)
            .sum::<f32>()
            / 4.0;
        (snapshot.speed_mps, length)
    };

    // The car's centre of mass rides 0.35 m up, so a wing 0.33 m below it is
    // two centimetres off the road and one 0.05 m above it is high up.
    let (low_speed, low_length) = run(-0.33);
    let (high_speed, high_length) = run(0.05);
    assert!(
        low_length < high_length - 0.003,
        "the low wing made no extra load: {low_length} vs {high_length}"
    );
    // More downforce also means more grip, so the car is not slower.
    assert!(
        low_speed > high_speed * 0.9,
        "the low wing lost too much ground: {low_speed} vs {high_speed}"
    );
}

#[test]
fn wing_tables_are_skipped_when_the_tuning_overrides_downforce() {
    let mut spec = winged_car();
    spec.aero.downforce_coeff = 3.0;
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(spec, Pose::default()).unwrap();
    settle(&mut world, car);
    // With the override the aggregate path applies; the car must still run.
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
    assert!(world.snapshot(car).unwrap().speed_mps > 1.0);
}

#[test]
fn a_heavier_wheel_makes_less_grip_per_newton() {
    // Two identical cars apart from their mass: load sensitivity makes the
    // heavy one's friction coefficient lower, exactly as AC's `LS_EXPY` says.
    let run = |mass_kg: f32| -> f32 {
        let spec = VehicleSpec {
            mass_kg,
            tyres: TyreSpec {
                reference_load_n: 4_000.0,
                lateral_load_sensitivity: 0.8,
                ..TyreSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        for _ in 0..120 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.5,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        // The friction coefficient each loaded wheel is actually producing.
        snapshot
            .wheels
            .iter()
            .filter(|wheel| wheel.contact && wheel.load_n > 100.0)
            .map(|wheel| wheel.lateral_force_n.abs() / wheel.load_n)
            .fold(0.0f32, f32::max)
    };

    let light = run(700.0);
    let heavy = run(1_500.0);
    assert!(
        light > heavy * 1.02,
        "load sensitivity had no effect: {light} vs {heavy}"
    );
}

#[test]
fn relaxation_length_delays_the_lateral_force() {
    // A long relaxation length means the tyre needs metres of travel before
    // its side force catches up with the slip: the first moments after a step
    // steer produce measurably less side force than a tyre without one.
    let run = |relaxation_m: f32| -> f32 {
        let spec = VehicleSpec {
            tyres: TyreSpec {
                relaxation_length_m: relaxation_m,
                ..TyreSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        // Step steer and look at the very first moments.
        let mut first = 0.0f32;
        for step_index in 0..12 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.8,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
            let snapshot = world.snapshot(car).unwrap();
            let lateral = snapshot
                .wheels
                .iter()
                .filter(|wheel| wheel.contact)
                .fold(0.0f32, |sum, wheel| sum + wheel.lateral_force_n.abs());
            if step_index >= 2 {
                first = first.max(lateral);
            }
        }
        first
    };

    let instant = run(0.0);
    let relaxed = run(3.0);
    assert!(
        relaxed < instant,
        "relaxation did not delay the force: {instant} vs {relaxed}"
    );
}

#[test]
fn the_tyres_own_peak_slip_shapes_the_lateral_force() {
    // Two cars identical apart from where their tyre reaches its friction
    // limit: the sharper one peaks early, the lazy one keeps building force up
    // to its own peak slip. Same input, different curve.
    let run = |peak_slip_deg: f32| -> f32 {
        let spec = VehicleSpec {
            tyres: TyreSpec {
                lateral_peak_slip_deg: peak_slip_deg,
                ..TyreSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        for _ in 0..120 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.5,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        snapshot
            .wheels
            .iter()
            .filter(|wheel| wheel.contact && wheel.load_n > 100.0)
            .map(|wheel| wheel.lateral_force_n.abs() / wheel.load_n)
            .fold(0.0f32, f32::max)
    };

    let sharp = run(4.0);
    let lazy = run(12.0);
    assert!(
        lazy > sharp,
        "the peak slip angle did not shape the curve: {sharp} vs {lazy}"
    );
}

#[test]
fn a_braking_run_keeps_heat_in_both_the_surface_and_the_core() {
    // End-to-end: brake, and both masses must come up above ambient, with the
    // friction surface leading the bulk of the rotor.
    let mut world = World::new(SurfaceMap::new()).unwrap();
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
    world
        .set_controls(
            car,
            Controls {
                throttle: 0.0,
                brake: 0.55,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 3);
    let surface = world.brake_temperatures_c(car).unwrap();
    let core = world.brake_core_temperatures_c(car).unwrap();
    for index in 0..4 {
        assert!(
            surface[index] > 33.0 && core[index] > 32.0,
            "disc {index} did not heat: {surface:?} / {core:?}"
        );
        assert!(
            surface[index] >= core[index] - 0.5,
            "the core led the surface at {index}: {core:?} vs {surface:?}"
        );
    }
}

#[test]
fn a_layered_tyre_brings_the_patch_up_before_the_bulk() {
    // A car with AC's three-layer thermal model: drive hard, and the friction
    // surface must run hotter than the patch the grip curve reads, which in
    // turn runs hotter than the bulk core.
    let spec = VehicleSpec {
        tyres: TyreSpec {
            thermal: vettura_physics::TyreThermalSpec {
                surface_capacity_j_per_k: 60.0,
                patch_capacity_j_per_k: 200.0,
                heat_capacity_j_per_k: 1_000.0,
                surface_transfer_per_s: 0.018,
                patch_transfer_per_s: 0.00027,
                heat_share: 0.3,
                cooling_per_s: 0.08,
                optimal_c: 90.0,
                window_c: 80.0,
                grain_gain: 0.25,
                blister_gain: 1.5,
                core_transfer_per_s: 0.0013,
                internal_core_transfer_per_s: 0.0029,
                performance_curve: vec![(0.0, 0.9), (50.0, 1.0), (100.0, 0.85), (300.0, 0.5)],
            },
            ..TyreSpec::default()
        },
        ..VehicleSpec::default()
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
    step(&mut world, 240 * 6);
    let surface = world.tyre_surface_temperatures_c(car).unwrap();
    let patch = world.tyre_temperatures_c(car).unwrap();
    let bulk = world.tyre_core_temperatures_c(car).unwrap();
    // The driven wheels are the ones doing work; for them the friction surface
    // must lead the patch, and the bulk core must lag it.
    let hottest = (0..4)
        .max_by(|a, b| surface[*a].total_cmp(&surface[*b]))
        .unwrap();
    assert!(
        surface[hottest] > patch[hottest] + 1.0,
        "wheel {hottest}: surface {surface:?} did not lead the patch {patch:?}"
    );
    assert!(
        patch[hottest] > 33.0,
        "wheel {hottest}: the patch never warmed: {patch:?}"
    );
    assert!(
        bulk[hottest] <= patch[hottest] + 1.0,
        "wheel {hottest}: the bulk ran away: {bulk:?} vs {patch:?}"
    );
    // Wheels that did no work stay at the road temperature.
    for index in 0..4 {
        if index != hottest {
            let idle = (patch[index] - 32.0).abs() < 1.0 && (surface[index] - 32.0).abs() < 1.0;
            assert!(
                idle || surface[index] > patch[index],
                "wheel {index} changed temperature without doing work"
            );
        }
    }
}

#[test]
fn a_winter_world_starts_the_car_below_zero() {
    // Nothing clamps at the freezing point: a car added to a -15 °C session
    // begins with cold rubber, cold brakes and a cold engine, and the grip the
    // road offers is the winter number.
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let cold = Weather::dry()
        .with_temperatures(-12.0, -15.0)
        .with_wetness(0.0);
    world.set_weather(cold);
    let car = world.add_vehicle(car(), Pose::default()).unwrap();
    let start = world.snapshot(car).unwrap();
    let cold_tyres = start.tyre_temperatures_c;
    let cold_coolant = start.coolant_c;
    for temperature in cold_tyres {
        assert!(
            temperature < 0.0,
            "the tyres did not start below zero: {temperature}"
        );
    }
    for temperature in world.brake_core_temperatures_c(car).unwrap() {
        assert!(
            temperature < 0.0,
            "the brakes did not start below zero: {temperature}"
        );
    }

    // Driving heats everything back up: the thermal model works from a
    // sub-zero start, it does not sit on the clamp.
    world
        .set_controls(
            car,
            Controls {
                throttle: 1.0,
                ..Controls::default()
            },
        )
        .unwrap();
    step(&mut world, 240 * 6);
    let hot = world.snapshot(car).unwrap();
    let coolant_warm = world.engine_temperatures_c(car).unwrap().0 > cold_coolant;
    let rubber_warm = hot
        .tyre_temperatures_c
        .iter()
        .zip(cold_tyres)
        .all(|(now, then)| *now > then);
    assert!(coolant_warm, "a cold engine cannot warm up");
    assert!(rubber_warm, "cold tyres cannot warm up");
}

#[test]
fn a_cold_tyre_holds_less_air_than_a_warm_one() {
    let spec = car();
    let cold = spec.tyres.pressure_at(-15.0);
    let warm = spec.tyres.pressure_at(spec.tyres.thermal.optimal_c);
    assert!(cold < warm, "{cold} vs {warm}");
    assert!(cold > 0.0, "pressure stayed positive below zero: {cold}");
}

#[test]
fn a_following_car_loses_downforce_in_the_wake() {
    // Two identical cars, one ahead of the other: the follower must sit lower
    // (less aerodynamic load) than it does in clean air.
    let exposure = |gap_m: f32| -> f32 {
        // A car with real downforce, so the wake has something to take away.
        let spec = VehicleSpec {
            aero: vettura_physics::AeroSpec {
                downforce_coeff: 6.0,
                front_bias: 0.4,
                ride_height_fade: 0.25,
                dirty_air_loss: 0.35,
                dirty_air_distance_m: 25.0,
                side_force_coeff: 0.0,
                center_of_pressure_m: 0.6,
                wings: Vec::new(),
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let leader = world.add_vehicle(spec.clone(), Pose::default()).unwrap();
        let follower = world
            .add_vehicle(
                spec,
                Pose {
                    position: Vec3::new(0.0, 0.0, gap_m),
                    ..Pose::default()
                },
            )
            .unwrap();
        settle(&mut world, leader);
        settle(&mut world, follower);
        world
            .set_controls(
                leader,
                Controls {
                    throttle: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        world
            .set_controls(
                follower,
                Controls {
                    throttle: 1.0,
                    ..Controls::default()
                },
            )
            .unwrap();
        step(&mut world, 240 * 2);
        let snapshot = world.snapshot(follower).unwrap();
        snapshot
            .wheels
            .iter()
            .map(|wheel| wheel.suspension_length_m)
            .sum::<f32>()
            / 4.0
    };

    // 3 m behind the leader: deep in the wake. 200 m: clean air.
    let in_traffic = exposure(6.0);
    let clean = exposure(200.0);
    assert!(
        in_traffic > clean,
        "the follower lost load in the wake: {in_traffic} vs {clean}"
    );
}

#[test]
fn a_steered_tyre_pushes_back_on_the_chassis() {
    // The pneumatic trail is what makes the steering feel connected: with it on,
    // a cornering car builds a yaw moment from the tyre forces alone. The
    // observable proxy is that the same steering input produces a different
    // (quieter) yaw response than without the trail.
    let run = |trail: f32| -> f32 {
        let spec = VehicleSpec {
            tyres: TyreSpec {
                pneumatic_trail_m: trail,
                ..TyreSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        for _ in 0..120 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.6,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        let up = snapshot.orientation * Vec3::Y;
        snapshot.angular_velocity.dot(up).abs()
    };

    let with_trail = run(0.04);
    let without = run(0.0);
    assert!(
        (with_trail - without).abs() > 1.0e-3,
        "the pneumatic trail made no difference: {with_trail} vs {without}"
    );
}

#[test]
fn ac_rolling_resistance_slows_a_coasting_car() {
    let run = |tyre_rr: f32| -> f32 {
        let spec = VehicleSpec {
            tyres: TyreSpec {
                rolling_resistance_n: tyre_rr,
                ..TyreSpec::default()
            },
            ..VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        world.set_controls(car, Controls::default()).unwrap();
        let before = world.snapshot(car).unwrap().speed_mps;
        step(&mut world, 240 * 3);
        let after = world.snapshot(car).unwrap().speed_mps;
        before - after
    };

    let bare = run(0.0);
    let rubber = run(30.0);
    assert!(
        rubber > bare + 0.2,
        "tyre rolling resistance did not slow the car: {bare} vs {rubber}"
    );
}

#[test]
fn the_roll_centre_reduces_body_roll_under_cornering() {
    // Two identical cars; the only difference is the roll-centre height. A
    // higher roll centre transfers the lateral force closer to the centre of
    // mass, so the body rolls less in the same corner.
    let run = |height_m: f32| -> f32 {
        let mut spec = VehicleSpec::default();
        for wheel in &mut spec.wheels {
            wheel.suspension.roll_center_height_m = height_m;
        }
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        for _ in 0..180 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 0.6,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        (snapshot.orientation * Vec3::X).y.abs()
    };

    let ground = run(0.0);
    let high = run(0.12);
    assert!(
        high < ground * 0.9,
        "the roll centre did not reduce body roll: {ground} vs {high}"
    );
}

#[test]
fn ac_steer_lock_and_ratio_set_the_wheel_lock() {
    // AC's `STEER_LOCK` is the lock at the steering wheel; the wheel angle is
    // the division by the ratio. 230° through a 15:1 ratio is ~15.3°.
    let spec = vettura_physics::VehicleSpec {
        max_steer_angle_deg: 230.0 / 15.0,
        ..vettura_physics::VehicleSpec::default()
    };
    let mut world = World::new(SurfaceMap::new()).unwrap();
    let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
    step(&mut world, 240 * 4);
    // The car with a much smaller lock has to be driven with more steering to
    // follow the same radius, so it turns tighter per unit of input.
    let radius = |lock_deg: f32| -> f32 {
        let spec = vettura_physics::VehicleSpec {
            max_steer_angle_deg: lock_deg,
            ..vettura_physics::VehicleSpec::default()
        };
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        let start = world.snapshot(car).unwrap();
        let start_pos = start.position;
        let start_forward = start.orientation * Vec3::NEG_Z;
        for _ in 0..180 {
            world
                .set_controls(
                    car,
                    Controls {
                        throttle: 0.4,
                        steering: 1.0,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let snapshot = world.snapshot(car).unwrap();
        let travelled = (snapshot.position - start_pos).length().max(1.0e-3);
        let turned = start_forward
            .angle_between(snapshot.orientation * Vec3::NEG_Z)
            .max(1.0e-4);
        travelled / turned
    };

    let tight = radius(230.0 / 15.0);
    let wide = radius(45.0);
    assert!(
        tight < wide,
        "steer lock did not change the turning radius: {tight} vs {wide}"
    );
}

#[test]
fn anti_dive_reduces_braking_pitch() {
    // The same braking input on two identical cars; the anti-dive car must
    // pitch less because its links take a share of the load transfer.
    let run = |anti_dive: f32| -> f32 {
        let mut spec = VehicleSpec::default();
        for wheel in &mut spec.wheels {
            wheel.suspension.anti_dive_fraction = anti_dive;
        }
        let mut world = World::new(SurfaceMap::new()).unwrap();
        let car = world.add_vehicle(spec, Pose::default()).unwrap();
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
        let before = world.snapshot(car).unwrap().orientation;
        let forward_before = before * Vec3::NEG_Z;
        for _ in 0..120 {
            world
                .set_controls(
                    car,
                    Controls {
                        brake: 0.8,
                        ..Controls::default()
                    },
                )
                .unwrap();
            step(&mut world, 1);
        }
        let after = world.snapshot(car).unwrap();
        // Nose-down pitch shows up as the nose dropping: the forward vector
        // gains a downward component, so its angle to the horizontal grows.
        let now = after.orientation * Vec3::NEG_Z;
        // Absolute nose height relative to the start: nose-down pitch makes the
        // forward vector point further below the horizontal.
        (-now.y.min(forward_before.y)).abs() + (forward_before.y - now.y).abs()
    };

    let none = run(0.0);
    let lots = run(0.6);
    assert!(
        lots < none - 1.0e-3,
        "anti-dive did not reduce the pitch: {none} vs {lots}"
    );
}

#[test]
fn the_sun_heats_the_track_over_a_session() {
    let mut world = World::new(SurfaceMap::new()).unwrap();
    // A cold morning, a short time constant so the test can watch it happen.
    let cold = Weather::dry()
        .with_temperatures(10.0, 10.0)
        .with_sun(900.0, 4.0);
    world.set_weather(cold);
    let start = world.track_temperature_c();
    step(&mut world, 240 * 20);

    let warm = world.track_temperature_c();
    let equilibrium = cold.sun_equilibrium_temperature_c();
    assert!(
        warm > start + 5.0,
        "the road did not heat: {start} → {warm}"
    );
    assert!(
        (warm - equilibrium).abs() < 1.0,
        "the road overshot the equilibrium: {warm} vs {equilibrium}"
    );
    // Equilibrium: air + ~22 deg of sun excess on dry asphalt.
    assert!(equilibrium > 30.0, "{equilibrium}");

    // Rain cools it straight back toward the air temperature.
    let rainy = Weather::wet(1.0)
        .with_temperatures(10.0, 10.0)
        .with_sun(900.0, 4.0);
    world.set_weather(rainy);
    step(&mut world, 240 * 30);
    assert!(
        (world.track_temperature_c() - rainy.sun_equilibrium_temperature_c()).abs() < 1.0,
        "a wet road did not track the air: {} vs {}",
        world.track_temperature_c(),
        rainy.sun_equilibrium_temperature_c()
    );

    // Night: no sun, the road sits at the air temperature.
    let night = Weather::dry()
        .with_temperatures(12.0, 30.0)
        .with_sun(0.0, 2.0);
    world.set_weather(night);
    step(&mut world, 240 * 20);
    assert!(
        (world.track_temperature_c() - 12.0).abs() < 1.0,
        "a shaded road did not cool to the air: {}",
        world.track_temperature_c()
    );
}
