//! Convert Assetto Corsa car data into the backend-neutral physics spec.
//!
//! Not every AC parameter has a home in [`VehicleSpec`] yet. The mapping uses
//! conservative defaults for anything missing and logs the fallback at debug
//! level, so a half-supported mod still drives instead of failing to load.

use std::path::Path;

use glam::Vec3;
use tracing::debug;
use vettura_acdata::{Ini, IniSection};
use vettura_physics::{
    ActiveSuspensionMode, ActiveSuspensionSpec, DiffSpec, DrivetrainLayout, DrivetrainSpec,
    EngineSpec, GearboxSpec, HeaveSpringSpec, SuspensionSpec, TorquePoint, TurboSpec, TyreSpec,
    TyreThermalSpec, TyreWearSpec, VehicleSpec, Wheel, WheelSpec, WingSpec,
};

use crate::car::CarData;
use crate::error::{AssetError, Result};

pub(crate) fn spec_from_car(root: &Path, data: &CarData) -> Result<VehicleSpec> {
    let car = optional_ini(data, "car.ini");
    let suspensions = optional_ini(data, "suspensions.ini");
    let engine = optional_ini(data, "engine.ini");
    let drivetrain = optional_ini(data, "drivetrain.ini");
    let tyres = optional_ini(data, "tyres.ini");
    let aero = optional_ini(data, "aero.ini");
    let brakes = optional_ini(data, "brakes.ini");

    let mut spec = VehicleSpec::default();
    spec.name = car
        .get("INFO", "SCREEN_NAME")
        .map(str::to_owned)
        .or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| spec.name.clone());

    spec.mass_kg = car.get_f32("BASIC", "TOTALMASS").unwrap_or(spec.mass_kg);

    spec.wheelbase_m = suspensions
        .get_f32("BASIC", "WHEELBASE")
        .or_else(|| car.get_f32("BASIC", "WHEELBASE"))
        .unwrap_or(spec.wheelbase_m);

    let front = AxleData::from_ini(&suspensions, &tyres, "FRONT");
    let rear = AxleData::from_ini(&suspensions, &tyres, "REAR");

    let front_radius = front
        .radius
        .unwrap_or(spec.wheels[Wheel::FrontLeft.index()].radius_m);
    let rear_radius = rear
        .radius
        .unwrap_or(spec.wheels[Wheel::RearLeft.index()].radius_m);
    let front_track = front.track.unwrap_or(1.5);
    let rear_track = rear.track.unwrap_or(1.5);

    spec.cg_height_m = front
        .base_y
        .map(|base_y| front_radius + base_y)
        .unwrap_or(spec.cg_height_m);

    let half_wb = spec.wheelbase_m * 0.5;
    for wheel in Wheel::ALL {
        let axle = if wheel.is_front() { &front } else { &rear };
        let track = if wheel.is_front() {
            front_track
        } else {
            rear_track
        };
        let radius = if wheel.is_front() {
            front_radius
        } else {
            rear_radius
        };
        let x = if wheel.is_left() {
            -track * 0.5
        } else {
            track * 0.5
        };
        let z = if wheel.is_front() { -half_wb } else { half_wb };
        let driven = match drivetrain_layout(&drivetrain) {
            DrivetrainLayout::Rwd => !wheel.is_front(),
            DrivetrainLayout::Fwd => wheel.is_front(),
            DrivetrainLayout::Awd => true,
        };

        spec.wheels[wheel.index()] = WheelSpec {
            position: glam_vec3(x, radius, z),
            radius_m: radius,
            width_m: axle.width.unwrap_or(0.25),
            steerable: wheel.is_front(),
            driven,
            suspension: SuspensionSpec {
                spring_rate_n_per_m: axle.spring_rate.unwrap_or(80_000.0),
                damper_bump_ns_per_m: axle.damper_bump.unwrap_or(4_000.0),
                damper_rebound_ns_per_m: axle.damper_rebound.unwrap_or(6_000.0),
                travel_m: axle.travel.unwrap_or(0.08),
                anti_roll_n_per_m: axle.anti_roll.unwrap_or(0.0) * 0.5,
                // `PACKER_RANGE` is the travel before the packers; the last
                // quarter of it is where the bump stop has to take over.
                progressive_rate_n_per_m2: axle.progressive_rate.unwrap_or(0.0).max(0.0),
                bump_stop_rate_n_per_m: axle.bump_stop_rate.unwrap_or(0.0).max(0.0),
                bump_stop_travel_m: (axle.travel.unwrap_or(0.08) * 0.75).max(0.005),
                rebound_stop_rate_n_per_m: 0.0,
                rebound_stop_travel_m: (axle.travel.unwrap_or(0.08) * 0.75).max(0.005),
                // AC authors static camber per axle; the gain and thrust stay
                // at the model defaults because AC does not publish them.
                static_camber_deg: axle.static_camber.unwrap_or(0.0).clamp(-15.0, 15.0),
                // A mod that publishes the roll centre uses it; the default
                // stays at the typical double-wishbone value. The clamp keeps
                // a bad hard-point derivation from placing the centre above
                // the centre of mass or below the contact patch.
                roll_center_height_m: axle.roll_center.unwrap_or(0.05).clamp(-0.1, 0.3),
                // Anti-dive / anti-squat are the links' geometry. AC does not
                // publish them, so a mod can override them here; the defaults
                // stay at typical road-car values.
                anti_dive_fraction: axle.anti_dive.unwrap_or(0.15).clamp(0.0, 1.0),
                anti_squat_fraction: axle.anti_squat.unwrap_or(0.10).clamp(0.0, 1.0),
                // Steering-axis geometry; AC does not publish it, so the
                // road-car defaults stay unless a mod authors it.
                caster_angle_deg: axle.caster_deg.unwrap_or(4.0).clamp(-15.0, 20.0),
                kingpin_angle_deg: axle.kingpin_deg.unwrap_or(8.0).clamp(-15.0, 20.0),
                fast_bump_damper_ns_per_m: axle.fast_bump.unwrap_or(0.0).clamp(0.0, 500_000.0),
                fast_bump_threshold_mps: axle.fast_bump_threshold.unwrap_or(0.0).clamp(0.0, 5.0),
                fast_rebound_damper_ns_per_m: axle
                    .fast_rebound
                    .unwrap_or(0.0)
                    .clamp(0.0, 500_000.0),
                fast_rebound_threshold_mps: axle
                    .fast_rebound_threshold
                    .unwrap_or(0.0)
                    .clamp(0.0, 5.0),
                camber_gain_rad_per_m: SuspensionSpec::default().camber_gain_rad_per_m,
                camber_thrust_coeff: SuspensionSpec::default().camber_thrust_coeff,
            },
        };
    }

    // A heave spring is a third element shared by both wheels of an axle. AC
    // gives it its own spring and progressive rate in `[HEAVE_FRONT]` /
    // `[HEAVE_REAR]`; the corner springs above do not include it.
    spec.heave_springs = [
        heave_spring(&suspensions, "HEAVE_FRONT"),
        heave_spring(&suspensions, "HEAVE_REAR"),
    ];
    // AC's inertia tensor, validated against the car's own mass and footprint
    // (mods are free to ship nonsense, and many do).
    spec.inertia = inertia_from_car(&car, &spec);
    spec.active_suspension = active_suspension(&suspensions, spec.mass_kg);
    // Steering: `STEER_LOCK` is the lock at the *steering wheel* in degrees and
    // `STEER_RATIO` is the ratio between them, so the wheel angle is the
    // division of the two. Without either the spec keeps its default. The
    // 3..60° clamp spans from a tight race rack to a road-car lock and keeps a
    // bad ratio from producing a zero or absurd wheel angle.
    if let (Some(lock_deg), Some(ratio)) = (
        car.get_f32("CONTROLS", "STEER_LOCK"),
        car.get_f32("CONTROLS", "STEER_RATIO"),
    ) && lock_deg.is_finite()
        && lock_deg > 0.0
        && ratio.is_finite()
        && ratio > 0.0
    {
        spec.max_steer_angle_deg = (lock_deg / ratio).clamp(3.0, 60.0);
    }

    let torque_curve = read_torque_curve(data);
    let curve_peak = torque_curve
        .iter()
        .map(|point| point.torque_nm)
        .fold(0.0f32, f32::max);
    spec.engine = EngineSpec {
        idle_rpm: engine
            .get_f32("ENGINE_DATA", "MINIMUM")
            .unwrap_or(spec.engine.idle_rpm),
        redline_rpm: engine
            .get_f32("ENGINE_DATA", "LIMITER")
            .filter(|value| *value > 0.0)
            .unwrap_or(spec.engine.redline_rpm),
        peak_torque_nm: if curve_peak > 0.0 {
            curve_peak
        } else {
            spec.engine.peak_torque_nm
        },
        torque_curve,
        turbo: turbo_from_engine(&engine),
        coast_torque_nm: engine
            .get_f32("COAST_REF", "TORQUE")
            .unwrap_or(0.0)
            .max(0.0),
        coast_reference_rpm: engine
            .get_f32("COAST_REF", "RPM")
            .unwrap_or(spec.engine.coast_reference_rpm)
            .clamp(500.0, 30_000.0),
        coast_non_linearity: engine
            .get_f32("COAST_REF", "NON_LINEARITY")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        limiter_hz: engine
            .get_f32("ENGINE_DATA", "LIMITER_HZ")
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(spec.engine.limiter_hz),
        altitude_sensitivity: engine
            .get_f32("ENGINE_DATA", "ALTITUDE_SENSITIVITY")
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(spec.engine.altitude_sensitivity),
        gear_ratios: read_gear_ratios(&drivetrain)
            .unwrap_or_else(|| spec.engine.gear_ratios.clone()),
        final_drive: drivetrain
            .get_f32("GEARS", "FINAL")
            .unwrap_or(spec.engine.final_drive),
        thermal: spec.engine.thermal,
    };

    spec.drivetrain = DrivetrainSpec {
        layout: drivetrain_layout(&drivetrain),
        efficiency: spec.drivetrain.efficiency,
        diff: DiffSpec {
            power_lock: drivetrain
                .get_f32("DIFFERENTIAL", "POWER")
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(spec.drivetrain.diff.power_lock),
            coast_lock: drivetrain
                .get_f32("DIFFERENTIAL", "COAST")
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(spec.drivetrain.diff.coast_lock),
            preload_nm: drivetrain
                .get_f32("DIFFERENTIAL", "PRELOAD")
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(spec.drivetrain.diff.preload_nm),
        },
    };

    spec.gearbox = GearboxSpec {
        automatic: spec.gearbox.automatic,
        // AC 的 CHANGE_UP_TIME / CHANGE_DN_TIME 单位是毫秒。
        switch_time: drivetrain
            .get_f32("GEARBOX", "CHANGE_UP_TIME")
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(spec.gearbox.switch_time),
        clutch_release_time: drivetrain
            .get_f32("GEARBOX", "CHANGE_DN_TIME")
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(spec.gearbox.clutch_release_time),
        clutch_on_shifts: drivetrain
            .get_i32("AUTOCLUTCH", "USE_ON_CHANGES")
            .map(|value| value != 0)
            .unwrap_or(spec.gearbox.clutch_on_shifts),
        rev_match_rpm: drivetrain
            .get_f32("GEARBOX", "VALID_SHIFT_RPM_WINDOW")
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(spec.gearbox.rev_match_rpm),
        clutch_max_torque_nm: drivetrain
            .get_f32("CLUTCH", "MAX_TORQUE")
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(spec.gearbox.clutch_max_torque_nm),
        inertia: drivetrain
            .get_f32("GEARBOX", "INERTIA")
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(spec.gearbox.inertia),
        // AC writes the reverse ratio negative (it is a direction); the spec
        // stores its magnitude and the backend applies the sign.
        reverse_ratio: drivetrain
            .get_f32("GEARS", "GEAR_R")
            .filter(|value| value.is_finite() && value.abs() > 0.0)
            .map(|value| value.abs())
            .unwrap_or(spec.gearbox.reverse_ratio),
        auto_cutoff_time: drivetrain
            .get_f32("GEARBOX", "AUTO_CUTOFF_TIME")
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(spec.gearbox.auto_cutoff_time),
        // A mod may ship any number; keep it inside a range Jolt can brake
        // with instead of letting a typo stall the whole rear axle.
        handbrake_torque_nm: brakes
            .get_f32("DATA", "HANDBRAKE_TORQUE")
            .filter(|value| value.is_finite() && (0.0..=50_000.0).contains(value))
            .unwrap_or(spec.gearbox.handbrake_torque_nm),
    };

    spec.tyres = TyreSpec {
        peak_friction: front
            .peak_friction
            .or(rear.peak_friction)
            .unwrap_or(spec.tyres.peak_friction),
        cold_pressure_psi: tyres
            .get_f32("FRONT", "PRESSURE_STATIC")
            .filter(|value| value.is_finite() && *value > 1.0)
            .unwrap_or(spec.tyres.cold_pressure_psi),
        ideal_pressure_psi: tyres
            .get_f32("FRONT", "PRESSURE_IDEAL")
            .filter(|value| value.is_finite() && *value > 1.0)
            .unwrap_or(spec.tyres.ideal_pressure_psi),
        thermal: tyre_thermal(&tyres, data, &spec.tyres.thermal),
        wear: TyreWearSpec::default(),
        // AC publishes the load-sensitivity reference and exponents per axle;
        // the tyre model is shared, so the axle average is what reaches the
        // spec. Without `FZ0` the effect stays off.
        reference_load_n: tyre_axis_average(&tyres, "FZ0"),
        lateral_load_sensitivity: tyre_axis_average(&tyres, "LS_EXPY"),
        longitudinal_load_sensitivity: tyre_axis_average(&tyres, "LS_EXPX"),
        relaxation_length_m: tyre_axis_average(&tyres, "RELAXATION_LENGTH"),
        // Rolling resistance: the constant term, its speed-squared part and the
        // slip-squared part, all scaled by how much pressure the tyre runs.
        rolling_resistance_n: tyre_axis_average(&tyres, "ROLLING_RESISTANCE_0"),
        rolling_resistance_v2: tyre_axis_average(&tyres, "ROLLING_RESISTANCE_1"),
        rolling_resistance_slip: tyre_axis_average(&tyres, "ROLLING_RESISTANCE_SLIP"),
        pressure_rr_gain: tyre_axis_average(&tyres, "PRESSURE_RR_GAIN"),
        // The tyre's own vertical stiffness, and how much stiffer it gets per
        // psi, turn the wheel into a series spring.
        spring_rate_n_per_m: tyre_axis_average(&tyres, "RATE"),
        pressure_spring_gain_n_per_m: tyre_axis_average(&tyres, "PRESSURE_SPRING_GAIN"),
        // The slip curve's own shape: peak slip angle and how much grip is
        // left at three times that slip.
        lateral_peak_slip_deg: tyre_axis_average(&tyres, "FRICTION_LIMIT_ANGLE"),
        falloff_level: tyre_axis_average(&tyres, "FALLOFF_LEVEL"),
        // Flat spots are a pure runtime effect; AC has no authored values, so
        // the generic defaults apply.
        ..spec.tyres
    };

    if let Some(chord) = aero.get_f32("WING_0", "CHORD")
        && let Some(span) = aero.get_f32("WING_0", "SPAN")
    {
        spec.frontal_area_m2 = chord * span;
    } else {
        debug!(
            car = %spec.name,
            frontal_area_m2 = spec.frontal_area_m2,
            "aero.ini WING_0 CHORD/SPAN missing, using default frontal area"
        );
    }
    if let Some(cd) = aero.get_f32("WING_0", "CD_GAIN") {
        spec.drag_coefficient = cd;
    } else {
        debug!(
            car = %spec.name,
            drag_coefficient = spec.drag_coefficient,
            "aero.ini WING_0 CD_GAIN missing, using default drag coefficient"
        );
    }
    apply_aero(&aero, data, &mut spec);
    apply_brakes(&brakes, &mut spec);
    apply_fuel(&car, &optional_ini(data, "fuel_cons.ini"), &mut spec, root);

    debug!(
        car = %spec.name,
        mass = spec.mass_kg,
        wheelbase = spec.wheelbase_m,
        gears = spec.engine.gear_ratios.len(),
        "mapped AC car data to VehicleSpec"
    );

    Ok(spec)
}

/// 把 `tyres.ini [THERMAL_*]` 搬进 `TyreThermalSpec`：三层的热容/传导率，以及
/// `PERFORMANCE_CURVE`（温度 → 抓地倍率）。
fn tyre_thermal(tyres: &Ini, data: &CarData, base: &TyreThermalSpec) -> TyreThermalSpec {
    let layer = |key: &str, fallback: f32| -> f32 {
        tyres
            .get_f32("THERMAL_FRONT", key)
            .or_else(|| tyres.get_f32("THERMAL_REAR", key))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(fallback)
    };
    let mut thermal = base.clone();
    thermal.grain_gain = layer("GRAIN_GAIN", base.grain_gain);
    thermal.blister_gain = layer("BLISTER_GAIN", base.blister_gain);
    // Three masses: a thin friction surface, a patch under the tread, and the
    // core the grip curve reads. AC publishes the rates, not the capacities.
    if layer("SURFACE_TRANSFER", 0.0) > 0.0 || layer("PATCH_TRANSFER", 0.0) > 0.0 {
        thermal.surface_capacity_j_per_k = 60.0;
        thermal.patch_capacity_j_per_k = 200.0;
    }
    thermal.surface_transfer_per_s = layer("SURFACE_TRANSFER", 0.0);
    thermal.patch_transfer_per_s = layer("PATCH_TRANSFER", 0.0);
    thermal.core_transfer_per_s = layer("CORE_TRANSFER", 0.0);
    thermal.internal_core_transfer_per_s = layer("INTERNAL_CORE_TRANSFER", 0.0);

    // The temperature-vs-grip curve AC ships with the compound.
    if let Some(name) = tyres
        .get("THERMAL_FRONT", "PERFORMANCE_CURVE")
        .map(unquote)
        .filter(|name| !name.is_empty())
        && let Ok(lut) = data.lut(&name)
    {
        thermal.performance_curve = lut
            .points()
            .iter()
            .filter(|point| point.x.is_finite() && point.y.is_finite())
            .map(|point| (point.x, point.y))
            .collect();
    }
    thermal
}

/// 读取一个可选的 ini 文件：缺失/损坏不报错，返回空表让调用方走默认值。
fn optional_ini(data: &CarData, name: &str) -> Ini {
    match data.ini(name) {
        Ok(ini) => ini,
        Err(err) => {
            debug!("optional data file '{name}' unavailable: {err}");
            Ini::default()
        }
    }
}

/// 汇总 `aero.ini` 的所有翼面得到总下压力、前后配平与离地敏感度。
///
/// AC 把每个翼面写成独立的 `WING_n` 段，`CL_GAIN`（升力系数）乘 `CHORD` 和
/// `SPAN` 得到该翼面的 `Cl·A`；超过 CG 的翼面算前翼（AC 帧 +Z 向前）。
fn apply_aero(aero: &Ini, data: &CarData, spec: &mut VehicleSpec) {
    let mut cl_area = 0.0f32;
    let mut cl_area_front = 0.0f32;
    let mut wings = 0usize;
    let mut per_wing: Vec<WingSpec> = Vec::new();
    for section in &aero.sections {
        let name = section.name.to_ascii_uppercase();
        if !name.starts_with("WING_") || name == "WING_0" {
            // WING_0 是车身阻力，不是翼面。
            continue;
        }
        let Some(cl) = section.get_f32("CL_GAIN") else {
            continue;
        };
        if !cl.is_finite() || cl <= 0.0 {
            continue;
        }
        let chord = section.get_f32("CHORD").unwrap_or(0.0).max(0.0);
        let span = section.get_f32("SPAN").unwrap_or(0.0).max(0.0);
        let contribution = cl * chord * span;
        if contribution <= 0.0 {
            continue;
        }
        per_wing.push(wing_from_section(section, cl, data));
        let z = section
            .get("POSITION")
            .and_then(|value| {
                value
                    .split(',')
                    .nth(2)
                    .and_then(|part| part.trim().parse::<f32>().ok())
            })
            .unwrap_or(0.0);
        cl_area += contribution;
        if z > 0.0 {
            cl_area_front += contribution;
        }
        wings += 1;
    }

    // Hacky fallback: some mods keep every wing at zero gain, in which case
    // the car simply has no aero load.
    if cl_area > 1.0e-6 {
        spec.aero.downforce_coeff = cl_area;
        spec.aero.front_bias = (cl_area_front / cl_area).clamp(0.0, 1.0);
    } else {
        spec.aero.downforce_coeff = 0.0;
    }

    // Brackets have most of their ground-effect near the road, so the total
    // fade is derived from how much of the downforce acts in a zone AC would
    // mark as front/rear. Keep it simple: the front wing loses more with ride
    // height than the diffuser.
    if wings == 0 {
        spec.aero.ride_height_fade = 0.0;
    } else {
        spec.aero.ride_height_fade = (0.05 + 0.2 * spec.aero.front_bias).clamp(0.0, 0.5);
    }

    // 侧向力：AC 的 `YAW_CL_GAIN` 是每度横摆的升力系数变化，乘 CHORD×SPAN
    // 得到该翼面对侧滑的横向贡献。累加后再换成“每弧度”，并对零值取绝对值。
    let side_coeff = apply_side_force(aero);
    if side_coeff > 1.0e-6 {
        spec.aero.side_force_coeff = side_coeff;
    } else {
        spec.aero.side_force_coeff = 0.0;
    }

    debug!(
        car = %spec.name,
        wings = wings,
        downforce_coeff = spec.aero.downforce_coeff,
        front_bias = spec.aero.front_bias,
        lut_wings = per_wing.len(),
        "mapped aero.ini wings into AeroSpec"
    );
    spec.aero.wings = per_wing;
}

/// Build one [`WingSpec`] from an AC `[WING_n]` section.
///
/// The four lookup tables are carried verbatim as `(x, y)` points: AC's
/// `LUT_AOA_CL`/`LUT_AOA_CD` map angle of attack to lift/drag coefficients and
/// `LUT_GH_CL`/`LUT_GH_CD` scale them by the surface's ground height. That is
/// the whole ground-effect and stall model, so it must not be flattened away.
fn wing_from_section(section: &IniSection, cl_gain: f32, data: &CarData) -> WingSpec {
    let points = |key: &str| -> Vec<(f32, f32)> {
        let Some(name) = section.get(key).map(unquote).filter(|s| !s.is_empty()) else {
            return Vec::new();
        };
        let Ok(lut) = data.lut(&name) else {
            return Vec::new();
        };
        lut.points()
            .iter()
            .filter(|point| point.x.is_finite() && point.y.is_finite())
            .map(|point| (point.x, point.y))
            .collect()
    };
    WingSpec {
        chord_m: section.get_f32("CHORD").unwrap_or(0.0).max(0.0),
        span_m: section.get_f32("SPAN").unwrap_or(0.0).max(0.0),
        cl_gain,
        cd_gain: section.get_f32("CD_GAIN").unwrap_or(0.0).max(0.0),
        position: ac_vec3(section.get("POSITION")),
        angle_deg: section.get_f32("ANGLE").unwrap_or(0.0),
        aoa_cl: points("LUT_AOA_CL"),
        gh_cl: points("LUT_GH_CL"),
        aoa_cd: points("LUT_AOA_CD"),
        gh_cd: points("LUT_GH_CD"),
    }
}

/// 汇总 `aero.ini` 各翼面的 `YAW_CL_GAIN × CHORD × SPAN` 得到侧滑侧向力系数。
///
/// AC 用“每度”作单位，这里换成“每弧度”（× 57.3）。ZONE_LEFT/RIGHT_CL 提供
/// 左右区域的附加侧向升力，也一并计入；两者都没有的赛道干脆没有侧向气动力。
fn apply_side_force(aero: &Ini) -> f32 {
    let mut total = 0.0f32;
    for section in &aero.sections {
        let name = section.name.to_ascii_uppercase();
        if !name.starts_with("WING_") || name == "WING_0" {
            continue;
        }
        let chord = section.get_f32("CHORD").unwrap_or(0.0).max(0.0);
        let span = section.get_f32("SPAN").unwrap_or(0.0).max(0.0);
        let area = chord * span;
        if area <= 0.0 {
            continue;
        }
        let yaw = section.get_f32("YAW_CL_GAIN").unwrap_or(0.0);
        let zones = section.get_f32("ZONE_LEFT_CL").unwrap_or(0.0).abs()
            + section.get_f32("ZONE_RIGHT_CL").unwrap_or(0.0).abs();
        // 每度 → 每弧度，并给区域侧向力一个保守加成。
        total += (yaw.abs() + zones) * 57.3 * area;
    }
    if total.is_finite() && total > 0.0 {
        total.clamp(0.0, 200.0)
    } else {
        0.0
    }
}

/// 把 `brakes.ini` 的 `[DATA]` 与 `TEMPS_*` 曲线搬进 `BrakeSpec`。
fn apply_brakes(brakes: &Ini, spec: &mut VehicleSpec) {
    spec.brakes.max_torque_nm = brakes
        .get_f32("DATA", "MAX_TORQUE")
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(spec.brakes.max_torque_nm);
    spec.brakes.front_share = brakes
        .get_f32("DATA", "FRONT_SHARE")
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .unwrap_or(spec.brakes.front_share);

    // AC writes the efficiency curve as `(|0=0.50|100=0.7|...|)`. The two
    // interesting points are where it reaches 1.0 and where it starts to fade.
    let curve = brakes
        .get("TEMPS_REAR", "PERF_CURVE")
        .or_else(|| brakes.get("TEMPS_FRONT", "PERF_CURVE"))
        .unwrap_or_default();
    let (optimal_c, fade_c, min_efficiency) = parse_perf_curve(curve);
    if let Some(optimal) = optimal_c {
        spec.brakes.optimal_c = optimal;
    }
    if let Some(fade) = fade_c {
        spec.brakes.fade_c = fade;
    }
    if let Some(minimum) = min_efficiency {
        spec.brakes.min_efficiency = minimum.clamp(0.0, 1.0);
    }

    // `TORQUE_K` scales the heat each newton-metre of brake torque generates.
    if let Some(torque_k) = brakes.get_f32("TEMPS_REAR", "TORQUE_K") {
        spec.brakes.heat_per_joule = torque_k.clamp(0.01, 1.0);
    }
    // `TRANSFER` moves heat between disc surface and core; it therefore sets
    // how much energy the rotor can absorb per kelvin.
    if let Some(transfer) = brakes.get_f32("TEMPS_REAR", "TRANSFER") {
        spec.brakes.thermal_mass_j_per_k = (transfer * 4_000.0).max(200.0);
    }
    // Rotor core: how fast heat soaks into the bulk of the disc, how fast it
    // comes back out into the pads, and how fast it escapes to the air.
    for (key, target) in [
        ("CORE_TRANSFER_IN", 0usize),
        ("CORE_TRANSFER_OUT", 1),
        ("CORE_TRANSFER_AIR", 2),
    ] {
        let value = brakes
            .get_f32("TEMPS_FRONT", key)
            .or_else(|| brakes.get_f32("TEMPS_REAR", key))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.clamp(0.0, 20.0));
        match (target, value) {
            (0, Some(value)) => spec.brakes.core_transfer_in_per_s = value,
            (1, Some(value)) => spec.brakes.core_transfer_out_per_s = value,
            (2, Some(value)) => spec.brakes.core_transfer_air_per_s = value,
            _ => {}
        }
    }
    if let Some(cooling) = brakes.get_f32("TEMPS_REAR", "COOL_TRANSFER") {
        // AC's raw coefficient is per-mille per second; the world applies it
        // as `cooling * speed_factor`, so scale it into a usable 1/s value.
        spec.brakes.cooling_per_s = (cooling * 50.0).clamp(0.001, 2.0);
    }

    debug!(
        car = %spec.name,
        max_torque_nm = spec.brakes.max_torque_nm,
        front_share = spec.brakes.front_share,
        optimal_c = spec.brakes.optimal_c,
        fade_c = spec.brakes.fade_c,
        "mapped brakes.ini into BrakeSpec"
    );
}

/// 把 `car.ini [FUEL]` / `[FUEL_EXT]` / `[FUELTANK]` 搬进 `FuelSpec`。
///
/// `fuel_cons.ini`（部分 mod 才有）给出每升公里数，比 `car.ini` 的
/// `CONSUMPTION` 更接近实际；两者都没有时退回默认的负荷油耗。
fn apply_fuel(car: &Ini, fuel_cons: &Ini, spec: &mut VehicleSpec, root: &Path) {
    let capacity = car
        .get_f32("FUEL", "MAX_FUEL")
        .filter(|value| value.is_finite() && *value > 0.0);
    let current = car.get_f32("FUEL", "FUEL");
    spec.fuel.capacity_l = capacity.unwrap_or(spec.fuel.capacity_l);
    spec.fuel.initial_fraction = match (current, spec.fuel.capacity_l) {
        (Some(current), capacity) if capacity.is_finite() && capacity > 0.0 => {
            (current / capacity).clamp(0.0, 1.0)
        }
        _ => spec.fuel.initial_fraction,
    };
    spec.fuel.density_kg_per_l = car
        .get_f32("FUEL_EXT", "KG_PER_LITER")
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(spec.fuel.density_kg_per_l);

    spec.fuel.km_per_liter = fuel_cons
        .get_f32("FUEL_EVAL", "KM_PER_LITER")
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(spec.fuel.km_per_liter);
    spec.fuel.consumption_l_per_s_at_full_load = car
        .get_f32("FUEL", "CONSUMPTION")
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(spec.fuel.consumption_l_per_s_at_full_load);

    if let Some(position) = car.get("FUELTANK", "POSITION") {
        if let Some(tank) = ac_position(position) {
            spec.fuel.position = tank;
        } else {
            debug!(
                car = %root.display(),
                position = %position,
                "FUELTANK POSITION not parseable, keeping default tank position"
            );
        }
    }

    debug!(
        car = %spec.name,
        capacity_l = spec.fuel.capacity_l,
        initial_fraction = spec.fuel.initial_fraction,
        km_per_liter = spec.fuel.km_per_liter,
        "mapped car.ini fuel data into FuelSpec"
    );
}

/// AC 的 `x,y,z`（+X 左 / +Z 前）转成物理帧（+X 右 / -Z 前）。
fn ac_position(value: &str) -> Option<Vec3> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<f32>().ok());
    let x = parts.next()??;
    let y = parts.next()??;
    let z = parts.next()??;
    let v = Vec3::new(-x, y, -z);
    v.is_finite().then_some(v)
}

/// `(|0=0.50|100=0.7|...|)` → (首次达到 1.0 的温度, 开始衰减的温度, 最低效率)。
fn parse_perf_curve(value: &str) -> (Option<f32>, Option<f32>, Option<f32>) {
    let mut pairs: Vec<(f32, f32)> = value
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .filter_map(|pair| {
            let (key, entry) = pair.split_once('=')?;
            Some((
                key.trim().parse::<f32>().ok()?,
                entry.trim().parse::<f32>().ok()?,
            ))
        })
        .collect();
    if pairs.is_empty() {
        return (None, None, None);
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let optimal = pairs
        .iter()
        .find(|(_, efficiency)| *efficiency >= 0.999)
        .map(|(temp, _)| *temp);
    // The first drop after the peak is where the fade starts.
    let fade = optimal.and_then(|peak| {
        pairs
            .iter()
            .filter(|(temp, _)| *temp > peak)
            .find(|(_, efficiency)| *efficiency < 0.999)
            .map(|(temp, _)| *temp)
    });
    let minimum = pairs
        .iter()
        .map(|(_, efficiency)| *efficiency)
        .fold(1.0, f32::min);
    (optimal, fade, Some(minimum))
}

/// 从 `[TRACTION] TYPE` 推导驱动形式；无法识别的值一律按 RWD 处理。
fn drivetrain_layout(ini: &Ini) -> DrivetrainLayout {
    match ini
        .get("TRACTION", "TYPE")
        .unwrap_or("RWD")
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "FWD" => DrivetrainLayout::Fwd,
        "AWD" | "4WD" => DrivetrainLayout::Awd,
        _ => DrivetrainLayout::Rwd,
    }
}

/// Read AC's `[BASIC] INERTIA` into the spec's tensor, kg·m².
///
/// AC lets any mod write three numbers, and plenty of mods (including the F1
/// 2008 pack) ship values that cannot be physical — `0.7` as a pitch moment is
/// orders of magnitude below what a real car needs. Each axis is therefore
/// validated against the car's own mass and footprint before it is accepted;
/// anything implausible falls back to the collider tensor so the car still
/// drives, and the fallback is reported at debug level.
fn inertia_from_car(car: &Ini, spec: &VehicleSpec) -> Vec3 {
    let derived = spec.collider_inertia();
    let Some(raw) = car.get("BASIC", "INERTIA") else {
        return derived;
    };
    let authored: Vec<f32> = raw
        .split(',')
        .filter_map(|value| {
            vettura_acdata::ini::strip_comment(value).and_then(|text| text.parse::<f32>().ok())
        })
        .collect();
    if authored.len() < 3 {
        debug!(
            car = %spec.name,
            value = raw,
            "car.ini INERTIA does not have three numbers; using the collider tensor"
        );
        return derived;
    }
    // AC's tuple is `(x, y, z)` in the AC frame, whose axes are
    // (lateral, vertical, longitudinal); the spec labels the same tensor
    // `(roll, pitch, yaw)`.
    let authored = Vec3::new(authored[2], authored[0], authored[1]);
    // Plausible band from the car's own footprint: a moment can neither be a
    // rounding error nor larger than a hoop of the car's own mass.
    let footprint = spec.wheelbase_m.max(spec.chassis_half_extents().0 * 2.0);
    let lower = 0.02 * spec.mass_kg.max(1.0) * footprint * footprint;
    let upper = 0.6 * spec.mass_kg.max(1.0) * footprint * footprint;
    let plausible = authored
        .to_array()
        .iter()
        .all(|value| value.is_finite() && *value >= lower && *value <= upper);
    if !plausible {
        debug!(
            car = %spec.name,
            ?authored,
            lower,
            upper,
            "car.ini INERTIA is not a plausible kg·m² tensor; using the collider tensor"
        );
        return derived;
    }
    authored
}

/// 读 1 起的 `GEAR_1..GEAR_n` 齿比列表；列表不完整时返回 `None`（调用方
/// 退回默认齿比，而不是装上一段残缺的变速箱）。
fn read_gear_ratios(ini: &Ini) -> Option<Vec<f32>> {
    let count = ini.get_i32("GEARS", "COUNT")?.max(1) as usize;
    let mut gears = Vec::with_capacity(count);
    for index in 1..=count {
        let key = format!("GEAR_{index}");
        let ratio = ini.get_f32("GEARS", &key)?;
        gears.push(ratio);
    }
    Some(gears)
}

/// Read `power.lut` into the spec's full torque curve.
///
/// AC's `power.lut` is `rpm|torque(N·m)`. The table often has a last point
/// past the limiter (e.g. `20000|0` for a 19000 rpm engine); keep it in the
/// curve data but let the backend drop everything above the redline.
fn read_torque_curve(data: &CarData) -> Vec<TorquePoint> {
    let Ok(lut) = data.lut("power.lut") else {
        return Vec::new();
    };
    let mut points: Vec<TorquePoint> = lut
        .points()
        .iter()
        .filter(|point| {
            point.x.is_finite() && point.y.is_finite() && point.x >= 0.0 && point.y >= 0.0
        })
        .map(|point| TorquePoint {
            rpm: point.x,
            torque_nm: point.y,
        })
        .collect();
    points.sort_by(|a, b| a.rpm.total_cmp(&b.rpm));
    points.dedup_by(|a, b| (a.rpm - b.rpm).abs() < f32::EPSILON);
    points
}

/// Average one `tyres.ini` tyre key over the front and rear axles.
///
/// AC quotes most tyre model coefficients per axle (`FZ0=3705` front,
/// `4295` rear); the shared `TyreSpec` keeps one value, and the load-weighted
/// midpoint of the two axles is the honest summary. Returns `0.0` when neither
/// axle publishes the key, which is the signal to leave the effect off.
fn tyre_axis_average(tyres: &Ini, key: &str) -> f32 {
    let values: Vec<f32> = ["FRONT", "REAR"]
        .iter()
        .filter_map(|axle| tyres.get_f32(axle, key))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    let average = values.iter().sum::<f32>() / values.len() as f32;
    if average.is_finite() { average } else { 0.0 }
}

/// Read AC's optional forced induction block.
///
/// `[TURBO_0]` carries `MAX_BOOST`/`WASTEGATE` in bar, the spool-up shape
/// (`REFERENCE_RPM`, `GAMMA`) and optionally the lag time constants
/// (`LAG_UP`, `LAG_DN`, seconds) and an extra `TORQUE_GAIN` per bar. A car with
/// no turbo section keeps `TurboSpec::default()`, which is disabled.
fn turbo_from_engine(engine: &Ini) -> TurboSpec {
    let mut turbo = TurboSpec::default();
    let Some(section) = engine.section("TURBO_0") else {
        return turbo;
    };
    let max_boost = section.get_f32("MAX_BOOST").unwrap_or(0.0).clamp(0.0, 5.0);
    if max_boost <= 0.0 {
        return turbo;
    }
    turbo.max_boost_bar = max_boost;
    turbo.wastegate_bar = section
        .get_f32("WASTEGATE")
        .or_else(|| section.get_f32("WASTE_GATE"))
        .unwrap_or(0.0)
        .clamp(0.0, 5.0);
    turbo.reference_rpm = section
        .get_f32("REFERENCE_RPM")
        .unwrap_or(3_000.0)
        .clamp(200.0, 30_000.0);
    turbo.gamma = section.get_f32("GAMMA").unwrap_or(1.0).clamp(0.1, 4.0);
    turbo.lag_up_s = section
        .get_f32("LAG_UP")
        .unwrap_or(0.20 + 0.25 * max_boost)
        .clamp(0.02, 2.0);
    turbo.lag_down_s = section
        .get_f32("LAG_DN")
        .unwrap_or(0.12 + 0.15 * max_boost)
        .clamp(0.02, 2.0);
    turbo.torque_gain_per_bar = section
        .get_f32("TORQUE_GAIN")
        .unwrap_or(1.0)
        .clamp(0.0, 3.0);
    turbo
}

struct AxleData {
    radius: Option<f32>,
    width: Option<f32>,
    track: Option<f32>,
    base_y: Option<f32>,
    spring_rate: Option<f32>,
    damper_bump: Option<f32>,
    damper_rebound: Option<f32>,
    travel: Option<f32>,
    anti_roll: Option<f32>,
    peak_friction: Option<f32>,
    progressive_rate: Option<f32>,
    bump_stop_rate: Option<f32>,
    static_camber: Option<f32>,
    roll_center: Option<f32>,
    anti_dive: Option<f32>,
    anti_squat: Option<f32>,
    caster_deg: Option<f32>,
    kingpin_deg: Option<f32>,
    fast_bump: Option<f32>,
    fast_bump_threshold: Option<f32>,
    fast_rebound: Option<f32>,
    fast_rebound_threshold: Option<f32>,
}

impl AxleData {
    /// 读一个轴（`FRONT` / `REAR`）的轮胎与悬挂字段；没写的字段是 `None`。
    fn from_ini(suspensions: &Ini, tyres: &Ini, axle: &str) -> Self {
        Self {
            radius: tyres.get_f32(axle, "RADIUS"),
            width: tyres.get_f32(axle, "WIDTH"),
            track: suspensions.get_f32(axle, "TRACK"),
            base_y: suspensions.get_f32(axle, "BASEY"),
            spring_rate: suspensions.get_f32(axle, "SPRING_RATE"),
            damper_bump: suspensions.get_f32(axle, "DAMP_BUMP"),
            damper_rebound: suspensions.get_f32(axle, "DAMP_REBOUND"),
            travel: suspensions
                .get_f32(axle, "PACKER_RANGE")
                .or_else(|| suspensions.get_f32(axle, "BUMPSTOP_UP")),
            anti_roll: suspensions.get_f32("ARB", axle),
            peak_friction: tyres
                .get_f32(axle, "DY0")
                .or_else(|| tyres.get_f32(axle, "DX0")),
            progressive_rate: suspensions.get_f32(axle, "PROGRESSIVE_SPRING_RATE"),
            bump_stop_rate: suspensions.get_f32(axle, "BUMP_STOP_RATE"),
            static_camber: suspensions.get_f32(axle, "STATIC_CAMBER"),
            // Optional; only some mods author the roll centre explicitly, and
            // AC's own `[ARB]` section is the other place it appears.
            roll_center: suspensions
                .get_f32(axle, "ROLL_CENTER")
                .or_else(|| suspensions.get_f32("ARB", &format!("{axle}_ROLL_CENTER"))),
            anti_dive: suspensions.get_f32(axle, "ANTI_DIVE"),
            anti_squat: suspensions.get_f32(axle, "ANTI_SQUAT"),
            caster_deg: suspensions.get_f32(axle, "CASTER"),
            kingpin_deg: suspensions.get_f32(axle, "KINGPIN"),
            // Two-rate dampers: the bleeder rate and the speed it kicks in.
            fast_bump: suspensions.get_f32(axle, "DAMP_FAST_BUMP"),
            fast_bump_threshold: suspensions.get_f32(axle, "DAMP_FAST_BUMPTHRESHOLD"),
            fast_rebound: suspensions.get_f32(axle, "DAMP_FAST_REBOUND"),
            fast_rebound_threshold: suspensions.get_f32(axle, "DAMP_FAST_REBOUNDTHRESHOLD"),
        }
    }
}

/// Read one `[HEAVE_FRONT]` / `[HEAVE_REAR]` third spring.
///
/// A car without the section gets all zeroes, which is exactly "no heave
/// spring". The packer range is used as the bump-stop travel, because AC
/// measures the heave element's travel in the same section.
fn heave_spring(suspensions: &Ini, section: &str) -> HeaveSpringSpec {
    let rate = suspensions
        .get_f32(section, "SPRING_RATE")
        .unwrap_or(0.0)
        .clamp(0.0, 2_000_000.0);
    if rate <= 0.0 {
        return HeaveSpringSpec::default();
    }
    HeaveSpringSpec {
        spring_rate_n_per_m: rate,
        progressive_rate_n_per_m2: suspensions
            .get_f32(section, "PROGRESSIVE_SPRING_RATE")
            .unwrap_or(0.0)
            .clamp(0.0, 2_000_000.0),
        bump_stop_rate_n_per_m: suspensions
            .get_f32(section, "BUMP_STOP_RATE")
            .unwrap_or(0.0)
            .clamp(0.0, 2_000_000.0),
        bump_stop_travel_m: suspensions
            .get_f32(section, "PACKER_RANGE")
            .unwrap_or(0.05)
            .clamp(0.005, 0.3),
        preload_m: suspensions
            .get_f32(section, "ROD_LENGTH")
            .unwrap_or(0.0)
            .clamp(-0.05, 0.05),
        damper_bump_ns_per_m: suspensions
            .get_f32(section, "DAMP_BUMP")
            .unwrap_or(0.0)
            .clamp(0.0, 500_000.0),
        damper_rebound_ns_per_m: suspensions
            .get_f32(section, "DAMP_REBOUND")
            .unwrap_or(0.0)
            .clamp(0.0, 500_000.0),
    }
}

/// Read the optional active-suspension block.
///
/// Vanilla AC does not expose active suspension in `suspensions.ini`; mods
/// that do use an `[ACTIVE]` section. Recognised keys are `ENABLED` (`0` off,
/// `1` fully active, `2` semi-active), `STIFFNESS`/`SPRING_RATE`, `DAMPING`,
/// `MAX_FORCE`, `RIDE_HEIGHT`, `RAKE`, `SPEED_DROP` and `SPEED_REF`. With no
/// section the car stays passive, so every existing asset keeps its behaviour.
fn active_suspension(suspensions: &Ini, mass_kg: f32) -> ActiveSuspensionSpec {
    let mode = match suspensions.get_i32("ACTIVE", "ENABLED").unwrap_or(0) {
        1 => ActiveSuspensionMode::Active,
        2 => ActiveSuspensionMode::SemiActive,
        _ => ActiveSuspensionMode::Off,
    };
    if mode == ActiveSuspensionMode::Off {
        return ActiveSuspensionSpec::default();
    }
    let stiffness = suspensions
        .get_f32("ACTIVE", "STIFFNESS")
        .or_else(|| suspensions.get_f32("ACTIVE", "SPRING_RATE"))
        .unwrap_or(0.0)
        .clamp(0.0, 2_000_000.0);
    let damping = suspensions
        .get_f32("ACTIVE", "DAMPING")
        .or_else(|| suspensions.get_f32("ACTIVE", "DAMP_BUMP"))
        .unwrap_or(0.0)
        .clamp(0.0, 500_000.0);
    // Without an authored limit, size the actuator at 1.25 times the static
    // load of one corner. That is enough to hold ride height under aero load
    // without letting it lift the whole car off the ground.
    let static_corner = (mass_kg.max(1.0) * 9.81 * 0.25).max(1.0);
    ActiveSuspensionSpec {
        mode,
        stiffness_n_per_m: stiffness,
        damping_ns_per_m: damping,
        max_force_n: suspensions
            .get_f32("ACTIVE", "MAX_FORCE")
            .unwrap_or(static_corner * 1.25)
            .clamp(0.0, 200_000.0),
        ride_height_m: suspensions
            .get_f32("ACTIVE", "RIDE_HEIGHT")
            .unwrap_or(0.0)
            .clamp(-0.1, 0.1),
        rake_m: suspensions
            .get_f32("ACTIVE", "RAKE")
            .unwrap_or(0.0)
            .clamp(-0.1, 0.1),
        speed_drop_m: suspensions
            .get_f32("ACTIVE", "SPEED_DROP")
            .unwrap_or(0.0)
            .clamp(0.0, 0.15),
        speed_ref_mps: suspensions
            .get_f32("ACTIVE", "SPEED_REF")
            .unwrap_or(50.0)
            .clamp(1.0, 150.0),
    }
}

/// Strip the quotes AC puts around some string values.
fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').trim_matches('\'').to_owned()
}

/// Parse an AC `x,y,z` triple in the AC frame (+X right, +Y up, +Z forward).
fn ac_vec3(value: Option<&str>) -> Vec3 {
    let Some(value) = value.map(unquote) else {
        return Vec3::ZERO;
    };
    let parts: Vec<f32> = value
        .split(',')
        .filter_map(|part| part.trim().parse::<f32>().ok())
        .collect();
    if parts.len() < 3 {
        return Vec3::ZERO;
    }
    Vec3::new(parts[0], parts[1], parts[2])
}

fn glam_vec3(x: f32, y: f32, z: f32) -> glam::Vec3 {
    glam::Vec3::new(x, y, z)
}

#[allow(dead_code)]
fn parse_float_list(value: &str) -> Option<Vec<f32>> {
    value
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok())
        .collect()
}

#[allow(dead_code)]
fn require_file(data: &CarData, name: &str) -> Result<()> {
    if data.get(name).is_some() {
        Ok(())
    } else {
        Err(AssetError::MissingFile(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_physics::BrakeSpec;

    #[test]
    fn perf_curve_maps_ac_semantics_to_a_window_and_a_fade() {
        // 取自 F1 2008 mod 的 brakes.ini TEMPS_REAR.PERF_CURVE。
        let curve = "(|0=0.50|100=0.7|200=0.9|300=1.0|400=1.0|500=1|600=1|700=1|800=0.95|900=0.9|1000=0.85|1100=0.8|1200=0.8|)";
        let (optimal, fade, minimum) = parse_perf_curve(curve);
        assert_eq!(optimal, Some(300.0));
        assert_eq!(fade, Some(800.0));
        assert_eq!(minimum, Some(0.5));

        assert_eq!(parse_perf_curve(""), (None, None, None));
        assert_eq!(parse_perf_curve("garbage"), (None, None, None));
    }

    #[test]
    fn ac_position_converts_to_the_physics_frame() {
        assert_eq!(
            ac_position("0,0.024,-0.1"),
            Some(Vec3::new(0.0, 0.024, 0.1))
        );
        // AC 的 +Z 是前方，因此它在物理帧里变成 -Z。
        assert_eq!(ac_position("1.5,2,-3"), Some(Vec3::new(-1.5, 2.0, 3.0)));
        assert_eq!(ac_position("NaN,1,1"), None);
        assert_eq!(ac_position("1,2"), None);
        assert_eq!(ac_position(""), None);
    }

    #[test]
    fn gearbox_settings_read_ac_millisecond_timings() {
        let ini = Ini::parse(
            "[GEARBOX]\nCHANGE_UP_TIME=10\nCHANGE_DN_TIME=20\nVALID_SHIFT_RPM_WINDOW=1200\nINERTIA=0.012\n\
             [CLUTCH]\nMAX_TORQUE=1150\n[AUTOCLUTCH]\nUSE_ON_CHANGES=1\n",
        );
        let spec = VehicleSpec::default();
        let gearbox = GearboxSpec {
            switch_time: ini
                .get_f32("GEARBOX", "CHANGE_UP_TIME")
                .unwrap_or(spec.gearbox.switch_time),
            clutch_release_time: ini
                .get_f32("GEARBOX", "CHANGE_DN_TIME")
                .unwrap_or(spec.gearbox.clutch_release_time),
            clutch_on_shifts: ini
                .get_i32("AUTOCLUTCH", "USE_ON_CHANGES")
                .map(|value| value != 0)
                .unwrap_or(spec.gearbox.clutch_on_shifts),
            rev_match_rpm: ini
                .get_f32("GEARBOX", "VALID_SHIFT_RPM_WINDOW")
                .unwrap_or(spec.gearbox.rev_match_rpm),
            clutch_max_torque_nm: ini
                .get_f32("CLUTCH", "MAX_TORQUE")
                .unwrap_or(spec.gearbox.clutch_max_torque_nm),
            inertia: ini
                .get_f32("GEARBOX", "INERTIA")
                .unwrap_or(spec.gearbox.inertia),
            ..GearboxSpec::default()
        };
        // Raw values stay in AC's units; the world converts milliseconds to
        // seconds when it builds the Jolt transmission.
        assert_eq!(gearbox.switch_time, 10.0);
        assert_eq!(gearbox.clutch_release_time, 20.0);
        assert_eq!(gearbox.rev_match_rpm, 1200.0);
        assert_eq!(gearbox.clutch_max_torque_nm, 1150.0);
        assert!(gearbox.clutch_on_shifts);

        // The spec's own defaults are in seconds, which is what the world
        // feeds to Jolt; only `drivetrain.ini` speaks milliseconds.
        let empty = GearboxSpec::default();
        assert!((empty.switch_time - 0.12).abs() < 1.0e-6);
        assert!(empty.automatic);
    }

    #[test]
    fn diff_settings_default_when_the_section_is_missing() {
        let spec = VehicleSpec::default();
        let mapping = |ini: &str| {
            let drivetrain = Ini::parse(ini);
            vec![
                drivetrain
                    .get_f32("DIFFERENTIAL", "POWER")
                    .unwrap_or(spec.drivetrain.diff.power_lock),
                drivetrain
                    .get_f32("DIFFERENTIAL", "COAST")
                    .unwrap_or(spec.drivetrain.diff.coast_lock),
            ]
        };
        assert_eq!(mapping("")[0], 1.0);
        assert_eq!(mapping("[DIFFERENTIAL]\nPOWER=0.25\n")[0], 0.25);
    }

    #[test]
    fn brake_perf_curve_feeds_the_efficiency_window() {
        // brakes.ini 的语义：0 °C 时只有一半效率，300 °C 起进入最佳窗口。
        let (optimal, _, minimum) = parse_perf_curve(
            "(|0=0.50|100=0.7|200=0.9|300=1.0|400=1.0|500=1|600=1|700=1|800=0.95|900=0.9|1000=0.85|1100=0.8|1200=0.8|)",
        );
        let brakes = BrakeSpec {
            optimal_c: optimal.unwrap_or(300.0),
            fade_c: 800.0,
            min_efficiency: minimum.unwrap_or(0.5),
            ..BrakeSpec::default()
        };
        assert!((brakes.efficiency_at(0.0) - 0.5).abs() < 0.05);
        // The window is flat at full efficiency right at the peak, then fades
        // linearly down to `min_efficiency` at `fade_c`.
        assert!((brakes.efficiency_at(optimal.unwrap()) - 1.0).abs() < 1.0e-6);
        assert!(brakes.efficiency_at(400.0) > 0.85);
        assert!(brakes.efficiency_at(1_000.0) < 0.9);
    }

    #[test]
    fn drivetrain_layout_normalizes_case_and_whitespace() {
        let parse = |traction: &str| {
            drivetrain_layout(&Ini::parse(&format!("[TRACTION]\nTYPE={traction}\n")))
        };

        assert_eq!(parse("FWD"), DrivetrainLayout::Fwd);
        assert_eq!(parse(" fwd "), DrivetrainLayout::Fwd);
        assert_eq!(parse("RWD"), DrivetrainLayout::Rwd);
        assert_eq!(parse("AWD"), DrivetrainLayout::Awd);
        assert_eq!(parse("4WD"), DrivetrainLayout::Awd);
        // 缺失或无法识别的值一律 RWD（与 AC 默认一致）。
        assert_eq!(parse(""), DrivetrainLayout::Rwd);
        assert_eq!(parse("banana"), DrivetrainLayout::Rwd);
        assert_eq!(
            drivetrain_layout(&Ini::parse("[OTHER]\nTYPE=FWD\n")),
            DrivetrainLayout::Rwd
        );
    }

    #[test]
    fn gear_ratios_need_a_complete_one_based_list() {
        let ratios = |ini: &str| read_gear_ratios(&Ini::parse(ini));

        assert_eq!(
            ratios("[GEARS]\nCOUNT=3\nGEAR_1=3.5\nGEAR_2=2.0\nGEAR_3=1.4\n"),
            Some(vec![3.5, 2.0, 1.4])
        );
        // 少一档 → None（调用方退回默认齿比，而不是塞一段残缺列表）。
        assert_eq!(ratios("[GEARS]\nCOUNT=3\nGEAR_1=3.5\nGEAR_2=2.0\n"), None);
        // COUNT 缺失 / GEARS 段缺失 → None。
        assert_eq!(ratios("[GEARS]\nGEAR_1=3.5\n"), None);
        assert_eq!(ratios("[TRACTION]\nTYPE=FWD\n"), None);
        // COUNT=0 被抬到 1，于是要求 GEAR_1 存在。
        assert_eq!(ratios("[GEARS]\nCOUNT=0\nGEAR_1=3.5\n"), Some(vec![3.5]));
        assert_eq!(ratios("[GEARS]\nCOUNT=0\n"), None);
    }

    #[test]
    fn axle_data_reads_tyre_and_suspension_fields() {
        let suspensions =
            Ini::parse("[FRONT]\nTRACK=1.6\nBASEY=0.1\nSPRING_RATE=120000\n[ARB]\nFRONT=8000\n");
        let tyres = Ini::parse("[FRONT]\nRADIUS=0.35\nWIDTH=0.28\nDY0=1.2\n");

        let front = AxleData::from_ini(&suspensions, &tyres, "FRONT");
        assert_eq!(front.radius, Some(0.35));
        assert_eq!(front.width, Some(0.28));
        assert_eq!(front.track, Some(1.6));
        assert_eq!(front.base_y, Some(0.1));
        assert_eq!(front.spring_rate, Some(120_000.0));
        assert_eq!(front.anti_roll, Some(8000.0));
        assert_eq!(front.peak_friction, Some(1.2));
        // 完全没写的 REAR 段 → 全 None，映射层用默认值兜底。
        let rear = AxleData::from_ini(&suspensions, &tyres, "REAR");
        assert!(rear.radius.is_none());
        assert!(rear.track.is_none());
    }
}
