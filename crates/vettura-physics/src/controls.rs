use serde::{Deserialize, Serialize};

/// Driver inputs. All axes are normalized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    /// `0..=1`
    pub throttle: f32,
    /// `0..=1`
    pub brake: f32,
    /// `0..=1`
    pub clutch: f32,
    /// `-1..=1`, positive = left
    pub steering: f32,
    pub handbrake: bool,
    pub shift_up: bool,
    pub shift_down: bool,
}

impl Controls {
    pub fn clamped(mut self) -> Self {
        self.throttle = self.throttle.clamp(0.0, 1.0);
        self.brake = self.brake.clamp(0.0, 1.0);
        self.clutch = self.clutch.clamp(0.0, 1.0);
        self.steering = self.steering.clamp(-1.0, 1.0);
        self
    }
}
