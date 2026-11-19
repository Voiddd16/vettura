use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Fuel tank, from AC `car.ini [FUEL]` / `[FUEL_EXT]` / `[FUELTANK]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuelSpec {
    /// Tank capacity, litres.
    pub capacity_l: f32,
    /// Fraction of the tank the car starts with, `0..=1`.
    pub initial_fraction: f32,
    /// Fuel density, kg/litre.
    pub density_kg_per_l: f32,
    /// Distance covered per litre at full load, km.
    pub km_per_liter: f32,
    /// Fuel consumption at full load, litres per second.
    pub consumption_l_per_s_at_full_load: f32,
    /// Tank position relative to the centre of mass, physics frame.
    pub position: Vec3,
}

impl Default for FuelSpec {
    fn default() -> Self {
        Self {
            capacity_l: 80.0,
            initial_fraction: 1.0,
            density_kg_per_l: 0.74,
            km_per_liter: 2.5,
            consumption_l_per_s_at_full_load: 0.0111,
            position: Vec3::new(0.0, 0.02, -0.1),
        }
    }
}

impl FuelSpec {
    /// Mass of a full tank, kg.
    pub fn full_mass_kg(&self) -> f32 {
        self.mass_at_fraction(1.0)
    }

    /// Fuel mass carried at a fraction of the tank, kg.
    pub fn mass_at_fraction(&self, fraction: f32) -> f32 {
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.capacity_l.max(0.0) * fraction * self.density_kg_per_l.max(0.0)
    }

    /// Fuel burnt per second at a road speed and engine load, litres.
    ///
    /// Two terms: a constant load term and a term that grows with speed,
    /// because pumping losses dominate at low engine load and drag does at
    /// high road speed.
    pub fn consumption_l_per_s(&self, speed_mps: f32, load: f32) -> f32 {
        if self.capacity_l <= 0.0 {
            return 0.0;
        }
        let speed = if speed_mps.is_finite() {
            speed_mps.abs()
        } else {
            0.0
        };
        let load = if load.is_finite() {
            load.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let full = self.consumption_l_per_s_at_full_load.max(0.0);
        if full <= 0.0 {
            return 0.0;
        }
        // At 25 m/s the car covers the same distance as the full-load figure;
        // slower than that the engine is only partly loaded.
        let speed_term = (0.25 + 0.75 * (speed / 25.0).clamp(0.0, 1.0)).clamp(0.25, 1.0);
        full * load * speed_term
    }
}
