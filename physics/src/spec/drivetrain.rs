use serde::{Deserialize, Serialize};

/// Which axles are driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrivetrainLayout {
    Rwd,
    Fwd,
    Awd,
}

/// Drivetrain layout and differential, from AC `drivetrain.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivetrainSpec {
    pub layout: DrivetrainLayout,
    pub efficiency: f32,
    pub diff: DiffSpec,
}

impl Default for DrivetrainSpec {
    fn default() -> Self {
        Self {
            layout: DrivetrainLayout::Rwd,
            efficiency: 0.9,
            diff: DiffSpec::default(),
        }
    }
}

/// Limited-slip differential, from AC `drivetrain.ini [DIFFERENTIAL]`.
///
/// AC expresses the lock as a fraction: `1.0` fully locked, `0` open. Jolt
/// instead asks for the wheel-speed ratio above which all torque goes to the
/// slower wheel, which is the reciprocal (see `vettura-physics`'s world layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSpec {
    /// Lock under power, `0..=1`.
    pub power_lock: f32,
    /// Lock under coasting, `0..=1`.
    pub coast_lock: f32,
    /// Preload torque, N·m.
    pub preload_nm: f32,
}

impl Default for DiffSpec {
    fn default() -> Self {
        // A mod that does not author the section keeps a locked diff, which is
        // the safer behaviour for a car we know nothing about.
        Self {
            power_lock: 1.0,
            coast_lock: 1.0,
            preload_nm: 0.0,
        }
    }
}

impl DiffSpec {
    /// Lock ratio averaged between power and coast.
    pub fn average_lock(&self) -> f32 {
        0.5 * (self.power_lock + self.coast_lock)
    }
}

/// Gearbox and clutch parameters from AC `drivetrain.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GearboxSpec {
    /// Whether the box shifts on its own.
    pub automatic: bool,
    /// Time a gear shift takes, seconds (`CHANGE_UP_TIME`).
    pub switch_time: f32,
    /// Time the clutch takes to engage after a shift, seconds
    /// (`CHANGE_DN_TIME`).
    pub clutch_release_time: f32,
    /// Whether the box blips the throttle on downshifts.
    pub clutch_on_shifts: bool,
    /// Rev-match window, rpm.
    pub rev_match_rpm: f32,
    /// AC `[AUTO_SHIFTER] UP`：自动升挡的绝对转速。
    ///
    /// 0 表示未写，由后端按红线推导。Jolt 原来固定用 94% 红线；对 992 GT3
    /// R 这类 1 挡极速刚好卡在 94% 红线附近的车，会一直升不上 2 挡。
    pub auto_shift_up_rpm: f32,
    /// Torque the clutch can carry, N·m.
    pub clutch_max_torque_nm: f32,
    /// Rotational inertia of the gearbox internals, kg·m².
    pub inertia: f32,
    /// Reverse gear ratio (positive; the sign is applied by the backend).
    /// AC's `GEAR_R`; `0` falls back to the default reverse ratio.
    pub reverse_ratio: f32,
    /// Time the fuel is cut during an upshift, seconds. AC's
    /// `AUTO_CUTOFF_TIME`; `0` disables the cut-off in backends that model it.
    pub auto_cutoff_time: f32,
    /// Handbrake torque at one rear wheel, N·m. AC's `HANDBRAKE_TORQUE`.
    pub handbrake_torque_nm: f32,
}

impl Default for GearboxSpec {
    fn default() -> Self {
        Self {
            automatic: true,
            switch_time: 0.12,
            clutch_release_time: 0.2,
            clutch_on_shifts: true,
            rev_match_rpm: 1_200.0,
            auto_shift_up_rpm: 0.0,
            clutch_max_torque_nm: 1_150.0,
            inertia: 0.012,
            reverse_ratio: 0.0,
            auto_cutoff_time: 0.0,
            handbrake_torque_nm: 0.0,
        }
    }
}
