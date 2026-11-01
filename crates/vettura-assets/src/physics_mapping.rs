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

    debug!(
        car = %spec.name,
        mass = spec.mass_kg,
        wheelbase = spec.wheelbase_m,
        gears = spec.engine.gear_ratios.len(),
        "mapped AC car data to VehicleSpec"
    );

    Ok(spec)
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

#[cfg(test)]
mod tests {
    use super::*;

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
