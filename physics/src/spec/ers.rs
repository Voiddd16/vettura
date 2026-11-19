use serde::{Deserialize, Serialize};

/// Hybrid energy system, from AC's ERS-capable cars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErsSpec {
    /// Whether the car has one at all.
    pub available: bool,
    /// Battery capacity, joules.
    pub capacity_j: f32,
    /// Peak deploy power, watts.
    pub max_deploy_w: f32,
    /// Peak harvest power, watts.
    pub max_harvest_w: f32,
    /// Deploy energy allowed per lap, joules.
    pub lap_deploy_limit_j: f32,
    /// How much of the deploy goes to the rear axle, `0..=1`.
    pub rear_bias: f32,
    /// Motor efficiency when deploying, `0..=1`.
    pub deploy_efficiency: f32,
    /// Motor efficiency when harvesting, `0..=1`.
    pub harvest_efficiency: f32,
}

impl Default for ErsSpec {
    fn default() -> Self {
        Self {
            available: false,
            capacity_j: 4.0e6,
            max_deploy_w: 120_000.0,
            max_harvest_w: 400_000.0,
            lap_deploy_limit_j: 4.0e6,
            rear_bias: 1.0,
            deploy_efficiency: 0.95,
            harvest_efficiency: 0.85,
        }
    }
}

/// What the driver asks of the hybrid system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErsMode {
    /// Nothing.
    #[default]
    Off,
    /// Deploy on demand.
    Deploy,
    /// Full-power deploy for an attack.
    Attack,
    /// Harvest on demand.
    Harvest,
}

impl ErsMode {
    /// Whether this mode asks the motor to deploy (`Attack` deploys harder).
    pub fn deploys(self) -> bool {
        matches!(self, ErsMode::Deploy | ErsMode::Attack)
    }

    /// Whether this mode asks the motor to harvest.
    pub fn harvests(self) -> bool {
        matches!(self, ErsMode::Harvest)
    }
}
