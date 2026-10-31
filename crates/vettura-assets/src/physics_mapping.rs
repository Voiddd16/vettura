//! Convert Assetto Corsa car data into the backend-neutral physics spec.
//!
//! Not every AC parameter has a home in [`VehicleSpec`] yet. The mapping uses
//! conservative defaults for anything missing and logs the fallback at debug
//! level, so a half-supported mod still drives instead of failing to load.

use std::path::Path;

use tracing::debug;
use vettura_acdata::Ini;
use vettura_physics::{
    DrivetrainLayout, DrivetrainSpec, EngineSpec, SuspensionSpec, TyreSpec, VehicleSpec, Wheel,
    WheelSpec,
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
            },
        };
    }

    spec.engine = EngineSpec {
        idle_rpm: engine
            .get_f32("ENGINE_DATA", "MINIMUM")
            .unwrap_or(spec.engine.idle_rpm),
        redline_rpm: engine
            .get_f32("ENGINE_DATA", "LIMITER")
            .unwrap_or(spec.engine.redline_rpm),
        peak_torque_nm: data
            .lut("power.lut")
            .ok()
            .and_then(|lut| lut.points().iter().map(|point| point.y).reduce(f32::max))
            .unwrap_or(spec.engine.peak_torque_nm),
        gear_ratios: read_gear_ratios(&drivetrain)
            .unwrap_or_else(|| spec.engine.gear_ratios.clone()),
        final_drive: drivetrain
            .get_f32("GEARS", "FINAL")
            .unwrap_or(spec.engine.final_drive),
    };

    spec.drivetrain = DrivetrainSpec {
        layout: drivetrain_layout(&drivetrain),
        efficiency: spec.drivetrain.efficiency,
    };

    spec.tyres = TyreSpec {
        peak_friction: front
            .peak_friction
            .or(rear.peak_friction)
            .unwrap_or(spec.tyres.peak_friction),
    };

    if let Some(chord) = aero.get_f32("WING_0", "CHORD")
        && let Some(span) = aero.get_f32("WING_0", "SPAN")
    {
        spec.frontal_area_m2 = chord * span;
    }
    if let Some(cd) = aero.get_f32("WING_0", "CD_GAIN") {
        spec.drag_coefficient = cd;
    }

    debug!(
        car = %spec.name,
        mass = spec.mass_kg,
        wheelbase = spec.wheelbase_m,
        gears = spec.engine.gear_ratios.len(),
        "mapped AC car data to VehicleSpec"
    );

    Ok(spec)
}

fn optional_ini(data: &CarData, name: &str) -> Ini {
    match data.ini(name) {
        Ok(ini) => ini,
        Err(err) => {
            debug!("optional data file '{name}' unavailable: {err}");
            Ini::default()
        }
    }
}

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
}

impl AxleData {
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
        }
    }
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
