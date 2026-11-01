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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_normalized_axes_into_range() {
        let controls = Controls {
            throttle: 5.0,
            brake: -2.0,
            clutch: f32::INFINITY,
            steering: -3.0,
            ..Controls::default()
        }
        .clamped();

        assert_eq!(controls.throttle, 1.0);
        assert_eq!(controls.brake, 0.0);
        assert_eq!(controls.clutch, 1.0);
        assert_eq!(controls.steering, -1.0);
    }

    #[test]
    fn keeps_in_range_values_and_toggles_untouched() {
        let controls = Controls {
            throttle: 0.75,
            steering: 0.5,
            handbrake: true,
            shift_up: true,
            ..Controls::default()
        }
        .clamped();

        assert_eq!(controls.throttle, 0.75);
        assert_eq!(controls.steering, 0.5);
        // 换挡/手刹是布尔量，不属于归一化轴，clamped() 不能动它们。
        assert!(controls.handbrake);
        assert!(controls.shift_up);
        assert!(!controls.shift_down);
    }

    #[test]
    fn clamped_is_idempotent() {
        let once = Controls {
            throttle: 9.0,
            steering: -0.25,
            ..Controls::default()
        }
        .clamped();
        assert_eq!(once.clamped(), once);
    }
}
