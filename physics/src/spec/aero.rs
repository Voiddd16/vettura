use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::spec::constants::{AERO_REFERENCE_SPEED_MPS, sample_curve};

/// One aero surface from AC `aero.ini [WING_n]`.
///
/// AC computes each surface's load from two lookup tables: `LUT_AOA_CL` gives
/// the lift coefficient at an angle of attack, and `LUT_GH_CL` scales it by the
/// surface's ground height. That is what makes a ground-effect car sensitive to
/// ride height and what lets a wing stall, so the tables are carried here
/// instead of being collapsed into one `Cl·A` number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WingSpec {
    /// Chord, metres.
    pub chord_m: f32,
    /// Span, metres.
    pub span_m: f32,
    /// `CL_GAIN` as authored, dimensionless, the fallback lift coefficient
    /// when the car ships no `LUT_AOA_CL`.
    pub cl_gain: f32,
    /// `CD_GAIN` as authored, the fallback drag coefficient.
    pub cd_gain: f32,
    /// Mount position in the AC frame (+X right, +Y up, +Z forward).
    pub position: Vec3,
    /// Installation angle, degrees. AC's `ANGLE`.
    pub angle_deg: f32,
    /// `(angle of attack in degrees, lift coefficient)` from `LUT_AOA_CL`.
    pub aoa_cl: Vec<(f32, f32)>,
    /// `(ground height in metres, lift multiplier)` from `LUT_GH_CL`.
    pub gh_cl: Vec<(f32, f32)>,
    /// `(angle of attack in degrees, drag coefficient)` from `LUT_AOA_CD`.
    pub aoa_cd: Vec<(f32, f32)>,
    /// `(ground height in metres, drag multiplier)` from `LUT_GH_CD`.
    pub gh_cd: Vec<(f32, f32)>,
}

impl Default for WingSpec {
    fn default() -> Self {
        Self {
            chord_m: 0.0,
            span_m: 0.0,
            cl_gain: 0.0,
            cd_gain: 0.0,
            position: Vec3::ZERO,
            angle_deg: 0.0,
            aoa_cl: Vec::new(),
            gh_cl: Vec::new(),
            aoa_cd: Vec::new(),
            gh_cd: Vec::new(),
        }
    }
}

impl WingSpec {
    /// Planform area, m².
    pub fn area_m2(&self) -> f32 {
        (self.chord_m * self.span_m).max(0.0)
    }

    /// `Cl·A` at an angle of attack and a ground height, m².
    ///
    /// The angle of attack is the installation angle plus whatever the flow
    /// gives it, and the height term is what the AC tables scale it by. Without
    /// the tables the authored `CL_GAIN` is used unchanged.
    pub fn cl_area(&self, aoa_deg: f32, ground_height_m: f32) -> f32 {
        let area = self.area_m2();
        let lift = if self.aoa_cl.len() >= 2 {
            sample_curve(&self.aoa_cl, aoa_deg)
        } else {
            self.cl_gain
        };
        if self.gh_cl.len() >= 2 {
            lift * sample_curve(&self.gh_cl, ground_height_m) * area
        } else {
            lift * area
        }
    }

    /// `Cd·A` at an angle of attack and a ground height, m².
    pub fn cd_area(&self, aoa_deg: f32, ground_height_m: f32) -> f32 {
        let area = self.area_m2();
        let drag = if self.aoa_cd.len() >= 2 {
            sample_curve(&self.aoa_cd, aoa_deg)
        } else {
            self.cd_gain
        };
        if self.gh_cd.len() >= 2 {
            drag * sample_curve(&self.gh_cd, ground_height_m) * area
        } else {
            drag * area
        }
    }
}

/// Aerodynamic parameters summed from AC `aero.ini`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AeroSpec {
    /// Total downforce coefficient `Cl·A`. Applied along the car's local down
    /// axis and scaled by `speed²/(speed² + reference²)` so a parked car is
    /// not pulled into the ground. Aggregate fallback for cars without the
    /// per-wing tables; [`Self::wings`] is the real model.
    pub downforce_coeff: f32,
    /// Front share of the downforce, `0` = all rear, `1` = all front.
    pub front_bias: f32,
    /// Downforce lost over full suspension travel, `0..=1`. Above the rest
    /// height the car makes less downforce, which models ride-height
    /// sensitivity.
    pub ride_height_fade: f32,
    /// Lateral lift coefficient per radian of sideslip, `Clβ·A`. The car gets
    /// a side force whenever it is travelling sideways relative to its
    /// heading, which is what makes a fast car feel planted and what gives a
    /// real car its aerodynamic yaw stability.
    pub side_force_coeff: f32,
    /// Distance behind the centre of mass where that side force acts, metres.
    /// Acting behind the CG turns the side force into a restoring yaw moment,
    /// the same way a weathervane points into the wind.
    pub center_of_pressure_m: f32,
    /// Per-surface data from `aero.ini`, empty for hand-authored specs.
    pub wings: Vec<WingSpec>,
    /// Downforce lost directly behind another car, `0..=1`. `0` disables the
    /// wake model, which is the case for hand-authored specs.
    pub dirty_air_loss: f32,
    /// Distance at which the wake stops mattering, metres.
    pub dirty_air_distance_m: f32,
}

impl Default for AeroSpec {
    fn default() -> Self {
        Self {
            downforce_coeff: 0.0,
            front_bias: 0.4,
            ride_height_fade: 0.25,
            side_force_coeff: 0.0,
            center_of_pressure_m: 0.6,
            wings: Vec::new(),
            // Off until the car authors it; a real aero map loses a real
            // fraction of its load in traffic.
            dirty_air_loss: 0.0,
            dirty_air_distance_m: 25.0,
        }
    }
}

impl AeroSpec {
    /// Lateral force for a sideslip of `sideslip_rad`, newtons.
    ///
    /// The force always opposes the sideslip, so it pushes the car back
    /// towards travelling straight ahead. The `speed²/(speed²+ref²)` term
    /// keeps a stationary car from being shoved sideways by a breeze.
    pub fn side_force_n(&self, air_density_kg_m3: f32, speed_mps: f32, sideslip_rad: f32) -> f32 {
        if !sideslip_rad.is_finite() || sideslip_rad == 0.0 {
            return 0.0;
        }
        let speed_sq = if speed_mps.is_finite() {
            speed_mps * speed_mps
        } else {
            0.0
        };
        let smoothing = speed_sq / (speed_sq + AERO_REFERENCE_SPEED_MPS * AERO_REFERENCE_SPEED_MPS);
        // Small-angle `sin` is fine: sideslip beyond ~30° is already a spin.
        let sine = if sideslip_rad.abs() < 0.5 {
            sideslip_rad
        } else {
            sideslip_rad.clamp(-1.5, 1.5).sin()
        };
        // The force is proportional to the sideslip, not just a constant:
        // a small slip gets a small correction, a big one a strong one.
        let magnitude =
            0.5 * air_density_kg_m3 * self.side_force_coeff.max(0.0) * speed_sq * smoothing;
        (-magnitude * sine).clamp(-50_000.0, 50_000.0)
    }

    /// Downforce multiplier from sitting in another car's wake, `0..=1`.
    ///
    /// `exposure` is how deep in the wake this car sits, `0..=1`: `0` when
    /// there is no car ahead and `1` when it is right on the gearbox.
    /// The loss peaks immediately behind the car and fades over
    /// `dirty_air_distance_m`, which is why following is hard and why the
    /// second car in a train often carries more speed into a corner.
    pub fn dirty_air_multiplier(&self, exposure: f32) -> f32 {
        let exposure = if exposure.is_finite() {
            exposure.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (1.0 - self.dirty_air_loss.clamp(0.0, 1.0) * exposure).clamp(0.0, 1.0)
    }

    /// Yaw moment the side force produces at `center_of_pressure_m`, N·m.
    pub fn side_force_yaw_moment_nm(
        &self,
        air_density_kg_m3: f32,
        speed_mps: f32,
        sideslip_rad: f32,
    ) -> f32 {
        self.side_force_n(air_density_kg_m3, speed_mps, sideslip_rad) * self.center_of_pressure_m
    }
}
