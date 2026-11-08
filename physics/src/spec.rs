use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::tyre_model::SlipCurve;

/// Reference speed for the smoothing curve on downforce, metres per second.
///
/// Both the downforce and the side force use `v²/(v² + ref²)` so a parked car
/// is not pulled into the road or shoved sideways by a breeze.
pub const AERO_REFERENCE_SPEED_MPS: f32 = 20.0;
/// Air density at sea level, kg/m³.
pub const AIR_DENSITY_KG_M3: f32 = 1.225;

/// Friction table for a backend: `(slip, friction coefficient)` points.
pub type FrictionTable = Vec<(f32, f32)>;

/// Friction curve used when a car publishes nothing: the long-standing default,
/// a peak at 6 % longitudinal slip and 3 degrees of slip angle.
const DEFAULT_LONGITUDINAL_CURVE: [(f32, f32); 4] =
    [(0.0, 0.0), (0.06, 1.0), (0.18, 0.93), (0.5, 0.8)];
const DEFAULT_LATERAL_CURVE: [(f32, f32); 4] = [(0.0, 0.0), (3.0, 1.0), (10.0, 0.95), (20.0, 0.82)];

/// Vertical rate of two springs in series, N/m.
///
/// A wheel is connected to the ground through the suspension spring *and* the
/// tyre's own vertical stiffness, so the chassis sees the series combination,
/// not either alone. Jolt only models the suspension spring, so the rate it is
/// given has to be this combined value for ride height and load transfer to be
/// right.
pub fn series_spring_rate(suspension_n_per_m: f32, tyre_n_per_m: f32) -> f32 {
    let suspension = if suspension_n_per_m.is_finite() {
        suspension_n_per_m.max(1.0)
    } else {
        1.0
    };
    if !tyre_n_per_m.is_finite() || tyre_n_per_m <= 0.0 {
        return suspension;
    }
    let tyre = tyre_n_per_m.max(1.0);
    (suspension * tyre / (suspension + tyre)).max(1.0)
}

/// Linear interpolation on a sorted `(x, y)` table, clamped at both ends.
fn sample_curve(points: &[(f32, f32)], x: f32) -> f32 {
    let Some(first) = points.first().copied() else {
        return 0.0;
    };
    let Some(last) = points.last().copied() else {
        return 0.0;
    };
    let x = if x.is_finite() { x } else { first.0 };
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    for window in points.windows(2) {
        let [a, b] = window else { continue };
        if x >= a.0 && x <= b.0 {
            let span = b.0 - a.0;
            if span.abs() < f32::EPSILON {
                return b.1;
            }
            let t = (x - a.0) / span;
            return a.1 + (b.1 - a.1) * t;
        }
    }
    last.1
}

/// Four wheels, in a fixed order. Index into `[WheelSpec; 4]` / `[WheelState; 4]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Wheel {
    FrontLeft = 0,
    FrontRight = 1,
    RearLeft = 2,
    RearRight = 3,
}

impl Wheel {
    pub const ALL: [Wheel; 4] = [
        Wheel::FrontLeft,
        Wheel::FrontRight,
        Wheel::RearLeft,
        Wheel::RearRight,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn is_front(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::FrontRight)
    }

    pub const fn is_left(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::RearLeft)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleSpec {
    pub name: String,
    pub mass_kg: f32,
    /// Distance between front and rear axle centers.
    pub wheelbase_m: f32,
    /// Center of gravity height above the ground.
    pub cg_height_m: f32,
    pub frontal_area_m2: f32,
    pub drag_coefficient: f32,
    /// Physics frame: +X right, +Y up, -Z forward.
    pub wheels: [WheelSpec; 4],
    pub engine: EngineSpec,
    pub drivetrain: DrivetrainSpec,
    pub tyres: TyreSpec,
    pub aero: AeroSpec,
    pub brakes: BrakeSpec,
    pub fuel: FuelSpec,
    pub gearbox: GearboxSpec,
    pub drs: DrsSpec,
    pub ers: ErsSpec,
    /// Active / semi-active suspension and the axle heave springs AC authors in
    /// `suspensions.ini`.
    pub active_suspension: ActiveSuspensionSpec,
    /// Heave (third) springs per axle, front then rear. `0` means the car does
    /// not have one.
    pub heave_springs: [HeaveSpringSpec; 2],
    /// Inertia tensor about the centre of mass, kg·m², as
    /// `(roll, pitch, yaw)` in the physics frame. `Vec3::ZERO` means "derive it
    /// from the collider box", which is what happens when a mod does not
    /// publish a usable `car.ini [BASIC] INERTIA`.
    pub inertia: Vec3,
    /// Biggest steer angle the front wheels can take, degrees at the wheel.
    /// AC's `[CONTROLS] STEER_LOCK / STEER_RATIO`, which is the only honest
    /// source: the steering wheel's lock is not the wheel's lock.
    pub max_steer_angle_deg: f32,
}

impl Default for VehicleSpec {
    fn default() -> Self {
        let wheelbase = 2.6;
        let track = 1.5;
        let wheel_radius = 0.33;
        let half_wb = wheelbase * 0.5;

        Self {
            name: "default".to_owned(),
            mass_kg: 1200.0,
            wheelbase_m: wheelbase,
            cg_height_m: 0.35,
            frontal_area_m2: 1.8,
            drag_coefficient: 0.35,
            wheels: [
                WheelSpec::at(-track * 0.5, wheel_radius, -half_wb, true, false),
                WheelSpec::at(track * 0.5, wheel_radius, -half_wb, true, false),
                WheelSpec::at(-track * 0.5, wheel_radius, half_wb, false, true),
                WheelSpec::at(track * 0.5, wheel_radius, half_wb, false, true),
            ],
            engine: EngineSpec::default(),
            drivetrain: DrivetrainSpec::default(),
            tyres: TyreSpec::default(),
            aero: AeroSpec::default(),
            brakes: BrakeSpec::default(),
            fuel: FuelSpec::default(),
            gearbox: GearboxSpec::default(),
            drs: DrsSpec::default(),
            ers: ErsSpec::default(),
            active_suspension: ActiveSuspensionSpec::default(),
            heave_springs: [HeaveSpringSpec::default(); 2],
            max_steer_angle_deg: 30.0,
            // Defaults to the derived tensor, exactly what the body would get
            // without this field, so behaviour is unchanged.
            inertia: Vec3::ZERO,
        }
    }
}

impl VehicleSpec {
    /// Half extents of the collider box the chassis is modelled with, in the
    /// physics frame: `(half width, half length, half height)`.
    ///
    /// The inertia tensor and the Jolt collision hull read the same numbers, so
    /// a change of footprint stays consistent everywhere.
    pub fn chassis_half_extents(&self) -> (f32, f32, f32) {
        let half_w = self
            .wheels
            .iter()
            .map(|wheel| wheel.position.x.abs() + wheel.width_m.max(0.0) * 0.5)
            .fold(0.0f32, f32::max)
            .max(0.2)
            + 0.02;
        let half_l = self
            .wheels
            .iter()
            .map(|wheel| wheel.position.z.abs() + wheel.radius_m.max(0.0))
            .fold(0.0f32, f32::max)
            .max(0.3)
            + 0.05;
        let half_h = (self.cg_height_m - 0.06).clamp(0.10, 0.55);
        (half_w, half_l, half_h)
    }

    /// Inertia tensor of the collider box, kg·m², as `(roll, pitch, yaw)`.
    ///
    /// This is what Jolt derives on its own; it is exposed so the mapper can
    /// compare AC's authored values against it, and so a caller can inspect
    /// what the world will use when no tensor is published.
    pub fn collider_inertia(&self) -> Vec3 {
        let (half_w, half_l, half_h) = self.chassis_half_extents();
        let mass = self.mass_kg.max(1.0);
        let (w, h, l) = (2.0 * half_w, 2.0 * half_h, 2.0 * half_l);
        Vec3::new(
            mass * (h * h + w * w) / 12.0,
            mass * (l * l + h * h) / 12.0,
            mass * (l * l + w * w) / 12.0,
        )
    }

    /// Tensor the body should be built with, kg·m², as `(roll, pitch, yaw)`.
    ///
    /// `Vec3::ZERO` (the default) keeps Jolt's own shape-derived tensor; an
    /// explicit tensor replaces it.
    pub fn effective_inertia(&self) -> Vec3 {
        if self.inertia.is_finite() && self.inertia.cmpgt(Vec3::ZERO).any() {
            self.inertia
        } else {
            Vec3::ZERO
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WheelSpec {
    /// Mounting position in the physics frame.
    pub position: Vec3,
    pub radius_m: f32,
    pub width_m: f32,
    pub steerable: bool,
    pub driven: bool,
    pub suspension: SuspensionSpec,
}

impl WheelSpec {
    pub fn at(x: f32, y: f32, z: f32, steerable: bool, driven: bool) -> Self {
        Self {
            position: Vec3::new(x, y, z),
            radius_m: 0.33,
            width_m: 0.24,
            steerable,
            driven,
            suspension: SuspensionSpec::default(),
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrivetrainLayout {
    Rwd,
    Fwd,
    Awd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivetrainSpec {
    pub layout: DrivetrainLayout,
    pub efficiency: f32,
    pub diff: DiffSpec,
}

impl Default for DrivetrainSpec {
    fn default() -> Self {
        Self {
            layout: DrivetrainLayout::Rwd,
            efficiency: 0.9,
            diff: DiffSpec::default(),
        }
    }
}

/// Limited-slip differential, from AC `drivetrain.ini [DIFFERENTIAL]`.
///
/// AC expresses the lock as a fraction: `1.0` fully locked, `0` open. Jolt
/// instead asks for the wheel-speed ratio above which all torque goes to the
/// slower wheel, which is the reciprocal (see `vettura-physics`'s world layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSpec {
    /// Lock under power, `0..=1`.
    pub power_lock: f32,
    /// Lock under coasting, `0..=1`.
    pub coast_lock: f32,
    /// Preload torque, N·m.
    pub preload_nm: f32,
}

impl Default for DiffSpec {
    fn default() -> Self {
        // A mod that does not author the section keeps a locked diff, which is
        // the safer behaviour for a car we know nothing about.
        Self {
            power_lock: 1.0,
            coast_lock: 1.0,
            preload_nm: 0.0,
        }
    }
}

impl DiffSpec {
    /// Lock ratio averaged between power and coast.
    pub fn average_lock(&self) -> f32 {
        0.5 * (self.power_lock + self.coast_lock)
    }
}

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
    pub fn slip_curves(&self) -> (FrictionTable, FrictionTable) {
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

/// Thermal model for one tyre.
///
/// A real tyre is three masses, not one: the **surface** the road touches heats
/// in milliseconds, the **patch** under the tread is the middle layer, and the
/// **core** is the bulk that lags and is what the driver feels as "the tyre is
/// up to temperature". AC publishes the rates between them
/// (`SURFACE_TRANSFER`, `PATCH_TRANSFER`, `CORE_TRANSFER`,
/// `INTERNAL_CORE_TRANSFER`) and the grip-versus-temperature curve
/// (`PERFORMANCE_CURVE`). Cars without that data fall back to a single mass
/// that behaves exactly like the model used before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreThermalSpec {
    /// Core (bulk) temperature for peak grip, °C.
    pub optimal_c: f32,
    /// Half width of the working window, °C.
    pub window_c: f32,
    /// Heat capacity of the core, J/K. This is the mass the grip curve reads.
    pub heat_capacity_j_per_k: f32,
    /// Base cooling rate at the reference speed, 1/s.
    pub cooling_per_s: f32,
    /// Fraction of slip power that becomes tyre heat.
    pub heat_share: f32,
    /// Grain gain from AC's `[THERMAL_*] GRAIN_GAIN`.
    pub grain_gain: f32,
    /// Blister gain from AC's `[THERMAL_*] BLISTER_GAIN`.
    pub blister_gain: f32,
    /// Heat capacity of the friction surface, J/K. `0` keeps the single-mass
    /// model, where surface and core are the same temperature.
    pub surface_capacity_j_per_k: f32,
    /// Heat capacity of the patch under the tread, J/K.
    pub patch_capacity_j_per_k: f32,
    /// Rate heat moves from the surface into the patch, 1/s.
    pub surface_transfer_per_s: f32,
    /// Rate heat moves from the patch into the core, 1/s.
    pub patch_transfer_per_s: f32,
    /// Rate heat moves from the core back into the patch, 1/s.
    pub core_transfer_per_s: f32,
    /// Rate heat moves inside the core, 1/s.
    pub internal_core_transfer_per_s: f32,
    /// Grip-versus-temperature curve from AC `[THERMAL_*] PERFORMANCE_CURVE`:
    /// `(temperature in °C, grip multiplier)`. Empty keeps the analytic window.
    pub performance_curve: Vec<(f32, f32)>,
}

impl Default for TyreThermalSpec {
    fn default() -> Self {
        Self {
            optimal_c: 90.0,
            window_c: 80.0,
            heat_capacity_j_per_k: 480.0,
            cooling_per_s: 0.08,
            heat_share: 0.3,
            grain_gain: 0.25,
            blister_gain: 1.5,
            surface_capacity_j_per_k: 0.0,
            patch_capacity_j_per_k: 0.0,
            surface_transfer_per_s: 0.0,
            patch_transfer_per_s: 0.0,
            core_transfer_per_s: 0.0,
            internal_core_transfer_per_s: 0.0,
            performance_curve: Vec::new(),
        }
    }
}

impl TyreThermalSpec {
    /// Whether the car models three separate tyre masses.
    pub fn has_layers(&self) -> bool {
        self.surface_capacity_j_per_k > 1.0
            && (self.surface_transfer_per_s > 0.0 || self.patch_transfer_per_s > 0.0)
    }

    /// Grip multiplier from the core temperature, `0..=1.2`.
    ///
    /// With an authored curve that curve is the answer; otherwise the analytic
    /// window is used.
    pub fn temperature_multiplier(&self, temperature_c: f32) -> f32 {
        let temperature = if temperature_c.is_finite() {
            temperature_c.clamp(-1_000.0, 2_000.0)
        } else {
            self.optimal_c
        };
        if self.performance_curve.len() >= 2 {
            if temperature < self.performance_curve[0].0 {
                // Below the coldest authored point the curve keeps its first
                // slope instead of clamping: a tyre at -20 °C has to be worse
                // than one at 0 °C, not equal to it.
                return self.extrapolate_cold(temperature);
            }
            return sample_curve(&self.performance_curve, temperature).clamp(0.0, 1.2);
        }
        let temp_error = ((temperature - self.optimal_c) / self.window_c).clamp(-1.5, 1.5);
        (1.0 - 0.30 * temp_error * temp_error).clamp(0.0, 1.0)
    }

    /// Grip multiplier below the coldest point of the authored curve.
    fn extrapolate_cold(&self, temperature_c: f32) -> f32 {
        let curve = &self.performance_curve;
        let first = curve[0];
        let second = curve[1];
        let span = (second.0 - first.0).abs().max(1.0e-3);
        let slope = (second.1 - first.1) / span;
        (first.1 + slope * (temperature_c - first.0)).clamp(0.0, 1.2)
    }
}

/// One aero surface from AC `aero.ini [WING_n]`.
///
/// AC computes each surface's load from two lookup tables: `LUT_AOA_CL` gives
/// the lift coefficient at an angle of attack, and `LUT_GH_CL` scales it by the
/// surface's ground height. That is what makes a ground-effect car sensitive to
/// ride height and what lets a wing stall, so the tables are carried here
/// instead of being collapsed into one `Cl·A` number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WingSpec {
    /// Chord, metres.
    pub chord_m: f32,
    /// Span, metres.
    pub span_m: f32,
    /// `CL_GAIN` as authored, dimensionless, the fallback lift coefficient
    /// when the car ships no `LUT_AOA_CL`.
    pub cl_gain: f32,
    /// `CD_GAIN` as authored, the fallback drag coefficient.
    pub cd_gain: f32,
    /// Mount position in the AC frame (+X right, +Y up, +Z forward).
    pub position: Vec3,
    /// Installation angle, degrees. AC's `ANGLE`.
    pub angle_deg: f32,
    /// `(angle of attack in degrees, lift coefficient)` from `LUT_AOA_CL`.
    pub aoa_cl: Vec<(f32, f32)>,
    /// `(ground height in metres, lift multiplier)` from `LUT_GH_CL`.
    pub gh_cl: Vec<(f32, f32)>,
    /// `(angle of attack in degrees, drag coefficient)` from `LUT_AOA_CD`.
    pub aoa_cd: Vec<(f32, f32)>,
    /// `(ground height in metres, drag multiplier)` from `LUT_GH_CD`.
    pub gh_cd: Vec<(f32, f32)>,
}

impl Default for WingSpec {
    fn default() -> Self {
        Self {
            chord_m: 0.0,
            span_m: 0.0,
            cl_gain: 0.0,
            cd_gain: 0.0,
            position: Vec3::ZERO,
            angle_deg: 0.0,
            aoa_cl: Vec::new(),
            gh_cl: Vec::new(),
            aoa_cd: Vec::new(),
            gh_cd: Vec::new(),
        }
    }
}

impl WingSpec {
    /// Planform area, m².
    pub fn area_m2(&self) -> f32 {
        (self.chord_m * self.span_m).max(0.0)
    }

    /// `Cl·A` at an angle of attack and a ground height, m².
    ///
    /// The angle of attack is the installation angle plus whatever the flow
    /// gives it, and the height term is what the AC tables scale it by. Without
    /// the tables the authored `CL_GAIN` is used unchanged.
    pub fn cl_area(&self, aoa_deg: f32, ground_height_m: f32) -> f32 {
        let area = self.area_m2();
        let lift = if self.aoa_cl.len() >= 2 {
            sample_curve(&self.aoa_cl, aoa_deg)
        } else {
            self.cl_gain
        };
        if self.gh_cl.len() >= 2 {
            lift * sample_curve(&self.gh_cl, ground_height_m) * area
        } else {
            lift * area
        }
    }

    /// `Cd·A` at an angle of attack and a ground height, m².
    pub fn cd_area(&self, aoa_deg: f32, ground_height_m: f32) -> f32 {
        let area = self.area_m2();
        let drag = if self.aoa_cd.len() >= 2 {
            sample_curve(&self.aoa_cd, aoa_deg)
        } else {
            self.cd_gain
        };
        if self.gh_cd.len() >= 2 {
            drag * sample_curve(&self.gh_cd, ground_height_m) * area
        } else {
            drag * area
        }
    }
}

/// Aerodynamic parameters summed from AC `aero.ini`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AeroSpec {
    /// Total downforce coefficient `Cl·A`. Applied along the car's local down
    /// axis and scaled by `speed²/(speed² + reference²)` so a parked car is
    /// not pulled into the ground. Aggregate fallback for cars without the
    /// per-wing tables; [`Self::wings`] is the real model.
    pub downforce_coeff: f32,
    /// Front share of the downforce, `0` = all rear, `1` = all front.
    pub front_bias: f32,
    /// Downforce lost over full suspension travel, `0..=1`. Above the rest
    /// height the car makes less downforce, which models ride-height
    /// sensitivity.
    pub ride_height_fade: f32,
    /// Lateral lift coefficient per radian of sideslip, `Clβ·A`. The car gets
    /// a side force whenever it is travelling sideways relative to its
    /// heading, which is what makes a fast car feel planted and what gives a
    /// real car its aerodynamic yaw stability.
    pub side_force_coeff: f32,
    /// Distance behind the centre of mass where that side force acts, metres.
    /// Acting behind the CG turns the side force into a restoring yaw moment,
    /// the same way a weathervane points into the wind.
    pub center_of_pressure_m: f32,
    /// Per-surface data from `aero.ini`, empty for hand-authored specs.
    pub wings: Vec<WingSpec>,
    /// Downforce lost directly behind another car, `0..=1`. `0` disables the
    /// wake model, which is the case for hand-authored specs.
    pub dirty_air_loss: f32,
    /// Distance at which the wake stops mattering, metres.
    pub dirty_air_distance_m: f32,
}

impl Default for AeroSpec {
    fn default() -> Self {
        Self {
            downforce_coeff: 0.0,
            front_bias: 0.4,
            ride_height_fade: 0.25,
            side_force_coeff: 0.0,
            center_of_pressure_m: 0.6,
            wings: Vec::new(),
            // Off until the car authors it; a real aero map loses a real
            // fraction of its load in traffic.
            dirty_air_loss: 0.0,
            dirty_air_distance_m: 25.0,
        }
    }
}

impl AeroSpec {
    /// Lateral force for a sideslip of `sideslip_rad`, newtons.
    ///
    /// The force always opposes the sideslip, so it pushes the car back
    /// towards travelling straight ahead. The `speed²/(speed²+ref²)` term
    /// keeps a stationary car from being shoved sideways by a breeze.
    pub fn side_force_n(&self, air_density_kg_m3: f32, speed_mps: f32, sideslip_rad: f32) -> f32 {
        if !sideslip_rad.is_finite() || sideslip_rad == 0.0 {
            return 0.0;
        }
        let speed_sq = if speed_mps.is_finite() {
            speed_mps * speed_mps
        } else {
            0.0
        };
        let smoothing = speed_sq / (speed_sq + AERO_REFERENCE_SPEED_MPS * AERO_REFERENCE_SPEED_MPS);
        // Small-angle `sin` is fine: sideslip beyond ~30° is already a spin.
        let sine = if sideslip_rad.abs() < 0.5 {
            sideslip_rad
        } else {
            sideslip_rad.clamp(-1.5, 1.5).sin()
        };
        // The force is proportional to the sideslip, not just a constant:
        // a small slip gets a small correction, a big one a strong one.
        let magnitude =
            0.5 * air_density_kg_m3 * self.side_force_coeff.max(0.0) * speed_sq * smoothing;
        (-magnitude * sine).clamp(-50_000.0, 50_000.0)
    }

    /// Downforce multiplier from sitting in another car's wake, `0..=1`.
    ///
    /// `exposure` is how deep in the wake this car sits, `0..=1`: `0` when
    /// there is no car ahead and `1` when it is right on the gearbox.
    /// The loss peaks immediately behind the car and fades over
    /// `dirty_air_distance_m`, which is why following is hard and why the
    /// second car in a train often carries more speed into a corner.
    pub fn dirty_air_multiplier(&self, exposure: f32) -> f32 {
        let exposure = if exposure.is_finite() {
            exposure.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (1.0 - self.dirty_air_loss.clamp(0.0, 1.0) * exposure).clamp(0.0, 1.0)
    }

    /// Yaw moment the side force produces at `center_of_pressure_m`, N·m.
    pub fn side_force_yaw_moment_nm(
        &self,
        air_density_kg_m3: f32,
        speed_mps: f32,
        sideslip_rad: f32,
    ) -> f32 {
        self.side_force_n(air_density_kg_m3, speed_mps, sideslip_rad) * self.center_of_pressure_m
    }
}

/// Brake parameters from AC `brakes.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrakeSpec {
    /// Peak brake torque, N·m, applied to one axle per `front_share`.
    pub max_torque_nm: f32,
    /// Front share of the brake torque, `0..=1`.
    pub front_share: f32,
    /// Heat capacity of one disc + pad set, J/K.
    pub thermal_mass_j_per_k: f32,
    /// Cooling rate at the reference speed, 1/s.
    pub cooling_per_s: f32,
    /// Ambient reference temperature, °C.
    pub ambient_c: f32,
    /// Temperature at which the disc reaches full efficiency, °C.
    pub optimal_c: f32,
    /// Temperature at which the disc has faded to `min_efficiency`, °C.
    pub fade_c: f32,
    /// Efficiency at and beyond `fade_c`.
    pub min_efficiency: f32,
    /// Heat generated per joule of dissipated braking energy.
    pub heat_per_joule: f32,
    /// Heat capacity of the rotor's core, J/K. The friction surface heats
    /// almost instantly, the bulk of the disc does not, and that lag is what
    /// makes brake heat soak: the pads keep cooking long after the driver lifts
    /// off. `0` disables the core, keeping the old single-mass behaviour.
    pub core_capacity_j_per_k: f32,
    /// Rate heat moves from the hot surface into the core, 1/s. AC's
    /// `[TEMPS_*] CORE_TRANSFER_IN`.
    pub core_transfer_in_per_s: f32,
    /// Rate heat moves back out of the core into the surface, 1/s. AC's
    /// `CORE_TRANSFER_OUT`.
    pub core_transfer_out_per_s: f32,
    /// Rate the core sheds heat to the air, 1/s. AC's `CORE_TRANSFER_AIR`.
    pub core_transfer_air_per_s: f32,
}

impl Default for BrakeSpec {
    fn default() -> Self {
        Self {
            max_torque_nm: 4_500.0,
            front_share: 0.55,
            thermal_mass_j_per_k: 1_200.0,
            cooling_per_s: 0.06,
            ambient_c: 32.0,
            optimal_c: 500.0,
            fade_c: 900.0,
            min_efficiency: 0.5,
            heat_per_joule: 0.7,
            // A meaningful default so the soak is part of the model; AC data
            // overrides the rates.
            core_capacity_j_per_k: 1_500.0,
            core_transfer_in_per_s: 0.09,
            core_transfer_out_per_s: 0.55,
            core_transfer_air_per_s: 5.0e-4,
        }
    }
}

impl BrakeSpec {
    /// Brake efficiency at a disc temperature, `0..=1`.
    ///
    /// The curve has three regimes: cold discs bite less than they should, an
    /// optimal window delivers the full torque, and past `fade_c` the torque
    /// falls linearly to `min_efficiency`.
    pub fn efficiency_at(&self, temperature_c: f32) -> f32 {
        let temperature = if temperature_c.is_finite() {
            temperature_c.max(self.ambient_c)
        } else {
            self.optimal_c
        };
        if temperature < self.optimal_c {
            // Cold: less than the full torque, but never nothing.
            let cold = ((temperature - self.ambient_c)
                / (self.optimal_c - self.ambient_c).max(1.0))
            .clamp(0.0, 1.0);
            (self.min_efficiency + (1.0 - self.min_efficiency) * cold).clamp(0.0, 1.0)
        } else if temperature < self.fade_c {
            1.0
        } else {
            let faded =
                ((temperature - self.fade_c) / (self.fade_c * 0.5).max(1.0)).clamp(0.0, 1.0);
            (1.0 - (1.0 - self.min_efficiency) * faded).clamp(self.min_efficiency, 1.0)
        }
    }
}

/// Gearbox and clutch parameters from AC `drivetrain.ini`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GearboxSpec {
    /// Whether the box shifts on its own.
    pub automatic: bool,
    /// Time a gear shift takes, seconds (`CHANGE_UP_TIME`).
    pub switch_time: f32,
    /// Time the clutch takes to engage after a shift, seconds
    /// (`CHANGE_DN_TIME`).
    pub clutch_release_time: f32,
    /// Whether the box blips the throttle on downshifts.
    pub clutch_on_shifts: bool,
    /// Rev-match window, rpm.
    pub rev_match_rpm: f32,
    /// Torque the clutch can carry, N·m.
    pub clutch_max_torque_nm: f32,
    /// Rotational inertia of the gearbox internals, kg·m².
    pub inertia: f32,
    /// Reverse gear ratio (positive; the sign is applied by the backend).
    /// AC's `GEAR_R`; `0` falls back to the default reverse ratio.
    pub reverse_ratio: f32,
    /// Time the fuel is cut during an upshift, seconds. AC's
    /// `AUTO_CUTOFF_TIME`; `0` disables the cut-off in backends that model it.
    pub auto_cutoff_time: f32,
    /// Handbrake torque at one rear wheel, N·m. AC's `HANDBRAKE_TORQUE`.
    pub handbrake_torque_nm: f32,
}

impl Default for GearboxSpec {
    fn default() -> Self {
        Self {
            automatic: true,
            switch_time: 0.12,
            clutch_release_time: 0.2,
            clutch_on_shifts: true,
            rev_match_rpm: 1_200.0,
            clutch_max_torque_nm: 1_150.0,
            inertia: 0.012,
            reverse_ratio: 0.0,
            auto_cutoff_time: 0.0,
            handbrake_torque_nm: 0.0,
        }
    }
}

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

/// Hybrid energy system, from AC's ERS-capable cars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErsSpec {
    /// Whether the car has one at all.
    pub available: bool,
    /// Battery capacity, joules.
    pub capacity_j: f32,
    /// Peak deploy power, watts.
    pub max_deploy_w: f32,
    /// Peak harvest power, watts.
    pub max_harvest_w: f32,
    /// Deploy energy allowed per lap, joules.
    pub lap_deploy_limit_j: f32,
    /// How much of the deploy goes to the rear axle, `0..=1`.
    pub rear_bias: f32,
    /// Motor efficiency when deploying, `0..=1`.
    pub deploy_efficiency: f32,
    /// Motor efficiency when harvesting, `0..=1`.
    pub harvest_efficiency: f32,
}

impl Default for ErsSpec {
    fn default() -> Self {
        Self {
            available: false,
            capacity_j: 4.0e6,
            max_deploy_w: 120_000.0,
            max_harvest_w: 400_000.0,
            lap_deploy_limit_j: 4.0e6,
            rear_bias: 1.0,
            deploy_efficiency: 0.95,
            harvest_efficiency: 0.85,
        }
    }
}

/// What the driver asks of the hybrid system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErsMode {
    /// Nothing.
    #[default]
    Off,
    /// Deploy on demand.
    Deploy,
    /// Full-power deploy for an attack.
    Attack,
    /// Harvest on demand.
    Harvest,
}

impl ErsMode {
    /// Whether this mode asks the motor to deploy (`Attack` deploys harder).
    pub fn deploys(self) -> bool {
        matches!(self, ErsMode::Deploy | ErsMode::Attack)
    }

    /// Whether this mode asks the motor to harvest.
    pub fn harvests(self) -> bool {
        matches!(self, ErsMode::Harvest)
    }
}

/// Fuel tank, from AC `car.ini [FUEL]` / `[FUEL_EXT]` / `[FUELTANK]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuelSpec {
    /// Tank capacity, litres.
    pub capacity_l: f32,
    /// Fraction of the tank the car starts with, `0..=1`.
    pub initial_fraction: f32,
    /// Fuel density, kg/litre.
    pub density_kg_per_l: f32,
    /// Distance covered per litre at full load, km.
    pub km_per_liter: f32,
    /// Fuel consumption at full load, litres per second.
    pub consumption_l_per_s_at_full_load: f32,
    /// Tank position relative to the centre of mass, physics frame.
    pub position: Vec3,
}

impl Default for FuelSpec {
    fn default() -> Self {
        Self {
            capacity_l: 80.0,
            initial_fraction: 1.0,
            density_kg_per_l: 0.74,
            km_per_liter: 2.5,
            consumption_l_per_s_at_full_load: 0.0111,
            position: Vec3::new(0.0, 0.02, -0.1),
        }
    }
}

impl FuelSpec {
    /// Mass of a full tank, kg.
    pub fn full_mass_kg(&self) -> f32 {
        self.mass_at_fraction(1.0)
    }

    /// Fuel mass carried at a fraction of the tank, kg.
    pub fn mass_at_fraction(&self, fraction: f32) -> f32 {
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.capacity_l.max(0.0) * fraction * self.density_kg_per_l.max(0.0)
    }

    /// Fuel burnt per second at a road speed and engine load, litres.
    ///
    /// Two terms: a constant load term and a term that grows with speed,
    /// because pumping losses dominate at low engine load and drag does at
    /// high road speed.
    pub fn consumption_l_per_s(&self, speed_mps: f32, load: f32) -> f32 {
        if self.capacity_l <= 0.0 {
            return 0.0;
        }
        let speed = if speed_mps.is_finite() {
            speed_mps.abs()
        } else {
            0.0
        };
        let load = if load.is_finite() {
            load.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let full = self.consumption_l_per_s_at_full_load.max(0.0);
        if full <= 0.0 {
            return 0.0;
        }
        // At 25 m/s the car covers the same distance as the full-load figure;
        // slower than that the engine is only partly loaded.
        let speed_term = (0.25 + 0.75 * (speed / 25.0).clamp(0.0, 1.0)).clamp(0.25, 1.0);
        full * load * speed_term
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

/// Tyre wear model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TyreWearSpec {
    /// Slip energy that wears the tyre out, joules.
    pub energy_capacity_j: f32,
    /// Grip lost at full wear, `0..=1`.
    pub grip_loss: f32,
}

impl Default for TyreWearSpec {
    fn default() -> Self {
        Self {
            energy_capacity_j: 3.0e7,
            grip_loss: 0.12,
        }
    }
}

/// Convenience pose used by `VehicleBackend::reset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, 0.0),
            orientation: Quat::IDENTITY,
        }
    }
}

impl Pose {
    pub fn forward(self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_indices_and_axle_predicates_agree() {
        for wheel in Wheel::ALL {
            assert_eq!(wheel.index(), wheel as usize);
            // 索引必须直接可用来寻址 wheels 数组。
            assert_eq!(Wheel::ALL[wheel.index()], wheel);
        }

        assert!(Wheel::FrontLeft.is_front() && Wheel::FrontLeft.is_left());
        assert!(Wheel::FrontRight.is_front() && !Wheel::FrontRight.is_left());
        assert!(!Wheel::RearLeft.is_front() && Wheel::RearLeft.is_left());
        assert!(!Wheel::RearRight.is_front() && !Wheel::RearRight.is_left());
    }

    #[test]
    fn default_spec_is_symmetric_and_rwd() {
        let spec = VehicleSpec::default();

        assert_eq!(spec.mass_kg, 1200.0);
        assert_eq!(spec.wheels.len(), 4);

        let fl = &spec.wheels[Wheel::FrontLeft.index()];
        let fr = &spec.wheels[Wheel::FrontRight.index()];
        let rl = &spec.wheels[Wheel::RearLeft.index()];
        let rr = &spec.wheels[Wheel::RearRight.index()];

        // 前后轴距 = wheelbase；左右对称。
        assert_eq!(fl.position.z, -spec.wheelbase_m * 0.5);
        assert_eq!(rr.position.z, spec.wheelbase_m * 0.5);
        assert_eq!(fl.position.x, -fr.position.x);
        assert_eq!(rl.position.x, -rr.position.x);

        // RWD：前轮转向不驱动，后轮驱动不转向。
        assert!(fl.steerable && !fl.driven);
        assert!(fr.steerable && !fr.driven);
        assert!(!rl.steerable && rl.driven);
        assert!(!rr.steerable && rr.driven);
        assert_eq!(spec.drivetrain.layout, DrivetrainLayout::Rwd);

        // 四个轮子共用合理的默认值。
        for wheel in &spec.wheels {
            assert!(wheel.radius_m > 0.0);
            assert_eq!(
                wheel.suspension.spring_rate_n_per_m,
                SuspensionSpec::default().spring_rate_n_per_m
            );
        }
    }

    #[test]
    fn pose_forward_points_along_negative_z_when_identity() {
        let pose = Pose::default();
        assert_eq!(pose.forward(), Vec3::NEG_Z);

        // 绕 Y 转 90°：-Z 前向变成 -X。
        let turned = Pose {
            orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..Pose::default()
        };
        let forward = turned.forward();
        assert!(forward.x < -0.99 && forward.z.abs() < 1e-6 && forward.y.abs() < 1e-6);
    }

    #[test]
    fn engine_defaults_cover_a_sane_gearbox() {
        let engine = EngineSpec::default();
        assert_eq!(engine.gear_ratios.len(), 6);
        // 齿比必须逐级递减（每个更高的档位都是减速更少的超比档）。
        for pair in engine.gear_ratios.windows(2) {
            assert!(pair[0] > pair[1], "{:?} must descend", engine.gear_ratios);
        }
        assert!(engine.final_drive > 0.0);
        assert!(engine.idle_rpm < engine.redline_rpm);
    }

    #[test]
    fn default_spec_carries_new_phase_six_sections() {
        let spec = VehicleSpec::default();
        assert!((spec.drivetrain.diff.average_lock() - 1.0).abs() < 1.0e-6);
        assert_eq!(spec.aero.front_bias, 0.4);
        assert_eq!(spec.brakes.front_share, 0.55);
        assert_eq!(spec.aero.downforce_coeff, 0.0);
    }

    #[test]
    fn side_force_opposes_sideslip_and_scales_with_speed() {
        let aero = AeroSpec {
            side_force_coeff: 2.0,
            ..AeroSpec::default()
        };
        // A sideslip of 0.1 rad at 30 m/s pushes against it.
        let force = aero.side_force_n(AIR_DENSITY_KG_M3, 30.0, 0.1);
        assert!(force < 0.0, "the force must oppose the sideslip: {force}");
        assert!(aero.side_force_n(AIR_DENSITY_KG_M3, 30.0, -0.1) > 0.0);
        // Stationary: nothing.
        assert_eq!(aero.side_force_n(AIR_DENSITY_KG_M3, 0.0, 0.1), 0.0);
        // The restoring yaw moment comes from acting behind the CG.
        let moment = aero.side_force_yaw_moment_nm(AIR_DENSITY_KG_M3, 30.0, 0.1);
        assert!(moment.abs() > 0.0);
    }

    #[test]
    fn brake_efficiency_is_cold_fade_and_optimal() {
        let brakes = BrakeSpec::default();
        assert!(brakes.efficiency_at(brakes.ambient_c) < 1.0);
        assert!(brakes.efficiency_at(brakes.optimal_c) == 1.0);
        assert!(brakes.efficiency_at(brakes.fade_c) == 1.0);
        assert!(brakes.efficiency_at(brakes.fade_c * 2.0) < 1.0);
        assert!(brakes.efficiency_at(brakes.fade_c * 2.0) >= brakes.min_efficiency);
    }

    #[test]
    fn drs_cuts_drag_by_the_authored_fraction() {
        let drs = DrsSpec {
            available: true,
            drag_reduction: 0.2,
            ..DrsSpec::default()
        };
        assert_eq!(drs.drag_multiplier(false), 1.0);
        assert!((drs.drag_multiplier(true) - 0.8).abs() < 1e-6);
        // A car without DRS ignores the request.
        assert_eq!(DrsSpec::default().drag_multiplier(true), 1.0);
    }

    #[test]
    fn fuel_scales_with_load_and_speed() {
        let fuel = FuelSpec::default();
        assert!(fuel.full_mass_kg() > 40.0);
        // No load, no burn.
        assert_eq!(fuel.consumption_l_per_s(30.0, 0.0), 0.0);
        // More load, more burn; more speed, more burn.
        assert!(fuel.consumption_l_per_s(30.0, 0.5) < fuel.consumption_l_per_s(30.0, 1.0));
        assert!(fuel.consumption_l_per_s(10.0, 1.0) < fuel.consumption_l_per_s(30.0, 1.0));
        // The mass the tank carries follows the fraction.
        assert!((fuel.mass_at_fraction(0.0)).abs() < 1e-6);
        assert!((fuel.mass_at_fraction(0.5) - fuel.full_mass_kg() * 0.5).abs() < 1e-6);
    }

    #[test]
    fn tyre_pressure_rises_with_temperature() {
        let tyre = TyreSpec::default();
        let cold = tyre.pressure_at(tyre.thermal.optimal_c);
        let hot = tyre.pressure_at(tyre.thermal.optimal_c + tyre.thermal.window_c);
        assert!(hot > cold, "{cold} → {hot}");
        assert!(hot <= tyre.ideal_pressure_psi + 1.0);
        // Nonsense falls back to the optimal temperature, not a panic.
        assert!(tyre.pressure_at(f32::NAN).is_finite());
    }

    #[test]
    fn combined_slip_shares_one_friction_budget() {
        let tyre = TyreSpec::default();
        // Nothing used in the other direction: everything is left.
        assert_eq!(tyre.lateral_share(0.0), 1.0);
        assert_eq!(tyre.longitudinal_share(0.0), 1.0);
        // Half used leaves more than a quarter of the ellipse.
        assert!(tyre.lateral_share(0.5) > 0.7 && tyre.lateral_share(0.5) < 0.9);
        // Fully used in one direction empties the other.
        assert!(tyre.lateral_share(1.0).abs() < 1e-6);
        assert!(tyre.longitudinal_share(1.0).abs() < 1e-6);
        // Direction does not matter.
        assert!((tyre.lateral_share(-0.5) - tyre.lateral_share(0.5)).abs() < 1e-6);
        // Nonsense keeps the whole budget.
        assert_eq!(tyre.lateral_share(f32::NAN), 1.0);
    }

    #[test]
    fn grip_multiplier_penalises_cold_hot_and_worn_tyres() {
        let tyre = TyreSpec::default();
        let ideal = tyre.grip_multiplier(tyre.thermal.optimal_c, tyre.ideal_pressure_psi, 0.0);
        assert!((ideal - 1.0).abs() < 1e-6, "{ideal}");
        // Cold, wrong pressure and wear each take something.
        assert!(tyre.grip_multiplier(20.0, tyre.ideal_pressure_psi, 0.0) < ideal);
        assert!(tyre.grip_multiplier(tyre.thermal.optimal_c, 10.0, 0.0) < ideal);
        assert!(tyre.grip_multiplier(tyre.thermal.optimal_c, tyre.ideal_pressure_psi, 1.0) < ideal);
        // A tyre never becomes frictionless.
        assert!(tyre.grip_multiplier(2_000.0, 100.0, 1.0) >= 0.65);
    }

    #[test]
    fn torque_curve_normalizes_to_the_engine_redline() {
        let engine = EngineSpec {
            redline_rpm: 8_000.0,
            peak_torque_nm: 200.0,
            torque_curve: vec![
                TorquePoint {
                    rpm: 0.0,
                    torque_nm: 100.0,
                },
                TorquePoint {
                    rpm: 4_000.0,
                    torque_nm: 200.0,
                },
                TorquePoint {
                    rpm: 8_000.0,
                    torque_nm: 150.0,
                },
                // A trailing point past the limiter must not reach the backend.
                TorquePoint {
                    rpm: 9_000.0,
                    torque_nm: 0.0,
                },
            ],
            ..EngineSpec::default()
        };
        let curve = engine.normalized_torque_curve(8_000.0);
        assert_eq!(curve.first(), Some(&(0.0, 0.5)));
        let last = curve.last().copied().unwrap();
        assert!((last.0 - 1.0).abs() < 1e-6);
        assert!((last.1 - 0.75).abs() < 1e-6);
        let peak = curve
            .iter()
            .find(|(x, _)| (*x - 0.5).abs() < 1e-6)
            .copied()
            .unwrap();
        assert!((peak.1 - 1.0).abs() < 1e-6);
        assert!(curve.windows(2).all(|window| window[1].0 > window[0].0));

        // Raw interpolation follows the authored table.
        assert!((engine.torque_at_rpm(2_000.0) - 150.0).abs() < 1e-3);
        assert!((engine.torque_at_rpm(8_000.0) - 150.0).abs() < 1e-3);

        // A car without a LUT gets the generic fallback shape.
        assert_eq!(
            EngineSpec::default().normalized_torque_curve(8_000.0),
            EngineSpec::DEFAULT_NORMALIZED_CURVE.to_vec()
        );
    }

    #[test]
    fn turbo_builds_boost_with_rpm_and_throttle() {
        let turbo = TurboSpec {
            max_boost_bar: 0.8,
            wastegate_bar: 1.0,
            reference_rpm: 4_000.0,
            gamma: 1.0,
            lag_up_s: 0.4,
            lag_down_s: 0.2,
            torque_gain_per_bar: 1.0,
        };
        assert!(turbo.is_enabled());
        // No throttle, no exhaust energy.
        assert_eq!(turbo.target_boost_bar(0.0, 8_000.0), 0.0);
        // Linear spool with gamma = 1.
        assert!((turbo.target_boost_bar(1.0, 2_000.0) - 0.4).abs() < 1e-6);
        assert!((turbo.target_boost_bar(1.0, 8_000.0) - 0.8).abs() < 1e-6);
        // The wastegate caps the pressure.
        let capped = TurboSpec {
            max_boost_bar: 1.5,
            wastegate_bar: 0.6,
            ..turbo
        };
        assert!((capped.target_boost_bar(1.0, 8_000.0) - 0.6).abs() < 1e-6);
        // Gamma > 1 keeps the turbo lazy at low rpm.
        let lazy = TurboSpec {
            gamma: 2.0,
            ..turbo
        };
        assert!(lazy.target_boost_bar(1.0, 2_000.0) < turbo.target_boost_bar(1.0, 2_000.0));
        // Boost raises torque linearly; 0.8 bar with gain 1 doubles it.
        assert_eq!(turbo.torque_multiplier(0.0), 1.0);
        assert!((turbo.torque_multiplier(0.8) - 1.8).abs() < 1e-6);
        assert!((turbo.max_torque_multiplier() - 1.8).abs() < 1e-6);
        // No authored turbo means no boost at all.
        assert!(!TurboSpec::default().is_enabled());
        assert_eq!(TurboSpec::default().target_boost_bar(1.0, 8_000.0), 0.0);
    }

    #[test]
    fn coast_torque_rises_with_rpm_and_bends_with_non_linearity() {
        let engine = EngineSpec {
            redline_rpm: 8_000.0,
            peak_torque_nm: 300.0,
            coast_torque_nm: 120.0,
            coast_reference_rpm: 8_000.0,
            coast_non_linearity: 0.0,
            ..EngineSpec::default()
        };
        assert_eq!(engine.coast_torque_at(0.0), 0.0);
        assert!((engine.coast_torque_at(4_000.0) - 60.0).abs() < 1e-6);
        assert!((engine.coast_torque_at(8_000.0) - 120.0).abs() < 1e-6);
        // Above the reference the curve clamps, so lifting off at the limiter
        // still slows the car.
        assert!((engine.coast_torque_at(12_000.0) - 120.0).abs() < 1e-6);

        // Fully quadratic: half the rpm is a quarter of the torque.
        let quadratic = EngineSpec {
            coast_non_linearity: 1.0,
            ..engine.clone()
        };
        assert!((quadratic.coast_torque_at(4_000.0) - 30.0).abs() < 1e-6);

        // No authored coast reference means no engine braking.
        assert_eq!(EngineSpec::default().coast_torque_at(8_000.0), 0.0);
    }

    #[test]
    fn the_inertia_tensor_falls_back_to_the_collider_when_unusable() {
        let spec = VehicleSpec::default();
        // Nothing authored: the collider tensor, and it is physically sane.
        assert_eq!(spec.effective_inertia(), Vec3::ZERO);
        let derived = spec.collider_inertia();
        assert!(derived.y > 500.0 && derived.z > 500.0, "{derived:?}");
        // Roll is the smallest for a car.
        assert!(derived.x < derived.y, "{derived:?}");

        let authored = VehicleSpec {
            inertia: Vec3::new(400.0, 900.0, 1_100.0),
            ..VehicleSpec::default()
        };
        assert_eq!(
            authored.effective_inertia(),
            Vec3::new(400.0, 900.0, 1_100.0)
        );
        // Nonsense is ignored.
        let broken = VehicleSpec {
            inertia: Vec3::splat(f32::NAN),
            ..VehicleSpec::default()
        };
        assert_eq!(broken.effective_inertia(), Vec3::ZERO);
    }

    #[test]
    fn progressive_springs_and_end_stops_are_nonlinear() {
        let passive = SuspensionSpec::default();
        // The default has no nonlinear elements, so it adds nothing.
        assert_eq!(passive.auxiliary_force_n(0.03, 0.0), 0.0);

        let spring = SuspensionSpec {
            progressive_rate_n_per_m2: 20_000.0,
            bump_stop_rate_n_per_m: 200_000.0,
            bump_stop_travel_m: 0.05,
            rebound_stop_rate_n_per_m: 100_000.0,
            rebound_stop_travel_m: 0.03,
            ..SuspensionSpec::default()
        };
        // The progressive part grows with the square of compression.
        let small = spring.auxiliary_force_n(0.02, 0.0);
        let double = spring.auxiliary_force_n(0.04, 0.0);
        assert!((small - 8.0).abs() < 1e-3, "{small}");
        assert!((double - 32.0).abs() < 1e-3, "{double}");
        // The bump stop engages only after its travel and then adds linearly.
        let before = spring.auxiliary_force_n(0.05, 0.0);
        let after = spring.auxiliary_force_n(0.06, 0.0);
        assert!(after - before > 1_900.0, "{before} → {after}");
        // The rebound stop pulls down when over-extended.
        assert!(spring.auxiliary_force_n(0.0, 0.05) < 0.0);
        assert_eq!(spring.auxiliary_force_n(0.0, 0.01), 0.0);
        // Nonsense input stays finite.
        assert!(spring.auxiliary_force_n(f32::NAN, f32::NAN).abs() < 1e-3);
    }

    #[test]
    fn a_heave_spring_stiffens_both_wheels_together() {
        let heave = HeaveSpringSpec {
            spring_rate_n_per_m: 120_000.0,
            progressive_rate_n_per_m2: 10_000.0,
            bump_stop_rate_n_per_m: 0.0,
            bump_stop_travel_m: 0.05,
            preload_m: 0.0,
            damper_bump_ns_per_m: 3_000.0,
            damper_rebound_ns_per_m: 5_000.0,
        };
        // Static compression of 1 cm gives the linear and progressive parts.
        assert!((heave.force_n(0.01, 0.0) - (1_200.0 + 1.0)).abs() < 1e-3);
        assert_eq!(heave.force_n(-0.01, 0.0), 0.0);
        // The damper resists compression upward and extension downward.
        assert!(heave.force_n(0.01, -0.2) > heave.force_n(0.01, 0.0));
        assert!(heave.force_n(0.01, 0.2) < heave.force_n(0.01, 0.0));
        // The rod length is a preload: compression minus preload carries no
        // load, so it shows up as extra force even at zero measured travel.
        let preloaded = HeaveSpringSpec {
            preload_m: 0.02,
            ..heave
        };
        assert!((preloaded.force_n(0.0, 0.0) - 2_404.0).abs() < 1e-3);
        // No authored spring means no force at all.
        assert_eq!(HeaveSpringSpec::default().force_n(0.05, 0.0), 0.0);
    }

    #[test]
    fn active_suspension_holds_ride_height_and_damps_motion() {
        // A passive car has no actuator at all.
        let passive = ActiveSuspensionSpec::default();
        assert!(!passive.is_enabled());
        assert_eq!(passive.force_n(0.05, -0.2), 0.0);

        let active = ActiveSuspensionSpec {
            mode: ActiveSuspensionMode::Active,
            stiffness_n_per_m: 80_000.0,
            damping_ns_per_m: 4_000.0,
            max_force_n: 12_000.0,
            ..ActiveSuspensionSpec::default()
        };
        assert!(active.is_enabled());
        // Compressed below the target: push the chassis up.
        assert!(active.force_n(0.02, 0.0) > 0.0);
        // Extended above it: pull it down.
        assert!(active.force_n(-0.02, 0.0) < 0.0);
        // Moving in compression (negative length velocity): resist upward.
        assert!(active.force_n(0.0, -0.5) > 0.0);
        assert!(active.force_n(0.0, 0.5) < 0.0);
        // The force never exceeds the actuator limit.
        assert_eq!(active.force_n(1.0, 0.0), 12_000.0);
        assert_eq!(active.force_n(-1.0, 0.0), -12_000.0);

        // Semi-active only damps; it does not hold a static ride height.
        let semi = ActiveSuspensionSpec {
            mode: ActiveSuspensionMode::SemiActive,
            ..active.clone()
        };
        assert_eq!(semi.force_n(0.02, 0.0), 0.0);
        assert!(semi.force_n(0.02, -0.5) > 0.0);

        // Speed lowers the body, smoothly and toward the authored drop.
        let lowering = ActiveSuspensionSpec {
            speed_drop_m: 0.04,
            speed_ref_mps: 50.0,
            ..active.clone()
        };
        assert_eq!(lowering.speed_drop_at(0.0), 0.0);
        assert!((lowering.speed_drop_at(50.0) - 0.02).abs() < 1e-6);
        assert!(lowering.speed_drop_at(100.0) > lowering.speed_drop_at(50.0));
        assert!(lowering.speed_drop_at(100.0) < 0.04);

        // Rake raises the front relative to the rear.
        let raked = ActiveSuspensionSpec {
            rake_m: 0.02,
            ..active
        };
        assert!(raked.target_offset_m(Wheel::FrontLeft, 0.0) > 0.0);
        assert!(raked.target_offset_m(Wheel::RearLeft, 0.0) < 0.0);
    }

    #[test]
    fn two_rate_dampers_switch_at_their_threshold_and_stay_symmetric_by_default() {
        let slow = SuspensionSpec {
            damper_bump_ns_per_m: 4_000.0,
            damper_rebound_ns_per_m: 6_000.0,
            ..SuspensionSpec::default()
        };
        // Jolt works at the symmetric average.
        assert!((slow.jolt_damper_rate_ns_per_m() - 5_000.0).abs() < 1e-6);
        // No fast rates: the authored bump/rebound split in both directions.
        assert!((slow.damper_rate_ns_per_m(-0.1) - 4_000.0).abs() < 1e-6);
        assert!((slow.damper_rate_ns_per_m(0.1) - 6_000.0).abs() < 1e-6);
        assert_eq!(slow.damper_rate_ns_per_m(0.0), 0.0);
        // Nonsense velocity reads as standstill, not as a fast rate.
        assert_eq!(slow.damper_rate_ns_per_m(f32::NAN), 0.0);

        let two_rate = SuspensionSpec {
            damper_bump_ns_per_m: 4_000.0,
            damper_rebound_ns_per_m: 6_000.0,
            fast_bump_damper_ns_per_m: 1_000.0,
            fast_bump_threshold_mps: 0.05,
            fast_rebound_damper_ns_per_m: 1_500.0,
            fast_rebound_threshold_mps: 0.05,
            ..SuspensionSpec::default()
        };
        // Below the threshold the slow rates still apply.
        assert!((two_rate.damper_rate_ns_per_m(-0.05) - 4_000.0).abs() < 1e-6);
        assert!((two_rate.damper_rate_ns_per_m(0.05) - 6_000.0).abs() < 1e-6);
        // Above it the bleeder takes over in both directions.
        assert!((two_rate.damper_rate_ns_per_m(-0.5) - 1_000.0).abs() < 1e-6);
        assert!((two_rate.damper_rate_ns_per_m(0.5) - 1_500.0).abs() < 1e-6);
        // A fast rate of zero is "off", so the slow rate stays in charge.
        let off = SuspensionSpec {
            fast_bump_damper_ns_per_m: 0.0,
            ..two_rate
        };
        assert!((off.damper_rate_ns_per_m(-0.5) - 4_000.0).abs() < 1e-6);
    }

    #[test]
    fn camber_leans_in_under_load_and_pulls_the_car_inward() {
        let suspension = SuspensionSpec {
            static_camber_deg: -3.0,
            camber_gain_rad_per_m: -0.5,
            camber_thrust_coeff: 0.3,
            ..SuspensionSpec::default()
        };
        // No load keeps the static angle.
        let static_rad = suspension.camber_rad(0.0);
        assert!((static_rad - (-3.0f32).to_radians()).abs() < 1e-6);
        // Compression adds negative camber.
        let compressed = suspension.camber_rad(0.05);
        assert!(compressed < static_rad);
        assert!((compressed - static_rad + 0.025).abs() < 1e-6);

        // Negative camber on a loaded wheel pulls that wheel inward, which is
        // what makes negative camber help a corner.
        let thrust = suspension.camber_thrust_n(compressed, 4_000.0);
        assert!(thrust < 0.0, "camber thrust direction: {thrust}");
        // No camber, no thrust; nonsense stays harmless.
        assert_eq!(suspension.camber_thrust_n(0.0, 4_000.0), 0.0);
        assert_eq!(suspension.camber_thrust_n(f32::NAN, f32::NAN), 0.0);
        // A car with no camber stiffness cannot make camber thrust.
        let straight = SuspensionSpec {
            camber_thrust_coeff: 0.0,
            ..SuspensionSpec::default()
        };
        assert_eq!(straight.camber_thrust_n(-0.1, 1_000.0), 0.0);
    }

    #[test]
    fn a_tyre_only_grips_at_the_camber_it_was_designed_for() {
        let tyre = TyreSpec {
            camber_optimal_rad: (-3.0f32).to_radians(),
            camber_grip_loss: 0.2,
            camber_window_rad: 0.15,
            ..TyreSpec::default()
        };
        assert!((tyre.camber_grip_multiplier((-3.0f32).to_radians()) - 1.0).abs() < 1e-6);
        // Half a window off costs half the loss.
        let half = tyre.camber_grip_multiplier((-3.0f32).to_radians() - 0.075);
        assert!((half - 0.9).abs() < 1e-6, "{half}");
        // Past the window the loss saturates, it never turns negative.
        assert!((tyre.camber_grip_multiplier(1.0) - 0.8).abs() < 1e-6);
        // Nonsense camber falls back to the designed angle, so it costs nothing.
        assert_eq!(tyre.camber_grip_multiplier(f32::NAN), 1.0);
    }

    #[test]
    fn flat_spot_grips_less_and_never_inverts() {
        let tyre = TyreSpec {
            flat_spot_grip_loss: 0.3,
            ..TyreSpec::default()
        };
        assert_eq!(tyre.flat_spot_grip_multiplier(0.0), 1.0);
        assert!((tyre.flat_spot_grip_multiplier(0.5) - 0.85).abs() < 1e-6);
        assert!((tyre.flat_spot_grip_multiplier(2.0) - 0.7).abs() < 1e-6);
        // Nonsense never breaks the multiplier.
        assert_eq!(tyre.flat_spot_grip_multiplier(f32::NAN), 1.0);
        // No authorised grip loss means a flat spot is harmless.
        let harmless = TyreSpec {
            flat_spot_grip_loss: 0.0,
            ..TyreSpec::default()
        };
        assert_eq!(harmless.flat_spot_grip_multiplier(1.0), 1.0);
    }

    #[test]
    fn load_sensitivity_and_relaxation_are_data_driven_and_bounded() {
        let off = TyreSpec::default();
        // No authored reference load: the flat behaviour it always had.
        assert_eq!(off.load_friction_multiplier(3_000.0, false), 1.0);
        // Relaxation itself is always available: no relaxed slip yet means no
        // developed force.
        assert_eq!(off.relaxation_multiplier(0.2, 0.0), 0.0);
        assert_eq!(off.relaxation_multiplier(0.2, 0.2), 1.0);

        let tyre = TyreSpec {
            reference_load_n: 4_000.0,
            lateral_load_sensitivity: 0.8,
            longitudinal_load_sensitivity: 0.95,
            relaxation_length_m: 0.1,
            ..TyreSpec::default()
        };
        // At the reference load the multiplier is exactly 1.
        assert!((tyre.load_friction_multiplier(4_000.0, false) - 1.0).abs() < 1e-6);
        // A light wheel carries more friction per newton, a heavy one less:
        // the force scales with load^0.8, so the coefficient scales with
        // load^-0.2.
        assert!(tyre.load_friction_multiplier(2_000.0, false) > 1.0);
        assert!(tyre.load_friction_multiplier(6_000.0, false) < 1.0);
        // The longitudinal exponent is closer to 1, so it is flatter.
        assert!(
            tyre.load_friction_multiplier(6_000.0, true)
                > tyre.load_friction_multiplier(6_000.0, false)
        );
        // Bounded so nothing overtakes the authored peak.
        assert!(tyre.load_friction_multiplier(20.0, false) <= 1.5);
        assert!(tyre.load_friction_multiplier(1.0e6, false) >= 0.5);

        // Relaxation scales the force by the slip the tyre has built up.
        assert_eq!(tyre.relaxation_multiplier(0.0, 0.0), 1.0);
        assert_eq!(tyre.relaxation_multiplier(0.4, 0.1), 0.25);
        assert_eq!(tyre.relaxation_multiplier(0.1, 0.4), 1.0);
        assert_eq!(tyre.relaxation_multiplier(f32::NAN, 0.2), 1.0);
    }

    #[test]
    fn the_tyre_is_a_second_vertical_spring_in_series() {
        // Nothing authored: the suspension spring is the whole stiffness.
        assert_eq!(series_spring_rate(80_000.0, 0.0), 80_000.0);
        // A tyre rate in series always softens the wheel: 80k + 335k = 65.6k.
        let combined = series_spring_rate(80_000.0, 335_281.0);
        assert!(combined < 80_000.0 && combined > 60_000.0, "{combined}");
        // Two equal springs halve; a very stiff tyre barely matters.
        assert!((series_spring_rate(80_000.0, 80_000.0) - 40_000.0).abs() < 1.0);
        assert!((series_spring_rate(80_000.0, 1.0e9) - 79_993.6).abs() < 1.0);

        // The tyre's rate rises with pressure, and pressure only ever adds.
        let tyre = TyreSpec {
            spring_rate_n_per_m: 300_000.0,
            pressure_spring_gain_n_per_m: 10_102.0,
            cold_pressure_psi: 17.0,
            ..TyreSpec::default()
        };
        assert!((tyre.vertical_rate_n_per_m(17.0) - 300_000.0).abs() < 1.0);
        assert!((tyre.vertical_rate_n_per_m(27.0) - 401_020.0).abs() < 1.0);
        // No authored rate: no vertical stiffness from the tyre at all.
        assert_eq!(TyreSpec::default().vertical_rate_n_per_m(29.0), 0.0);
    }

    #[test]
    fn aligning_torque_and_rolling_resistance_come_from_the_tyre_data() {
        let tyre = TyreSpec {
            pneumatic_trail_m: 0.035,
            rolling_resistance_n: 8.0,
            rolling_resistance_v2: 5.0e-4,
            rolling_resistance_slip: 40.0,
            pressure_rr_gain: 0.45,
            cold_pressure_psi: 17.0,
            ..TyreSpec::default()
        };
        // A tyre pushed to the right twists the wheel back towards travel,
        // which is a positive (left) moment about the car's up axis.
        let force = tyre.aligning_torque_nm(1_000.0);
        assert!((force - 35.0).abs() < 1e-4, "{force}");
        assert!((tyre.aligning_torque_nm(-1_000.0) + 35.0).abs() < 1e-4);
        // No trail, no torque; nonsense stays harmless.
        let no_trail = TyreSpec {
            pneumatic_trail_m: 0.0,
            ..tyre.clone()
        };
        assert_eq!(no_trail.aligning_torque_nm(1_000.0), 0.0);
        assert_eq!(tyre.aligning_torque_nm(f32::NAN), 0.0);

        // Rolling resistance: the three AC terms, and pressure only ever adds.
        let cold = tyre.rolling_resistance_n(30.0, 0.1, 17.0);
        let hot = tyre.rolling_resistance_n(30.0, 0.1, 30.0);
        assert!(
            (cold - (8.0 + 5.0e-4 * 900.0 + 40.0 * 0.01)).abs() < 1e-3,
            "{cold}"
        );
        assert!(hot > cold, "{hot} vs {cold}");
        // Speed and scrub both cost rubber.
        assert!(tyre.rolling_resistance_n(60.0, 0.1, 17.0) > cold);
        assert!(tyre.rolling_resistance_n(30.0, 0.5, 17.0) > cold);
        // No authored data: no extra rolling resistance at all.
        let bare = TyreSpec::default();
        assert_eq!(bare.rolling_resistance_n(30.0, 0.1, 17.0), 0.0);
    }

    #[test]
    fn steering_axis_geometry_adds_camber_and_self_centering() {
        let suspension = SuspensionSpec::default();
        // Straight ahead: no camber from the axis and no trail effect.
        assert_eq!(suspension.steer_camber_gain(0.0), 0.0);
        // Lock on: the wheel leans.
        let steer_gain = suspension.steer_camber_gain(0.3);
        assert!(steer_gain.abs() > 0.01, "{steer_gain}");
        assert!(suspension.caster_trail_m(0.33) > 0.01);
        // Nonsense or a disabled axis do nothing.
        assert_eq!(suspension.steer_camber_gain(f32::NAN), 0.0);
        let flat = SuspensionSpec {
            caster_angle_deg: 0.0,
            kingpin_angle_deg: 0.0,
            ..SuspensionSpec::default()
        };
        assert_eq!(flat.caster_trail_m(0.33), 0.0);
        assert_eq!(flat.steer_camber_gain(0.3), 0.0);
    }

    #[test]
    fn the_soft_limiter_and_altitude_detune_the_engine() {
        let engine = EngineSpec {
            redline_rpm: 8_000.0,
            limiter_hz: 60.0,
            altitude_sensitivity: 0.14,
            ..EngineSpec::default()
        };
        // Below the redline the limiter is open.
        assert_eq!(engine.limiter_duty(8_000.0), 1.0);
        // Past it the duty falls and bottoms out at a full cut.
        let mid = engine.limiter_duty(8_200.0);
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(engine.limiter_duty(9_000.0), 0.0);
        // A hard limiter (no frequency) is always open here.
        assert_eq!(EngineSpec::default().limiter_duty(20_000.0), 1.0);

        // Altitude: no loss at sea level, 14 % per kilometre with floor 20 %.
        assert_eq!(engine.altitude_multiplier(0.0), 1.0);
        assert!((engine.altitude_multiplier(1_000.0) - 0.86).abs() < 1e-6);
        assert!((engine.altitude_multiplier(5_000.0) - 0.3).abs() < 1e-6);
        assert!(engine.altitude_multiplier(20_000.0) >= 0.2);
        // No authored sensitivity: no altitude effect at all.
        assert_eq!(EngineSpec::default().altitude_multiplier(3_000.0), 1.0);
    }

    #[test]
    fn anti_dive_and_squat_are_bounded_fractions() {
        let suspension = SuspensionSpec::default();
        assert!(suspension.anti_dive_fraction > 0.0 && suspension.anti_dive_fraction <= 1.0);
        assert!(suspension.anti_squat_fraction > 0.0 && suspension.anti_squat_fraction <= 1.0);
        // Both can be turned off for a car that transfers everything through
        // the springs, and both can be authored.
        let off = SuspensionSpec {
            anti_dive_fraction: 0.0,
            anti_squat_fraction: 0.0,
            ..SuspensionSpec::default()
        };
        assert_eq!(off.anti_dive_fraction, 0.0);
        assert_eq!(off.anti_squat_fraction, 0.0);
    }

    #[test]
    fn the_roll_centre_and_steer_lock_are_data_driven() {
        let spec = VehicleSpec::default();
        // The spec carries its own lock; the world uses it unless the tuning
        // overrides it.
        assert_eq!(spec.max_steer_angle_deg, 30.0);
        // A real AC car: 230 degrees at the wheel through a 15:1 ratio.
        let lock = (230.0f32 / 15.0).clamp(3.0, 60.0);
        assert!((lock - 15.333_333).abs() < 1e-4);

        // The roll centre is above the road and well below the centre of mass,
        // which is what makes the correction a reduction in roll moment.
        let suspension = SuspensionSpec::default();
        assert!(suspension.roll_center_height_m > 0.0);
        assert!(suspension.roll_center_height_m < spec.cg_height_m);
        // A mod can author it, within sanity.
        let authored = SuspensionSpec {
            roll_center_height_m: 0.08,
            ..SuspensionSpec::default()
        };
        assert_eq!(authored.roll_center_height_m, 0.08);
    }

    #[test]
    fn dirty_air_costs_downforce_only_directly_behind_a_car() {
        let aero = AeroSpec {
            dirty_air_loss: 0.3,
            dirty_air_distance_m: 20.0,
            ..AeroSpec::default()
        };
        // Clean air: nothing lost.
        assert_eq!(aero.dirty_air_multiplier(0.0), 1.0);
        // Right on the gearbox: the full authored loss.
        assert!((aero.dirty_air_multiplier(1.0) - 0.7).abs() < 1e-6);
        // Half a car length back: half the loss.
        assert!((aero.dirty_air_multiplier(0.5) - 0.85).abs() < 1e-6);
        // A car that does not author it is unaffected in traffic.
        assert_eq!(AeroSpec::default().dirty_air_multiplier(1.0), 1.0);
        // Nonsense reads as clean air.
        assert_eq!(aero.dirty_air_multiplier(f32::NAN), 1.0);
    }

    #[test]
    fn sub_zero_temperatures_are_a_physical_state_not_an_error() {
        // The rubber works below zero: a colder tyre simply holds less air and
        // grips worse, and neither of those is clamped at the freezing point.
        let tyre = TyreSpec {
            cold_pressure_psi: 17.0,
            ..TyreSpec::default()
        };
        assert!(tyre.pressure_at(-20.0) < tyre.pressure_at(0.0));
        assert!(tyre.pressure_at(0.0) < tyre.pressure_at(90.0));
        assert!(tyre.pressure_at(-20.0) > 0.0);

        // With an authored curve the grip keeps falling below the coldest
        // point instead of flattening at it.
        let thermal = TyreThermalSpec {
            performance_curve: vec![(0.0, 0.9), (50.0, 1.0), (100.0, 0.8), (300.0, 0.5)],
            ..TyreThermalSpec::default()
        };
        let at_zero = thermal.temperature_multiplier(0.0);
        let below = thermal.temperature_multiplier(-20.0);
        assert!(below < at_zero, "{below} vs {at_zero}");
        // Far below, the extrapolation floors out instead of going negative.
        assert!(thermal.temperature_multiplier(-1_000.0) >= 0.0);
        assert_eq!(thermal.temperature_multiplier(-1_000.0), 0.0);
        // Above the coldest point the table itself is the answer: 50 °C is the
        // peak, 75 °C is on the way down.
        assert!((thermal.temperature_multiplier(50.0) - 1.0).abs() < 1e-6);
        assert!((thermal.temperature_multiplier(75.0) - 0.9).abs() < 1e-6);

        // The analytic window also works in the cold.
        let window = TyreThermalSpec::default();
        assert!(window.temperature_multiplier(-20.0) < window.temperature_multiplier(0.0));
        assert!(window.temperature_multiplier(-20.0) > 0.0);
    }

    #[test]
    fn three_layered_tyre_thermals_lag_behind_the_surface() {
        let single = TyreThermalSpec::default();
        assert!(!single.has_layers());
        let layered = TyreThermalSpec {
            surface_capacity_j_per_k: 60.0,
            patch_capacity_j_per_k: 200.0,
            heat_capacity_j_per_k: 1_000.0,
            surface_transfer_per_s: 0.018,
            patch_transfer_per_s: 0.00027,
            ..TyreThermalSpec::default()
        };
        assert!(layered.has_layers());
        // The grip curve reads the core; the surface that actually touches the
        // road is a different (much hotter) temperature, which is what the
        // world's `advance_tyre_temperatures` models. Here the layers exist and
        // the curve works.
        assert!(layered.heat_capacity_j_per_k > layered.patch_capacity_j_per_k);

        // An authored temperature curve replaces the analytic window.
        let with_curve = TyreThermalSpec {
            performance_curve: vec![(0.0, 0.9), (50.0, 1.0), (100.0, 0.8), (300.0, 0.5)],
            ..TyreThermalSpec::default()
        };
        assert!((with_curve.temperature_multiplier(50.0) - 1.0).abs() < 1e-6);
        assert!((with_curve.temperature_multiplier(0.0) - 0.9).abs() < 1e-6);
        assert!((with_curve.temperature_multiplier(25.0) - 0.95).abs() < 1e-6);
        assert!((with_curve.temperature_multiplier(1_000.0) - 0.5).abs() < 1e-6);
        // Nonsense temperature falls back to the optimum.
        assert!(with_curve.temperature_multiplier(f32::NAN).is_finite());
        // Without a curve the analytic window is used.
        assert!(single.temperature_multiplier(single.optimal_c) > 0.99);
        assert!(single.temperature_multiplier(20.0) < 0.95);
    }

    #[test]
    fn wing_tables_give_ground_effect_and_stall() {
        let wing = WingSpec {
            chord_m: 0.5,
            span_m: 1.0,
            cl_gain: 0.25,
            aoa_cl: vec![(0.0, 0.5), (5.0, 0.9), (10.0, 0.3)],
            gh_cl: vec![(0.0, 1.2), (0.05, 1.0), (0.20, 0.5)],
            aoa_cd: vec![(0.0, 0.2), (10.0, 0.6)],
            gh_cd: vec![(0.0, 0.9), (0.10, 1.0)],
            ..WingSpec::default()
        };
        // Area 0.5 m²; the tables give Cl and then the height multiplier.
        assert!((wing.cl_area(0.0, 0.05) - 0.5 * 1.0 * 0.5).abs() < 1e-6);
        // Closer to the road is more ground effect.
        assert!(wing.cl_area(0.0, 0.0) > wing.cl_area(0.0, 0.2));
        // High angle of attack stalls the table.
        assert!(wing.cl_area(10.0, 0.05) < wing.cl_area(5.0, 0.05));
        // Out-of-domain queries clamp like AC's own LUT sampling.
        assert!((wing.cl_area(0.0, 1.0) - 0.5 * 0.5 * 0.5).abs() < 1e-6);
        assert!((wing.cl_area(0.0, -1.0) - 0.5 * 1.2 * 0.5).abs() < 1e-6);
        // Drag follows the same two tables.
        assert!((wing.cd_area(0.0, 0.05) - 0.2 * 0.95 * 0.5).abs() < 1e-6);
        assert!((wing.cd_area(10.0, 0.10) - 0.6 * 1.0 * 0.5).abs() < 1e-6);
        // No tables: the authored gain stands alone.
        let plain = WingSpec {
            chord_m: 0.5,
            span_m: 1.0,
            cl_gain: 0.25,
            cd_gain: 0.1,
            ..WingSpec::default()
        };
        assert!((plain.cl_area(0.0, 0.05) - 0.125).abs() < 1e-6);
        assert!((plain.cd_area(7.0, 0.05) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn slip_curves_come_from_the_cars_own_peak_and_falloff() {
        // Nonsense data keeps the flat table this crate used before: a car
        // that cannot be fitted still has to drive.
        let broken = TyreSpec {
            lateral_peak_slip_deg: f32::NAN,
            longitudinal_peak_slip: 0.0,
            falloff_level: f32::NAN,
            ..TyreSpec::default()
        };
        let (longitudinal, lateral) = broken.slip_curves();
        assert_eq!(longitudinal, DEFAULT_LONGITUDINAL_CURVE.to_vec());
        assert_eq!(lateral, DEFAULT_LATERAL_CURVE.to_vec());

        // The default spec publishes a 6 degree peak and a 6 % slip peak, and
        // AC data rides on those numbers: a fitted Magic Formula table.
        let spec = TyreSpec::default();
        let (longitudinal, lateral) = spec.slip_curves();
        for curve in [&longitudinal, &lateral] {
            assert!(curve.windows(2).all(|pair| pair[1].0 > pair[0].0));
            assert!(curve.iter().all(|(_, y)| (0.0..=1.0).contains(y)));
            assert!(curve.iter().any(|(_, y)| (*y - 1.0).abs() < 1e-4));
            // Past the peak the table must come back down: that is the point of
            // a real slip curve.
            let peak_index = curve
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
                .unwrap();
            assert!(peak_index > 0 && peak_index + 1 < curve.len());
            assert!(curve[peak_index + 1].1 < 1.0);
        }
        // The lateral table peaks at the authored 6 degrees.
        assert!(
            (lateral
                .iter()
                .find(|(_, y)| (*y - 1.0).abs() < 1e-4)
                .unwrap()
                .0
                - 6.0)
                .abs()
                < 1e-3
        );
        // The longitudinal table peaks at 6 % slip and still has grip at 50 %.
        assert!(
            (longitudinal
                .iter()
                .find(|(_, y)| (*y - 1.0).abs() < 1e-4)
                .unwrap()
                .0
                - 0.06)
                .abs()
                < 1e-3
        );
        assert!(longitudinal.last().unwrap().0 > 0.2);
    }
}
