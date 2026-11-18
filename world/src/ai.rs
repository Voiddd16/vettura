//! AI drivers on top of an AC racing line: pure-pursuit/Stanley steering,
//! curvature-based speed planning, traffic awareness and AC difficulty tiers.
//!
//! The line itself (parsing, projection, sampling) lives in `vettura-track`;
//! this module only turns a projection plus a vehicle snapshot into
//! [`DriveInput`]. It deliberately never touches [`World`](crate::World): the
//! caller reports positions and speeds, so the same driver works for a race,
//! a replay and a test fixture.

use glam::Vec3;
use vettura_physics::{ErsMode, VehicleSpec};
use vettura_track::{AiProjection, AiSpline};

use crate::vehicle::{DriveInput, VehicleSnapshot, VehicleTuning, steer_lock_deg};
use crate::world::VehicleView;

/// Tuning for [`AiDriver`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiDriverTuning {
    /// Lookahead time, seconds. Multiplied by current speed.
    pub lookahead_s: f32,
    /// Minimum lookahead distance, metres.
    pub min_lookahead_m: f32,
    /// Maximum lookahead distance, metres.
    pub max_lookahead_m: f32,
    /// Throttle per metre per second of speed error.
    pub throttle_gain: f32,
    /// Brake per metre per second of overspeed.
    pub brake_gain: f32,
    /// Steering feedforward per m/s² of lateral acceleration, radians.
    ///
    /// Ackermann geometry (`atan(wheelbase · κ)`) is only right for a tyre
    /// that makes force at zero slip angle. Above ~0.6 g the Jolt tyres need
    /// several degrees of extra lock to hold the same radius; without this
    /// term the car drifts metres wide before the feedback notices.
    pub understeer_gain: f32,
    /// Heading-error gain of the path feedback.
    pub heading_gain: f32,
    /// Cross-track gain of the path feedback, radians per metre at low speed.
    /// The demand is `atan(gain · lateral / (speed + 3))`, so a fast car does
    /// not saw at the wheel over a small offset.
    pub lateral_gain: f32,
    /// Hard speed cap, metres per second.
    pub max_speed_mps: f32,
    /// Target speed used when the spline has no extras or a zero sample.
    pub fallback_speed_mps: f32,
    /// How far ahead traffic is searched, multiples of the current lookahead.
    pub traffic_lookahead_factor: f32,
    /// Radius around a traffic car that blocks the racing line, metres.
    pub traffic_radius_m: f32,
    /// Lateral shift used to pass a blocking car, metres. Clamped by the
    /// authored track width at the blocking point.
    pub traffic_max_shift_m: f32,
    /// Maximum number of positions sampled along the lookahead corridor.
    /// The real count adapts so the spacing stays near three metres.
    pub traffic_max_samples: usize,
    /// Longitudinal gap kept behind a car that cannot be passed, metres.
    pub traffic_follow_gap_m: f32,
    /// 难度档位，AC 的 AI level：`0`（新手）～[`MAX_AI_SKILL`]（110，Pro）。
    ///
    /// 它不改动上面任何一个旋钮，而是单独决定司机「会多少」：横向抓地
    /// 使用率、刹车时机、预瞄距离、方向可用量、贴线精度和避让幅度。
    /// 默认 [`DEFAULT_AI_SKILL`]（100）就是原本调好的职业水平，0 与 110
    /// 之间是连续的能力梯度。
    pub skill: f32,
}

impl Default for AiDriverTuning {
    fn default() -> Self {
        Self {
            lookahead_s: 0.9,
            min_lookahead_m: 6.0,
            max_lookahead_m: 40.0,
            throttle_gain: 0.25,
            brake_gain: 0.9,
            understeer_gain: 0.0085,
            heading_gain: 0.8,
            lateral_gain: 1.6,
            max_speed_mps: 90.0,
            fallback_speed_mps: 30.0,
            traffic_lookahead_factor: 4.0,
            traffic_radius_m: 2.4,
            traffic_max_shift_m: 2.2,
            traffic_max_samples: 24,
            traffic_follow_gap_m: 6.0,
            skill: DEFAULT_AI_SKILL,
        }
    }
}

/// Half the width of a car, metres. Used to decide whether a pass fits on one
/// side of the racing line.
const TRAFFIC_CAR_HALF_WIDTH_M: f32 = 0.95;

/// Target spacing between traffic samples along the lookahead corridor.
const TRAFFIC_SAMPLE_SPACING_M: f32 = 3.0;

/// Spacing of the speed-planning samples along the braking corridor, metres.
/// 4 m keeps a 250 m horizon at 62 samples per call — trivial against the
/// spline's binary search, yet fine enough to catch a hairpin entry.
const PLAN_SPACING_M: f32 = 4.0;

/// Hard cap of the speed-planning horizon, metres. `v²/2a` at 90 m/s with
/// 7 m/s² planning deceleration is 578 m; 600 m covers a full braking zone
/// (150 four-metre samples per call, still trivial). The cap only guards
/// degenerate specs so a tiny deceleration cannot ask for kilometres.
const PLAN_MAX_HORIZON_M: f32 = 600.0;

/// Floor of the speed-planning deceleration, m/s². Below this a car brakes so
/// badly that planning cannot help; keep braking like a road car at least.
const PLAN_DECEL_MIN_MPS2: f32 = 5.0;

/// Ceiling of the speed-planning deceleration, m/s². Cap independent of how
/// strong a mod's brakes are: the planning must stay conservative.
///
/// The allowed speed at distance `d` is `√(v_target² + 2·a_plan·d)`, so a
/// larger `a_plan` *raises* the allowed speed at a fixed distance — i.e. it
/// makes the AI brake later. Measured braking with ABS cycling and
/// combined-slip understeer delivers only ~5–8 m/s² on this F1 at 20–50 m/s,
/// far below the brake-torque limit, so planning at 15 m/s² arrived at every
/// corner hot and understeered wide. 6 m/s² sits at the bottom of the real
/// envelope, which is what "plan for what the car can do" means.
const PLAN_DECEL_MAX_MPS2: f32 = 10.0;

/// Safety factor applied to the brake-torque-derived planning deceleration.
/// Real braking gains grip from aero downforce that the planning ignores, so
/// planning at ~80 % of the torque capacity makes the AI brake early rather
/// than arrive hot.
const PLAN_DECEL_SAFETY: f32 = 0.8;

/// Share of `peak_friction · g` a planner may spend on lateral load with no
/// downforce.
///
/// Measured on the F1 (peak friction 1.73): a constant-steer circle tops out
/// at ~0.86 g of path acceleration, and at 15 m/s only ~0.60 g. The inner
/// wheels unload completely and the Jolt tyre curve loses force past its peak
/// slip angle, so the usable lateral force is far below `μ·g`. 38 % leaves
/// the feedback loop and the racing line's own curvature noise a margin.
const PLAN_LATERAL_GRIP_SHARE: f32 = 0.38;

/// Ceiling of the tyre-only lateral limit, as a share of `g`.
///
/// A flat road does not give a road-based car more than ~0.6 g of lateral
/// acceleration no matter the tyre compound: the inner wheel lifts and the
/// Jolt curve falls off past the peak slip angle, so the *usable* lateral
/// force is capped by physics, not by the friction coefficient the data
/// quotes. Planning above this ceiling sends any car into a hairpin faster
/// than it can turn. Expressed as a fraction of `g` so the same rule holds
/// for a 600 kg F1 and a 1500 kg road car.
const PLAN_LATERAL_BASE_MAX_G: f32 = 0.55;

/// Floor of the tyre-only lateral limit, as a share of `g`. A surface that
/// barely holds the car (wet grass, ice) must still have a plan.
const PLAN_LATERAL_BASE_MIN_G: f32 = 0.35;

/// Fraction of the computed corner speed the planner actually targets.
///
/// The model is calibrated to the tyre's limit, so driving at exactly that
/// speed leaves nothing for the steering feedback to correct with: the car
/// arrives hot, scrubs a wall and loses time. 5 % off the theoretical corner
/// speed is the usual "planning margin" a racing driver carries too.
const PLAN_SPEED_MARGIN: f32 = 0.95;

/// Share of the aerodynamic downforce that becomes usable lateral grip.
///
/// Calibrated against the only high-speed measurement available: the real F1
/// holds ~1.9 g of *tyre* lateral acceleration at 60–70 m/s on Daytona's
/// 31° banked oval, where `downforce/m` is 24 m/s² and the bank itself adds
/// ~5 m/s². Solving `1.9·g − bank − base` for the share gives ≈ 0.36, so the
/// remaining 64 % is load sensitivity, inner-wheel unload, the friction curve
/// falling past its peak slip angle and suspension geometry.
const PLAN_LATERAL_DOWNFORCE_SHARE: f32 = 0.36;

/// Air density used for the downforce estimate, kg/m³.
const AIR_DENSITY_KG_M3: f32 = 1.225;

/// Aerodynamic downforce at a given speed, newtons.
fn downforce_n(spec: &VehicleSpec, speed_mps: f32) -> f32 {
    0.5 * AIR_DENSITY_KG_M3 * spec.aero.downforce_coeff.max(0.0) * speed_mps.max(0.0).powi(2)
}

/// 轮胎侧偏补偿随气动下压力衰减的比例。
///
/// 低速时轮胎要靠 4–5° 的侧偏角才能攒出 0.8 g，Ackermann 角远远不够；
/// 高速时下压力把轮胎压得更紧，同样横向力只需要很小的侧偏角。实测
/// 50 m/s 的 Daytona 椭圆弯里，固定 0.0085·v²κ 会多打约 4° 方向，车
/// 一路切向弯内、从 bank 上滑下去（y 比 AI 线低 2.5 m），最后撞内侧墙。
/// 用 `mg/(mg+下压力)` 缩放侧偏项：22 m/s 时约 0.74，50 m/s 时约 0.37。
fn understeer_load_scale(spec: &VehicleSpec, speed_mps: f32) -> f32 {
    let weight = spec.mass_kg.max(1.0) * 9.81;
    weight / (weight + downforce_n(spec, speed_mps))
}

/// AC 的 AI 难度默认档：100 是 AC 自己的「标准职业」水平。
pub const DEFAULT_AI_SKILL: f32 = 100.0;
/// AC AI 难度的上界：110 是 AC 社区里著名的「超人类抓地」档。
pub const MAX_AI_SKILL: f32 = 110.0;

/// 一个难度档位对应的驾驶能力剖面。
///
/// AC 的 `0…110` 不是把速度整体缩放，而是分别按下表的维度缩放司机的
/// 能力：低难度司机看得近、打得少、刹得软、压不住线。这样同一台车在
/// 同一个难度梯度上会表现为真实的水平差距，而不只是「快一点/慢一点」。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SkillProfile {
    /// 横向抓地使用倍率，乘在 [`lateral_limit_mps2`] 上。
    pub grip: f32,
    /// 规划减速度倍率，乘在 [`planning_decel_mps2`] 上。
    pub brake: f32,
    /// 曲率预瞄时间倍率。
    pub preview: f32,
    /// 方向盘可用上限，`0..=1` 相对锁角：新手不敢把方向打完。
    pub steer: f32,
    /// 反馈权限倍率：前馈不准时，低难度没有能力修正。
    pub reaction: f32,
    /// 稳态侧偏补偿倍率：低难度不知道该补多少锁。
    pub understeer: f32,
    /// 行驶线晃动幅度，米（0 = 完全贴线）。
    pub wander_m: f32,
    /// 避让横移倍率。
    pub traffic: f32,
    /// 出界后的恢复倍率：大于 1 表示更晚、更保守地爬回线。
    pub recovery: f32,
    /// 油门倍率：低难度出弯不敢全油。
    pub throttle: f32,
    /// 前馈刹车倍率：低难度只会等速度掉下来才踩刹车。
    pub feedforward: f32,
}

/// 归一化难度：`0..=100` 平滑映射到 `0..=1`，`100..=110` 单独给一段
/// 「超人类」加成——AC 里 105 以上 AI 抓地会超出车辆物理极限。
fn skill_ratio(skill: f32) -> (f32, f32) {
    let skill = if skill.is_finite() {
        skill.clamp(0.0, MAX_AI_SKILL)
    } else {
        DEFAULT_AI_SKILL
    };
    let normal = (skill / 100.0).clamp(0.0, 1.0);
    let smooth = normal * normal * (3.0 - 2.0 * normal);
    let pro = ((skill - 100.0).max(0.0) / (MAX_AI_SKILL - 100.0)).clamp(0.0, 1.0);
    (smooth, pro)
}

/// 由难度档位推导 [`SkillProfile`]。
pub(crate) fn skill_profile(skill: f32) -> SkillProfile {
    let (smooth, pro) = skill_ratio(skill);
    // 100 → 1.0，110 → 1.0 + 额外 12% 的「不合理」加成。
    let between = |low: f32, high: f32, extra: f32| low + (high - low) * smooth + extra * pro;
    SkillProfile {
        grip: between(0.52, 1.0, 0.12),
        brake: between(0.45, 1.0, 0.15),
        preview: between(0.65, 1.0, 0.15),
        steer: between(0.55, 1.0, 0.0),
        reaction: between(0.55, 1.0, 0.15),
        understeer: between(0.60, 1.0, 0.05),
        wander_m: (1.0 - smooth) * (1.0 - smooth) * 3.0,
        traffic: between(0.60, 1.0, 0.0),
        recovery: between(1.5, 1.0, -0.15),
        throttle: between(0.65, 1.0, 0.15),
        feedforward: between(0.40, 1.0, 0.15),
    }
}

/// 低难度司机的行驶线晃动，米：把理想线偏移一个只随里程变化的正弦量。
///
/// 用累计里程而不是时间做相位，所以 [`AiDriver`] 不需要积分状态也能复现
/// 同一条「生手路线」——一个在弯心来回摆动的司机，比一个贴线但开得慢的
/// 司机更像低难度 AI。高难度下幅度归零，什么都不改变。
pub(crate) fn line_wander_m(profile: &SkillProfile, along_m: f32) -> (f32, f32) {
    if profile.wander_m <= 1.0e-3 {
        return (0.0, 0.0);
    }
    let slow = (along_m / 37.0) * std::f32::consts::TAU + 1.3;
    let fast = (along_m / 101.0) * std::f32::consts::TAU + 4.2;
    let lateral = slow.sin() + 0.5 * fast.sin();
    let speed = slow.cos();
    (profile.wander_m * lateral, profile.wander_m * speed)
}

/// 曲率预瞄时间，秒。前馈要提前于车，而不是等车身已经偏出去。
const PREVIEW_S: f32 = 0.45;
/// 预瞄距离下限，米。
const PREVIEW_MIN_M: f32 = 7.0;
/// 预瞄距离上限，米。
const PREVIEW_MAX_M: f32 = 28.0;
/// 三点圆估算曲率时的采样间隔，米。
const PREVIEW_SPAN_M: f32 = 5.0;

/// 路面横向单位向量：水平切线 × 路面法向。平坦路面就是 `+X`（右）。
///
/// 倾斜赛道上它是路面平面里的横向方向，所以「比线低 2 m」能换算成真实的
/// 横向偏移，也能算出倾斜替轮胎分担了多少横向加速度。
fn road_right(normal: Vec3, tangent: Vec3) -> Vec3 {
    if normal.length_squared() > 1.0e-6 {
        let right = tangent.cross(normal.normalize());
        if right.length_squared() > 1.0e-6 {
            return right.normalize();
        }
    }
    Vec3::X
}

/// 摩擦圆剩下的纵向抓地份额，`0..=1`。
///
/// `demand` 是横向抓地利用率（`v²κ / a_lat_limit`）。轮胎只有一个抓地
/// 预算：横向用掉 `demand`，纵向只剩 `√(1 − demand²)`（椭圆近似）。刹车和
/// 油门都由它定上限——这就是循迹刹车/加速时收油的物理，也是商业竞速 AI
/// 管理联合滑移的标准做法。
///
/// `share` 是保留比例、`floor` 是地板：真车也永远留一点纵向余量（用来调车
/// 尾、稳定车身），所以地板上 0.15 而不是 0。
fn friction_budget(demand: f32, share: f32, floor: f32) -> f32 {
    let demand = demand.clamp(0.0, 1.0);
    ((1.0 - demand * demand).max(0.0).sqrt() * share + floor).clamp(0.0, 1.0)
}

/// 路面倾斜替轮胎分担的横向加速度，m/s²。
///
/// 内侧低、外侧高（正确倾斜）时重力把车压向弯心；反之外高内低就是缺口，
/// 返回负值由调用方裁掉。平坦路面返回 0，所以这条规则对普通赛道完全无感。
fn bank_assist_mps2(right: Vec3, curvature: f32) -> f32 {
    9.81 * right.y * curvature.signum()
}

/// Speed-dependent lateral acceleration limit, m/s².
///
/// At low speed the tyres set the limit; with aero the same corner can be
/// taken faster, so the planner must not use one constant for a 60 km/h
/// hairpin and a 250 km/h bend. Every term comes from the car's own data
/// (`peak_friction`, `mass_kg`, `downforce_coeff`), so a road car and a
/// downforce single-seater get the same rule and different numbers.
fn lateral_limit_mps2(spec: &VehicleSpec, speed_mps: f32) -> f32 {
    let base = (spec.tyres.peak_friction * 9.81 * PLAN_LATERAL_GRIP_SHARE).clamp(
        9.81 * PLAN_LATERAL_BASE_MIN_G,
        9.81 * PLAN_LATERAL_BASE_MAX_G,
    );
    let speed = speed_mps.max(0.0).min(120.0);
    let dynamic = 0.5 * AIR_DENSITY_KG_M3 * spec.aero.downforce_coeff.max(0.0) * speed * speed
        / spec.mass_kg.max(1.0);
    (base + dynamic * PLAN_LATERAL_DOWNFORCE_SHARE).clamp(base, 30.0)
}

/// Conservative braking deceleration used by speed planning, m/s².
///
/// Derived from the car's own brakes: every wheel receives its axle's torque
/// (`brakes.ini` MAX_TORQUE × front/rear share), so the ground force is
/// `Σ torque / wheel radius` and the deceleration is that over the mass. The
/// safety factor and the floor keep the number on the cautious side: planning
/// must never assume more braking than the car can actually deliver.
fn planning_decel_mps2(spec: &VehicleSpec) -> f32 {
    let peak = spec.brakes.max_torque_nm.max(0.0);
    let share = spec.brakes.front_share.clamp(0.0, 1.0);
    let force = spec
        .wheels
        .iter()
        .enumerate()
        .map(|(index, wheel)| {
            let axle = if index < 2 { share } else { 1.0 - share };
            peak * axle / wheel.radius_m.max(0.05)
        })
        .sum::<f32>();
    let decel = force / spec.mass_kg.max(1.0) * PLAN_DECEL_SAFETY;
    decel.clamp(PLAN_DECEL_MIN_MPS2, PLAN_DECEL_MAX_MPS2)
}

/// Maximum number of traffic entries a driver keeps.
const TRAFFIC_CAPACITY: usize = 32;

impl AiDriverTuning {
    /// Replace non-finite values and clamp every knob to a usable range.
    pub fn clamped(mut self) -> Self {
        self.lookahead_s = sane(self.lookahead_s, 0.9, 0.1, 3.0);
        self.min_lookahead_m = sane(self.min_lookahead_m, 6.0, 1.0, 100.0);
        self.max_lookahead_m = sane(self.max_lookahead_m, 40.0, self.min_lookahead_m, 200.0);
        self.throttle_gain = sane(self.throttle_gain, 0.25, 0.01, 2.0);
        self.brake_gain = sane(self.brake_gain, 0.9, 0.01, 3.0);
        self.understeer_gain = sane(self.understeer_gain, 0.0085, 0.0, 0.05);
        self.heading_gain = sane(self.heading_gain, 0.7, 0.0, 3.0);
        self.lateral_gain = sane(self.lateral_gain, 0.8, 0.0, 4.0);
        self.max_speed_mps = sane(self.max_speed_mps, 90.0, 1.0, 150.0);
        self.fallback_speed_mps = sane(self.fallback_speed_mps, 30.0, 0.0, self.max_speed_mps);
        self.traffic_lookahead_factor = sane(self.traffic_lookahead_factor, 1.5, 0.5, 4.0);
        self.traffic_radius_m = sane(self.traffic_radius_m, 2.4, 1.0, 8.0);
        self.traffic_max_shift_m = sane(self.traffic_max_shift_m, 2.2, 0.0, 5.0);
        self.traffic_max_samples = self.traffic_max_samples.clamp(2, 64);
        self.traffic_follow_gap_m = sane(self.traffic_follow_gap_m, 6.0, 1.0, 30.0);
        self.skill = sane(self.skill, DEFAULT_AI_SKILL, 0.0, MAX_AI_SKILL);
        self
    }
}

/// One other car as seen by an [`AiDriver`].
///
/// The driver never touches the [`World`](crate::World): the caller reports
/// the positions and speeds it already knows, so the same driver works for a
/// race, a replay and a test fixture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiTraffic {
    /// Physics-frame reference position of the other car.
    pub position: Vec3,
    /// Forward speed of the other car, metres per second.
    pub speed_mps: f32,
}

impl AiTraffic {
    pub fn new(position: Vec3, speed_mps: f32) -> Self {
        Self {
            position,
            speed_mps: if speed_mps.is_finite() {
                speed_mps
            } else {
                0.0
            },
        }
    }
}

/// Follows an [`AiSpline`] with curvature feedforward and path feedback.
///
/// Steering is split into a preview-curvature feedforward (Ackermann plus a
/// tyre-slip term) and a Stanley-style feedback on heading and cross-track
/// error. Pure pursuit alone needed metres of offset before it commanded
/// enough lock for the tyres, which made the car run wide in every long
/// corner.
///
/// The driver is deliberately independent of [`World`](crate::World): call
/// [`AiDriver::update_view`] with the latest vehicle view and pass the
/// returned [`DriveInput`] to
/// [`World::set_input`](crate::World::set_input). This keeps AI drivers usable
/// from tests, replays and the Godot bridge without adding hidden state to the
/// world.
#[derive(Clone, Debug)]
pub struct AiDriver {
    line: AiSpline,
    tuning: AiDriverTuning,
    last_projection: Option<AiProjection>,
    traffic: Vec<AiTraffic>,
}

/// The traffic car matched to one sample of the lookahead corridor.
#[derive(Clone, Copy, Debug)]
struct BlockedCar {
    /// Index of the line sample the car was matched to.
    sample_index: usize,
    /// Offset of the car from the line, signed.
    lateral_m: f32,
    /// Offset of the car along the sample tangent, metres.
    longitudinal_m: f32,
    /// Distance from the driver to the sample, metres.
    gap_m: f32,
    /// The driver's own offset from the line.
    ego_lateral_m: f32,
    /// How close the blocking point is, `1` at the driver's nose.
    proximity: f32,
}

impl AiDriver {
    pub fn new(line: AiSpline) -> Self {
        Self::with_tuning(line, AiDriverTuning::default())
    }

    pub fn with_tuning(line: AiSpline, tuning: AiDriverTuning) -> Self {
        Self {
            line,
            tuning: tuning.clamped(),
            last_projection: None,
            traffic: Vec::new(),
        }
    }

    pub fn line(&self) -> &AiSpline {
        &self.line
    }

    /// Report traffic at construction time. Same cap as [`Self::set_traffic`].
    pub fn with_traffic(mut self, traffic: &[AiTraffic]) -> Self {
        self.set_traffic(traffic);
        self
    }

    /// Report the other cars the driver should react to.
    ///
    /// Only [`TRAFFIC_CAPACITY`] entries are kept so a race cannot make a
    /// driver's cost explode; the closest cars matter most, so sort the list
    /// by distance before calling.
    pub fn set_traffic(&mut self, traffic: &[AiTraffic]) {
        self.traffic.clear();
        self.traffic.extend(
            traffic
                .iter()
                .copied()
                .filter(|car| car.position.is_finite()),
        );
        self.traffic.truncate(TRAFFIC_CAPACITY);
    }

    /// Drop every other car: the driver only follows the line again.
    pub fn clear_traffic(&mut self) {
        self.traffic.clear();
    }

    /// The traffic currently reported to the driver.
    pub fn traffic(&self) -> &[AiTraffic] {
        &self.traffic
    }

    pub fn tuning(&self) -> AiDriverTuning {
        self.tuning
    }

    /// 当前难度档位，AC 的 AI level：`0..=110`。
    pub fn skill(&self) -> f32 {
        self.tuning.skill
    }

    /// 运行时改难度档位（AC 的 AI level），越界值按 [`MAX_AI_SKILL`] 钳制。
    ///
    /// 同一个 driver 上直接切档：`0` 是新手，`100` 是职业，`110` 是 AC 社区
    /// 里那个抓地超出物理极限的「不可战胜」档。
    pub fn set_skill(&mut self, skill: f32) {
        self.tuning.skill = if skill.is_finite() {
            skill.clamp(0.0, MAX_AI_SKILL)
        } else {
            DEFAULT_AI_SKILL
        };
    }

    /// 方便链式调用：构造时直接指定难度档位。
    pub fn with_skill(mut self, skill: f32) -> Self {
        self.set_skill(skill);
        self
    }

    /// Last projection computed by [`Self::update`], useful for telemetry.
    pub fn last_projection(&self) -> Option<AiProjection> {
        self.last_projection
    }

    /// Update from a borrowed [`VehicleView`].
    pub fn update_view(&mut self, view: VehicleView<'_>) -> DriveInput {
        self.update(view.snapshot, view.spec, view.tuning)
    }

    /// Compute the next driver input.
    pub fn update(
        &mut self,
        snapshot: &VehicleSnapshot,
        spec: &VehicleSpec,
        tuning: VehicleTuning,
    ) -> DriveInput {
        if !snapshot.reference_position.is_finite() {
            return DriveInput::default();
        }
        let Some(projection) = self.line.project(snapshot.reference_position) else {
            return DriveInput::default();
        };
        self.last_projection = Some(projection);

        let forward = snapshot.forward();
        let forward = Vec3::new(forward.x, 0.0, forward.z);
        let forward = if forward.length_squared() > 1.0e-6 {
            forward.normalize()
        } else {
            Vec3::NEG_Z
        };
        let speed = snapshot
            .forward_speed_mps
            .max(0.0)
            .min(self.tuning.max_speed_mps);
        let lookahead = (speed * self.tuning.lookahead_s)
            .clamp(self.tuning.min_lookahead_m, self.tuning.max_lookahead_m);
        let Some(target) = self.line.sample_at_distance(projection.along_m + lookahead) else {
            return DriveInput::default();
        };

        // Traffic may ask for a lateral offset and cap the target speed.
        let (shift, follow_speed) = self.traffic_avoidance(&projection, lookahead);

        // The car's authored lock, overridable through the tuning exactly as
        // the physics does it.
        let max_steer = steer_lock_deg(spec, &tuning).to_radians().max(0.01);

        // 曲率前馈 + 航向/横向误差反馈（Stanley 形式）。
        //
        // 纯追踪只给 `atan(2L·x/d²)`：默认 `2L/d²` 在大前瞻下只有 0.01 rad/m，
        // 车要偏线 20 m 才攒出弯中需要的 5° 额外锁角。实测 Horseshoe 一路
        // 外漂 10–14 m 才达到稳态。根因是轮胎在 0.6 g 以上需要明显大于
        // Ackermann 的锁角（实测 20.5 m/s、半径 61.6 m 时方向盘角 7.7°，
        // 而运动学只要 3.0°），小增益反馈补不齐这个稳态差。
        //
        // 因此把转向拆成两部分：
        //   1. 预瞄点曲率前馈：Ackermann + 轮胎侧偏补偿 `k·v²κ`；
        //   2. 反馈：航向误差 + 横向误差（低速时横向项更强，高速时减弱）。
        // 前馈负责弯中稳态，反馈只修残余误差，车就不会先漂出去再救。
        // 难度剖面：AC 的 0..110 在这里变成一组能力缩放，而不是速度缩放。
        let skill = skill_profile(self.tuning.skill);
        let preview = (speed * PREVIEW_S * skill.preview).clamp(PREVIEW_MIN_M, PREVIEW_MAX_M);
        let curvature = self.preview_curvature(projection.along_m + preview);
        // 侧偏补偿吃下压力衰减：`v²` 同时出现在“需要多少横向力”和“下压力
        // 给多少抓地”里，不除以总载荷的话高速弯会多打好几度方向。
        let load_scale = understeer_load_scale(spec, speed);
        let feedforward = (spec.wheelbase_m * curvature).atan()
            + self.tuning.understeer_gain
                * speed
                * speed
                * curvature
                * load_scale
                * skill.understeer;

        let heading_error = forward
            .cross(projection.tangent)
            .y
            .atan2(forward.dot(projection.tangent));
        // 低难度司机不贴线：理想线按里程偏一个缓慢变化的正弦量，作为虚拟
        // 线的目标偏移。用里程做相位，所以无状态、可复现。
        let (wander, wander_speed) = line_wander_m(&skill, projection.along_m);
        // 倾斜路面（Daytona oval 的 31° bank）上，水平投影看不见车顺着 bank
        // 滑下去：实测车比线低 1.5 m、横向误差却只有 1 m。横向误差要沿路面
        // 横向（水平切线 × 路面法向）投影，控制器才看得见爬出 bank 的车。
        let surface_lateral = {
            let normal = self
                .line
                .extra(projection.index)
                .map(|extra| extra.normal)
                .filter(|normal| normal.length_squared() > 0.25)
                .map(glam::Vec3::normalize)
                .unwrap_or(Vec3::Y);
            let offset = snapshot.reference_position - projection.position;
            offset.dot(road_right(normal, projection.tangent))
        };
        // 交通避让不是「多打一点方向」：反馈会立刻把车拉回线上。把横移量
        // 当成虚拟线的目标偏移，横向误差相对它计算，车才会真正让出空间。
        let lateral_error = surface_lateral - shift * skill.traffic - wander;
        let mut feedback = self.tuning.heading_gain * heading_error * skill.reaction
            + (self.tuning.lateral_gain * lateral_error / (speed + 3.0)).atan() * skill.reaction;
        // 已经贴着抓地极限时不要再加锁：前轮越过峰值侧偏角只会更推头。
        // 实测 F1 在 15 m/s 满舵稳态半径 46 m，而 60% 锁角反而 40.6 m——
        // 多打的方向全变成了前轮的额外滑移。
        let demand = (speed * speed * curvature.abs() / lateral_limit_mps2(spec, speed).max(1.0))
            .clamp(0.0, 1.0);
        feedback *= 1.0 - 0.5 * demand;
        // 反馈权限限制在 35% 锁角：前馈已经给出弯中所需的大部分转向，
        // 反馈只修几度残余；再大就会越过前轮峰值侧偏角空转。
        let feedback_cap = 0.35 * max_steer * skill.reaction;
        let feedback = feedback.clamp(-feedback_cap, feedback_cap);
        // 方向盘可用量也吃难度：新手只敢用前段行程，Pro 才敢用满锁。
        let steering = ((feedforward + feedback) / max_steer).clamp(-skill.steer, skill.steer);
        // 一条没有 speed 列的线（部分模组的 `ideal_line` 或小型自制赛道）不能
        // 被 fallback 速度拖成全程龟速：那就交给曲率限速和最高速说话。
        let fallback = if self.line.extras().is_empty() {
            self.tuning.max_speed_mps
        } else {
            self.tuning.fallback_speed_mps
        };
        let mut target_speed = target.speed_mps.unwrap_or(fallback);
        if !target_speed.is_finite() || target_speed < 0.5 {
            target_speed = self.tuning.fallback_speed_mps;
        }
        // 刹车感知速度规划：单一 lookahead 点在高速段看不见下一个慢弯——
        // 43 m/s 的车前瞻只有 ≈39 m（`speed × lookahead_s`），等目标点掉进
        // 弯道时已离弯心不到一个车身的刹车距离，重刹 + ABS 兜底也救不回
        // （实测 horseshoe 双倍限速进弯、撞墙搁浅）。这里把「未来一整段
        // 刹车距离内」的授权速度都折算回当前位置取最小：
        // `v_allow(d)² = v_target(d)² + 2·a_plan·d`。
        // 规划减速度吃难度：低难度晚刹（包络松），100 以上（Pro）甚至刹得
        // 比车辆真实能力更狠——AC 里 >100 的 AI 就是这么不讲道理。
        let plan_decel =
            (planning_decel_mps2(spec) * skill.brake).clamp(2.0, PLAN_DECEL_MAX_MPS2 * 1.2);
        let horizon = (speed * speed / (2.0 * plan_decel) + self.tuning.max_lookahead_m)
            .min(PLAN_MAX_HORIZON_M);
        // 现在就必须维持的最小减速度，m/s²。
        //
        // 只看「当前速度 vs 当前包络」永远刹得太晚：912 m 处到 933 m 的
        // 半径 26 m 回头弯之间，包络在 921 m 处还允许 17.6 m/s，等目标速度
        // 掉到车速以下时离弯心只剩 8 m，车于是带着 15 m/s 进弯、满舵推头
        // 外抛 14 m。
        //
        // 正确的不变量是：如果从这一时刻起以恒定减速度 a' 刹车，到达每个
        // 采样点时的速度都必须已经不高于该点的弯速 ，即
        // `v² − 2·a'·d ≤ v_corner²`。对所有采样点取最大值就是「现在就得刹
        // 这么多」——贴着包络行驶时它正好等于 plan_decel，低于包络时它为正
        // 但更小，于是车会平滑地贴着包络提前刹车，而不是等到超速才反应。
        let mut required_decel = 0.0f32;
        let mut distance = PLAN_SPACING_M;
        let mut prev_tangent: Option<Vec3> = None;
        while distance <= horizon {
            if let Some(sample) = self.line.sample_at_distance(projection.along_m + distance) {
                // 曲率限速：取这一段里程内的最大水平曲率 κ，再以横向加速度
                // 上限折算限速 v=√(a_lat_max·R)。AI 线自己的 speed 在某些弯
                // 后段偏乐观（实测线速 25 m/s 处曲率需要 <20 m/s），纯靠线速
                // 会让车在弯中外漂撞墙。固定网格切线差会把半径 26 m 的回头弯
                // 平均成 40 m，等于把限速放宽到两倍，所以这里取区间最大值。
                if let Some(prev) = prev_tangent {
                    let interval_start = projection.along_m + distance - PLAN_SPACING_M;
                    let curved = self
                        .line
                        .max_curvature_between(interval_start, projection.along_m + distance);
                    // 点太少（手工制作的线）时退回相邻采样点的带符号切线差。
                    let curvature = if curved.abs() > 1.0e-4 {
                        curved
                    } else {
                        (prev.x * sample.tangent.z - prev.z * sample.tangent.x) / PLAN_SPACING_M
                    };
                    if curvature.abs() > 1.0e-4 {
                        let radius = 1.0 / curvature.abs();
                        // 限速本身也吃下压力：同一曲率在高速下允许更高。
                        let reference = sample
                            .speed_mps
                            .filter(|value| value.is_finite() && *value > 0.5)
                            .unwrap_or(speed)
                            .clamp(0.0, self.tuning.max_speed_mps);
                        // 抓地使用倍率：低难度只用一半抓地，弯速低一大截。倾斜赛道额外吃
                        // bank 的加成——同一条弯在 31° bank 上就是比平地上快。
                        // 5% 的规划余量：贴着极限进弯的车没有修正转向误差的余地。
                        let normal = self
                            .line
                            .extra(sample.index)
                            .map(|extra| extra.normal)
                            .filter(|normal| normal.length_squared() > 0.25)
                            .unwrap_or(Vec3::Y);
                        let bank = bank_assist_mps2(road_right(normal, sample.tangent), curvature);
                        let grip_at = |speed: f32| {
                            (lateral_limit_mps2(spec, speed) * skill.grip + bank).max(2.0)
                        };
                        let mut v_lat = (grip_at(reference) * radius).sqrt() * PLAN_SPEED_MARGIN;
                        for _ in 0..2 {
                            v_lat = (grip_at(v_lat) * radius).sqrt() * PLAN_SPEED_MARGIN;
                        }
                        let allowed = (v_lat * v_lat + 2.0 * plan_decel * distance).sqrt();
                        target_speed = target_speed.min(allowed);
                        let need = speed * speed - v_lat * v_lat;
                        if need > 0.0 {
                            required_decel = required_decel.max(need / (2.0 * distance));
                        }
                    }
                }
                prev_tangent = Some(sample.tangent);
                if let Some(v_target) = sample
                    .speed_mps
                    .filter(|speed| speed.is_finite() && *speed > 0.5)
                {
                    let allowed = (v_target * v_target + 2.0 * plan_decel * distance).sqrt();
                    target_speed = target_speed.min(allowed);
                    let need = speed * speed - v_target * v_target;
                    if need > 0.0 {
                        required_decel = required_decel.max(need / (2.0 * distance));
                    }
                }
            }
            distance += PLAN_SPACING_M;
        }
        // A car that cannot be passed is followed instead of rammed.
        if let Some(follow) = follow_speed {
            target_speed = target_speed.min(follow);
        }
        // 偏线不等于出赛道：用 AI 线自带的路宽判断真正的边界。
        // 老逻辑只要偏线超过 8 m 就限速 15 m/s，在 Daytona 的宽弯里把圈速
        // 拖到 250 s 以上。现在只有越过本侧路肩一定距离才开始回收速度，
        // 并按超出量渐进降速，而不是一刀切。
        let (side_left, side_right) = self.sample_widths(projection.index);
        let edge = if surface_lateral >= 0.0 {
            side_right
        } else {
            side_left
        };
        let over_edge = surface_lateral.abs() - edge;
        if over_edge > 1.0 {
            // severity 也吃难度：低难度要偏出很多才意识到自己出界，而且
            // 只会慢慢爬回来；Pro 一压线就开始回收速度。
            let severity = (over_edge / 12.0 * skill.recovery).clamp(0.0, 1.0);
            let recovery_cap = self.tuning.fallback_speed_mps * (1.0 - 0.6 * severity);
            target_speed = target_speed.min(recovery_cap);
        }
        // 行驶线晃动同时带来速度节律：同一个弯有时快有时慢，这正是低难度
        // AI 圈速不稳定的来源。
        let wander_scale = if skill.wander_m > 0.05 {
            (1.0 - 0.12 * (wander_speed / skill.wander_m)).clamp(0.8, 1.05)
        } else {
            1.0
        };
        let target_speed = (target_speed * wander_scale).clamp(0.0, self.tuning.max_speed_mps);
        let speed_error = target_speed - speed;
        // 抓地利用率：横向已经用掉多少。刹车和油门都按摩擦圆从它推导——
        // 这正是循迹刹车的物理：刹车不是入弯点一刀切掉，而是随横向需求增长
        // 按椭圆把纵向预算让出去。
        let demand = (speed * speed * curvature.abs() / lateral_limit_mps2(spec, speed).max(1.0))
            .clamp(0.0, 1.0);
        let (mut throttle, mut brake) = if speed_error > 0.5 {
            (
                ((speed_error * self.tuning.throttle_gain + 0.1) * skill.throttle).clamp(0.0, 1.0),
                0.0,
            )
        } else if speed_error < -0.3 {
            (
                0.0,
                ((-speed_error) * self.tuning.brake_gain).clamp(0.0, 1.0),
            )
        } else {
            (0.05, 0.0)
        };
        // 摩擦圆给出的纵向余量：`√(1 − demand²)`。demand=0（直道）→ 满刹；
        // demand→1（弯心满载）→ 只留 15% 的地板（真车也永远留一点，用来
        // 调车尾rotation / 稳定车身）。这比旧的 `1 − 0.25·|转向|` 正确得多：
        // 旧规则按转向行程砍刹车，与抓地预算无关，入弯点就把刹车掐光了，
        // 于是完全没有循迹刹车（实测整圈只有 4% 的采样点刹车与转向重叠）。
        // 0.85 的折减 + 0.45 的地板：地板必须留够，否则弯里刹车被砍到
        // 15% 会刹不住（实测整圈跑不完）；0.45 也接近旧 `1 − 0.25·|steer|`
        // 在满舵时的取值。
        brake *= friction_budget(demand, 0.85, 0.45);
        // 包络前馈：速度误差为 0 时仍要按 plan_decel 减速，否则会“贴着
        // 上限滑行”，到弯口才发现刹不住。`required_decel` 由上面的反向包络
        // 算出，是「为了在每个采样点都不超速，现在必须维持的减速度」。
        // 前馈刹车也吃难度：低难度不会因为包络在往下走就提前踩刹车，
        // 只会在速度已经掉到目标以下之后才反应——所以给一个折减系数。
        let feedforward = (required_decel / plan_decel).clamp(0.0, 1.0) * skill.feedforward;
        if feedforward > 0.1 {
            brake = brake.max(feedforward);
            throttle = 0.0;
        }
        // Ease off in corners; the AI speed already accounts for grip, and
        // this keeps a high-power car from spinning its rear tyres.
        // 已经在抓地极限上就不能再给油：纵向需求会吃掉横向预算，车会继续
        // 推着外抛。实测 Daytona 回头弯里超速 15% 的车一边全油一边满舵，
        // 偏线 15 m 还越跑越远。（demand 已在刹车段按摩擦圆算过。）
        // 油门同理：横向需求越高，能给的后轴纵向抓地越少。
        // share 0.8 让这条曲线贴合旧的 `1 − 0.35·|steer|` × `1 − 0.7·demand`
        // （实测直接用 0.55 会把弯中出弯油砍太狠，整圈慢 8 s）。
        throttle *= (1.0 - 0.35 * steering.abs()).max(0.2) * friction_budget(demand, 0.8, 0.25);
        if speed < 0.5 && speed_error > 0.5 {
            throttle = throttle.max(0.4);
        }
        DriveInput {
            throttle,
            steering,
            brake,
            handbrake: false,
            // The AI does not manage DRS, the hybrid system or the driver aids:
            // they are driver inputs, so they stay off unless the caller turns
            // them on.
            drs: false,
            abs: false,
            tc: false,
            ers: ErsMode::Off,
        }
    }

    /// 用三点圆估算线在某个累计距离附近的带符号曲率，1/m，正值为左弯。
    ///
    /// 只取水平面：坡度/倾斜不应改变转向需求。三点间隔 5 m，既压掉
    /// fast_lane 采样噪声，又比相邻两点的切线差更接近真实弯心曲率。
    fn preview_curvature(&self, distance_m: f32) -> f32 {
        let (Some(a), Some(b), Some(c)) = (
            self.line.sample_at_distance(distance_m - PREVIEW_SPAN_M),
            self.line.sample_at_distance(distance_m),
            self.line.sample_at_distance(distance_m + PREVIEW_SPAN_M),
        ) else {
            return 0.0;
        };
        let ab = Vec3::new(
            b.position.x - a.position.x,
            0.0,
            b.position.z - a.position.z,
        );
        let bc = Vec3::new(
            c.position.x - b.position.x,
            0.0,
            c.position.z - b.position.z,
        );
        let ca = Vec3::new(
            a.position.x - c.position.x,
            0.0,
            a.position.z - c.position.z,
        );
        let denom = ab.length() * bc.length() * ca.length();
        if denom < 1.0e-6 {
            return 0.0;
        }
        // κ = 4A/(abc)；水平叉积的 +Y 分量给符号，左转为正。
        let area_twice = ab.cross(bc).y;
        (2.0 * area_twice / denom).clamp(-0.5, 0.5)
    }

    /// Decide how to deal with traffic ahead of the car.
    ///
    /// Returns a lateral shift for the aim point and, when the block cannot be
    /// passed, the speed to follow instead. The corridor from just in front of
    /// the car to the end of the lookahead is sampled; the nearest blocking
    /// car wins so the driver reacts to what it meets first.
    fn traffic_avoidance(&self, projection: &AiProjection, lookahead_m: f32) -> (f32, Option<f32>) {
        if self.traffic.is_empty() {
            return (0.0, None);
        }

        let start = projection.along_m + TRAFFIC_CAR_HALF_WIDTH_M;
        let end = projection.along_m
            + (lookahead_m * self.tuning.traffic_lookahead_factor).max(start + 4.0);
        // Keep the spacing near three metres so a car right in front of the
        // car is matched, but never flood the loop on a long corridor.
        let samples = ((end - start) / TRAFFIC_SAMPLE_SPACING_M)
            .ceil()
            .clamp(2.0, self.tuning.traffic_max_samples as f32) as usize;
        let spacing = (end - start) / samples as f32;

        for step in 1..=samples {
            let fraction = step as f32 / samples as f32;
            let distance = start + (end - start) * fraction;
            let Some(sample) = self.line.sample_at_distance(distance) else {
                continue;
            };
            let tangent = sample.tangent;
            if tangent.length_squared() < 1.0e-8 {
                continue;
            }
            let right = tangent.cross(Vec3::Y);
            // The car is "in the corridor" when it sits at this sample's
            // longitudinal position and close enough to the line.
            let longitudinal_tolerance = spacing * 0.5 + TRAFFIC_CAR_HALF_WIDTH_M;

            for car in &self.traffic {
                let offset = car.position - sample.position;
                let longitudinal = offset.dot(tangent);
                if longitudinal.abs() > longitudinal_tolerance {
                    continue;
                }
                let lateral = offset.dot(right);
                if lateral.abs() >= self.tuning.traffic_radius_m {
                    continue;
                }
                return self.avoid_one(
                    car,
                    &BlockedCar {
                        sample_index: sample.index,
                        longitudinal_m: longitudinal,
                        lateral_m: lateral,
                        gap_m: distance - projection.along_m,
                        ego_lateral_m: projection.lateral_m,
                        proximity: 1.0 - 0.5 * fraction,
                    },
                );
            }
        }

        (0.0, None)
    }

    /// Pick a side to pass on, or slow down when no side fits.
    fn avoid_one(&self, car: &AiTraffic, block: &BlockedCar) -> (f32, Option<f32>) {
        let half = TRAFFIC_CAR_HALF_WIDTH_M;
        // Already left the car safely to one side: nothing to avoid any more,
        // which keeps a pass from turning into a runaway towards the wall.
        if (block.ego_lateral_m - block.lateral_m).abs() > 2.0 * half + 0.6 {
            return (0.0, None);
        }

        let (side_left, side_right) = self.sample_widths(block.sample_index);
        let gap = (block.gap_m + block.longitudinal_m).max(0.0);
        let pass = self
            .tuning
            .traffic_max_shift_m
            .min(side_left.max(side_right) - half);
        if pass <= 0.5 {
            return (0.0, Some(follow_speed(car, gap, &self.tuning)));
        }

        let toward_line = block.lateral_m.abs() > 0.5;
        let mut side = if toward_line {
            -block.lateral_m.signum()
        } else if side_right >= side_left {
            1.0
        } else {
            -1.0
        };
        let room = |side: f32| if side > 0.0 { side_right } else { side_left };
        if room(side) - half < pass {
            let other = -side;
            if room(other) - half >= pass {
                side = other;
            } else {
                // No room on either side: queue behind the car instead.
                return (0.0, Some(follow_speed(car, gap, &self.tuning)));
            }
        }

        let weight = block.proximity.clamp(0.0, 1.0);
        (side * pass * weight, None)
    }

    /// Authored width to either side of the line at a sample.
    fn sample_widths(&self, index: usize) -> (f32, f32) {
        const FALLBACK_M: f32 = 5.0;
        match self.line.extras().get(index) {
            Some(extra) if extra.side_left_m.is_finite() && extra.side_right_m.is_finite() => {
                (extra.side_left_m.max(0.5), extra.side_right_m.max(0.5))
            }
            _ => (FALLBACK_M, FALLBACK_M),
        }
    }
}

/// Speed to hold while queueing behind a car that cannot be passed.
fn follow_speed(car: &AiTraffic, gap_m: f32, tuning: &AiDriverTuning) -> f32 {
    let speed = car.speed_mps.max(0.0);
    if gap_m < tuning.traffic_follow_gap_m {
        speed.min(gap_m * 0.35).max(0.0)
    } else {
        speed
    }
}

fn sane(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vettura_physics::Pose;
    use vettura_track::SPLINE_VERSION;

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_vec3(bytes: &mut Vec<u8>, value: Vec3) {
        push_f32(bytes, value.x);
        push_f32(bytes, value.y);
        push_f32(bytes, value.z);
    }
    fn synthetic_spline() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, SPLINE_VERSION);
        push_i32(&mut bytes, 2);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, 2);

        // Two AC-frame points heading along +Z.
        push_vec3(&mut bytes, Vec3::new(10.0, 1.0, 0.0));
        push_f32(&mut bytes, 0.0);
        push_i32(&mut bytes, 0);
        push_vec3(&mut bytes, Vec3::new(10.0, 1.0, 20.0));
        push_f32(&mut bytes, 20.0);
        push_i32(&mut bytes, 1);

        push_i32(&mut bytes, 2);
        for _ in 0..2 {
            push_f32(&mut bytes, 30.0); // speed
            push_f32(&mut bytes, 0.8); // gas
            push_f32(&mut bytes, 0.0); // brake
            push_f32(&mut bytes, 1.2); // obsolete lat g
            push_f32(&mut bytes, 120.0); // radius
            push_f32(&mut bytes, 4.0); // side left
            push_f32(&mut bytes, 5.0); // side right
            push_f32(&mut bytes, 0.05); // camber
            push_f32(&mut bytes, 1.0); // direction
            push_vec3(&mut bytes, Vec3::new(0.0, 1.0, 0.0));
            push_f32(&mut bytes, 20.0);
            push_vec3(&mut bytes, Vec3::new(0.0, 0.0, 1.0));
            push_f32(&mut bytes, 2.0);
            push_f32(&mut bytes, 0.1);
        }
        push_i32(&mut bytes, 0);
        bytes
    }
    /// 直道 70 m/s 四段 + 15 m/s 两段的测试线：规划器必须在直道上就开始
    /// 为 100 m 外的慢弯刹车，而不是等车速误差变负（旧逻辑：纯 lookahead
    /// 点在 40 m 处看到的仍是 70 m/s，进弯速度快一倍——线上 grove 撞墙）。
    fn braking_test_spline() -> Vec<u8> {
        let mut bytes = Vec::new();
        let speeds = [70.0, 70.0, 70.0, 70.0, 15.0, 15.0];
        push_i32(&mut bytes, SPLINE_VERSION);
        push_i32(&mut bytes, 6); // points
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, 6); // samples
        for index in 0..6 {
            push_vec3(&mut bytes, Vec3::new(10.0, 1.0, index as f32 * 25.0));
            push_f32(&mut bytes, index as f32 * 25.0);
            push_i32(&mut bytes, index as i32);
        }
        push_i32(&mut bytes, 6); // extras
        for (index, speed) in speeds.iter().enumerate() {
            push_f32(&mut bytes, *speed);
            push_f32(&mut bytes, 0.8); // gas
            push_f32(&mut bytes, 0.0); // brake
            push_f32(&mut bytes, 1.2); // obsolete lat g
            push_f32(&mut bytes, 120.0); // radius
            push_f32(&mut bytes, 4.0); // side left
            push_f32(&mut bytes, 5.0); // side right
            push_f32(&mut bytes, 0.05); // camber
            push_f32(&mut bytes, 1.0); // direction
            push_vec3(&mut bytes, Vec3::new(0.0, 1.0, 0.0));
            push_f32(&mut bytes, index as f32 * 25.0);
            push_vec3(&mut bytes, Vec3::new(0.0, 0.0, 1.0));
            push_f32(&mut bytes, 2.0);
            push_f32(&mut bytes, 0.1);
        }
        push_i32(&mut bytes, 0);
        bytes
    }
    #[test]
    fn driver_throttles_forward_and_steers_toward_the_line() {
        let spline = AiSpline::parse(&synthetic_spline()).unwrap();
        let mut driver = AiDriver::new(spline);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();

        let centered = VehicleSnapshot {
            reference_position: Vec3::new(-10.0, 0.35, 0.0),
            ..VehicleSnapshot::default()
        };
        let input = driver.update(&centered, &spec, tuning);
        assert!(input.throttle > 0.5, "{input:?}");
        assert!(input.steering.abs() < 1.0e-3, "{input:?}");

        // Right of the line: the car must steer left (positive input).
        let right = VehicleSnapshot {
            reference_position: Vec3::new(-9.0, 0.35, 0.0),
            ..VehicleSnapshot::default()
        };
        assert!(
            driver.update(&right, &spec, tuning).steering > 0.0,
            "right-of-line car should steer left"
        );

        // Left of the line: negative steering.
        let left = VehicleSnapshot {
            reference_position: Vec3::new(-11.0, 0.35, 0.0),
            ..VehicleSnapshot::default()
        };
        assert!(
            driver.update(&left, &spec, tuning).steering < 0.0,
            "left-of-line car should steer right"
        );
    }
    /// 把一组物理帧点写成 AI spline（含统一的 extras）。
    fn spline_from_points(points: &[Vec3], speed: f32, normal_ac: Vec3) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, SPLINE_VERSION);
        push_i32(&mut bytes, points.len() as i32);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, points.len() as i32);
        for (index, point) in points.iter().enumerate() {
            // 解析时会把 AC 帧转成物理帧，这里反向写回去。
            push_vec3(&mut bytes, Vec3::new(-point.x, point.y, -point.z));
            push_f32(&mut bytes, 0.0);
            push_i32(&mut bytes, index as i32);
        }
        push_i32(&mut bytes, points.len() as i32);
        for _ in 0..points.len() {
            push_f32(&mut bytes, speed);
            push_f32(&mut bytes, 0.8);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.2);
            push_f32(&mut bytes, 40.0);
            push_f32(&mut bytes, 5.0);
            push_f32(&mut bytes, 5.0);
            push_f32(&mut bytes, 0.05);
            push_f32(&mut bytes, 1.0);
            push_vec3(&mut bytes, normal_ac);
            push_f32(&mut bytes, 0.0);
            push_vec3(&mut bytes, Vec3::new(0.0, -1.0, 0.0));
            push_f32(&mut bytes, 2.0);
            push_f32(&mut bytes, 0.1);
        }
        push_i32(&mut bytes, 0);
        bytes
    }

    /// 半径 `radius` 的闭合圆环线：整圈都是同一半径的弯。
    fn circle_spline(radius: f32) -> Vec<u8> {
        let count = 96;
        let points: Vec<Vec3> = (0..count)
            .map(|index| {
                let theta = std::f32::consts::TAU * index as f32 / count as f32;
                Vec3::new(radius * theta.cos(), 0.0, radius * theta.sin())
            })
            .collect();
        spline_from_points(&points, 30.0, Vec3::new(0.0, 1.0, 0.0))
    }
    #[test]
    fn corner_speed_comes_from_the_car_not_from_the_track() {
        // 一台普通的公路车：抓地一般、没有下压力。
        let mut road = VehicleSpec::default();
        road.tyres.peak_friction = 1.1;
        road.aero.downforce_coeff = 0.0;
        road.mass_kg = 1500.0;
        // 一台带空力的单座赛车。
        let mut racer = VehicleSpec::default();
        racer.tyres.peak_friction = 1.73;
        racer.aero.downforce_coeff = 6.64;
        racer.mass_kg = 606.0;

        // 轮胎基础项（无空力时）被钳在 0.55 g 以内：越野级的抓地也不该让
        // AI 以为能过悬崖。下压力是额外加上去的，所以分开算。
        for spec in [&road, &racer] {
            let ceiling = 9.81 * PLAN_LATERAL_BASE_MAX_G
                + downforce_n(spec, 12.0) * PLAN_LATERAL_DOWNFORCE_SHARE / spec.mass_kg;
            let low = lateral_limit_mps2(spec, 12.0);
            assert!(low <= ceiling + 1.0e-6, "{low} m/s² 超过了上限 {ceiling}");
        }
        // 高速上赛车靠下压力拿到更多横向极限，且公路车永远拿不到。
        let road_fast = lateral_limit_mps2(&road, 25.0);
        let racer_fast = lateral_limit_mps2(&racer, 25.0);
        assert!(
            racer_fast > road_fast * 1.5,
            "下压力应该拉开差距：racer {racer_fast:.2} vs road {road_fast:.2}"
        );
        assert!(
            road_fast < lateral_limit_mps2(&road, 5.0) + 0.5,
            "没有空力的车高速不该突然变强：{road_fast:.2}"
        );
    }

    #[test]
    fn skill_levels_order_pace_precision_and_mistakes() {
        let rookie = skill_profile(0.0);
        let decent = skill_profile(40.0);
        let pro = skill_profile(100.0);
        let alien = skill_profile(110.0);

        // 抓地使用率、刹车距离、预瞄距离都随难度单调上升。
        assert!(rookie.grip < decent.grip && decent.grip < pro.grip && pro.grip <= alien.grip);
        assert!(rookie.brake < pro.brake && pro.brake <= alien.brake);
        assert!(rookie.preview < pro.preview && pro.preview <= alien.preview);
        // 方向盘可用量：新手只敢用前半段，100 以上（Pro）才用满。
        assert!(rookie.steer < decent.steer && decent.steer < pro.steer);
        assert!(pro.steer <= 1.0);
        // 反馈权限与前馈刹车同理。
        assert!(rookie.reaction < pro.reaction && pro.reaction <= alien.reaction);
        assert!(rookie.feedforward < pro.feedforward && pro.feedforward <= alien.feedforward);
        // 120 以上（超人类）才拿到超过「车能做到」的抓地。
        assert!(alien.grip > pro.grip * 1.05, "{:?}", alien);
        // 贴线抖动只在低难度出现，100 完全消失。
        assert!(
            rookie.wander_m >= 2.9,
            "rookie should wander {} m",
            rookie.wander_m
        );
        assert!(pro.wander_m < 1.0e-6);
        // 越界后新手更晚、更保守地爬回去。
        assert!(rookie.recovery > pro.recovery);
    }

    #[test]
    fn wander_only_shows_up_on_low_skill_and_is_stateless() {
        let low = skill_profile(0.0);
        let pro = skill_profile(100.0);
        let (lateral, speed) = line_wander_m(&low, 1200.0);
        assert!((-4.5..=4.5).contains(&lateral), "{lateral}");
        assert!((-3.5..=3.5).contains(&speed), "{speed}");
        // 同一个里程永远得到同一个偏移：无状态、可复现。
        let (again_lateral, again_speed) = line_wander_m(&low, 1200.0);
        assert_eq!((lateral, speed), (again_lateral, again_speed));
        // 高难度什么都不加。
        assert_eq!(line_wander_m(&pro, 1200.0), (0.0, 0.0));
    }

    #[test]
    fn banked_corner_lateral_offset_sees_the_drop() {
        // 车就在线的正下方 1.5 m：水平投影看不出任何偏移，沿路面横向才看得见。
        let flat = AiSpline::parse(&spline_from_points(
            &[Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, -15.0)],
            20.0,
            Vec3::new(0.0, 1.0, 0.0),
        ))
        .unwrap();
        let banked = AiSpline::parse(&spline_from_points(
            &[Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, -15.0)],
            20.0,
            // AC 帧法向绕 +X 倾斜 25°：右半边低下去（ Daytona 那种 bank）。
            Vec3::new(0.423, 0.906, 0.0),
        ))
        .unwrap();
        let mut flat_driver = AiDriver::new(flat);
        let mut banked_driver = AiDriver::new(banked);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();
        let dropped = VehicleSnapshot {
            // 水平位移为零、比线低 1.5 m：只有沿路面横向的量才看得见。
            reference_position: Vec3::new(0.0, -1.15, -5.0),
            forward_speed_mps: 20.0,
            speed_mps: 20.0,
            ..VehicleSnapshot::default()
        };
        let flat_input = flat_driver.update(&dropped, &spec, tuning);
        let banked_input = banked_driver.update(&dropped, &spec, tuning);
        assert!(
            flat_input.steering.abs() < 0.02,
            "flat road: a vertical drop is not a lateral offset, {flat_input:?}"
        );
        assert!(
            banked_input.steering.abs() > 0.05,
            "banked road: sliding 1.5 m down must be corrected, {banked_input:?}"
        );
    }

    #[test]
    fn planner_brakes_harder_the_faster_the_car_arrives() {
        let spline = AiSpline::parse(&braking_test_spline()).unwrap();
        let mut driver = AiDriver::new(spline);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();
        let brake_at = |speed: f32, driver: &mut AiDriver| {
            let snapshot = VehicleSnapshot {
                reference_position: Vec3::new(-10.0, 0.35, 0.0),
                forward_speed_mps: speed,
                speed_mps: speed,
                ..VehicleSnapshot::default()
            };
            driver.update(&snapshot, &spec, tuning).brake
        };
        let cruising = brake_at(20.0, &mut driver);
        let arriving = brake_at(70.0, &mut driver);
        assert!(
            cruising < 0.1,
            "20 m/s 不该为一个 100 m 外的慢弯重刹：{cruising}"
        );
        assert!(arriving > 0.5, "70 m/s 必须在直道上就开始刹车：{arriving}");
    }

    #[test]
    fn a_tighter_circle_brakes_the_same_car_sooner() {
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();
        // 同一台车、同一步入速度：半径越小，弯速越低，就越该刹车。
        let brake_for_radius = |radius: f32, speed: f32| -> f32 {
            let spline = AiSpline::parse(&circle_spline(radius)).unwrap();
            let mut driver = AiDriver::new(spline);
            let snapshot = VehicleSnapshot {
                reference_position: Vec3::new(radius, speed * 0.0 + 0.35, 0.0),
                forward_speed_mps: speed,
                speed_mps: speed,
                ..VehicleSnapshot::default()
            };
            driver.update(&snapshot, &spec, tuning).brake
        };
        let wide = brake_for_radius(80.0, 20.0);
        let tight = brake_for_radius(25.0, 20.0);
        assert!(
            tight > wide + 0.3,
            "半径 25 m 的弯应该比 80 m 的更早要求刹车：{tight} vs {wide}"
        );
        // 20 m/s 恰好是 80 m 弯的弯速附近：不该无脑全刹。
        assert!(wide < 0.6, "过弯速度附近不该全刹：{wide}");
    }

    #[test]
    fn planner_brakes_for_a_corner_beyond_the_lookahead() {
        let spline = AiSpline::parse(&braking_test_spline()).unwrap();
        let mut driver = AiDriver::new(spline);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();

        // 70 m/s 线车速，距 100 m 外的 15 m/s 弯口。纯 lookahead（40 m 上限）
        // 看到的还是直道的 70 m/s——速度误差为零，不会刹车；刹车感知规划器
        // 必须已经收油/踩刹。
        let fast = VehicleSnapshot {
            reference_position: Vec3::new(-10.0, 0.35, 0.0),
            forward_speed_mps: 70.0,
            speed_mps: 70.0,
            ..VehicleSnapshot::default()
        };
        let input = driver.update(&fast, &spec, tuning);
        assert!(
            input.brake > 0.1,
            "no braking for the corner ahead: {input:?}"
        );
        assert!(
            input.throttle < 0.1,
            "still accelerating toward a 100 m corner: {input:?}"
        );

        // 同样的线上，弯已经过了（沿着线再走 125 m，仍在 15 m/s 段）：
        // 目标速度就是当前速度，规划器不该继续施压刹车超过 0.1。
        let slow = VehicleSnapshot {
            reference_position: Vec3::new(-10.0, 0.35, -125.0),
            forward_speed_mps: 15.0,
            speed_mps: 15.0,
            ..VehicleSnapshot::default()
        };
        let input = driver.update(&slow, &spec, tuning);
        assert!(input.brake < 0.1, "braking on the slow straight: {input:?}");
    }
    fn synthetic_straight_spline(count: usize, spacing_m: f32, speed_mps: f32) -> Vec<u8> {
        synthetic_straight(count, spacing_m, speed_mps, 4.0, 5.0)
    }

    /// A straight line with configurable widths, so a test can build a road
    /// that is too narrow to pass on.
    fn synthetic_straight(
        count: usize,
        spacing_m: f32,
        speed_mps: f32,
        side_left_m: f32,
        side_right_m: f32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, SPLINE_VERSION);
        push_i32(&mut bytes, count as i32);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, count as i32);

        for index in 0..count {
            // Physics frame: x = 0, z = -index * spacing. AC flips both signs.
            push_vec3(&mut bytes, Vec3::new(0.0, 0.0, index as f32 * spacing_m));
            push_f32(&mut bytes, index as f32 * spacing_m);
            push_i32(&mut bytes, index as i32);
        }

        push_i32(&mut bytes, count as i32);
        for index in 0..count {
            push_f32(&mut bytes, speed_mps);
            push_f32(&mut bytes, 0.8);
            push_f32(&mut bytes, 0.0);
            push_f32(&mut bytes, 1.2);
            push_f32(&mut bytes, 120.0);
            push_f32(&mut bytes, side_left_m);
            push_f32(&mut bytes, side_right_m);
            push_f32(&mut bytes, 0.05);
            push_f32(&mut bytes, 1.0);
            push_vec3(&mut bytes, Vec3::new(0.0, 1.0, 0.0));
            push_f32(&mut bytes, index as f32 * spacing_m);
            push_vec3(&mut bytes, Vec3::new(0.0, 0.0, 1.0));
            push_f32(&mut bytes, 2.0);
            push_f32(&mut bytes, 0.1);
        }
        push_i32(&mut bytes, 0);
        bytes
    }

    #[test]
    fn traffic_on_the_line_shifts_the_aim_point_sideways() {
        let spline = AiSpline::parse(&synthetic_straight_spline(200, 10.0, 30.0)).unwrap();
        let mut driver = AiDriver::new(spline);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();

        // Moving, so the corridor reaches the car 12 m ahead.
        let ego = VehicleSnapshot {
            reference_position: Vec3::new(0.0, 0.35, 0.0),
            forward_speed_mps: 25.0,
            ..VehicleSnapshot::default()
        };

        // Without traffic the car drives straight.
        let free = driver.update(&ego, &spec, tuning);
        assert!(free.steering.abs() < 1.0e-3, "{free:?}");

        // A car dead on the line ahead: the driver must pick a side, and the
        // wider right side (5.0 m vs 4.0 m authored) is the one it takes.
        driver.set_traffic(&[AiTraffic::new(Vec3::new(0.0, 0.35, -12.0), 20.0)]);
        let avoiding = driver.update(&ego, &spec, tuning);
        assert!(
            avoiding.steering < -0.01,
            "a car on the line should be avoided towards the right: {avoiding:?}"
        );

        // The same car sitting on the right half of the road forces a pass on
        // the left instead.
        driver.set_traffic(&[AiTraffic::new(Vec3::new(2.0, 0.35, -12.0), 20.0)]);
        let other_side = driver.update(&ego, &spec, tuning);
        assert!(
            other_side.steering > 0.01,
            "a right-side car should be avoided on the left: {other_side:?}"
        );
    }

    /// A slow car far ahead on a wide road.
    fn near_far() -> AiTraffic {
        AiTraffic::new(Vec3::new(0.0, 0.35, -12.0), 4.0)
    }

    #[test]
    fn traffic_that_cannot_be_passed_slows_the_driver_down() {
        // A road only 0.8 m wide: no pass fits on either side, so the driver
        // has to queue behind the car in front.
        let spline = AiSpline::parse(&synthetic_straight(200, 10.0, 30.0, 0.4, 0.4)).unwrap();
        assert!((spline.extras()[0].width_m() - 0.8).abs() < 1.0e-6);

        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();
        let fast = VehicleSnapshot {
            reference_position: Vec3::new(0.0, 0.35, 0.0),
            forward_speed_mps: 25.0,
            ..VehicleSnapshot::default()
        };
        let close = AiTraffic::new(Vec3::new(0.0, 0.35, -3.0), 4.0);

        // Three metres ahead, well inside the follow gap: the speed target has
        // to drop below the line's 30 m/s.
        let mut following = AiDriver::new(spline.clone()).with_traffic(&[close]);
        let blocked = following.update(&fast, &spec, tuning);
        assert!(
            blocked.brake > 0.0 || blocked.throttle < 0.1,
            "a car 3 m ahead must be braked for: {blocked:?}"
        );

        // Even a slow car 60 m ahead has to be followed on that road: with no
        // room to pass, speed has to come down to its 4 m/s.
        let far = AiTraffic::new(Vec3::new(0.0, 0.35, -60.0), 4.0);
        let mut following_far = AiDriver::new(spline).with_traffic(&[far]);
        let away = following_far.update(&fast, &spec, tuning);
        assert!(
            away.brake > 0.0 || away.throttle < 0.1,
            "an impassable road must slow the driver even far away: {away:?}"
        );

        // On a wide road the same slow car is passable, so the driver keeps
        // its speed and heads for a side instead.
        let wide = AiSpline::parse(&synthetic_straight_spline(200, 10.0, 30.0)).unwrap();
        let mut passable = AiDriver::new(wide).with_traffic(&[near_far()]);
        let passing = passable.update(&fast, &spec, tuning);
        assert!(
            passing.brake.abs() < 1.0e-6 && passing.throttle > 0.1,
            "a passable car should not slow the driver: {passing:?}"
        );
        assert!(
            passing.steering < -0.01,
            "the driver should move aside to pass: {passing:?}"
        );
    }

    #[test]
    fn driver_passes_a_slower_car_on_the_straight() {
        let spline = AiSpline::parse(&synthetic_straight_spline(200, 10.0, 30.0)).unwrap();
        let spawn = spline.start_pose().unwrap();
        let mut world = crate::World::new(crate::SurfaceMap::new()).unwrap();
        let mut place = spawn;
        place.position.y = 0.05;
        let ego = world.add_vehicle(VehicleSpec::default(), place).unwrap();
        // A 40 m slower car parked on the line: the ego has to go around it.
        let obstacle = world
            .add_vehicle(
                VehicleSpec::default(),
                Pose {
                    position: Vec3::new(0.0, 0.05, -40.0),
                    orientation: spawn.orientation,
                },
            )
            .unwrap();

        let mut driver = AiDriver::new(spline);
        let mut max_pass_offset_m = 0.0f32;
        for _ in 0..(240 * 8) {
            let other = world.snapshot(obstacle).unwrap();
            let traffic = [AiTraffic::new(
                other.reference_position,
                other.forward_speed_mps,
            )];
            let other_z = other.reference_position.z;
            driver.set_traffic(&traffic);
            let input = driver.update_view(world.vehicle(ego).unwrap());
            world.set_input(ego, input).unwrap();
            world.step_exact(1.0 / 240.0).unwrap();
            // Track the biggest offset from the racing line the driver chose
            // while the obstacle was still ahead of the car.
            let ego_now = world.snapshot(ego).unwrap().reference_position;
            if ego_now.z < other_z + 2.0 {
                max_pass_offset_m = max_pass_offset_m.max(ego_now.x);
            }
        }

        let ego_state = world.snapshot(ego).unwrap();
        let other_state = world.snapshot(obstacle).unwrap();
        assert!(
            max_pass_offset_m > 1.5,
            "the AI car did not move off the line to pass: {max_pass_offset_m} m"
        );
        assert!(
            ego_state.reference_position.z < other_state.reference_position.z - 1.0,
            "the AI car did not get past the obstacle: {ego_state:?} vs {other_state:?}"
        );
        // The obstacle stayed where it was spawned: no shoving it out of the
        // way, only a clean pass.
        assert!(
            (other_state.reference_position - Vec3::new(0.0, 0.05, -40.0)).length() < 3.0,
            "the AI car pushed the obstacle aside: {other_state:?}"
        );
    }

    #[test]
    fn traffic_ahead_of_the_lookahead_is_ignored() {
        let spline = AiSpline::parse(&synthetic_straight_spline(200, 10.0, 30.0)).unwrap();
        let mut driver = AiDriver::new(spline);
        let spec = VehicleSpec::default();
        let tuning = VehicleTuning::default();
        let ego = VehicleSnapshot {
            reference_position: Vec3::new(0.0, 0.35, 0.0),
            ..VehicleSnapshot::default()
        };

        driver.set_traffic(&[AiTraffic::new(Vec3::new(0.0, 0.35, -600.0), 5.0)]);
        let input = driver.update(&ego, &spec, tuning);
        assert!(input.steering.abs() < 1.0e-3, "{input:?}");
        assert_eq!(driver.traffic().len(), 1);
        driver.clear_traffic();
        assert!(driver.traffic().is_empty());
    }

    #[test]
    fn driver_moves_a_car_along_a_straight_line() {
        let spline = AiSpline::parse(&synthetic_straight_spline(200, 10.0, 30.0)).unwrap();
        let spawn = spline.start_pose().unwrap();
        let mut world = crate::World::new(crate::SurfaceMap::new()).unwrap();
        let car = world
            .add_vehicle(
                VehicleSpec::default(),
                Pose {
                    position: Vec3::new(spawn.position.x, 0.05, spawn.position.z),
                    orientation: spawn.orientation,
                },
            )
            .unwrap();
        let mut driver = AiDriver::new(spline);

        for _ in 0..720 {
            let input = {
                let view = world.vehicle(car).unwrap();
                driver.update_view(view)
            };
            world.set_input(car, input).unwrap();
            world.step_exact(1.0 / 240.0).unwrap();
        }

        let snapshot = world.snapshot(car).unwrap();
        assert!(
            snapshot.reference_position.z < -10.0,
            "car did not follow the line: {snapshot:?}"
        );
        assert!(
            snapshot.reference_position.x.abs() < 1.5,
            "car wandered off the straight line: {snapshot:?}"
        );
        assert!(
            snapshot.forward_speed_mps > 3.0,
            "car did not accelerate: {snapshot:?}"
        );
    }
}
/// QSS 诊断用的最高速（等于 `AiDriverTuning::default().max_speed_mps`）。
#[cfg(test)]
const QSS_MAX_SPEED_MPS: f32 = 90.0;

// -----------------------------------------------------------------------
// 准稳态（QSS）圈速诊断：把这条线的物理极限跑一遍，看规划器离极限有多远。
// 权威方法见 docs/architecture.md「AI 司机与 AC 难度档」：反向刹车/横向
// 极限 + 正向动力极限，然后逐点取最小。
// -----------------------------------------------------------------------

#[test]
#[ignore = "reads the real Daytona line under mods/"]
fn qss_optimum_lap_time() {
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mods/daytona_2017");
    let Some(module) = vettura_assets::TrackModule::load_layout(&root, "roadcourse").ok() else {
        eprintln!("skipping: mods/daytona_2017 not found");
        return;
    };
    let Some(line) = module.load_fast_lane().ok().flatten() else {
        eprintln!("skipping: no fast_lane.ai");
        return;
    };
    let spec = VehicleSpec::default();
    let lap = line.lap_length_m();
    let step = 2.0f32;
    let count = (lap / step) as usize;

    // 采样：里程、线速、该点带符号曲率、该点 bank 加成。
    let mut distance = Vec::with_capacity(count + 1);
    let mut line_speed = Vec::with_capacity(count + 1);
    let mut corner = Vec::with_capacity(count + 1);
    let mut normal = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let along = index as f32 * step;
        distance.push(along);
        let sample = line.sample_at_distance(along).unwrap();
        line_speed.push(sample.speed_mps.unwrap_or(30.0).max(0.5));
        let curvature = line.max_curvature_between(along - step, along);
        corner.push(curvature);
        normal.push(
            line.extra(sample.index)
                .map(|extra| extra.normal)
                .filter(|value| value.length_squared() > 0.25)
                .unwrap_or(Vec3::Y),
        );
    }

    // 横向极限 + bank 加成，换算成弯速。
    let mut lateral = vec![0.0f32; count + 1];
    for index in 0..=count {
        let curvature = corner[index].abs();
        let radius = if curvature > 1.0e-4 {
            1.0 / curvature
        } else {
            f32::INFINITY
        };
        if !radius.is_finite() {
            lateral[index] = QSS_MAX_SPEED_MPS;
            continue;
        }
        let bank = bank_assist_mps2(
            road_right(
                normal[index],
                line.sample_at_distance(distance[index]).unwrap().tangent,
            ),
            corner[index],
        );
        let grip_at = |speed: f32| (lateral_limit_mps2(&spec, speed) + bank).max(2.0);
        let mut v = (grip_at(15.0) * radius).sqrt();
        for _ in 0..3 {
            v = (grip_at(v) * radius).sqrt();
        }
        lateral[index] = v.min(QSS_MAX_SPEED_MPS);
    }

    let lap_time = |limit: &[f32]| -> f32 {
        let mut total = 0.0;
        for index in 0..count {
            let v = limit[index].max(1.0);
            total += step / v;
        }
        total
    };
    let line_only = lap_time(&line_speed);
    let lateral_only = lap_time(&lateral);

    // 反向刹车包络 + 正向动力包络。
    let run_pass = |a_brake: f32, a_accel: f32| -> Vec<f32> {
        let mut brake = vec![0.0f32; count + 1];
        let mut accel = vec![0.0f32; count + 1];
        brake[count] = lateral[count];
        for index in (0..count).rev() {
            let v = lateral[index];
            let ahead = brake[index + 1];
            brake[index] = v.min((ahead * ahead + 2.0 * a_brake * step).sqrt());
        }
        accel[0] = brake[0].min(8.0);
        for index in 1..=count {
            let behind = accel[index - 1];
            accel[index] = brake[index]
                .min((behind * behind + 2.0 * a_accel * step).sqrt())
                .min(QSS_MAX_SPEED_MPS);
        }
        accel
    };

    println!(
        "[qss] lap={lap:.1} m, 线自身速度积分={line_only:.1} s, 纯横向极限={lateral_only:.1} s"
    );
    for (brake, accel) in [
        (6.0f32, 4.0f32),
        (8.0, 5.0),
        (10.0, 7.0),
        (12.0, 9.0),
        (15.0, 12.0),
    ] {
        let profile = run_pass(brake, accel);
        let total = lap_time(&profile);
        let top = profile.iter().cloned().fold(0.0f32, f32::max);
        println!("[qss] a_brake={brake:.0} a_accel={accel:.0} -> {total:.1} s (最高 {top:.1} m/s)");
    }
    println!("[qss] 当前 AI 实测圈速 ≈ 175 s（Godot 探针）/ 169.5 s（Rust 回归）");
}
