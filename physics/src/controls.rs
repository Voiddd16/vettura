use serde::{Deserialize, Serialize};

use crate::spec::ErsMode;

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
    /// Drag-reduction system request. Only effective when the car's
    /// [`DrsSpec`](crate::DrsSpec) has one.
    pub drs: bool,
    /// Anti-lock braking system request. Only effective when the car has one,
    /// which the world models from measured wheel slip.
    pub abs: bool,
    /// Traction control request. Only effective when the car has one, likewise
    /// derived from measured wheel slip.
    pub tc: bool,
    /// Hybrid energy mode. Only effective when the car has an [`ErsSpec`](crate::ErsSpec).
    pub ers: ErsMode,
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

/// 降挡补油（rev-match / throttle blip）。
///
/// 真实变速箱在降挡时会瞬间开一段油门，把发动机转速拉到与新挡位匹配，离合器
/// 接合时才不会把车轮拽一把。Jolt 的公开 API 不让你直接设定发动机转速
/// （见 `docs/physics.md` 的已知边界），所以这里做驾驶输入层面的等价物：换挡
/// 窗口内给一段从 `peak` 线性衰减到 0 的额外油门，与驾驶员的油门取较大值。
///
/// 设计上与其它子系统一致：默认参数不改变既有行为，只有 AC 数据打开
/// （`GearboxSpec::clutch_on_shifts`）或者显式调用 [`Self::start`] 才生效。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShiftBlip {
    /// 剩余补油时间，秒。
    remaining_s: f32,
    /// 本次补油总时长，秒。
    total_s: f32,
    /// 补油峰值（归一化油门）。
    peak: f32,
}

impl Default for ShiftBlip {
    fn default() -> Self {
        Self {
            remaining_s: 0.0,
            total_s: 0.0,
            peak: 0.0,
        }
    }
}

impl ShiftBlip {
    /// 默认补油窗口与峰值，对应真实序列式变速箱的量级。
    pub const DEFAULT_WINDOW_S: f32 = 0.12;
    pub const DEFAULT_PEAK: f32 = 0.7;

    pub fn new() -> Self {
        Self::default()
    }

    /// 开始一次补油；时长/峰值为 0 或负数时视为“不补油”。
    pub fn start(&mut self, window_s: f32, peak: f32) {
        let window_s = if window_s.is_finite() {
            window_s.max(0.0)
        } else {
            0.0
        };
        let peak = if peak.is_finite() {
            peak.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.total_s = window_s;
        self.peak = peak;
        // 已在补油中则延长窗口，而不是从头开始（连续降挡时补油不中断）。
        self.remaining_s = self.remaining_s.max(window_s);
    }

    /// 当前是否还在补油窗口内。
    pub fn is_active(&self) -> bool {
        self.remaining_s > 0.0 && self.peak > 0.0
    }

    /// 补油窗口内的当前油门量（线性衰减）。
    pub fn throttle(&self) -> f32 {
        if !self.is_active() || self.total_s <= 0.0 {
            return 0.0;
        }
        self.peak * (self.remaining_s / self.total_s).clamp(0.0, 1.0)
    }

    /// 与驾驶员油门合并：补油永远比驾驶员当前油门更“想知道转速”，所以取大。
    pub fn merged(&self, driver_throttle: f32) -> f32 {
        let driver_throttle = if driver_throttle.is_finite() {
            driver_throttle.clamp(0.0, 1.0)
        } else {
            0.0
        };
        driver_throttle.max(self.throttle())
    }

    /// 推进计时器。
    pub fn tick(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.remaining_s = (self.remaining_s - dt).max(0.0);
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
    fn keeps_the_driver_aid_toggles_untouched() {
        let controls = Controls {
            abs: true,
            tc: true,
            ..Controls::default()
        }
        .clamped();
        assert!(controls.abs, "ABS switch is not a normalized axis");
        assert!(controls.tc, "TC switch is not a normalized axis");
        assert!(!Controls::default().abs);
        assert!(!Controls::default().tc);
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

    /// 默认状态：没有补油，合并任何油门都原样返回。
    #[test]
    fn default_blip_does_not_touch_the_throttle() {
        let blip = ShiftBlip::new();
        assert!(!blip.is_active());
        assert_eq!(blip.throttle(), 0.0);
        assert_eq!(blip.merged(0.3), 0.3);
        assert_eq!(blip.merged(-1.0), 0.0);
    }

    /// 补油窗口内油门从峰值线性衰减，窗口结束后归零。
    #[test]
    fn blip_decays_linearly_then_stops() {
        let mut blip = ShiftBlip::new();
        blip.start(0.1, 0.8);

        assert!(blip.is_active());
        assert!((blip.throttle() - 0.8).abs() < 1.0e-6);

        blip.tick(0.05);
        let half = blip.throttle();
        assert!((half - 0.4).abs() < 1.0e-6, "{half}");

        // 补油永远盖过驾驶员的小油门，但盖不过驾驶员的全油门。
        assert!((blip.merged(0.1) - 0.4).abs() < 1.0e-6);
        assert_eq!(blip.merged(0.9), 0.9);

        blip.tick(0.05);
        assert!(!blip.is_active());
        assert_eq!(blip.throttle(), 0.0);
        assert_eq!(blip.merged(0.2), 0.2);
    }

    /// 连续降挡：第二次补油延长窗口，不会把已有补油清零。
    #[test]
    fn repeated_downshifts_keep_the_blip_running() {
        let mut blip = ShiftBlip::new();
        blip.start(0.2, 0.5);
        blip.tick(0.15);
        let before = blip.throttle();
        assert!(before > 0.0);

        blip.start(0.2, 0.5);
        assert!(blip.remaining_s >= before);
        assert!(blip.throttle() >= before);
    }

    /// 非法参数不能把油门推到 1 以上或产生 NaN。
    #[test]
    fn blip_rejects_nonsense_parameters() {
        let mut blip = ShiftBlip::new();
        blip.start(f32::NAN, f32::INFINITY);
        assert!(!blip.is_active());

        blip.start(-1.0, -1.0);
        assert!(!blip.is_active());

        blip.start(0.1, 4.0);
        assert!(blip.throttle() <= 1.0);
        assert!(blip.merged(f32::NAN) <= 1.0);
    }
}
