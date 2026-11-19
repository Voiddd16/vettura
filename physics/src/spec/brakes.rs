use serde::{Deserialize, Serialize};

/// Brake parameters from AC `brakes.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrakeSpec {
    /// Peak brake torque, N·m, applied to one axle per `front_share`.
    pub max_torque_nm: f32,
    /// Front share of the brake torque, `0..=1`.
    pub front_share: f32,
    /// Heat capacity of one disc + pad set, J/K.
    pub thermal_mass_j_per_k: f32,
    /// Cooling rate at the reference speed, 1/s.
    pub cooling_per_s: f32,
    /// Ambient reference temperature, °C.
    pub ambient_c: f32,
    /// Temperature at which the disc reaches full efficiency, °C.
    pub optimal_c: f32,
    /// Temperature at which the disc has faded to `min_efficiency`, °C.
    pub fade_c: f32,
    /// Efficiency at and beyond `fade_c`.
    pub min_efficiency: f32,
    /// Heat generated per joule of dissipated braking energy.
    pub heat_per_joule: f32,
    /// Heat capacity of the rotor's core, J/K. The friction surface heats
    /// almost instantly, the bulk of the disc does not, and that lag is what
    /// makes brake heat soak: the pads keep cooking long after the driver lifts
    /// off. `0` disables the core, keeping the old single-mass behaviour.
    pub core_capacity_j_per_k: f32,
    /// Rate heat moves from the hot surface into the core, 1/s. AC's
    /// `[TEMPS_*] CORE_TRANSFER_IN`.
    pub core_transfer_in_per_s: f32,
    /// Rate heat moves back out of the core into the surface, 1/s. AC's
    /// `CORE_TRANSFER_OUT`.
    pub core_transfer_out_per_s: f32,
    /// Rate the core sheds heat to the air, 1/s. AC's `CORE_TRANSFER_AIR`.
    pub core_transfer_air_per_s: f32,
}

impl Default for BrakeSpec {
    fn default() -> Self {
        Self {
            max_torque_nm: 4_500.0,
            front_share: 0.55,
            thermal_mass_j_per_k: 1_200.0,
            cooling_per_s: 0.06,
            ambient_c: 32.0,
            optimal_c: 500.0,
            fade_c: 900.0,
            min_efficiency: 0.5,
            heat_per_joule: 0.7,
            // A meaningful default so the soak is part of the model; AC data
            // overrides the rates.
            core_capacity_j_per_k: 1_500.0,
            core_transfer_in_per_s: 0.09,
            core_transfer_out_per_s: 0.55,
            core_transfer_air_per_s: 5.0e-4,
        }
    }
}

impl BrakeSpec {
    /// Brake efficiency at a disc temperature, `0..=1`.
    ///
    /// The curve has three regimes: cold discs bite less than they should, an
    /// optimal window delivers the full torque, and past `fade_c` the torque
    /// falls linearly to `min_efficiency`.
    pub fn efficiency_at(&self, temperature_c: f32) -> f32 {
        let temperature = if temperature_c.is_finite() {
            temperature_c.max(self.ambient_c)
        } else {
            self.optimal_c
        };
        if temperature < self.optimal_c {
            // Cold: less than the full torque, but never nothing.
            let cold = ((temperature - self.ambient_c)
                / (self.optimal_c - self.ambient_c).max(1.0))
            .clamp(0.0, 1.0);
            (self.min_efficiency + (1.0 - self.min_efficiency) * cold).clamp(0.0, 1.0)
        } else if temperature < self.fade_c {
            1.0
        } else {
            let faded =
                ((temperature - self.fade_c) / (self.fade_c * 0.5).max(1.0)).clamp(0.0, 1.0);
            (1.0 - (1.0 - self.min_efficiency) * faded).clamp(self.min_efficiency, 1.0)
        }
    }
}
