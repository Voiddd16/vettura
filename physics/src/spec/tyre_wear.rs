use serde::{Deserialize, Serialize};

/// Tyre wear model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreWearSpec {
    /// Slip energy that wears the tyre out, joules.
    pub energy_capacity_j: f32,
    /// Grip lost at full wear, `0..=1`.
    pub grip_loss: f32,
}

impl Default for TyreWearSpec {
    fn default() -> Self {
        Self {
            energy_capacity_j: 3.0e7,
            grip_loss: 0.12,
        }
    }
}
