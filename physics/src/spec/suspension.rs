use serde::{Deserialize, Serialize};

use crate::spec::wheel::Wheel;

/// Suspension parameters for one corner, from AC `suspensions.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionSpec {
    pub spring_rate_n_per_m: f32,
    pub damper_bump_ns_per_m: f32,
    pub damper_rebound_ns_per_m: f32,
    pub travel_m: f32,
    pub anti_roll_n_per_m: f32,
    /// Extra spring force that rises with the square of compression, N/m².
    /// AC's `PROGRESSIVE_SPRING_RATE`: a real spring gets stiffer the more it
    /// is compressed, which keeps the car off the bump stops under load.
    pub progressive_rate_n_per_m2: f32,
    /// Spring rate of the bump stop, N/m. `0` disables the modelled stop and
    /// leaves Jolt's hard suspension limit as the only end stop.
    pub bump_stop_rate_n_per_m: f32,
    /// Compression at which the bump stop engages, metres.
    pub bump_stop_travel_m: f32,
    /// Spring rate of the rebound stop, N/m. `0` disables it.
    pub rebound_stop_rate_n_per_m: f32,
    /// Extension at which the rebound stop engages, metres.
    pub rebound_stop_travel_m: f32,
    /// Low-speed damper rate in compression, Ns/m. AC's `DAMP_FAST_BUMP`.
    pub fast_bump_damper_ns_per_m: f32,
    /// Compression speed above which the fast (bleeder) rate applies, m/s.
    /// AC's `DAMP_FAST_BUMPTHRESHOLD`.
    pub fast_bump_threshold_mps: f32,
    /// Low-speed damper rate in extension, Ns/m. AC's `DAMP_FAST_REBOUND`.
    pub fast_rebound_damper_ns_per_m: f32,
    /// Extension speed above which the fast rate applies, m/s. AC's
    /// `DAMP_FAST_REBOUNDTHRESHOLD`.
    pub fast_rebound_threshold_mps: f32,
    /// Static camber in degrees, negative leaning the tyre tops inward. AC's
    /// `STATIC_CAMBER`.
    pub static_camber_deg: f32,
    /// Camber change per metre of compression, rad/m. A wishbone gains
    /// negative camber as the wheel moves up, which is what keeps the contact
    /// patch flat in a corner.
    pub camber_gain_rad_per_m: f32,
    /// Camber thrust, the lateral fraction of the vertical load per radian of
    /// camber. It points toward the side the tyre leans, so a negative wheel
    /// helps the corner instead of fighting it.
    pub camber_thrust_coeff: f32,
    /// Height of this axle's roll centre above the road, metres.
    ///
    /// Jolt applies the tyre's lateral force at the contact patch, which gives
    /// the maximum possible roll moment about the centre of mass. A real car
    /// transfers that force at the roll centre instead, so the moment is
    /// smaller by `roll_center_height × Fy`. `0` keeps Jolt's (maximum) lever
    /// arm, which is the case for hand-authored specs.
    pub roll_center_height_m: f32,
    /// Fraction of the braking load transfer this axle's suspension geometry
    /// reacts, `0..=1`. Jolt applies the longitudinal tyre force at the contact
    /// patch, the longest lever arm about the centre of mass; a real axle picks
    /// up part of that force in its links, so the pitch is smaller.
    pub anti_dive_fraction: f32,
    /// Fraction of the acceleration load transfer reacted by the geometry,
    /// `0..=1`. Same mechanism as [`Self::anti_dive_fraction`] under power,
    /// where a real rear axle resists squat.
    pub anti_squat_fraction: f32,
    /// Caster angle of the steering axis, degrees. Positive leans the axis
    /// backwards, which puts the contact patch behind the steering pivot and
    /// makes the wheel self-centre.
    pub caster_angle_deg: f32,
    /// Kingpin (steering-axis) inclination, degrees. Steering a wheel with an
    /// inclined axis leans it, which is how a steered corner also picks up
    /// camber without any body roll.
    pub kingpin_angle_deg: f32,
}

impl Default for SuspensionSpec {
    fn default() -> Self {
        Self {
            spring_rate_n_per_m: 80_000.0,
            damper_bump_ns_per_m: 4_000.0,
            damper_rebound_ns_per_m: 6_000.0,
            travel_m: 0.08,
            anti_roll_n_per_m: 0.0,
            progressive_rate_n_per_m2: 0.0,
            bump_stop_rate_n_per_m: 0.0,
            bump_stop_travel_m: 0.08,
            rebound_stop_rate_n_per_m: 0.0,
            rebound_stop_travel_m: 0.08,
            fast_bump_damper_ns_per_m: 0.0,
            fast_bump_threshold_mps: 0.0,
            fast_rebound_damper_ns_per_m: 0.0,
            fast_rebound_threshold_mps: 0.0,
            static_camber_deg: 0.0,
            camber_gain_rad_per_m: -0.5,
            camber_thrust_coeff: 0.3,
            // A typical double-wishbone race axle: a few centimetres above the
            // road, well below the centre of mass. AC's own hard points are
            // authored in a frame that cannot be cross-referenced with
            // TRACK, so a mod that knows the real number uses ROLL_CENTER.
            roll_center_height_m: 0.05,
            // A road car reacts a modest share of its load transfer in the
            // suspension links; racers run more. Both can be authored.
            anti_dive_fraction: 0.15,
            anti_squat_fraction: 0.10,
            // Typical road-car steering-axis geometry.
            caster_angle_deg: 4.0,
            kingpin_angle_deg: 8.0,
        }
    }
}

impl SuspensionSpec {
    /// Force from the progressive spring and the two end stops, newtons.
    ///
    /// `compression_m` is how far the suspension is compressed from its rest
    /// length and `extension_m` how far it is extended. Positive is upward on
    /// the chassis, so a compressed suspension pushes up and an over-extended
    /// one pulls down. These are the nonlinear parts Jolt's linear spring does
    /// not model; adding them in parallel is exact, not a duplicate.
    pub fn auxiliary_force_n(&self, compression_m: f32, extension_m: f32) -> f32 {
        let compression = if compression_m.is_finite() {
            compression_m.max(0.0)
        } else {
            0.0
        };
        let extension = if extension_m.is_finite() {
            extension_m.max(0.0)
        } else {
            0.0
        };
        let mut force = self.progressive_rate_n_per_m2.max(0.0) * compression * compression;
        let bump = self.bump_stop_travel_m.max(0.0);
        if self.bump_stop_rate_n_per_m > 0.0 && compression > bump {
            force += self.bump_stop_rate_n_per_m * (compression - bump);
        }
        let rebound = self.rebound_stop_travel_m.max(0.0);
        if self.rebound_stop_rate_n_per_m > 0.0 && extension > rebound {
            force -= self.rebound_stop_rate_n_per_m * (extension - rebound);
        }
        if force.is_finite() {
            force.clamp(-200_000.0, 200_000.0)
        } else {
            0.0
        }
    }

    /// Camber a steered wheel picks up from the steering axis, radians.
    ///
    /// Turning a wheel whose steering axis is inclined leans the wheel, so a
    /// car running lock has camber on the steered axle without any body roll
    /// at all. This is the reason a steering-heavy corner also loads the
    /// outside tyre's edge.
    pub fn steer_camber_gain(&self, steer_angle_rad: f32) -> f32 {
        if !steer_angle_rad.is_finite() {
            return 0.0;
        }
        -self.kingpin_angle_deg.to_radians() * steer_angle_rad.sin()
    }

    /// Caster trail: how far behind the steering pivot the contact patch sits,
    /// metres. It is the lever the vertical load pushes on to centre the
    /// wheel, and it is what makes a shopping-trolley wheel track straight.
    pub fn caster_trail_m(&self, wheel_radius_m: f32) -> f32 {
        let radius = if wheel_radius_m.is_finite() {
            wheel_radius_m.max(0.05)
        } else {
            0.05
        };
        (radius * self.caster_angle_deg.to_radians().tan()).clamp(0.0, 0.5)
    }

    /// Camber of the wheel at a given compression, radians.
    pub fn camber_rad(&self, compression_m: f32) -> f32 {
        let compression = if compression_m.is_finite() {
            compression_m.max(0.0)
        } else {
            0.0
        };
        self.static_camber_deg.to_radians() + self.camber_gain_rad_per_m * compression
    }

    /// Camber thrust of one wheel, newtons, on an axis that points away from
    /// the car.
    ///
    /// Negative camber produces a negative (inward) force on both sides, so a
    /// loaded outside wheel pulls the car into the corner. Signed this way, a
    /// pair of equally loaded wheels with equal static camber cancels out.
    pub fn camber_thrust_n(&self, camber_rad: f32, load_n: f32) -> f32 {
        let camber = if camber_rad.is_finite() {
            camber_rad
        } else {
            0.0
        };
        let load = if load_n.is_finite() {
            load_n.max(0.0)
        } else {
            0.0
        };
        self.camber_thrust_coeff.max(0.0) * load * camber
    }

    /// Rate Jolt's linear damper works at, Ns/m.
    ///
    /// Jolt's wheel settings carry one symmetric rate, so the average of the
    /// two authored rates is what the solver applies; the difference and the
    /// fast-regime rates are added back in parallel by the world.
    pub fn jolt_damper_rate_ns_per_m(&self) -> f32 {
        0.5 * (self.damper_bump_ns_per_m.max(0.0) + self.damper_rebound_ns_per_m.max(0.0))
    }

    /// Damper rate a wheel should see at a suspension velocity, Ns/m.
    ///
    /// A real damper is neither symmetric nor linear: it has separate bump and
    /// rebound rates, and most racing dampers also have a bleeder (fast) rate
    /// that takes over above a speed threshold so a kerb strike is not
    /// transmitted straight into the chassis.
    pub fn damper_rate_ns_per_m(&self, suspension_velocity_mps: f32) -> f32 {
        let velocity = if suspension_velocity_mps.is_finite() {
            suspension_velocity_mps
        } else {
            0.0
        };
        if velocity < 0.0 {
            let threshold = self.fast_bump_threshold_mps.clamp(0.0, 5.0);
            if self.fast_bump_damper_ns_per_m > 0.0 && -velocity > threshold {
                self.fast_bump_damper_ns_per_m.max(0.0)
            } else {
                self.damper_bump_ns_per_m.max(0.0)
            }
        } else if velocity > 0.0 {
            let threshold = self.fast_rebound_threshold_mps.clamp(0.0, 5.0);
            if self.fast_rebound_damper_ns_per_m > 0.0 && velocity > threshold {
                self.fast_rebound_damper_ns_per_m.max(0.0)
            } else {
                self.damper_rebound_ns_per_m.max(0.0)
            }
        } else {
            0.0
        }
    }
}

/// How the active suspension actuator is allowed to work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveSuspensionMode {
    /// Passive springs and dampers only.
    Off,
    /// The actuator adds damping but does not hold a ride height.
    SemiActive,
    /// The actuator holds a target ride height and body attitude.
    Active,
}

/// Active or semi-active suspension.
///
/// The actuator works in parallel with the spring: it measures the suspension
/// length and pushes the chassis up or down to hold a target. Because it acts
/// on the chassis at the wheel mounts it automatically keeps the body flat in
/// roll and pitch, which is what active road cars and ground-effect racers use
/// it for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSuspensionSpec {
    pub mode: ActiveSuspensionMode,
    /// Actuator stiffness against ride-height error, N/m.
    pub stiffness_n_per_m: f32,
    /// Actuator damping against suspension velocity, Ns/m.
    pub damping_ns_per_m: f32,
    /// Maximum force the actuator can exert at one corner, N.
    pub max_force_n: f32,
    /// Target ride height relative to the passive rest position, metres.
    /// Positive raises the body.
    pub ride_height_m: f32,
    /// Extra front ride height relative to the rear (positive rake), metres.
    pub rake_m: f32,
    /// Ride-height reduction once at speed, metres. Positive lowers the body.
    pub speed_drop_m: f32,
    /// Speed at which the speed-dependent drop is half applied, m/s.
    pub speed_ref_mps: f32,
}

impl Default for ActiveSuspensionSpec {
    fn default() -> Self {
        Self {
            mode: ActiveSuspensionMode::Off,
            stiffness_n_per_m: 0.0,
            damping_ns_per_m: 0.0,
            max_force_n: 0.0,
            ride_height_m: 0.0,
            rake_m: 0.0,
            speed_drop_m: 0.0,
            speed_ref_mps: 50.0,
        }
    }
}

impl ActiveSuspensionSpec {
    /// Whether the actuator can do anything.
    pub fn is_enabled(&self) -> bool {
        self.mode != ActiveSuspensionMode::Off
            && (self.stiffness_n_per_m > 0.0 || self.damping_ns_per_m > 0.0)
    }

    /// Ride-height lowering from speed, metres. `0` at rest, approaching
    /// `speed_drop_m` at high speed.
    pub fn speed_drop_at(&self, speed_mps: f32) -> f32 {
        let speed = if speed_mps.is_finite() {
            speed_mps.max(0.0)
        } else {
            0.0
        };
        let reference = self.speed_ref_mps.max(1.0);
        let speed_sq = speed * speed;
        self.speed_drop_m.max(0.0) * speed_sq / (speed_sq + reference * reference)
    }

    /// Target suspension length relative to the passive rest length, metres.
    pub fn target_offset_m(&self, wheel: Wheel, speed_mps: f32) -> f32 {
        let rake = if wheel.is_front() {
            self.rake_m * 0.5
        } else {
            -self.rake_m * 0.5
        };
        (self.ride_height_m + rake - self.speed_drop_at(speed_mps)).clamp(-0.15, 0.15)
    }

    /// Actuator force for one corner, newtons upward on the chassis.
    pub fn force_n(&self, length_error_m: f32, velocity_mps: f32) -> f32 {
        if !self.is_enabled() {
            return 0.0;
        }
        let error = if length_error_m.is_finite() {
            length_error_m
        } else {
            0.0
        };
        let velocity = if velocity_mps.is_finite() {
            velocity_mps
        } else {
            0.0
        };
        let stiffness = if self.mode == ActiveSuspensionMode::Active {
            self.stiffness_n_per_m.max(0.0) * error
        } else {
            0.0
        };
        // Damping always resists motion: compressing (negative velocity) gets
        // an upward force, extending gets a downward one.
        let damping = -self.damping_ns_per_m.max(0.0) * velocity;
        let force = stiffness + damping;
        let limit = self.max_force_n.max(0.0);
        if limit > 0.0 {
            force.clamp(-limit, limit)
        } else {
            force
        }
    }
}

/// A heave (third) spring connecting both wheels of one axle to the chassis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HeaveSpringSpec {
    /// Spring rate, N/m, from AC `[HEAVE_FRONT]`/`[HEAVE_REAR]` `SPRING_RATE`.
    pub spring_rate_n_per_m: f32,
    /// Progressive rate, N/m².
    pub progressive_rate_n_per_m2: f32,
    /// Bump-stop rate, N/m.
    pub bump_stop_rate_n_per_m: f32,
    /// Compression at which the bump stop engages, metres.
    pub bump_stop_travel_m: f32,
    /// Compression built in at the design ride height, metres. AC's
    /// `ROD_LENGTH`: the heave element is already loaded before the car moves.
    pub preload_m: f32,
    /// Damper rate in compression, Ns/m.
    pub damper_bump_ns_per_m: f32,
    /// Damper rate in rebound, Ns/m.
    pub damper_rebound_ns_per_m: f32,
}

impl Default for HeaveSpringSpec {
    fn default() -> Self {
        Self {
            spring_rate_n_per_m: 0.0,
            progressive_rate_n_per_m2: 0.0,
            bump_stop_rate_n_per_m: 0.0,
            bump_stop_travel_m: 0.05,
            preload_m: 0.0,
            damper_bump_ns_per_m: 0.0,
            damper_rebound_ns_per_m: 0.0,
        }
    }
}

impl HeaveSpringSpec {
    /// Upward force on the chassis for an average axle compression, newtons.
    ///
    /// `velocity_mps` is the rate of change of the average suspension length:
    /// negative while the axle compresses, positive while it extends. The
    /// damper always opposes that motion.
    pub fn force_n(&self, compression_m: f32, velocity_mps: f32) -> f32 {
        let compression = if compression_m.is_finite() {
            (compression_m + self.preload_m).max(0.0)
        } else {
            self.preload_m.max(0.0)
        };
        let velocity = if velocity_mps.is_finite() {
            velocity_mps
        } else {
            0.0
        };
        let mut force = self.spring_rate_n_per_m.max(0.0) * compression
            + self.progressive_rate_n_per_m2.max(0.0) * compression * compression;
        let travel = self.bump_stop_travel_m.max(0.0);
        if self.bump_stop_rate_n_per_m > 0.0 && compression > travel {
            force += self.bump_stop_rate_n_per_m * (compression - travel);
        }
        let damper = if velocity < 0.0 {
            self.damper_bump_ns_per_m.max(0.0)
        } else {
            self.damper_rebound_ns_per_m.max(0.0)
        };
        force -= damper * velocity;
        if force.is_finite() {
            force.clamp(-100_000.0, 300_000.0)
        } else {
            0.0
        }
    }
}
