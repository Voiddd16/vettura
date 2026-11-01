use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Four wheels, in a fixed order. Index into `[WheelSpec; 4]` / `[WheelState; 4]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Wheel {
    FrontLeft = 0,
    FrontRight = 1,
    RearLeft = 2,
    RearRight = 3,
}

impl Wheel {
    pub const ALL: [Wheel; 4] = [
        Wheel::FrontLeft,
        Wheel::FrontRight,
        Wheel::RearLeft,
        Wheel::RearRight,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn is_front(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::FrontRight)
    }

    pub const fn is_left(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::RearLeft)
    }
}

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
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionSpec {
    pub spring_rate_n_per_m: f32,
    pub damper_bump_ns_per_m: f32,
    pub damper_rebound_ns_per_m: f32,
    pub travel_m: f32,
    pub anti_roll_n_per_m: f32,
}

impl Default for SuspensionSpec {
    fn default() -> Self {
        Self {
            spring_rate_n_per_m: 80_000.0,
            damper_bump_ns_per_m: 4_000.0,
            damper_rebound_ns_per_m: 6_000.0,
            travel_m: 0.08,
            anti_roll_n_per_m: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSpec {
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// Peak torque, used by the simple backend. A real backend should consume
    /// the full torque curve instead.
    pub peak_torque_nm: f32,
    pub gear_ratios: Vec<f32>,
    pub final_drive: f32,
}

impl Default for EngineSpec {
    fn default() -> Self {
        Self {
            idle_rpm: 1_000.0,
            redline_rpm: 8_000.0,
            peak_torque_nm: 350.0,
            gear_ratios: vec![3.2, 2.1, 1.6, 1.3, 1.1, 0.95],
            final_drive: 3.7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrivetrainLayout {
    Rwd,
    Fwd,
    Awd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivetrainSpec {
    pub layout: DrivetrainLayout,
    pub efficiency: f32,
}

impl Default for DrivetrainSpec {
    fn default() -> Self {
        Self {
            layout: DrivetrainLayout::Rwd,
            efficiency: 0.9,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TyreSpec {
    /// Simplified peak friction coefficient. A real tyre model replaces this
    /// with a Pacejka/brush model and separate longitudinal/lateral curves.
    pub peak_friction: f32,
}

impl Default for TyreSpec {
    fn default() -> Self {
        Self { peak_friction: 1.4 }
    }
}

/// Convenience pose used by `VehicleBackend::reset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, 0.0),
            orientation: Quat::IDENTITY,
        }
    }
}

impl Pose {
    pub fn forward(self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_indices_and_axle_predicates_agree() {
        for wheel in Wheel::ALL {
            assert_eq!(wheel.index(), wheel as usize);
            // 索引必须直接可用来寻址 wheels 数组。
            assert_eq!(Wheel::ALL[wheel.index()], wheel);
        }

        assert!(Wheel::FrontLeft.is_front() && Wheel::FrontLeft.is_left());
        assert!(Wheel::FrontRight.is_front() && !Wheel::FrontRight.is_left());
        assert!(!Wheel::RearLeft.is_front() && Wheel::RearLeft.is_left());
        assert!(!Wheel::RearRight.is_front() && !Wheel::RearRight.is_left());
    }

    #[test]
    fn default_spec_is_symmetric_and_rwd() {
        let spec = VehicleSpec::default();

        assert_eq!(spec.mass_kg, 1200.0);
        assert_eq!(spec.wheels.len(), 4);

        let fl = &spec.wheels[Wheel::FrontLeft.index()];
        let fr = &spec.wheels[Wheel::FrontRight.index()];
        let rl = &spec.wheels[Wheel::RearLeft.index()];
        let rr = &spec.wheels[Wheel::RearRight.index()];

        // 前后轴距 = wheelbase；左右对称。
        assert_eq!(fl.position.z, -spec.wheelbase_m * 0.5);
        assert_eq!(rr.position.z, spec.wheelbase_m * 0.5);
        assert_eq!(fl.position.x, -fr.position.x);
        assert_eq!(rl.position.x, -rr.position.x);

        // RWD：前轮转向不驱动，后轮驱动不转向。
        assert!(fl.steerable && !fl.driven);
        assert!(fr.steerable && !fr.driven);
        assert!(!rl.steerable && rl.driven);
        assert!(!rr.steerable && rr.driven);
        assert_eq!(spec.drivetrain.layout, DrivetrainLayout::Rwd);

        // 四个轮子共用合理的默认值。
        for wheel in &spec.wheels {
            assert!(wheel.radius_m > 0.0);
            assert_eq!(
                wheel.suspension.spring_rate_n_per_m,
                SuspensionSpec::default().spring_rate_n_per_m
            );
        }
    }

    #[test]
    fn pose_forward_points_along_negative_z_when_identity() {
        let pose = Pose::default();
        assert_eq!(pose.forward(), Vec3::NEG_Z);

        // 绕 Y 转 90°：-Z 前向变成 -X。
        let turned = Pose {
            orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..Pose::default()
        };
        let forward = turned.forward();
        assert!(forward.x < -0.99 && forward.z.abs() < 1e-6 && forward.y.abs() < 1e-6);
    }

    #[test]
    fn engine_defaults_cover_a_sane_gearbox() {
        let engine = EngineSpec::default();
        assert_eq!(engine.gear_ratios.len(), 6);
        // 齿比必须逐级递减（每个更高的档位都是减速更少的超比档）。
        for pair in engine.gear_ratios.windows(2) {
            assert!(pair[0] > pair[1], "{:?} must descend", engine.gear_ratios);
        }
        assert!(engine.final_drive > 0.0);
        assert!(engine.idle_rpm < engine.redline_rpm);
    }
}
