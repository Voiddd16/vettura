use serde::{Deserialize, Serialize};

use crate::spec::constants::{DEFAULT_LATERAL_CURVE, DEFAULT_LONGITUDINAL_CURVE};
use crate::spec::tyre_thermal::TyreThermalSpec;
use crate::spec::tyre_wear::TyreWearSpec;
use crate::tyre_model::SlipCurve;

/// Complete tyre specification from AC `tyres.ini` and compound data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreSpec {
    /// Peak friction coefficient of the tyre compound, `DY0`/`DX0` in AC. The
    /// slip curves the backend uses peak at 1.0, so this number is what decides
    /// how much grip the car really has.
    pub peak_friction: f32,
    /// Thermal, pressure, wear, load sensitivity and relaxation behaviour.
    pub thermal: TyreThermalSpec,
    /// Wear behaviour.
    pub wear: TyreWearSpec,
    /// Cold inflation pressure, psi.
    pub cold_pressure_psi: f32,
    /// Inflation pressure for peak grip, psi.
    pub ideal_pressure_psi: f32,
    /// Vertical load the peak friction is quoted at, N. AC's `FZ0`. `0`
    /// disables load sensitivity, which is the case for hand-authored specs.
    pub reference_load_n: f32,
    /// Load sensitivity of the lateral grip, `0..=1`, below 1 so a heavily
    /// loaded tyre makes less friction per newton. AC's `LS_EXPY`.
    pub lateral_load_sensitivity: f32,
    /// Load sensitivity of the longitudinal grip. AC's `LS_EXPX`.
    pub longitudinal_load_sensitivity: f32,
    /// Distance the tyre travels before its lateral force catches up with the
    /// slip it is actually seeing, metres. AC's `RELAXATION_LENGTH`; `0`
    /// disables it.
    pub relaxation_length_m: f32,
    /// Slip angle at which the tyre reaches its friction limit, degrees. AC's
    /// `FRICTION_LIMIT_ANGLE`; this is where the lateral slip curve peaks.
    pub lateral_peak_slip_deg: f32,
    /// Longitudinal slip ratio at which the tyre peaks. AC does not publish
    /// one, so the usual 6 % is kept.
    pub longitudinal_peak_slip: f32,
    /// Share of the peak grip the tyre still has at three times the peak slip.
    /// AC's `FALLOFF_LEVEL`.
    pub falloff_level: f32,
    /// Vertical stiffness of the tyre itself, N/m. AC's `RATE`; `0` leaves the
    /// suspension spring as the whole vertical stiffness.
    pub spring_rate_n_per_m: f32,
    /// Extra vertical stiffness per psi of inflation, N/m/psi. AC's
    /// `PRESSURE_SPRING_GAIN`, referenced to [`Self::cold_pressure_psi`].
    pub pressure_spring_gain_n_per_m: f32,
    /// Camber angle the tyre wants to work at, radians (negative = top in).
    pub camber_optimal_rad: f32,
    /// Grip lost at the far edge of the camber window, `0..=1`.
    pub camber_grip_loss: f32,
    /// Half width of the camber window that costs no grip, radians.
    pub camber_window_rad: f32,
    /// Sliding energy it takes to flatten a spot into the tyre, J. A locked
    /// wheel dissipates real power in its contact patch, and that energy goes
    /// into a permanent local wear mark.
    pub flat_spot_capacity_j: f32,
    /// Grip a fully formed flat spot costs, `0..=1`.
    pub flat_spot_grip_loss: f32,
    /// Amplitude of the once-per-revolution chassis kick a flat spot produces,
    /// as a fraction of the wheel's vertical load.
    pub flat_spot_vibration_gain: f32,
    /// Distance behind the contact patch centre where the lateral force acts,
    /// metres. This is the pneumatic trail that produces the self-aligning
    /// torque: it is what makes a real steering wheel fight back and centre
    /// itself. `0` disables the torque.
    pub pneumatic_trail_m: f32,
    /// Rolling resistance, constant term, N. AC's `ROLLING_RESISTANCE_0`.
    pub rolling_resistance_n: f32,
    /// Rolling resistance, per (m/s)², N/(m/s)². AC's `ROLLING_RESISTANCE_1`.
    pub rolling_resistance_v2: f32,
    /// Rolling resistance per square radian of slip, N/rad². AC's
    /// `ROLLING_RESISTANCE_SLIP`.
    pub rolling_resistance_slip: f32,
    /// Extra rolling resistance per psi above cold, fraction. AC's
    /// `PRESSURE_RR_GAIN`.
    pub pressure_rr_gain: f32,
}

impl Default for TyreSpec {
    fn default() -> Self {
        Self {
            peak_friction: 1.4,
            thermal: TyreThermalSpec::default(),
            wear: TyreWearSpec::default(),
            cold_pressure_psi: 17.0,
            ideal_pressure_psi: 29.0,
            spring_rate_n_per_m: 0.0,
            pressure_spring_gain_n_per_m: 0.0,
            // Load sensitivity and relaxation are opt-in through AC data
            // (`FZ0`, `LS_EXP*`, `RELAXATION_LENGTH`); a hand-authored spec
            // keeps the flat behaviour it always had.
            reference_load_n: 0.0,
            lateral_load_sensitivity: 0.85,
            longitudinal_load_sensitivity: 0.95,
            relaxation_length_m: 0.0,
            lateral_peak_slip_deg: 6.0,
            longitudinal_peak_slip: 0.06,
            falloff_level: 0.85,
            camber_optimal_rad: (-3.0f32).to_radians(),
            camber_grip_loss: 0.2,
            camber_window_rad: 0.15,
            flat_spot_capacity_j: 25_000.0,
            flat_spot_grip_loss: 0.3,
            flat_spot_vibration_gain: 0.06,
            // ~20 % of a racing contact patch length: enough for the wheel to
            // feel connected at speed, not enough to fight the driver.
            pneumatic_trail_m: 0.035,
            // AC's rolling resistance is opt-in through `tyres.ini`; without
            // it the per-surface coefficient the world already had is the whole
            // model.
            rolling_resistance_n: 0.0,
            rolling_resistance_v2: 0.0,
            rolling_resistance_slip: 0.0,
            pressure_rr_gain: 0.0,
        }
    }
}

impl TyreSpec {
    /// Combined-slip friction-circle multiplier, `0..=1`.
    ///
    /// A tyre has one friction budget. Using it all longitudinally (braking)
    /// leaves nothing laterally (steering), and vice versa. The multiplier is
    /// the fraction of the budget left after `longitudinal_share` is spent,
    /// which is the classic elliptical approximation: it is smooth, symmetric
    /// and zero when the tyre is fully saturated in one direction.
    pub fn lateral_share(&self, longitudinal_share: f32) -> f32 {
        if !longitudinal_share.is_finite() {
            return 1.0;
        }
        // Magnitude matters, not direction: a wheel spinning backwards spends
        // its budget just like one spinning forwards.
        let used = longitudinal_share.abs().clamp(0.0, 1.0);
        // √(1 − u²): an ellipse, not a circle, because the longitudinal and
        // lateral friction curves have different peaks in practice.
        let left = (1.0 - used * used).max(0.0);
        left.sqrt()
    }

    /// Longitudinal grip multiplier for a given lateral share of the budget.
    pub fn longitudinal_share(&self, lateral_share: f32) -> f32 {
        if !lateral_share.is_finite() {
            return 1.0;
        }
        let used = lateral_share.abs().clamp(0.0, 1.0);
        (1.0 - used * used).max(0.0).sqrt()
    }

    /// Grip multiplier from a formed flat spot, `0..=1`.
    ///
    /// `spot` is the fraction of the flat-spot energy budget already spent. A
    /// fully formed spot takes a real bite out of the grip of that corner: the
    /// flat patch cannot conform to the road, so the contact pressure is eaten
    /// by one point once per revolution.
    pub fn flat_spot_grip_multiplier(&self, spot: f32) -> f32 {
        let spot = if spot.is_finite() {
            spot.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (1.0 - self.flat_spot_grip_loss.clamp(0.0, 1.0) * spot).clamp(0.0, 1.0)
    }

    /// Grip multiplier from a wheel's camber, `0..=1`.
    ///
    /// A tyre only makes its full contact patch at the camber it is designed
    /// for; too little (or the wrong) camber makes it run on the shoulder or
    /// the inner edge, and the grip falls off linearly across
    /// `camber_window_rad`.
    pub fn camber_grip_multiplier(&self, camber_rad: f32) -> f32 {
        let camber = if camber_rad.is_finite() {
            camber_rad
        } else {
            self.camber_optimal_rad
        };
        let window = self.camber_window_rad.max(1.0e-3);
        let error = (camber - self.camber_optimal_rad).abs() / window;
        (1.0 - self.camber_grip_loss.clamp(0.0, 1.0) * error.clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }

    /// Grip multiplier from the tyre's vertical load, `> 0`.
    ///
    /// The friction coefficient is not a constant: the *force* a tyre carries
    /// grows with the load raised to an exponent below one, so the grip per
    /// newton falls as the tyre is loaded. AC publishes that exponent as
    /// `LS_EXPY`/`LS_EXPX` against a reference load `FZ0`, and the coefficient
    /// multiplier is the same curve differentiated: `ratio^(exponent − 1)`.
    /// `1` is returned when the car does not publish a reference load.
    pub fn load_friction_multiplier(&self, load_n: f32, longitudinal: bool) -> f32 {
        let reference = if self.reference_load_n.is_finite() {
            self.reference_load_n
        } else {
            0.0
        };
        if reference <= 0.0 {
            return 1.0;
        }
        let load = if load_n.is_finite() {
            load_n.max(0.0)
        } else {
            0.0
        };
        let exponent = if longitudinal {
            self.longitudinal_load_sensitivity
        } else {
            self.lateral_load_sensitivity
        }
        .clamp(0.2, 1.0);
        let ratio = (load / reference).clamp(0.1, 3.0);
        // A light wheel makes more grip per newton than a heavily loaded one.
        ratio.powf(exponent - 1.0).clamp(0.5, 1.5)
    }

    /// Grip multiplier from how far the tyre's force lags its slip.
    ///
    /// A tyre needs `relaxation_length` metres of travel to develop the force
    /// its slip asks for. Scaling the available force by the ratio of the
    /// relaxed slip to the measured one reproduces that exactly in the linear
    /// range: the force is proportional to the *relaxed* slip, so it builds up
    /// over a distance instead of appearing in one step.
    pub fn relaxation_multiplier(&self, measured: f32, relaxed: f32) -> f32 {
        let measured = if measured.is_finite() {
            measured.abs()
        } else {
            0.0
        };
        let relaxed = if relaxed.is_finite() {
            relaxed.abs()
        } else {
            0.0
        };
        if measured <= 1.0e-4 {
            return 1.0;
        }
        (relaxed / measured).clamp(0.0, 1.0)
    }

    /// Friction curves of this tyre: `(longitudinal over slip ratio, lateral
    /// over slip angle in degrees)`.
    ///
    /// Both tables are Magic Formula curves fitted to the car's own peak slip
    /// and falloff, normalised to a peak of 1.0 so a backend's own peak-friction
    /// setting decides how grippy the tyre really is. Cars whose data cannot be
    /// fitted (or that publish nothing) fall back to the flat table that this
    /// crate used before, so nothing regresses.
    pub fn slip_curves(&self) -> (crate::spec::FrictionTable, crate::spec::FrictionTable) {
        let longitudinal = SlipCurve::fitted(self.longitudinal_peak_slip, self.falloff_level)
            .map(|curve| curve.points(true))
            .unwrap_or_else(|| DEFAULT_LONGITUDINAL_CURVE.to_vec());
        let lateral = SlipCurve::fitted(self.lateral_peak_slip_deg, self.falloff_level)
            .map(|curve| curve.points(false))
            .unwrap_or_else(|| DEFAULT_LATERAL_CURVE.to_vec());
        (longitudinal, lateral)
    }

    /// Grip multiplier for a wheel from its temperature, pressure and wear.
    ///
    /// Three effects combine: the temperature window (a cold tyre and an
    /// overheated one both lose grip), the pressure deviation from the ideal
    /// value, and wear. Each term is bounded so a tyre never becomes
    /// frictionless.
    pub fn grip_multiplier(&self, temperature_c: f32, pressure_psi: f32, wear: f32) -> f32 {
        let thermal = &self.thermal;
        let temperature = if temperature_c.is_finite() {
            temperature_c.clamp(0.0, 2_000.0)
        } else {
            thermal.optimal_c
        };
        // Temperature: the authored curve when the car has one, otherwise the
        // analytic window.
        let temp_loss = 1.0 - thermal.temperature_multiplier(temperature);

        let pressure_width = (0.35 * self.ideal_pressure_psi).max(1.0);
        let pressure_error = if pressure_psi.is_finite() {
            ((pressure_psi - self.ideal_pressure_psi) / pressure_width).clamp(-1.5, 1.5)
        } else {
            0.0
        };
        let pressure_loss = pressure_error * pressure_error;

        let worn = if wear.is_finite() {
            wear.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let wear_loss = worn * worn * self.wear.grip_loss;

        // A hot tyre past its window also grains and blisters; the slip part
        // of that is applied by the caller.
        let hot_slip = ((temperature - thermal.optimal_c) / thermal.window_c).clamp(0.0, 1.5)
            * thermal.blister_gain;

        let loss = 0.30 * temp_loss + 0.12 * pressure_loss + wear_loss + 0.08 * hot_slip;
        (1.0 - loss).clamp(0.65, 1.0)
    }

    /// Inflation pressure at a carcass temperature, psi.
    ///
    /// The air in the tyre obeys the gas law: pressure rises linearly with the
    /// absolute temperature, so a hot tyre runs at a higher pressure and the
    /// grip window moves with it.
    pub fn pressure_at(&self, temperature_c: f32) -> f32 {
        let thermal = &self.thermal;
        let temperature = if temperature_c.is_finite() {
            temperature_c
        } else {
            thermal.optimal_c
        };
        // The denominator is the temperature rise to the hot-pressure point;
        // it must stay positive even for a hand-authored curve whose window
        // sits at or below zero.
        let denominator = (thermal.optimal_c + thermal.window_c).max(1.0);
        let gain = ((self.ideal_pressure_psi - self.cold_pressure_psi) / denominator).max(0.02);
        // Negative rubber temperatures are legal and simply mean a flatter
        // tyre than the cold pressure; only the absolute floor stays positive.
        (self.cold_pressure_psi + gain * temperature).clamp(0.5, 120.0)
    }

    /// Slip energy needed to wear a tyre out, joules.
    pub fn wear_capacity_j(&self) -> f32 {
        self.wear.energy_capacity_j.max(1.0)
    }

    /// Self-aligning torque for one wheel, N·m.
    ///
    /// The lateral force acts `pneumatic_trail_m` behind the contact patch
    /// centre, so it produces a moment about the wheel's own vertical axis that
    /// always rotates the wheel back towards the direction of travel. Signed so
    /// that it opposes the steer the way a real steering wheel does.
    pub fn aligning_torque_nm(&self, lateral_force_n: f32) -> f32 {
        if self.pneumatic_trail_m <= 0.0 || !lateral_force_n.is_finite() {
            return 0.0;
        }
        (lateral_force_n * self.pneumatic_trail_m).clamp(-2_000.0, 2_000.0)
    }

    /// Rolling resistance force of one wheel, N.
    ///
    /// AC's model is three terms: a constant, one that grows with the square of
    /// the road speed, and one that grows with the square of the slip. Pressure
    /// above the cold value adds a fractional amount, which is why a hot tyre
    /// rolls slightly harder.
    pub fn rolling_resistance_n(&self, speed_mps: f32, slip_rad: f32, pressure_psi: f32) -> f32 {
        let constant = self.rolling_resistance_n.max(0.0);
        let speed = if speed_mps.is_finite() {
            speed_mps.abs().min(120.0)
        } else {
            0.0
        };
        let slip = if slip_rad.is_finite() {
            slip_rad.abs().min(2.0)
        } else {
            0.0
        };
        let pressure = if pressure_psi.is_finite() {
            (pressure_psi - self.cold_pressure_psi).clamp(-10.0, 40.0)
        } else {
            0.0
        };
        let gain = (1.0 + self.pressure_rr_gain.clamp(0.0, 1.0) * pressure.max(0.0)).max(0.0);
        let force = constant
            + self.rolling_resistance_v2.max(0.0) * speed * speed
            + self.rolling_resistance_slip.max(0.0) * slip * slip;
        (force * gain).clamp(0.0, 5_000.0)
    }

    /// Vertical rate of this tyre at a given inflation pressure, N/m.
    ///
    /// AC publishes the tyre's own rate (`RATE`) and how much stiffer it gets
    /// per psi of pressure (`PRESSURE_SPRING_GAIN`, referenced to the cold
    /// pressure). `0` means the car does not author it, in which case the
    /// suspension spring is the whole vertical stiffness, which is what Jolt
    /// models on its own.
    pub fn vertical_rate_n_per_m(&self, pressure_psi: f32) -> f32 {
        let base = if self.spring_rate_n_per_m.is_finite() {
            self.spring_rate_n_per_m.max(0.0)
        } else {
            0.0
        };
        if base <= 0.0 {
            return 0.0;
        }
        let pressure = if pressure_psi.is_finite() {
            pressure_psi
        } else {
            self.cold_pressure_psi
        };
        let gain = if self.pressure_spring_gain_n_per_m.is_finite() {
            self.pressure_spring_gain_n_per_m.clamp(-50_000.0, 50_000.0)
        } else {
            0.0
        };
        let rate = base + gain * (pressure - self.cold_pressure_psi);
        rate.clamp(1_000.0, 2_000_000.0)
    }
}
