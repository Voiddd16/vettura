use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::spec::aero::AeroSpec;
use crate::spec::brakes::BrakeSpec;
use crate::spec::drivetrain::{DrivetrainSpec, GearboxSpec};
use crate::spec::drs::DrsSpec;
use crate::spec::engine::EngineSpec;
use crate::spec::ers::ErsSpec;
use crate::spec::fuel::FuelSpec;
use crate::spec::suspension::SuspensionSpec;
use crate::spec::suspension::{ActiveSuspensionSpec, HeaveSpringSpec};
use crate::spec::tyre::TyreSpec;

/// Complete vehicle specification assembled from AC INI files.
///
/// The AC format splits the car across many INI sections; this struct
/// unifies them into a single source of truth for the backends. Every
/// field corresponds to a published parameter (or a sensible default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleSpec {
    pub name: String,
    pub mass_kg: f32,
    /// Distance between front and rear axle centers.
    pub wheelbase_m: f32,
    /// Center of gravity height above the ground.
    pub cg_height_m: f32,
    pub frontal_area_m2: f32,
    pub drag_coefficient: f32,
    /// Physics frame: +X right, +Y up, -Z forward.
    pub wheels: [WheelSpec; 4],
    pub engine: EngineSpec,
    pub drivetrain: DrivetrainSpec,
    pub tyres: TyreSpec,
    pub aero: AeroSpec,
    pub brakes: BrakeSpec,
    pub fuel: FuelSpec,
    pub gearbox: GearboxSpec,
    pub drs: DrsSpec,
    pub ers: ErsSpec,
    /// Active / semi-active suspension and the axle heave springs AC authors in
    /// `suspensions.ini`.
    pub active_suspension: ActiveSuspensionSpec,
    /// Heave (third) springs per axle, front then rear. `0` means the car does
    /// not have one.
    pub heave_springs: [HeaveSpringSpec; 2],
    /// Inertia tensor about the centre of mass, kg·m², as
    /// `(roll, pitch, yaw)` in the physics frame. `Vec3::ZERO` means "derive it
    /// from the collider box", which is what happens when a mod does not
    /// publish a usable `car.ini [BASIC] INERTIA`.
    pub inertia: Vec3,
    /// Biggest steer angle the front wheels can take, degrees at the wheel.
    /// AC's `[CONTROLS] STEER_LOCK / STEER_RATIO`, which is the only honest
    /// source: the steering wheel's lock is not the wheel's lock.
    pub max_steer_angle_deg: f32,
}

impl Default for VehicleSpec {
    fn default() -> Self {
        let wheelbase = 2.6;
        let track = 1.5;
        let wheel_radius = 0.33;
        let half_wb = wheelbase * 0.5;

        Self {
            name: "default".to_owned(),
            mass_kg: 1200.0,
            wheelbase_m: wheelbase,
            cg_height_m: 0.35,
            frontal_area_m2: 1.8,
            drag_coefficient: 0.35,
            wheels: [
                WheelSpec::at(-track * 0.5, wheel_radius, -half_wb, true, false),
                WheelSpec::at(track * 0.5, wheel_radius, -half_wb, true, false),
                WheelSpec::at(-track * 0.5, wheel_radius, half_wb, false, true),
                WheelSpec::at(track * 0.5, wheel_radius, half_wb, false, true),
            ],
            engine: EngineSpec::default(),
            drivetrain: DrivetrainSpec::default(),
            tyres: TyreSpec::default(),
            aero: AeroSpec::default(),
            brakes: BrakeSpec::default(),
            fuel: FuelSpec::default(),
            gearbox: GearboxSpec::default(),
            drs: DrsSpec::default(),
            ers: ErsSpec::default(),
            active_suspension: ActiveSuspensionSpec::default(),
            heave_springs: [HeaveSpringSpec::default(); 2],
            max_steer_angle_deg: 30.0,
            // Defaults to the derived tensor, exactly what the body would get
            // without this field, so behaviour is unchanged.
            inertia: Vec3::ZERO,
        }
    }
}

impl VehicleSpec {
    /// Half extents of the collider box the chassis is modelled with, in the
    /// physics frame: `(half width, half length, half height)`.
    ///
    /// The inertia tensor and the Jolt collision hull read the same numbers, so
    /// a change of footprint stays consistent everywhere.
    pub fn chassis_half_extents(&self) -> (f32, f32, f32) {
        let half_w = self
            .wheels
            .iter()
            .map(|wheel| wheel.position.x.abs() + wheel.width_m.max(0.0) * 0.5)
            .fold(0.0f32, f32::max)
            .max(0.2)
            + 0.02;
        let half_l = self
            .wheels
            .iter()
            .map(|wheel| wheel.position.z.abs() + wheel.radius_m.max(0.0))
            .fold(0.0f32, f32::max)
            .max(0.3)
            + 0.05;
        let half_h = (self.cg_height_m - 0.06).clamp(0.10, 0.55);
        (half_w, half_l, half_h)
    }

    /// Inertia tensor of the collider box, kg·m², as `(roll, pitch, yaw)`.
    ///
    /// This is what Jolt derives on its own; it is exposed so the mapper can
    /// compare AC's authored values against it, and so a caller can inspect
    /// what the world will use when no tensor is published.
    pub fn collider_inertia(&self) -> Vec3 {
        let (half_w, half_l, half_h) = self.chassis_half_extents();
        let mass = self.mass_kg.max(1.0);
        let (w, h, l) = (2.0 * half_w, 2.0 * half_h, 2.0 * half_l);
        Vec3::new(
            mass * (h * h + w * w) / 12.0,
            mass * (l * l + h * h) / 12.0,
            mass * (l * l + w * w) / 12.0,
        )
    }

    /// Tensor the body should be built with, kg·m², as `(roll, pitch, yaw)`.
    ///
    /// `Vec3::ZERO` (the default) keeps Jolt's own shape-derived tensor; an
    /// explicit tensor replaces it.
    pub fn effective_inertia(&self) -> Vec3 {
        if self.inertia.is_finite() && self.inertia.cmpgt(Vec3::ZERO).any() {
            self.inertia
        } else {
            Vec3::ZERO
        }
    }
}

/// One wheel's static specification: position, geometry and suspension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WheelSpec {
    /// Mounting position in the physics frame.
    pub position: Vec3,
    pub radius_m: f32,
    pub width_m: f32,
    pub steerable: bool,
    pub driven: bool,
    pub suspension: SuspensionSpec,
}

impl WheelSpec {
    pub fn at(x: f32, y: f32, z: f32, steerable: bool, driven: bool) -> Self {
        Self {
            position: Vec3::new(x, y, z),
            radius_m: 0.33,
            width_m: 0.24,
            steerable,
            driven,
            suspension: SuspensionSpec::default(),
        }
    }
}
