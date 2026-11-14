//! Assetto Corsa AI splines (`fast_lane.ai` / `ideal_line.ai`).
//!
//! Version 7 is the format used by current AC tracks:
//!
//! ```text
//! i32 version (= 7)
//! i32 point_count
//! i32 lap_time_ms
//! i32 sample_count
//! Point[point_count]          // 20 bytes each
//! i32 extra_count
//! PointExtra[extra_count]     // 68 bytes each
//! i32 has_grid
//! ...optional AiSplineGrid...
//! ```
//!
//! The points and extra vectors are converted to the Vettura physics frame
//! (`+X` right / `+Y` up / `-Z` forward) while parsing. The optional grid is
//! intentionally ignored: it is editor metadata, not needed to drive or spawn.

use std::path::Path;

use glam::{Quat, Vec3};
use vettura_core::coords::ac_to_godot;
use vettura_physics::{ErsMode, Pose, VehicleSpec};

use crate::vehicle::{DriveInput, VehicleSnapshot, VehicleTuning, steer_lock_deg};
use crate::world::{VehicleView, WorldError, WorldResult};

const VERSION: i32 = 7;
const POINT_SIZE: usize = 20;
const EXTRA_SIZE: usize = 68;

/// One sampled point on an AI spline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiPoint {
    /// Physics-frame position on the racing line.
    pub position: Vec3,
    /// Cumulative distance along the spline, as stored by AC.
    pub length_m: f32,
    /// Original sample id.
    pub id: i32,
}

/// Per-point AI metadata stored after the position array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiPointExtra {
    /// Target speed at this point, m/s.
    pub speed_mps: f32,
    /// Throttle hint, normally `0..=1`.
    pub throttle: f32,
    /// Brake hint, normally `0..=1`.
    pub brake: f32,
    /// Obsolete lateral-g hint kept for compatibility.
    pub lateral_g: f32,
    /// Corner radius hint in metres.
    pub radius_m: f32,
    /// Track width to the left of the line, metres.
    pub side_left_m: f32,
    /// Track width to the right of the line, metres.
    pub side_right_m: f32,
    /// Banking hint.
    pub camber: f32,
    /// Direction hint.
    pub direction: f32,
    /// Physics-frame surface normal hint.
    pub normal: Vec3,
    /// Cumulative distance along the spline.
    pub length_m: f32,
    /// Physics-frame forward vector hint.
    pub forward: Vec3,
    /// Authoring tag.
    pub tag: f32,
    /// Grade hint.
    pub grade: f32,
}

impl AiPointExtra {
    /// Total authored width, metres.
    pub fn width_m(&self) -> f32 {
        self.side_left_m + self.side_right_m
    }
}

/// A parsed AC AI spline.
#[derive(Clone, Debug)]
pub struct AiSpline {
    version: i32,
    lap_time_ms: i32,
    sample_count: i32,
    points: Vec<AiPoint>,
    extras: Vec<AiPointExtra>,
    /// Cumulative horizontal distance at each point. The final entry is the
    /// closing lap length, including the segment from the last point back to
    /// the first.
    cumulative_m: Vec<f32>,
    has_grid: bool,
}

impl AiSpline {
    /// Parse an AI spline from a file.
    pub fn from_path(path: impl AsRef<Path>) -> WorldResult<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| WorldError::TrackIo {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&bytes).map_err(|error| match error {
            WorldError::TrackFormat(message) => {
                WorldError::TrackFormat(format!("{}: {message}", path.display()))
            }
            other => other,
        })
    }

    /// Parse an AI spline from memory.
    pub fn parse(bytes: &[u8]) -> WorldResult<Self> {
        let mut reader = Reader::new(bytes);

        let version = reader.read_i32()?;
        if version != VERSION {
            return Err(format_error(format!(
                "unsupported AI spline version {version} (expected {VERSION})"
            )));
        }

        let point_count = reader.read_count("point", POINT_SIZE)?;
        let lap_time_ms = reader.read_i32()?;
        let sample_count = reader.read_i32()?;

        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            let position = ac_to_godot(reader.read_vec3()?);
            let length_m = reader.read_f32()?;
            let id = reader.read_i32()?;
            if !position.is_finite() || !length_m.is_finite() {
                return Err(format_error(
                    "AI spline contains a non-finite point".to_owned(),
                ));
            }
            points.push(AiPoint {
                position,
                length_m,
                id,
            });
        }

        let extra_count = reader.read_count("extra point", EXTRA_SIZE)?;
        let mut extras = Vec::with_capacity(extra_count);
        for _ in 0..extra_count {
            extras.push(AiPointExtra {
                speed_mps: reader.read_f32()?,
                throttle: reader.read_f32()?,
                brake: reader.read_f32()?,
                lateral_g: reader.read_f32()?,
                radius_m: reader.read_f32()?,
                side_left_m: reader.read_f32()?,
                side_right_m: reader.read_f32()?,
                camber: reader.read_f32()?,
                direction: reader.read_f32()?,
                normal: ac_to_godot(reader.read_vec3()?),
                length_m: reader.read_f32()?,
                forward: ac_to_godot(reader.read_vec3()?),
                tag: reader.read_f32()?,
                grade: reader.read_f32()?,
            });
        }

        // Older exporters sometimes omit the grid flag; treat that as "no grid".
        let has_grid = if reader.remaining() >= 4 {
            reader.read_i32()? != 0
        } else {
            false
        };

        Ok(Self {
            version,
            lap_time_ms,
            sample_count,
            cumulative_m: cumulative_lengths(&points),
            points,
            extras,
            has_grid,
        })
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    /// Authored lap time; normally zero in exported files.
    pub fn lap_time_ms(&self) -> i32 {
        self.lap_time_ms
    }

    /// Number of speed samples advertised by the file.
    pub fn sample_count(&self) -> i32 {
        self.sample_count
    }

    pub fn points(&self) -> &[AiPoint] {
        &self.points
    }

    pub fn extras(&self) -> &[AiPointExtra] {
        &self.extras
    }

    pub fn extra(&self, index: usize) -> Option<&AiPointExtra> {
        self.extras.get(index)
    }

    /// Whether the file contains an optional editor grid.
    pub fn has_grid(&self) -> bool {
        self.has_grid
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Exact pose on the line at `index`.
    ///
    /// The position is the authored point (physics frame) and the orientation
    /// only contains yaw, so a car stays upright on banked or sloped sections.
    pub fn pose_at(&self, index: usize) -> Option<Pose> {
        let point = self.points.get(index)?;
        let forward = if let Some(next) = self.points.get(index + 1) {
            next.position - point.position
        } else if let Some(previous) = index.checked_sub(1).and_then(|i| self.points.get(i)) {
            point.position - previous.position
        } else {
            Vec3::NEG_Z
        };
        let forward = Vec3::new(forward.x, 0.0, forward.z);
        let orientation = if forward.length_squared() > 1.0e-8 {
            let forward = forward.normalize();
            Quat::from_rotation_y((-forward.x).atan2(-forward.z))
        } else {
            Quat::IDENTITY
        };
        Some(Pose {
            position: point.position,
            orientation,
        })
    }

    /// Spawn pose at the first line point, with a small clearance above the
    /// surface so the tyres do not start intersecting the collider.
    pub fn start_pose(&self) -> Option<Pose> {
        let mut pose = self.pose_at(0)?;
        pose.position.y += SPAWN_CLEARANCE_M;
        Some(pose)
    }

    /// Total horizontal length of one lap, including the closing segment from
    /// the last point back to the first.
    pub fn lap_length_m(&self) -> f32 {
        self.cumulative_m.last().copied().unwrap_or(0.0)
    }

    /// Project a world-space point onto the spline.
    ///
    /// The nearest segment is chosen using horizontal distance. `along_m` is
    /// the cumulative horizontal distance to the projected point, and
    /// `lateral_m` is signed: positive means the point is to the right of the
    /// driven direction. This is the query used by [`AiDriver`].
    pub fn project(&self, position: Vec3) -> Option<AiProjection> {
        let count = self.points.len();
        if count == 0 || !position.is_finite() {
            return None;
        }

        if count == 1 {
            let point = self.points[0].position;
            let dx = position.x - point.x;
            let dz = position.z - point.z;
            return Some(AiProjection {
                index: 0,
                t: 0.0,
                position: point,
                tangent: Vec3::NEG_Z,
                along_m: 0.0,
                lateral_m: dx,
                distance_m: (dx * dx + dz * dz).sqrt(),
            });
        }

        let mut best: Option<(usize, f32, Vec3, f32, f32)> = None;
        for index in 0..count {
            let a = self.points[index].position;
            let b = self.points[(index + 1) % count].position;
            let dx = b.x - a.x;
            let dz = b.z - a.z;
            let length_sq = dx * dx + dz * dz;
            let t = if length_sq > 1.0e-8 {
                (((position.x - a.x) * dx + (position.z - a.z) * dz) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = Vec3::new(a.x + dx * t, a.y + (b.y - a.y) * t, a.z + dz * t);
            let offset_x = position.x - closest.x;
            let offset_z = position.z - closest.z;
            let distance_sq = offset_x * offset_x + offset_z * offset_z;
            if best.is_none_or(|(_, _, _, _, best_distance)| distance_sq < best_distance) {
                let along = self.cumulative_m[index] + t * length_sq.sqrt();
                best = Some((index, t, closest, along, distance_sq));
            }
        }

        let (index, t, closest, along_m, distance_sq) = best?;
        let tangent = self.segment_tangent(index);
        let right = tangent.cross(Vec3::Y);
        let lateral_m = (position.x - closest.x) * right.x + (position.z - closest.z) * right.z;
        Some(AiProjection {
            index,
            t,
            position: closest,
            tangent,
            along_m,
            lateral_m,
            distance_m: distance_sq.sqrt(),
        })
    }

    /// Sample the spline at a cumulative distance, wrapping around the lap.
    pub fn sample_at_distance(&self, distance_m: f32) -> Option<AiSample> {
        let count = self.points.len();
        if count == 0 {
            return None;
        }
        if count == 1 {
            return Some(self.sample_segment(0, 0.0));
        }

        let lap_length = self.lap_length_m();
        if !lap_length.is_finite() || lap_length <= 1.0e-6 {
            return Some(self.sample_segment(0, 0.0));
        }
        let distance = if distance_m.is_finite() {
            distance_m.rem_euclid(lap_length)
        } else {
            0.0
        };
        let segment = self.cumulative_m[1..]
            .partition_point(|&value| value <= distance)
            .min(count - 1);
        let start = self.cumulative_m[segment];
        let end = self.cumulative_m[segment + 1];
        let t = if end - start > 1.0e-6 {
            ((distance - start) / (end - start)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(self.sample_segment(segment, t))
    }

    fn sample_segment(&self, index: usize, t: f32) -> AiSample {
        let count = self.points.len();
        let a = self.points[index].position;
        let b = self.points[(index + 1) % count].position;
        let position = a.lerp(b, t);
        let speed_mps = (self.extras.len() == count).then(|| {
            let start = self.extras[index].speed_mps;
            let end = self.extras[(index + 1) % count].speed_mps;
            if start.is_finite() && end.is_finite() {
                start + (end - start) * t
            } else {
                start
            }
        });
        AiSample {
            index,
            t,
            position,
            tangent: self.segment_tangent(index),
            speed_mps,
        }
    }

    fn segment_tangent(&self, index: usize) -> Vec3 {
        let count = self.points.len();
        if count < 2 {
            return Vec3::NEG_Z;
        }
        for offset in 0..count {
            let current = (index + offset) % count;
            let delta = self.points[(current + 1) % count].position - self.points[current].position;
            let horizontal = Vec3::new(delta.x, 0.0, delta.z);
            if horizontal.length_squared() > 1.0e-8 {
                return horizontal.normalize();
            }
        }
        Vec3::NEG_Z
    }
}

/// The result of projecting a point onto an AI spline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiProjection {
    /// Index of the segment start point.
    pub index: usize,
    /// Interpolation parameter within the segment, `0..=1`.
    pub t: f32,
    /// Closest point on the spline, physics frame.
    pub position: Vec3,
    /// Horizontal unit tangent at the projection, physics frame.
    pub tangent: Vec3,
    /// Cumulative horizontal distance along the lap.
    pub along_m: f32,
    /// Signed lateral offset: positive means right of the driven direction.
    pub lateral_m: f32,
    /// Horizontal distance from the query point to the spline.
    pub distance_m: f32,
}

/// One interpolated sample on an AI spline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiSample {
    /// Index of the segment start point.
    pub index: usize,
    /// Interpolation parameter within the segment, `0..=1`.
    pub t: f32,
    /// Interpolated physics-frame position.
    pub position: Vec3,
    /// Horizontal unit tangent at the sample.
    pub tangent: Vec3,
    /// Interpolated authored target speed, when extras are present.
    pub speed_mps: Option<f32>,
}

fn cumulative_lengths(points: &[AiPoint]) -> Vec<f32> {
    let mut cumulative = Vec::with_capacity(points.len() + 1);
    cumulative.push(0.0);
    for index in 0..points.len() {
        let a = points[index].position;
        let b = points[(index + 1) % points.len()].position;
        let dx = b.x - a.x;
        let dz = b.z - a.z;
        cumulative.push(cumulative[index] + (dx * dx + dz * dz).sqrt());
    }
    cumulative
}

/// Clearance added by [`AiSpline::start_pose`], metres.
pub const SPAWN_CLEARANCE_M: f32 = 0.05;

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
}

impl Default for AiDriverTuning {
    fn default() -> Self {
        Self {
            lookahead_s: 0.9,
            min_lookahead_m: 6.0,
            max_lookahead_m: 40.0,
            throttle_gain: 0.25,
            brake_gain: 0.4,
            max_speed_mps: 90.0,
            fallback_speed_mps: 30.0,
            traffic_lookahead_factor: 4.0,
            traffic_radius_m: 2.4,
            traffic_max_shift_m: 2.2,
            traffic_max_samples: 24,
            traffic_follow_gap_m: 6.0,
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
/// 35 m/s² planning deceleration is 116 m; the cap only guards degenerate
/// specs (tiny deceleration would otherwise ask for kilometres of lookahead).
const PLAN_MAX_HORIZON_M: f32 = 250.0;

/// Floor of the speed-planning deceleration, m/s². Below this a car brakes so
/// badly that planning cannot help; keep braking like a road car at least.
const PLAN_DECEL_MIN_MPS2: f32 = 6.0;

/// Ceiling of the speed-planning deceleration, m/s². Cap independent of how
/// strong a mod's brakes are: the planning must stay conservative. Real
/// braking with ABS cycling and combined-slip understeer delivers far less
/// than the torque limit suggests — measured ~5–8 m/s² on this F1 at
/// 20–50 m/s — so planning at 15 m/s² makes the AI start braking early
/// enough to actually arrive at the corner speed instead of understeering
/// wide off the line.
const PLAN_DECEL_MAX_MPS2: f32 = 15.0;

/// Safety factor applied to the brake-torque-derived planning deceleration.
/// Real braking gains grip from aero downforce that the planning ignores, so
/// planning at ~80 % of the torque capacity makes the AI brake early rather
/// than arrive hot.
const PLAN_DECEL_SAFETY: f32 = 0.8;

/// Lateral acceleration ceiling used by the curvature speed limit, m/s².
/// ~1 g is conservative for a downforce car but achievable without aero at
/// low speed; combined braking+cornering eats into it, which is exactly the
/// understeer regime the limit protects against.
const PLAN_LATERAL_MPS2: f32 = 10.0;

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
        self.brake_gain = sane(self.brake_gain, 0.4, 0.01, 3.0);
        self.max_speed_mps = sane(self.max_speed_mps, 90.0, 1.0, 150.0);
        self.fallback_speed_mps = sane(self.fallback_speed_mps, 30.0, 0.0, self.max_speed_mps);
        self.traffic_lookahead_factor = sane(self.traffic_lookahead_factor, 1.5, 0.5, 4.0);
        self.traffic_radius_m = sane(self.traffic_radius_m, 2.4, 1.0, 8.0);
        self.traffic_max_shift_m = sane(self.traffic_max_shift_m, 2.2, 0.0, 5.0);
        self.traffic_max_samples = self.traffic_max_samples.clamp(2, 64);
        self.traffic_follow_gap_m = sane(self.traffic_follow_gap_m, 6.0, 1.0, 30.0);
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

/// Follows an [`AiSpline`] with a pure-pursuit steering controller.
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

        // Traffic shifts the aim point sideways and may cap the target speed,
        // which keeps the pure-pursuit maths below unchanged.
        let (shift, follow_speed) = self.traffic_avoidance(&projection, lookahead);
        let line_right = target.tangent.cross(Vec3::Y);
        let target_position = target.position + line_right * shift;

        let to_target = target_position - snapshot.reference_position;
        let horizontal_sq = to_target.x * to_target.x + to_target.z * to_target.z;
        let right = forward.cross(Vec3::Y);
        let local_x = to_target.dot(right);
        let local_z = to_target.dot(forward);
        // The car's authored lock, overridable through the tuning exactly as
        // the physics does it.
        let max_steer = steer_lock_deg(spec, &tuning).to_radians().max(0.01);

        // Pure pursuit when the target is in front. When the car is pointing
        // away from the line, fall back to a heading correction so it can
        // recover instead of driving straight past the target.
        let desired_steer = if horizontal_sq > 1.0e-4 && local_z > 0.5 {
            -(2.0 * spec.wheelbase_m * local_x / horizontal_sq).atan()
        } else if horizontal_sq > 1.0e-4 {
            let desired = Vec3::new(to_target.x, 0.0, to_target.z).normalize();
            forward.cross(desired).y.atan2(forward.dot(desired))
        } else {
            0.0
        };
        let steering = (desired_steer / max_steer).clamp(-1.0, 1.0);

        let mut target_speed = target.speed_mps.unwrap_or(self.tuning.fallback_speed_mps);
        if !target_speed.is_finite() || target_speed < 0.5 {
            target_speed = self.tuning.fallback_speed_mps;
        }
        // 刹车感知速度规划：单一 lookahead 点在高速段看不见下一个慢弯——
        // 43 m/s 的车前瞻只有 ≈39 m（`speed × lookahead_s`），等目标点掉进
        // 弯道时已离弯心不到一个车身的刹车距离，重刹 + ABS 兜底也救不回
        // （实测 horseshoe 双倍限速进弯、撞墙搁浅）。这里把「未来一整段
        // 刹车距离内」的授权速度都折算回当前位置取最小：
        // `v_allow(d)² = v_target(d)² + 2·a_plan·d`。
        let plan_decel = planning_decel_mps2(spec);
        let horizon = (speed * speed / (2.0 * plan_decel) + self.tuning.max_lookahead_m)
            .min(PLAN_MAX_HORIZON_M);
        let mut distance = PLAN_SPACING_M;
        let mut prev_tangent: Option<Vec3> = None;
        while distance <= horizon {
            if let Some(sample) = self.line.sample_at_distance(projection.along_m + distance) {
                // 曲率限速：用相邻采样点的切线变化估算曲率 κ=|Δt|/Δs，
                // 再以横向加速度上限折算限速 v=√(a_lat_max·R)。AI 线自己的
                // speed 在某些弯后段偏乐观（实测线速 25 m/s 处曲率需要
                // <20 m/s），纯靠线速会让车在弯中外漂撞墙。
                if let Some(prev) = prev_tangent {
                    let curvature = (sample.tangent - prev).length() / PLAN_SPACING_M;
                    if curvature > 1.0e-4 {
                        let radius = 1.0 / curvature;
                        let v_lat = (PLAN_LATERAL_MPS2 * radius).sqrt();
                        let allowed = (v_lat * v_lat + 2.0 * plan_decel * distance).sqrt();
                        target_speed = target_speed.min(allowed);
                    }
                }
                prev_tangent = Some(sample.tangent);
                if let Some(v_target) = sample
                    .speed_mps
                    .filter(|speed| speed.is_finite() && *speed > 0.5)
                {
                    let allowed = (v_target * v_target + 2.0 * plan_decel * distance).sqrt();
                    target_speed = target_speed.min(allowed);
                }
            }
            distance += PLAN_SPACING_M;
        }
        // A car that cannot be passed is followed instead of rammed.
        if let Some(follow) = follow_speed {
            target_speed = target_speed.min(follow);
        }
        // 偏离赛道线太远时必须低速：车偏线 30 m 以上时若仍按线上速度跑，
        // 回线需要的转弯半径远小于当前速度下的最小转弯半径（实测 20 m/s
        // 转弯半径 ~77 m，赛道弯 ~45 m），车会一路漂出去撞外墙。限速到
        // fallback 的一半，让车能收窄路线回到线上。
        let off_line_m = projection.lateral_m.abs();
        if off_line_m > 8.0 {
            let recovery_cap = self.tuning.fallback_speed_mps * 0.5;
            target_speed = target_speed.min(recovery_cap);
        }
        let target_speed = target_speed.clamp(0.0, self.tuning.max_speed_mps);
        let speed_error = target_speed - speed;
        let (mut throttle, mut brake) = if speed_error > 0.5 {
            (
                (speed_error * self.tuning.throttle_gain + 0.1).clamp(0.0, 1.0),
                0.0,
            )
        } else if speed_error < -0.8 {
            (
                0.0,
                ((-speed_error) * self.tuning.brake_gain).clamp(0.0, 1.0),
            )
        } else {
            (0.05, 0.0)
        };
        // 转向幅度大时削减刹车：联合滑移（braking + cornering）会让前轮
        // 纵向抓地占满、横向不足，车头直往外飘（实测刹车中转向 -0.9 时
        // 前内轮完全卸载、偏离线 20 m）。保留一部分横向抓地给转向。
        brake *= (1.0 - 0.6 * steering.abs()).max(0.0);
        // Ease off in corners; the AI speed already accounts for grip, and
        // this keeps a high-power car from spinning its rear tyres.
        throttle *= (1.0 - 0.35 * steering.abs()).max(0.2);
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

fn format_error(message: String) -> WorldError {
    WorldError::TrackFormat(message)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize) -> WorldResult<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| format_error(format!("AI spline offset overflow at {}", self.offset)))?;
        let slice = self.bytes.get(self.offset..end).ok_or_else(|| {
            format_error(format!(
                "truncated AI spline at offset {}: need {length} bytes, have {}",
                self.offset,
                self.remaining()
            ))
        })?;
        self.offset = end;
        Ok(slice)
    }

    fn read_i32(&mut self) -> WorldResult<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u32(&mut self) -> WorldResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_f32(&mut self) -> WorldResult<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    fn read_vec3(&mut self) -> WorldResult<Vec3> {
        Ok(Vec3::new(
            self.read_f32()?,
            self.read_f32()?,
            self.read_f32()?,
        ))
    }

    fn read_count(&mut self, what: &str, record_size: usize) -> WorldResult<usize> {
        let count = self.read_i32()?;
        if count < 0 {
            return Err(format_error(format!(
                "AI spline has a negative {what} count ({count})"
            )));
        }
        let count = count as usize;
        if count.saturating_mul(record_size) > self.remaining() {
            return Err(format_error(format!(
                "AI spline {what} count {count} exceeds the remaining {} bytes",
                self.remaining()
            )));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        push_i32(&mut bytes, VERSION);
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
        push_i32(&mut bytes, VERSION);
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
    fn parses_v7_points_and_converts_the_frame() {
        let spline = AiSpline::parse(&synthetic_spline()).unwrap();
        assert_eq!(spline.len(), 2);
        assert_eq!(spline.extras().len(), 2);
        assert_eq!(spline.points()[0].position, Vec3::new(-10.0, 1.0, 0.0));
        assert_eq!(spline.points()[1].position, Vec3::new(-10.0, 1.0, -20.0));
        assert_eq!(spline.extras()[0].forward, Vec3::new(-0.0, 0.0, -1.0));
        assert_eq!(spline.extras()[0].width_m(), 9.0);
        assert!(!spline.has_grid());
    }

    #[test]
    fn start_pose_heads_along_the_line() {
        let spline = AiSpline::parse(&synthetic_spline()).unwrap();
        let pose = spline.start_pose().unwrap();
        assert_eq!(pose.position, Vec3::new(-10.0, 1.05, 0.0));
        let forward = pose.forward();
        assert!((forward - Vec3::NEG_Z).length() < 1.0e-5, "{forward:?}");
    }

    #[test]
    fn rejects_unknown_versions_and_truncated_files() {
        let mut bytes = synthetic_spline();
        bytes[0..4].copy_from_slice(&6i32.to_le_bytes());
        assert!(AiSpline::parse(&bytes).is_err());

        let bytes = synthetic_spline();
        assert!(AiSpline::parse(&bytes[..bytes.len() - 8]).is_err());
    }

    #[test]
    fn projection_reports_along_distance_and_signed_lateral_offset() {
        let spline = AiSpline::parse(&synthetic_spline()).unwrap();
        let right = spline.project(Vec3::new(-9.0, 1.0, -5.0)).unwrap();
        assert!((right.along_m - 5.0).abs() < 1.0e-4);
        assert!((right.lateral_m - 1.0).abs() < 1.0e-4, "{right:?}");
        assert!((right.distance_m - 1.0).abs() < 1.0e-4);

        let left = spline.project(Vec3::new(-11.0, 1.0, -5.0)).unwrap();
        assert!((left.lateral_m + 1.0).abs() < 1.0e-4, "{left:?}");

        assert_eq!(spline.lap_length_m(), 40.0);
    }

    #[test]
    fn distance_sampling_interpolates_and_wraps() {
        let spline = AiSpline::parse(&synthetic_spline()).unwrap();
        let sample = spline.sample_at_distance(5.0).unwrap();
        assert!((sample.position - Vec3::new(-10.0, 1.0, -5.0)).length() < 1.0e-4);
        assert!((sample.tangent - Vec3::NEG_Z).length() < 1.0e-4);
        assert!((sample.speed_mps.unwrap() - 30.0).abs() < 1.0e-4);

        // One full lap plus five metres lands at the same place. The synthetic
        // two-point line has a 20 m closing segment, so its lap is 40 m.
        let wrapped = spline.sample_at_distance(45.0).unwrap();
        assert!((wrapped.position - sample.position).length() < 1.0e-4);
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
        push_i32(&mut bytes, VERSION);
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
