use serde::{Deserialize, Serialize};

use crate::spec::constants::sample_curve;

/// Thermal model for one tyre.
///
/// A real tyre is three masses, not one: the **surface** the road touches heats
/// in milliseconds, the **patch** under the tread is the middle layer, and the
/// **core** is the bulk that lags and is what the driver feels as "the tyre is
/// up to temperature". AC publishes the rates between them
/// (`SURFACE_TRANSFER`, `PATCH_TRANSFER`, `CORE_TRANSFER`,
/// `INTERNAL_CORE_TRANSFER`) and the grip-versus-temperature curve
/// (`PERFORMANCE_CURVE`). Cars without that data fall back to a single mass
/// that behaves exactly like the model used before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreThermalSpec {
    /// Core (bulk) temperature for peak grip, °C.
    pub optimal_c: f32,
    /// Half width of the working window, °C.
    pub window_c: f32,
    /// Heat capacity of the core, J/K. This is the mass the grip curve reads.
    pub heat_capacity_j_per_k: f32,
    /// Base cooling rate at the reference speed, 1/s.
    pub cooling_per_s: f32,
    /// Fraction of slip power that becomes tyre heat.
    pub heat_share: f32,
    /// Grain gain from AC's `[THERMAL_*] GRAIN_GAIN`.
    pub grain_gain: f32,
    /// Blister gain from AC's `[THERMAL_*] BLISTER_GAIN`.
    pub blister_gain: f32,
    /// Heat capacity of the friction surface, J/K. `0` keeps the single-mass
    /// model, where surface and core are the same temperature.
    pub surface_capacity_j_per_k: f32,
    /// Heat capacity of the patch under the tread, J/K.
    pub patch_capacity_j_per_k: f32,
    /// Rate heat moves from the surface into the patch, 1/s.
    pub surface_transfer_per_s: f32,
    /// Rate heat moves from the patch into the core, 1/s.
    pub patch_transfer_per_s: f32,
    /// Rate heat moves from the core back into the patch, 1/s.
    pub core_transfer_per_s: f32,
    /// Rate heat moves inside the core, 1/s.
    pub internal_core_transfer_per_s: f32,
    /// Grip-versus-temperature curve from AC `[THERMAL_*] PERFORMANCE_CURVE`:
    /// `(temperature in °C, grip multiplier)`. Empty keeps the analytic window.
    pub performance_curve: Vec<(f32, f32)>,
}

impl Default for TyreThermalSpec {
    fn default() -> Self {
        Self {
            optimal_c: 90.0,
            window_c: 80.0,
            heat_capacity_j_per_k: 480.0,
            cooling_per_s: 0.08,
            heat_share: 0.3,
            grain_gain: 0.25,
            blister_gain: 1.5,
            surface_capacity_j_per_k: 0.0,
            patch_capacity_j_per_k: 0.0,
            surface_transfer_per_s: 0.0,
            patch_transfer_per_s: 0.0,
            core_transfer_per_s: 0.0,
            internal_core_transfer_per_s: 0.0,
            performance_curve: Vec::new(),
        }
    }
}

impl TyreThermalSpec {
    /// Whether the car models three separate tyre masses.
    pub fn has_layers(&self) -> bool {
        self.surface_capacity_j_per_k > 1.0
            && (self.surface_transfer_per_s > 0.0 || self.patch_transfer_per_s > 0.0)
    }

    /// Grip multiplier from the core temperature, `0..=1.2`.
    ///
    /// With an authored curve that curve is the answer; otherwise the analytic
    /// window is used.
    pub fn temperature_multiplier(&self, temperature_c: f32) -> f32 {
        let temperature = if temperature_c.is_finite() {
            temperature_c.clamp(-1_000.0, 2_000.0)
        } else {
            self.optimal_c
        };
        if self.performance_curve.len() >= 2 {
            if temperature < self.performance_curve[0].0 {
                // Below the coldest authored point the curve keeps its first
                // slope instead of clamping: a tyre at -20 °C has to be worse
                // than one at 0 °C, not equal to it.
                return self.extrapolate_cold(temperature);
            }
            return sample_curve(&self.performance_curve, temperature).clamp(0.0, 1.2);
        }
        let temp_error = ((temperature - self.optimal_c) / self.window_c).clamp(-1.5, 1.5);
        (1.0 - 0.30 * temp_error * temp_error).clamp(0.0, 1.0)
    }

    /// Grip multiplier below the coldest point of the authored curve.
    fn extrapolate_cold(&self, temperature_c: f32) -> f32 {
        let curve = &self.performance_curve;
        let first = curve[0];
        let second = curve[1];
        let span = (second.0 - first.0).abs().max(1.0e-3);
        let slope = (second.1 - first.1) / span;
        (first.1 + slope * (temperature_c - first.0)).clamp(0.0, 1.2)
    }
}
