use serde::{Deserialize, Serialize};

use crate::spec::constants::sample_curve;

/// One point on the engine torque curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TorquePoint {
    /// Engine speed, rpm.
    pub rpm: f32,
    /// Crank torque at that speed, N·m.
    pub torque_nm: f32,
}

/// Forced induction: one turbocharger (or an equivalent supercharger) fed from
/// AC's `[TURBO_0]` block.
///
/// AC stores the boost in bar and shapes the spool-up with `REFERENCE_RPM` and
/// `GAMMA`. The pressure itself is a runtime state: the turbine needs time to
/// spin up, so `lag_up_s` / `lag_down_s` are the time constants for building
/// and losing boost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TurboSpec {
    /// Maximum gauge pressure the turbo can make, bar.
    pub max_boost_bar: f32,
    /// Pressure at which the wastegate opens, bar. `0` means no separate limit.
    pub wastegate_bar: f32,
    /// Engine speed at which the turbo reaches full boost, rpm.
    pub reference_rpm: f32,
    /// Spool-up exponent (`GAMMA`): 1 is linear in rpm, > 1 keeps the turbo
    /// lazy until higher rpm.
    pub gamma: f32,
    /// Time constant for building boost, seconds.
    pub lag_up_s: f32,
    /// Time constant for losing boost, seconds.
    pub lag_down_s: f32,
    /// Extra torque per bar of boost, as a fraction of the naturally aspirated
    /// torque. `1.0` roughly doubles the torque at 1 bar.
    pub torque_gain_per_bar: f32,
}

impl Default for TurboSpec {
    fn default() -> Self {
        Self {
            max_boost_bar: 0.0,
            wastegate_bar: 0.0,
            reference_rpm: 3_000.0,
            gamma: 1.0,
            lag_up_s: 0.35,
            lag_down_s: 0.2,
            torque_gain_per_bar: 1.0,
        }
    }
}

impl TurboSpec {
    /// Whether the car is actually forced-inducted.
    pub fn is_enabled(&self) -> bool {
        self.max_boost_bar > 0.0 && self.torque_gain_per_bar > 0.0
    }

    /// Boost pressure the wastegate allows, bar.
    pub fn effective_max_boost_bar(&self) -> f32 {
        let max = self.max_boost_bar.clamp(0.0, 5.0);
        if self.wastegate_bar > 0.0 {
            max.min(self.wastegate_bar.clamp(0.0, 5.0))
        } else {
            max
        }
    }

    /// Boost the turbo is trying to build at this throttle and rpm, bar.
    ///
    /// No throttle means no exhaust energy, and below `reference_rpm` the
    /// turbine is only partially spooled. The runtime then relaxes the actual
    /// pressure toward this target with the two lag time constants.
    pub fn target_boost_bar(&self, throttle: f32, rpm: f32) -> f32 {
        if !self.is_enabled() {
            return 0.0;
        }
        let throttle = if throttle.is_finite() {
            throttle.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rpm = if rpm.is_finite() { rpm.max(0.0) } else { 0.0 };
        let reference = self.reference_rpm.clamp(200.0, 30_000.0);
        let gamma = self.gamma.clamp(0.1, 4.0);
        let spool = (rpm / reference).powf(gamma).clamp(0.0, 1.0);
        let max = self.effective_max_boost_bar();
        (max * throttle * spool).clamp(0.0, max)
    }

    /// Engine torque multiplier from a boost pressure, `≥ 1`.
    pub fn torque_multiplier(&self, boost_bar: f32) -> f32 {
        if !self.is_enabled() {
            return 1.0;
        }
        let boost = if boost_bar.is_finite() {
            boost_bar.clamp(0.0, self.effective_max_boost_bar())
        } else {
            0.0
        };
        1.0 + self.torque_gain_per_bar.max(0.0) * boost
    }

    /// Torque multiplier at full boost.
    pub fn max_torque_multiplier(&self) -> f32 {
        self.torque_multiplier(self.effective_max_boost_bar())
    }
}

/// Engine parameters from AC `engine.ini` and `power.lut`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSpec {
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// Peak torque, used by the simple backend. A real backend should consume
    /// the full torque curve instead.
    pub peak_torque_nm: f32,
    /// Full `power.lut` torque curve, rpm → N·m. Empty means "no authored
    /// curve"; the backends then fall back to [`Self::DEFAULT_NORMALIZED_CURVE`].
    pub torque_curve: Vec<TorquePoint>,
    /// Turbocharger / forced induction, if any.
    pub turbo: TurboSpec,
    /// Engine braking (coast) torque at [`Self::coast_reference_rpm`], N·m
    /// (`engine.ini [COAST_REF] TORQUE`).
    pub coast_torque_nm: f32,
    /// Engine speed at which the coast torque above is quoted, rpm
    /// (`[COAST_REF] RPM`).
    pub coast_reference_rpm: f32,
    /// Shape of the coast curve above the reference, `0..=1`. AC's
    /// `[COAST_REF] NON_LINEARITY`: `0` linear in rpm, `1` fully quadratic.
    pub coast_non_linearity: f32,
    /// Frequency the rev limiter cuts fuel at, Hz. AC's `LIMITER_HZ`; `0`
    /// means the limiter simply stops the engine, which is what a hard rpm
    /// clamp does anyway.
    pub limiter_hz: f32,
    /// Power lost per kilometre of altitude, fraction. AC's
    /// `ALTITUDE_SENSITIVITY`; `0` disables the effect.
    pub altitude_sensitivity: f32,
    pub gear_ratios: Vec<f32>,
    pub final_drive: f32,
    pub thermal: EngineThermalSpec,
}

impl Default for EngineSpec {
    fn default() -> Self {
        Self {
            idle_rpm: 1_000.0,
            redline_rpm: 8_000.0,
            peak_torque_nm: 350.0,
            torque_curve: Vec::new(),
            turbo: TurboSpec::default(),
            // Without an authored coast reference, keep a small default so the
            // coast model stays switched off until data arrives.
            coast_torque_nm: 0.0,
            coast_reference_rpm: 3_000.0,
            coast_non_linearity: 0.0,
            limiter_hz: 0.0,
            altitude_sensitivity: 0.0,
            gear_ratios: vec![3.2, 2.1, 1.6, 1.3, 1.1, 0.95],
            final_drive: 3.7,
            thermal: EngineThermalSpec::default(),
        }
    }
}

impl EngineSpec {
    /// Generic fallback shape `(rpm fraction, torque fraction)` when the car
    /// does not ship a `power.lut`.
    pub const DEFAULT_NORMALIZED_CURVE: [(f32, f32); 5] = [
        (0.0, 0.72),
        (0.45, 0.92),
        (0.7, 1.0),
        (0.9, 0.95),
        (1.0, 0.82),
    ];

    /// Sort, filter and de-duplicate the authored curve.
    fn torque_curve_points(&self) -> Vec<TorquePoint> {
        let mut points: Vec<TorquePoint> = self
            .torque_curve
            .iter()
            .copied()
            .filter(|point| {
                point.rpm.is_finite()
                    && point.torque_nm.is_finite()
                    && point.rpm >= 0.0
                    && point.torque_nm >= 0.0
            })
            .collect();
        points.sort_by(|a, b| a.rpm.total_cmp(&b.rpm));
        points.dedup_by(|a, b| (a.rpm - b.rpm).abs() < f32::EPSILON);
        points
    }

    /// Interpolated crank torque at `rpm`, N·m.
    pub fn torque_at_rpm(&self, rpm: f32) -> f32 {
        let rpm = if rpm.is_finite() { rpm.max(0.0) } else { 0.0 };
        let points = self.torque_curve_points();
        if points.len() >= 2 {
            return sample_curve(
                &points
                    .iter()
                    .map(|point| (point.rpm, point.torque_nm))
                    .collect::<Vec<_>>(),
                rpm,
            );
        }
        if let Some(point) = points.first() {
            return point.torque_nm.max(0.0);
        }
        let redline = self.redline_rpm.max(1.0);
        let normalized = sample_curve(&Self::DEFAULT_NORMALIZED_CURVE, rpm / redline);
        self.peak_torque_nm.max(0.0) * normalized
    }

    /// Torque curve normalized for a physics backend: `(rpm / max_rpm,
    /// torque / peak)`, strictly increasing in x.
    ///
    /// `max_rpm` is the engine's redline as clamped by the backend, which can
    /// differ slightly from [`Self::redline_rpm`]. Points above it are dropped
    /// and a final point is interpolated exactly at the redline, so the engine
    /// never reads past the authored table.
    pub fn normalized_torque_curve(&self, max_rpm: f32) -> Vec<(f32, f32)> {
        let max_rpm = if max_rpm.is_finite() && max_rpm > 0.0 {
            max_rpm
        } else {
            self.redline_rpm.max(1.0)
        };
        let points = self.torque_curve_points();
        let peak = points
            .iter()
            .map(|point| point.torque_nm)
            .fold(0.0f32, f32::max);
        if points.len() < 2 || peak <= 0.0 {
            return Self::DEFAULT_NORMALIZED_CURVE.to_vec();
        }
        let raw: Vec<(f32, f32)> = points
            .iter()
            .map(|point| (point.rpm, point.torque_nm))
            .collect();
        let torque_at = |rpm: f32| sample_curve(&raw, rpm);

        let mut curve: Vec<(f32, f32)> = Vec::with_capacity(points.len() + 2);
        let mut push = |x: f32, torque: f32| {
            let x = x.clamp(0.0, 1.0);
            let y = (torque / peak).clamp(0.0, 10.0);
            if let Some(last) = curve.last()
                && x - last.0 < 0.0015
            {
                // Keep the redline endpoint even if the last authored point
                // is a hair below it.
                if x >= 1.0 {
                    curve.pop();
                } else {
                    return;
                }
            }
            curve.push((x, y));
        };
        push(0.0, torque_at(0.0));
        for point in &points {
            if point.rpm > 0.0 && point.rpm < max_rpm {
                push(point.rpm / max_rpm, point.torque_nm);
            }
        }
        push(1.0, torque_at(max_rpm));
        if curve.len() < 2 {
            return Self::DEFAULT_NORMALIZED_CURVE.to_vec();
        }
        curve
    }

    /// Engine braking (coast) torque at `rpm`, N·m.
    ///
    /// Pumping losses and friction scale with engine speed, so the curve rises
    /// from zero at a standstill to [`Self::coast_torque_nm`] at the reference
    /// rpm. `coast_non_linearity` bends that rise towards quadratic, which is
    /// what `[COAST_REF] NON_LINEARITY` expresses in AC.
    pub fn coast_torque_at(&self, rpm: f32) -> f32 {
        let rpm = if rpm.is_finite() { rpm.abs() } else { 0.0 };
        let reference = if self.coast_reference_rpm > 0.0 {
            self.coast_reference_rpm
        } else {
            self.redline_rpm.max(1.0)
        }
        .max(1.0);
        let torque = if self.coast_torque_nm.is_finite() {
            self.coast_torque_nm.max(0.0)
        } else {
            0.0
        };
        if torque <= 0.0 {
            return 0.0;
        }
        let ratio = (rpm / reference).clamp(0.0, 1.0);
        let non_linearity = self.coast_non_linearity.clamp(0.0, 1.0);
        let shape = ratio * (1.0 - non_linearity) + ratio * ratio * non_linearity;
        torque * shape
    }

    /// Rev-limiter duty cycle at an engine speed, `0..=1`.
    ///
    /// A soft limiter does not stop the engine; it cuts the fuel repeatedly at
    /// `limiter_hz`, and the fraction of time the fuel is on falls as the rpm
    /// climbs past the redline. That is what makes a limiter feel like a
    /// bounce instead of a wall.
    pub fn limiter_duty(&self, rpm: f32) -> f32 {
        if self.limiter_hz <= 0.0 || !rpm.is_finite() {
            return 1.0;
        }
        let redline = self.redline_rpm.max(1.0);
        if rpm <= redline {
            return 1.0;
        }
        // 5 % of overspeed is a full cut.
        let excess = ((rpm - redline) / (redline * 0.05)).clamp(0.0, 1.0);
        (1.0 - excess).clamp(0.0, 1.0)
    }

    /// Power multiplier from altitude, `0..=1`.
    ///
    /// A naturally aspirated engine breathes the thinning air, so it loses
    /// power with height; AC quantifies that as a fraction per kilometre.
    pub fn altitude_multiplier(&self, altitude_m: f32) -> f32 {
        if self.altitude_sensitivity <= 0.0 || !altitude_m.is_finite() {
            return 1.0;
        }
        let km = altitude_m / 1_000.0;
        (1.0 - self.altitude_sensitivity.max(0.0) * km).clamp(0.2, 1.0)
    }

    /// Power multiplier from coolant and oil temperature, `0..=1`.
    ///
    /// The engine works at its best when hot; too cold it is detuned to protect
    /// it, and too hot the power is cut to avoid destroying it. The caller
    /// multiplies the engine torque by this value, which is the cheap way to
    /// model a protection strategy without a full engine model.
    pub fn power_multiplier(&self, coolant_c: f32, oil_c: f32) -> f32 {
        let thermal = &self.thermal;
        let coolant = if coolant_c.is_finite() {
            coolant_c.clamp(-20.0, 300.0)
        } else {
            thermal.optimal_c
        };
        let oil = if oil_c.is_finite() {
            oil_c.clamp(-20.0, 300.0)
        } else {
            thermal.optimal_c
        };
        let lost = |temperature: f32| -> f32 {
            if temperature < thermal.optimal_c {
                // Cold: the ECU holds the engine back until it warms up.
                ((thermal.optimal_c - temperature) / thermal.window_c).clamp(0.0, 1.0)
                    * thermal.cold_cut
            } else {
                // Hot: progressive power cut to save the engine.
                ((temperature - thermal.optimal_c) / thermal.window_c).clamp(0.0, 1.0)
                    * thermal.hot_cut
            }
        };
        (1.0 - lost(coolant).max(lost(oil))).clamp(thermal.min_multiplier, 1.0)
    }
}

/// Engine thermal model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineThermalSpec {
    /// Working temperature of coolant and oil, °C.
    pub optimal_c: f32,
    /// Half width of the working window, °C.
    pub window_c: f32,
    /// Power lost when fully cold, `0..=1`.
    pub cold_cut: f32,
    /// Power lost when fully overheated, `0..=1`.
    pub hot_cut: f32,
    /// Lowest power multiplier ever, `0..=1`.
    pub min_multiplier: f32,
    /// Heat capacity of the coolant loop, J/K.
    pub heat_capacity_j_per_k: f32,
    /// Fraction of the fuel's energy that ends up in the coolant.
    pub heat_share: f32,
    /// Base cooling rate at the reference speed, 1/s.
    pub cooling_per_s: f32,
}

impl Default for EngineThermalSpec {
    fn default() -> Self {
        Self {
            optimal_c: 90.0,
            window_c: 40.0,
            cold_cut: 0.1,
            hot_cut: 0.45,
            min_multiplier: 0.4,
            heat_capacity_j_per_k: 8_000.0,
            heat_share: 0.35,
            cooling_per_s: 0.02,
        }
    }
}
