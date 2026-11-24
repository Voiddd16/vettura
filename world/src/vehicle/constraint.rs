//! AC `VehicleSpec` to Jolt `wheeled vehicle` constraint and body geometry.
//!
//! This layer only translates sizes: springs/dampers/travel, wheel radius and
//! mount points, brake torque split, gearbox and differentials, the chassis
//! inertia derived from the AC mass box, and spec sanity checks.

use glam::{Quat, Vec3};
use oxijolt::{
    ObjectLayer, SpringSettings, VehicleAntiRollBar, VehicleCollisionTester,
    VehicleDifferentialSettings, VehicleEngineSettings, VehicleTransmissionSettings, WheelSettings,
    WheeledVehicleSettings,
};
use vettura_physics::{DrivetrainLayout, VehicleSpec, Wheel, WingSpec, series_spring_rate};

use crate::frame::p2j;
use crate::{WorldError, WorldResult};

use super::{VehicleHandle, VehicleTuning, steer_lock_deg};

/// Normalize an orientation, falling back to identity for degenerate input.
pub(crate) fn normalized_orientation(q: Quat) -> Quat {
    if q.is_finite() && q.length_squared() > 1.0e-8 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

pub(crate) fn validate_spec(spec: &VehicleSpec) -> WorldResult<()> {
    if !(spec.mass_kg.is_finite() && spec.mass_kg >= 1.0) {
        return Err(WorldError::InvalidVehicle(
            "mass_kg must be finite and at least 1 kg".to_owned(),
        ));
    }
    if !(spec.cg_height_m.is_finite() && spec.cg_height_m > 0.05) {
        return Err(WorldError::InvalidVehicle(
            "cg_height_m must be finite and above 0.05 m".to_owned(),
        ));
    }
    if !(spec.wheelbase_m.is_finite() && spec.wheelbase_m > 0.1) {
        return Err(WorldError::InvalidVehicle(
            "wheelbase_m must be finite and above 0.1 m".to_owned(),
        ));
    }
    for wheel in &spec.wheels {
        if !wheel.position.is_finite() {
            return Err(WorldError::InvalidVehicle(
                "wheel positions must be finite".to_owned(),
            ));
        }
        if !(wheel.radius_m.is_finite() && wheel.radius_m > 0.02) {
            return Err(WorldError::InvalidVehicle(
                "wheel radius must be finite and above 0.02 m".to_owned(),
            ));
        }
        if !(wheel.width_m.is_finite() && wheel.width_m > 0.0) {
            return Err(WorldError::InvalidVehicle(
                "wheel width must be finite and positive".to_owned(),
            ));
        }
        let suspension = &wheel.suspension;
        if !(suspension.spring_rate_n_per_m.is_finite()
            && suspension.spring_rate_n_per_m > 0.0
            && suspension.damper_bump_ns_per_m.is_finite()
            && suspension.damper_bump_ns_per_m >= 0.0
            && suspension.damper_rebound_ns_per_m.is_finite()
            && suspension.damper_rebound_ns_per_m >= 0.0
            && suspension.travel_m.is_finite()
            && suspension.travel_m > 0.0
            && suspension.anti_roll_n_per_m.is_finite()
            && suspension.anti_roll_n_per_m >= 0.0)
        {
            return Err(WorldError::InvalidVehicle(
                "suspension rates, travel and anti-roll must be finite and non-negative".to_owned(),
            ));
        }
    }
    if !(spec.tyres.peak_friction.is_finite() && spec.tyres.peak_friction >= 0.0) {
        return Err(WorldError::InvalidVehicle(
            "tyre peak friction must be finite and non-negative".to_owned(),
        ));
    }
    if !(spec.engine.idle_rpm.is_finite()
        && spec.engine.idle_rpm > 0.0
        && spec.engine.redline_rpm.is_finite()
        && spec.engine.redline_rpm > spec.engine.idle_rpm
        && spec.engine.peak_torque_nm.is_finite()
        && spec.engine.peak_torque_nm > 0.0
        && spec.engine.final_drive.is_finite()
        && spec.engine.final_drive > 0.0)
    {
        return Err(WorldError::InvalidVehicle(
            "engine rpm, peak torque and final drive must be finite and positive".to_owned(),
        ));
    }
    if !(spec.drivetrain.efficiency.is_finite() && spec.drivetrain.efficiency > 0.0) {
        return Err(WorldError::InvalidVehicle(
            "drivetrain efficiency must be finite and positive".to_owned(),
        ));
    }
    if !(spec.frontal_area_m2.is_finite()
        && spec.frontal_area_m2 >= 0.0
        && spec.drag_coefficient.is_finite()
        && spec.drag_coefficient >= 0.0)
    {
        return Err(WorldError::InvalidVehicle(
            "aero area and drag coefficient must be finite and non-negative".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn chassis_half_extents(spec: &VehicleSpec) -> (f32, f32, f32) {
    spec.chassis_half_extents()
}

/// Vertical rate of one tyre at its cold inflation, N/m (`0` = not authored).
///
/// The geometry and the wheel settings are both built once, so the cold
/// pressure is what they use; the warm-rate variation is a runtime effect the
/// immutable Jolt settings cannot follow. AC quotes the rate per axle; the
/// shared tyre model keeps the axle average, so every wheel sees the same
/// number.
fn wheel_vertical_rate(spec: &VehicleSpec) -> f32 {
    spec.tyres
        .vertical_rate_n_per_m(spec.tyres.pressure_at(spec.brakes.ambient_c))
        .max(0.0)
}

/// Suspension geometry derived from the spec and tuning.
///
/// Both [`vehicle_settings`] and the ride-height fade read the same numbers so
/// the fade always agrees with the suspension Jolt actually builds.
#[derive(Clone, Copy, Debug)]
/// Static suspension geometry per wheel.
///
/// Plain data: the runtime reads it every step to size the spring force, so the
/// fields are crate-visible rather than hidden behind accessors.
pub(crate) struct WheelGeometry {
    /// Suspension length with no load, metres.
    pub(crate) rest_length: f32,
    /// Longest suspension length available, metres.
    pub(crate) max_length: f32,
    /// Shortest suspension length available, metres.
    pub(crate) min_length: f32,
    /// Wheel mount position relative to the centre of mass, physics frame.
    pub(crate) mount: Vec3,
    /// Wheel radius, metres.
    pub(crate) radius: f32,
}

pub(crate) fn wheel_geometries(spec: &VehicleSpec) -> [WheelGeometry; 4] {
    let cg = spec.cg_height_m;
    let spring_force_scale = spec.mass_kg * 9.81 * 0.25;
    std::array::from_fn(|index| {
        let wheel_spec = &spec.wheels[index];
        let radius = wheel_spec.radius_m.max(0.02);
        let travel = wheel_spec.suspension.travel_m.clamp(0.02, 0.35);
        // The wheel is connected to the ground through the suspension spring
        // *and* the tyre's own vertical stiffness, so the chassis sees the
        // series combination. Without an authored tyre rate this is just the
        // suspension spring, which is what Jolt models on its own.
        let spring = series_spring_rate(
            wheel_spec.suspension.spring_rate_n_per_m.max(1_000.0),
            wheel_vertical_rate(spec),
        );
        // 线性弹簧在静载下的压缩量。注意这里用整个角载荷（`spring_force_scale`）
        // 而不是实际值——heave/第三弹簧会分担一部分，所以真实压缩更小，车会
        // 比几何假设多高几厘米，这正好留给 bump 使用。
        let static_compression = (spring_force_scale / spring).clamp(0.002, 0.15);
        // 悬挂长度在接触点处等于 `rest_length`（见 `mount` 的推导），所以
        // `rest_length - min_length` 就是真实的 bump 可用行程。
        //
        // AC 的 `PACKER_RANGE` 与 `SPRING_RATE` 经常不自洽：本车前轴
        // `PACKER_RANGE=0.04`，而静载压缩就要 0.037，等于静置就快顶到
        // bump stop，1 g 就顶死并弹飞。`BUMPSTOP_UP/DN` 才是真实的上下
        // 止点行程，所以 bump 行程取两者中较大的一个。
        let bump_room = wheel_spec
            .suspension
            .bump_stop_travel_m
            .max(travel * 0.5)
            .clamp(0.01, 0.20);
        let min_length = 0.005f32;
        let rest_length = bump_room + min_length;
        // Jolt 的弹簧只能推不能拉，所以拉伸行程最多就是静载压缩量；再往上
        // 由 `rebound_stop` 提供下拉。
        let max_length = rest_length + static_compression;
        WheelGeometry {
            rest_length,
            max_length,
            min_length,
            mount: Vec3::new(
                wheel_spec.position.x,
                radius + rest_length - cg,
                wheel_spec.position.z,
            ),
            radius,
        }
    })
}

/// AC wing mount position, in the physics frame.
///
/// AC authors positions as `(+X left, +Y up, +Z forward)`; the physics frame
/// keeps X and Y and flips Z, because physics forward is `-Z`.
pub(crate) fn wing_local_position(wing: &WingSpec) -> Vec3 {
    Vec3::new(wing.position.x, wing.position.y, -wing.position.z)
}

/// Maximum crank torque the Jolt engine is built with.
///
/// Jolt takes one maximum torque and a normalized curve. The engine is built at
/// the fully boosted torque, then the runtime scales the throttle by how much
/// boost is actually available, so at zero boost the car makes its naturally
/// aspirated `power.lut` torque. The boost torque, the engine tuning and the
/// driveline efficiency all live in this number.
fn engine_max_torque_nm(spec: &VehicleSpec, tuning: &VehicleTuning) -> f32 {
    let efficiency = spec.drivetrain.efficiency.clamp(0.3, 1.0);
    let turbo_scale = spec.engine.turbo.max_torque_multiplier().max(1.0);
    (spec.engine.peak_torque_nm * tuning.engine_scale * efficiency * turbo_scale)
        .clamp(1.0, 5_000.0)
}

pub(crate) fn vehicle_settings(
    spec: &VehicleSpec,
    tuning: &VehicleTuning,
) -> WorldResult<WheeledVehicleSettings> {
    let brake_scale = tuning.brake_scale.clamp(0.0, 4.0);
    let geometries = wheel_geometries(spec);

    // Same tyre shape for all four corners: the per-wheel differences (load,
    // temperature, wear, camber, water film) live in the grip callback chain.
    let curves = spec.tyres.slip_curves();
    let mut wheels = Vec::with_capacity(4);
    for wheel in Wheel::ALL {
        let wheel_spec = &spec.wheels[wheel.index()];
        let geometry = geometries[wheel.index()];
        let radius = geometry.radius;
        let width = wheel_spec.width_m.max(0.04);
        let spring = wheel_spec.suspension.spring_rate_n_per_m.max(1_000.0);
        let damper = (0.5
            * (wheel_spec.suspension.damper_bump_ns_per_m.max(0.0)
                + wheel_spec.suspension.damper_rebound_ns_per_m.max(0.0)))
        .max(100.0);
        let max_length = geometry.max_length;
        let min_length = geometry.min_length;
        let position = p2j(geometry.mount);
        // Brake torque now comes from `brakes.ini`: a peak torque shared by
        // the front/rear bias, split across the pair on each axle.
        let peak = spec.brakes.max_torque_nm.max(0.0) * brake_scale;
        let front_axle_torque = peak * spec.brakes.front_share.clamp(0.0, 1.0);
        let brake_torque = if wheel.is_front() {
            front_axle_torque
        } else {
            peak - front_axle_torque
        };

        let settings = WheelSettings::new(position)
            .radius(radius)
            .width(width)
            .inertia((0.5 * 12.0 * radius * radius).max(0.05))
            .angular_damping(0.05)
            // The car's own lock wins unless the tuning overrides it, so an AC
            // car steers by the steering ratio it was authored with.
            .max_steer_angle(if wheel_spec.steerable {
                steer_lock_deg(spec, tuning).to_radians()
            } else {
                0.0
            })
            .max_brake_torque(brake_torque.max(1.0))
            // The handbrake only ever acts on the rear; AC can author its own
            // torque, and without it we keep a share of the brake torque.
            .max_hand_brake_torque(if wheel.is_front() {
                0.0
            } else if spec.gearbox.handbrake_torque_nm > 0.0 {
                spec.gearbox.handbrake_torque_nm / 2.0
            } else {
                brake_torque * 0.7
            })
            .suspension_min_length(min_length)
            .suspension_max_length(max_length)
            .suspension_preload_length(0.0)
            .suspension_spring(SpringSettings::StiffnessAndDamping {
                stiffness: spring,
                damping: damper,
            })
            // The callback scales these curves by tyre peak × surface × weather ×
            // load × camber × …, so they only carry the tyre's own shape: where it
            // peaks and how quickly it gives the grip back. That comes from the
            // car's own data as a Magic Formula fit (see `vettura_physics`'s
            // `tyre_model`); both tables peak at exactly 1.0.
            .longitudinal_friction(curves.0.clone())
            .lateral_friction(curves.1.clone());

        wheels.push(settings);
    }

    let final_drive = spec.engine.final_drive.max(0.5);
    // AC authors the reverse ratio in `GEARS GEAR_R`; the default keeps the
    // long-standing value so hand-authored cars behave as before.
    let reverse_ratio = if spec.gearbox.reverse_ratio > 0.0 {
        spec.gearbox.reverse_ratio.clamp(0.5, 8.0)
    } else {
        3.2
    };
    // AC's differential is a clutch-type unit: `POWER`/`COAST` are the locking
    // rates under drive/overrun, `PRELOAD` is the base locking torque that acts
    // even with no speed difference. Jolt only has one speed-ratio based
    // limited-slip model (1 = fully locked, larger = more open) and no
    // drive/overrun state, so the three numbers collapse into one equivalent
    // ratio. `PRELOAD` is folded in as extra base locking — it used to be read
    // into the spec and then never used. A true ramp/preload differential needs
    // a custom unit; Jolt's model cannot express it.
    let diff = &spec.drivetrain.diff;
    let preload_lock = if diff.preload_nm.is_finite() {
        (diff.preload_nm / (2.0 * spec.engine.peak_torque_nm.max(1.0))).clamp(0.0, 0.5)
    } else {
        0.0
    };
    let lock = (0.5 * (diff.power_lock.clamp(0.01, 1.0) + diff.coast_lock.clamp(0.01, 1.0))
        + preload_lock)
        .clamp(0.01, 1.0);
    // Jolt needs a ratio strictly above 1: fully locked becomes just over 1, an
    // open diff becomes a large number. This single ratio is what the per-axle
    // differentials use; for RWD/FWD it is the *only* thing that shapes the
    // left/right torque split (the controller's global ratio only matters when
    // several axles share the engine, i.e. AWD).
    let lock_ratio = (1.0 / lock).clamp(1.0001, 100.0);

    let differentials = match spec.drivetrain.layout {
        DrivetrainLayout::Rwd => vec![
            VehicleDifferentialSettings::new(Some(2), Some(3))
                .differential_ratio(final_drive)
                .engine_torque_ratio(1.0)
                .limited_slip_ratio(lock_ratio),
        ],
        DrivetrainLayout::Fwd => vec![
            VehicleDifferentialSettings::new(Some(0), Some(1))
                .differential_ratio(final_drive)
                .engine_torque_ratio(1.0)
                .limited_slip_ratio(lock_ratio),
        ],
        DrivetrainLayout::Awd => vec![
            VehicleDifferentialSettings::new(Some(0), Some(1))
                .differential_ratio(final_drive)
                .engine_torque_ratio(0.42)
                .limited_slip_ratio(lock_ratio),
            VehicleDifferentialSettings::new(Some(2), Some(3))
                .differential_ratio(final_drive)
                .engine_torque_ratio(0.58)
                .limited_slip_ratio(lock_ratio),
        ],
    };

    let max_rpm = spec.engine.redline_rpm.clamp(2_000.0, 30_000.0);
    let min_rpm = spec.engine.idle_rpm.clamp(300.0, max_rpm * 0.5);
    let shift_up_rpm = (max_rpm * 0.94).max(min_rpm + 500.0).min(max_rpm - 1.0);
    let shift_down_rpm = (max_rpm * 0.42)
        .max(min_rpm * 0.8)
        .min(shift_up_rpm - 1.0)
        .max(1.0);

    let engine = VehicleEngineSettings::default()
        .max_torque(engine_max_torque_nm(spec, tuning))
        .min_rpm(min_rpm)
        .max_rpm(max_rpm)
        .normalized_torque(spec.engine.normalized_torque_curve(max_rpm))
        .inertia(spec.gearbox.inertia.clamp(0.005, 2.0))
        .angular_damping(0.1);

    let mut gear_ratios: Vec<f32> = spec
        .engine
        .gear_ratios
        .iter()
        .copied()
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .collect();
    if gear_ratios.is_empty() {
        gear_ratios = vec![3.2, 2.1, 1.6, 1.3, 1.1, 0.95];
    }
    let transmission = VehicleTransmissionSettings::default()
        .gear_ratios(gear_ratios)
        .reverse_gear_ratios(vec![-reverse_ratio])
        // `drivetrain.ini` reports shift and clutch times in milliseconds;
        // the clutch strength comes from the clutch torque capacity.
        .switch_time((spec.gearbox.switch_time * 0.001).clamp(0.01, 0.5))
        .clutch_release_time((spec.gearbox.clutch_release_time * 0.001).clamp(0.01, 1.0))
        .switch_latency(0.1)
        .shift_up_rpm(shift_up_rpm)
        .shift_down_rpm(shift_down_rpm)
        .clutch_strength((spec.gearbox.clutch_max_torque_nm.max(1.0) / 50.0).clamp(1.0, 200.0));

    let mut anti_roll_bars = Vec::new();
    let front_arb = spec.wheels[Wheel::FrontLeft.index()]
        .suspension
        .anti_roll_n_per_m
        .max(0.0);
    let rear_arb = spec.wheels[Wheel::RearLeft.index()]
        .suspension
        .anti_roll_n_per_m
        .max(0.0);
    if front_arb > 0.0 {
        anti_roll_bars.push(VehicleAntiRollBar::new(0, 1).stiffness(front_arb));
    }
    if rear_arb > 0.0 {
        anti_roll_bars.push(VehicleAntiRollBar::new(2, 3).stiffness(rear_arb));
    }

    let average_radius =
        spec.wheels.iter().map(|wheel| wheel.radius_m).sum::<f32>() / spec.wheels.len() as f32;
    let tester_radius = (average_radius * 0.5).clamp(0.03, 0.30);
    // Query as a moving body so static ground/walls (NON_MOVING) are hit;
    // NON_MOVING-vs-NON_MOVING is filtered out by the default layer table.
    let collision_tester = VehicleCollisionTester::cast_sphere(ObjectLayer::MOVING, tester_radius);

    Ok(
        WheeledVehicleSettings::new(wheels, differentials, collision_tester)
            .engine(engine)
            .transmission(transmission)
            .anti_roll_bars(anti_roll_bars)
            .differential_limited_slip_ratio(lock_ratio)
            .max_pitch_roll_angle(70.0f32.to_radians()),
    )
}

const TAG_VEHICLE: u64 = 1 << 32;

pub(crate) fn tag_vehicle(handle: VehicleHandle) -> u64 {
    TAG_VEHICLE | (handle.0 & 0xffff_ffff)
}

#[cfg(test)]
mod geometry_probe {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore]
    fn print_real_car_geometry() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari");
        let spec = vettura_assets::CarFolder::open(&root)
            .unwrap()
            .vehicle_spec()
            .unwrap();
        println!("mass={} cg={}", spec.mass_kg, spec.cg_height_m);
        for (index, geometry) in wheel_geometries(&spec).iter().enumerate() {
            println!(
                "wheel {index}: radius={:.3} rest={:.4} min={:.4} max={:.4} mount={:?} travel={:.3} spring={:.0}",
                geometry.radius,
                geometry.rest_length,
                geometry.min_length,
                geometry.max_length,
                geometry.mount,
                spec.wheels[index].suspension.travel_m,
                spec.wheels[index].suspension.spring_rate_n_per_m,
            );
        }
    }
}
