use serde::{Deserialize, Serialize};

/// Drag-reduction system, from AC `aero.ini`'s DRS-capable rear wings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrsSpec {
    /// Whether the car has one at all.
    pub available: bool,
    /// Fraction of the drag the open flap removes, `0..=1`.
    pub drag_reduction: f32,
    /// Downforce multiplier with the flap open, `0..=1`.
    pub downforce_multiplier_open: f32,
}

impl Default for DrsSpec {
    fn default() -> Self {
        Self {
            available: false,
            drag_reduction: 0.12,
            downforce_multiplier_open: 0.85,
        }
    }
}

impl DrsSpec {
    /// Drag multiplier for a requested DRS state, `0..=1`.
    pub fn drag_multiplier(&self, requested: bool) -> f32 {
        if self.available && requested {
            (1.0 - self.drag_reduction.clamp(0.0, 1.0)).max(0.0)
        } else {
            1.0
        }
    }
}
