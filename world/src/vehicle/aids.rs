//! 驾驶员辅助：ABS 与牵引力控制。
//!
//! 都是**基于上一步快照**的滑移率控制器：判定用车轮真实滑移（有载荷且着地），
//! 低速（<2 m/s）直接关闭——刹停时抱死是正常现象，不能因此松刹车。

use vettura_physics::VehicleSpec;

use super::{DriveInput, VehicleSnapshot};

pub(crate) const ABS_SLIP_TARGET: f32 = 0.14;
/// Slip ratio below which the ABS hands brake pressure back.
const ABS_SLIP_RELEASE: f32 = 0.08;
/// Lowest fraction of the requested brake the ABS ever applies.
///
/// 必须低到能咬住「低载荷轮」：Jolt 的刹车是整车一个标量，压弯内侧轮在
/// 坡面/重量转移下法向载荷可能只剩几百牛，其抱死扭矩（μ·N·r）远低于
/// 全车刹车的 25%——旧下限 0.25 在数学上保证该轮永久锁死（实测 horseshoe
/// 进弯左前轮 slip=-1 持续 1 s 以上，车滚上对角两轮后撞墙）。下限 2% 让
/// 比例控制器真正能收敛到目标滑移率。
pub(crate) const ABS_FACTOR_MIN: f32 = 0.02;
/// Highest fraction of the requested brake the ABS applies.
pub(crate) const ABS_FACTOR_MAX: f32 = 1.0;
/// Rate at which the ABS hands brake pressure back after a release, per
/// second. Cut fast, recover slowly: an instant return re-locks the wheel
/// within one slip build-up (~0.2 s) and turns the controller into a hard
/// bang-bang on the raw pedal.
pub(crate) const ABS_RECOVERY_PER_S: f32 = 1.5;
/// The physics step the ABS recovery rate is integrated on (`World`'s fixed
/// step; `World::step` only ever calls `step_exact` with this).
pub(crate) const ABS_STEP_DT_S: f32 = 1.0 / 240.0;
/// Slip ratio the traction controller aims for.
///
/// 必须低于 Jolt 自己的升挡门槛：`WheeledVehicleController` 在任何驱动轮
/// `mLongitudinalSlip > 0.1` 时禁止升挡（can_shift_up）。若 TC 把滑移率
/// 压在 0.12 附近抖动，全油门起步阶段升挡会被永久封锁——车只能卡在
/// 1 挡、rpm 顶限制器。目标 0.08 给门槛留出抖动余量。
pub(crate) const TC_SLIP_TARGET: f32 = 0.08;
/// Lowest fraction of the requested throttle the TC ever applies.
///
/// 必须允许到接近 0：引擎惯性很小（AC `INERTIA=0.012`），两成油门就足以把
/// 引擎钉在限制器上，粘性离合便持续把能量灌进烧胎——滑移率永远压不下来，
/// Jolt 的升挡门槛（驱动轮 slip > 0.1 禁止升挡）也永远打不开。真牵引力
/// 控制断油就是完全切断，让引擎转速掉回轮速，滑移才能收敛。
const TC_MIN_FACTOR: f32 = 0.0;

/// Apply the closed-loop brake and traction controllers to this step's input.
///
/// Both are the car's own systems, not race rules: they watch the wheel slip
/// they actually produced last step and release brake pressure or cut torque
/// before a wheel locks or spins up. Jolt offers one brake input for the whole
/// car, so the ABS scales that input instead of pulsing a single wheel; the
/// effect on the wheel slip is the same.
///
/// `abs_factor` is the ABS modulation kept between steps (`0..=1`): pressure
/// cuts instantly when any touching wheel over-shoots the slip target and
/// returns at a limited rate once the slip has clearly fallen. Without the
/// memory the controller snaps back to the full pedal the moment slip dips
/// and immediately re-locks the wheel.
pub(crate) fn apply_driver_aids(
    spec: &VehicleSpec,
    input: &mut DriveInput,
    snapshot: &VehicleSnapshot,
    abs_factor: &mut f32,
) {
    // 滑移率以地面速度为分母，静止或爬行时数值虚高（轮子没转就是"满滑移"）。
    // 低速下 ABS/TC 的判定没有意义：刹停时车轮抱死是正常的，不能因此松刹车。
    if snapshot.speed_mps < 2.0 {
        input.abs = false;
        input.tc = false;
    }
    if input.abs && input.brake > 1.0e-3 {
        // 没有着地轮 = 没有测量（三轮着地弹跳/腾空），此刻压力既不加深也
        // 不回收，保持现状——否则"无测量"会被当成"滑移率 0"而回压。
        if let Some(worst) = worst_measured_slip(spec, snapshot, false) {
            if worst > ABS_SLIP_TARGET {
                // 立即减压：减压量正比于超出目标滑移率的程度。
                *abs_factor = (ABS_SLIP_TARGET / worst).clamp(ABS_FACTOR_MIN, ABS_FACTOR_MAX);
            } else if worst < ABS_SLIP_RELEASE {
                // 滑移率明确回落才回压，且回压限速——切得快、回得慢。
                *abs_factor =
                    (*abs_factor + ABS_RECOVERY_PER_S * ABS_STEP_DT_S).min(ABS_FACTOR_MAX);
            }
            // 目标滑移率附近的死区（target..release 之间）保持当前压力，
            // 既不加深也不回收，避免在目标附近来回打摆。
            input.brake *= *abs_factor;
        }
    } else {
        *abs_factor = ABS_FACTOR_MAX;
    }
    if input.tc && input.throttle > 1.0e-3 {
        if let Some(worst) = worst_measured_slip(spec, snapshot, true) {
            if worst > TC_SLIP_TARGET {
                let factor = (TC_SLIP_TARGET / worst).clamp(TC_MIN_FACTOR, 1.0);
                input.throttle *= factor;
            }
        }
    }
}

/// Worst absolute slip ratio of the wheels that are touching the ground,
/// with load. `None` when no wheel offers a measurement, so the controllers
/// can tell "zero slip" from "nothing measured".
fn worst_measured_slip(
    spec: &VehicleSpec,
    snapshot: &VehicleSnapshot,
    driven_only: bool,
) -> Option<f32> {
    let mut worst = 0.0f32;
    let mut measured = false;
    for (index, wheel) in snapshot.wheels.iter().enumerate() {
        if driven_only && !spec.wheels[index].driven {
            continue;
        }
        if !wheel.contact || wheel.load_n <= 0.0 {
            continue;
        }
        measured = true;
        worst = worst.max(wheel.slip_ratio.abs());
    }
    if !measured {
        return None;
    }
    if worst.is_finite() {
        Some(worst)
    } else {
        Some(0.0)
    }
}
