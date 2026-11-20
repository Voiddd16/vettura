//! Jolt 轮胎回调的共享输入：表面/水膜/橡胶/轮胎窗 + 天气标量。
//!
//! 回调跑在 Jolt 求解器内部，取锁既慢又可能 panic-展开；所以每个物理帧把当轮
//! 的采样发布成一组 relaxed 原子量，回调只做原子读 + 查表 + 纯函数运算。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use oxijolt::{TireFriction, TireMaxImpulseInput};
use vettura_track::Weather;
use vettura_track::{SurfaceId, SurfaceTable};

use super::VehicleSnapshot;
use super::friction::{combined_slip_circle, slip_floor};

/// Shared, atomically-updated grip inputs for the Jolt tyre callback.
///
/// The callback runs inside Jolt's solver, where taking a lock would be both
/// slow and an unwind hazard. Each step publishes one surface/water sample per
/// wheel plus the current weather scalars; the callback only performs relaxed
/// atomic loads and a table lookup.
///
/// The publishers below are the only way the vehicle side writes to this state:
/// they keep the atomic bookkeeping in one place instead of letting every
/// subsystem poke at raw bit patterns.
pub(crate) struct GripState {
    surfaces: Arc<SurfaceTable>,
    tyre_peak_friction: f32,
    /// Tyre model, for the friction circle in the callback.
    tyre_model: vettura_physics::TyreSpec,
    surface_ids: [AtomicU32; 4],
    water_depth_bits: [AtomicU32; 4],
    /// Rubber multiplier under each wheel, one per wheel.
    groove_bits: [AtomicU32; 4],
    /// Tyre window multiplier per wheel (temperature, pressure, wear).
    tyre_bits: [AtomicU32; 4],
    /// Water film carried by each tyre, mm.
    water_film_bits: [AtomicU32; 4],
    /// Longitudinal slip demand per wheel, `0..=1` (friction-circle usage).
    longitudinal_demand_bits: [AtomicU32; 4],
    /// Lateral slip demand per wheel, `0..=1` (friction-circle usage).
    lateral_demand_bits: [AtomicU32; 4],
    /// Relaxation multiplier on the lateral force per wheel, `0..=1`.
    relaxation_lateral_bits: [AtomicU32; 4],
    /// Relaxation multiplier on the longitudinal force per wheel, `0..=1`.
    relaxation_longitudinal_bits: [AtomicU32; 4],
    /// Extra vertical load per wheel from the auxiliary springs, newtons.
    ///
    /// Jolt's `suspension_impulse` only carries its own linear spring/damper/
    /// anti-roll bar. AC cars also have progressive springs, bump/rebound
    /// stops, a heave (third) spring and active actuators; those are applied
    /// as external forces on the chassis, so their share of the load never
    /// reaches the tyre callback. Without adding it back here the tyre would
    /// think it is loaded with only the linear spring — on this F1 that is
    /// 47 % of the real corner weight, i.e. roughly half the grip.
    extra_load_bits: [AtomicU32; 4],
    speed_bits: AtomicU32,
    wetness_bits: AtomicU32,
    track_temperature_bits: AtomicU32,
    grip_scale_bits: AtomicU32,
}

impl GripState {
    /// Publish the water film one tyre carries, mm.
    pub(crate) fn publish_water_film(&self, index: usize, film_mm: f32) {
        self.water_film_bits[index].store(film_mm.to_bits(), Ordering::Relaxed);
    }

    /// Publish the tyre-window multiplier for one wheel (absolute).
    ///
    /// Temperature, pressure and wear live in one number, so the thermal model
    /// writes the whole value rather than scaling what is already there; that
    /// keeps the window from drifting when the model is re-run each step.
    pub(crate) fn publish_tyre_window(&self, index: usize, multiplier: f32) {
        let value = if multiplier.is_finite() {
            multiplier.clamp(0.0, 8.0)
        } else {
            1.0
        };
        self.tyre_bits[index].store(value.to_bits(), Ordering::Relaxed);
    }

    /// Raise the published tyre-window multiplier for one wheel.
    ///
    /// The camber and flat-spot models each fold their own factor into the
    /// window the thermal model published, so this reads the current value,
    /// multiplies and writes it back.
    pub(crate) fn scale_tyre_window(&self, index: usize, factor: f32) {
        let current = f32::from_bits(self.tyre_bits[index].load(Ordering::Relaxed));
        let next = (current * factor).clamp(0.0, 8.0);
        self.tyre_bits[index].store(next.to_bits(), Ordering::Relaxed);
    }

    /// Raise the published relaxation multiplier for one wheel, both directions.
    ///
    /// `1.0` means "no lag": the callback then sees the full slip this step.
    pub(crate) fn publish_relaxation(&self, index: usize, longitudinal: f32, lateral: f32) {
        let long = if longitudinal.is_finite() {
            longitudinal.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let lat = if lateral.is_finite() {
            lateral.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.relaxation_longitudinal_bits[index].store(long.to_bits(), Ordering::Relaxed);
        self.relaxation_lateral_bits[index].store(lat.to_bits(), Ordering::Relaxed);
    }

    /// Publish the auxiliary-spring load one wheel carries, newtons.
    ///
    /// Published every step for all four wheels (zero when the wheel has no
    /// contact) so a wheel that leaves the ground cannot keep a stale load.
    pub(crate) fn publish_extra_load(&self, index: usize, newtons: f32) {
        let value = if newtons.is_finite() {
            newtons.clamp(-300_000.0, 300_000.0)
        } else {
            0.0
        };
        self.extra_load_bits[index].store(value.to_bits(), Ordering::Relaxed);
    }

    /// Publish the rubber multiplier under one wheel.
    pub(crate) fn publish_groove(&self, index: usize, multiplier: f32) {
        let value = if multiplier.is_finite() {
            multiplier.clamp(0.0, 8.0)
        } else {
            1.0
        };
        self.groove_bits[index].store(value.to_bits(), Ordering::Relaxed);
    }
}

impl GripState {
    pub(crate) fn new(
        surfaces: Arc<SurfaceTable>,
        tyre_peak_friction: f32,
        tyre_model: vettura_physics::TyreSpec,
    ) -> Self {
        Self {
            surfaces,
            tyre_peak_friction: tyre_peak_friction.max(0.0),
            tyre_model,
            surface_ids: [
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
                AtomicU32::new(u32::from(SurfaceId::ASPHALT.0)),
            ],
            water_depth_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            groove_bits: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            tyre_bits: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            water_film_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            longitudinal_demand_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            relaxation_lateral_bits: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            relaxation_longitudinal_bits: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            extra_load_bits: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            lateral_demand_bits: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            speed_bits: AtomicU32::new(0.0f32.to_bits()),
            wetness_bits: AtomicU32::new(0.0f32.to_bits()),
            track_temperature_bits: AtomicU32::new(32.0f32.to_bits()),
            grip_scale_bits: AtomicU32::new(1.0f32.to_bits()),
        }
    }

    pub(crate) fn set_weather(&self, weather: &Weather) {
        self.wetness_bits
            .store(weather.wetness.to_bits(), Ordering::Relaxed);
        self.track_temperature_bits
            .store(weather.track_temperature_c.to_bits(), Ordering::Relaxed);
        self.grip_scale_bits
            .store(weather.grip_scale.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn publish_snapshot(&self, snapshot: &VehicleSnapshot) {
        for (index, wheel) in snapshot.wheels.iter().enumerate() {
            self.surface_ids[index].store(u32::from(wheel.surface.0), Ordering::Relaxed);
            self.water_depth_bits[index].store(wheel.water_depth_m.to_bits(), Ordering::Relaxed);
            // How much of the friction budget each direction is asking for.
            // The normalisation points are where the slip curve has given up
            // most of its grip, so a normal power-on or cornering wheel still
            // keeps almost all of its budget in both directions.
            // Normalisation points are deep into slipping, so a normal
            // power-on or cornering wheel keeps almost all of its budget.
            let longitudinal = (wheel.slip_ratio.abs() / 1.0).clamp(0.0, 1.0);
            let lateral = (wheel.slip_angle.abs() / 0.8).clamp(0.0, 1.0);
            self.longitudinal_demand_bits[index].store(longitudinal.to_bits(), Ordering::Relaxed);
            self.lateral_demand_bits[index].store(lateral.to_bits(), Ordering::Relaxed);
        }
        self.speed_bits.store(
            snapshot.linear_velocity.length().to_bits(),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn friction(&self, input: &TireMaxImpulseInput) -> TireFriction {
        let index = (input.wheel as usize).min(3);
        let surface = self
            .surfaces
            .get(SurfaceId(
                self.surface_ids[index].load(Ordering::Relaxed) as u16
            ))
            .unwrap_or_else(|| &self.surfaces.entries()[0]);
        let water_depth_m = f32::from_bits(self.water_depth_bits[index].load(Ordering::Relaxed));
        let water_film_mm = f32::from_bits(self.water_film_bits[index].load(Ordering::Relaxed));
        let speed_mps = f32::from_bits(self.speed_bits.load(Ordering::Relaxed));

        let weather = Weather {
            wetness: f32::from_bits(self.wetness_bits.load(Ordering::Relaxed)),
            track_temperature_c: f32::from_bits(
                self.track_temperature_bits.load(Ordering::Relaxed),
            ),
            grip_scale: f32::from_bits(self.grip_scale_bits.load(Ordering::Relaxed)),
            ..Weather::dry()
        };

        // A rough road costs grip: the tyre load is never steady, so the
        // effective friction falls in proportion to how steep the undulation
        // is. AC's own run-off areas (0.03 m over 0.5 m) lose about 40%.
        let undulation = (surface.bump_height_m / surface.bump_length_m.max(0.25)).min(0.6);
        let roughness_loss = (1.0 - undulation).clamp(0.5, 1.0);

        // `input.default_friction()` is Jolt's curve already combined with the
        // ground body's friction via sqrt. The ground is 1.0, so the combined
        // peak is still 1.0 and `scale` sets the final peak exactly; only the
        // non-peak points are lifted slightly by the sqrt.
        let environmental =
            weather.grip_for_wheel(surface, speed_mps, water_depth_m, water_film_mm)
                * roughness_loss;
        // Rubber laid down by earlier laps, sampled per wheel.
        let groove = f32::from_bits(self.groove_bits[index].load(Ordering::Relaxed));
        let groove = if groove.is_finite() {
            groove.clamp(0.5, 2.0)
        } else {
            1.0
        };
        let tyre_window = f32::from_bits(self.tyre_bits[index].load(Ordering::Relaxed));
        let tyre_window = if tyre_window.is_finite() {
            tyre_window.clamp(0.5, 1.0)
        } else {
            1.0
        };
        // Combined slip: 每个轮子自己的动态摩擦圆。demand 来自本轮的滑移
        // 率/滑移角，floor 来自轮胎衰减特性与本轮所压的路面，relax 是松弛
        // 长度——四个轮子因此各有一个独立的圆。
        let longitudinal_demand =
            f32::from_bits(self.longitudinal_demand_bits[index].load(Ordering::Relaxed));
        let lateral_demand =
            f32::from_bits(self.lateral_demand_bits[index].load(Ordering::Relaxed));
        let tyres = &self.tyre_model;
        let relaxation_long =
            f32::from_bits(self.relaxation_longitudinal_bits[index].load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let relaxation_lateral =
            f32::from_bits(self.relaxation_lateral_bits[index].load(Ordering::Relaxed))
                .clamp(0.0, 1.0);
        let floor = slip_floor(tyres, surface);
        let (circle_long, circle_lateral) = combined_slip_circle(
            longitudinal_demand,
            lateral_demand,
            floor,
            relaxation_long,
            relaxation_lateral,
        );

        // Total vertical load on this wheel: Jolt's linear spring plus the
        // auxiliary springs (progressive, stops, heave, active). Jolt caps the
        // tyre impulse at `coefficient × suspension_impulse`, and its impulse
        // only knows the linear spring, so the coefficient is scaled by
        // `total / linear` to make the tyre see the real corner weight.
        let delta_time = input.delta_time.max(1.0e-6);
        let linear_load = (input.suspension_impulse / delta_time).max(0.0);
        let extra_load = f32::from_bits(self.extra_load_bits[index].load(Ordering::Relaxed));
        let load_n = (linear_load + extra_load).max(0.0);
        let load_boost = load_boost(linear_load, load_n);
        let load_long = tyres.load_friction_multiplier(load_n, true);
        let load_lateral = tyres.load_friction_multiplier(load_n, false);

        let scale =
            (self.tyre_peak_friction * environmental * groove * tyre_window).clamp(0.0, 8.0);
        let base = input.default_friction();
        TireFriction {
            longitudinal: base.longitudinal * scale * circle_long * load_long * load_boost,
            lateral: base.lateral * scale * circle_lateral * load_lateral * load_boost,
        }
    }
}

/// `total / linear` load factor for the tyre callback's impulse cap.
///
/// Jolt always multiplies the returned coefficient by its own linear-spring
/// impulse, so a car whose auxiliary springs carry part of the weight has to
/// scale the coefficient up by the same ratio. The floor on `linear_load`
/// keeps a barely-touching wheel (or an airborne one) from producing an
/// unbounded factor; the upper clamp stops a degenerate bump-stop spike from
/// asking for more grip than any tyre could make.
fn load_boost(linear_load: f32, total_load: f32) -> f32 {
    if linear_load <= 20.0 || !linear_load.is_finite() {
        return 1.0;
    }
    (total_load / linear_load).clamp(0.0, 4.0)
}

#[cfg(test)]
mod load_boost_tests {
    use super::load_boost;

    #[test]
    fn no_auxiliary_spring_is_neutral() {
        // Jolt's linear spring carries everything: the coefficient must not move.
        assert_eq!(load_boost(1_500.0, 1_500.0), 1.0);
    }

    #[test]
    fn heave_spring_doubles_the_cap() {
        // Half the weight on the linear spring, half on a heave spring.
        assert_eq!(load_boost(1_500.0, 3_000.0), 2.0);
    }

    #[test]
    fn barely_loaded_wheel_is_not_amplified() {
        // An airborne wheel: no meaningful linear load, so no boost, otherwise
        // a stray auxiliary force would invent grip out of nothing.
        assert_eq!(load_boost(5.0, 400.0), 1.0);
        assert_eq!(load_boost(0.0, 0.0), 1.0);
    }

    #[test]
    fn degenerate_inputs_are_clamped() {
        // A bump-stop spike can ask for a lot; the cap keeps it physical.
        assert_eq!(load_boost(100.0, 10_000.0), 4.0);
        assert_eq!(load_boost(f32::NAN, 1_000.0), 1.0);
        // Negative total load (extension pulling down) never boosts.
        assert_eq!(load_boost(1_000.0, -100.0), 0.0);
    }
}
