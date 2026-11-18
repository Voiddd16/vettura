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
use vettura_physics::Pose;
use vettura_physics::coords::ac_to_godot;

use crate::{TrackError, TrackResult};

/// AC AI spline format version this parser understands.
pub const SPLINE_VERSION: i32 = 7;
const VERSION: i32 = SPLINE_VERSION;
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
    /// Horizontal curvature at each point, `1/m`, always non-negative. The
    /// turn the car makes at that point, measured in the horizontal plane
    /// only, so a banked oval does not look like a staircase.
    curvatures: Vec<f32>,
    has_grid: bool,
}

impl AiSpline {
    /// Parse an AI spline from a file.
    pub fn from_path(path: impl AsRef<Path>) -> TrackResult<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| TrackError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&bytes).map_err(|error| match error {
            TrackError::Format(message) => {
                TrackError::Format(format!("{}: {message}", path.display()))
            }
            other => other,
        })
    }

    /// Parse an AI spline from memory.
    pub fn parse(bytes: &[u8]) -> TrackResult<Self> {
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
            curvatures: point_curvatures(&points),
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

    /// 每个点的水平曲率，`1/m`，带符号（左转为正）。
    ///
    /// 逐点计算才能让规划器看见真实点间距——按固定网格采样会把尖弯（例如
    /// Daytona 半径 26 m 的回头弯）整段漏掉，限速因此偏到该速度的两倍。
    pub fn point_curvature(&self) -> &[f32] {
        &self.curvatures
    }

    /// 区间 `[from_m, to_m]`（累计里程，可跨界）内最紧的那一点的**带符号**
    /// 水平曲率，`1/m`，左转为正。
    ///
    /// 取极值而不是平均值：限速必须服从区间内最紧的那一点，尖弯不会被相邻
    /// 的直道平均掉。保留符号，规划器才能知道倾斜是帮忙还是拖后腿。
    pub fn max_curvature_between(&self, from_m: f32, to_m: f32) -> f32 {
        let count = self.points.len();
        let lap = self.lap_length_m();
        if count == 0 || count != self.curvatures.len() || !(lap > 1.0e-6) {
            return 0.0;
        }
        let from = if from_m.is_finite() {
            from_m.rem_euclid(lap)
        } else {
            0.0
        };
        let to = if to_m.is_finite() {
            to_m.rem_euclid(lap)
        } else {
            0.0
        };
        // 点的索引取 cumulative_m 上最后一个不大于距离的点，两侧各多看一个
        // 点以覆盖区间端点的转角。
        let first_index = |distance: f32| -> usize {
            self.cumulative_m[1..]
                .partition_point(|&value| value <= distance)
                .min(count - 1)
        };
        let mut best = 0.0f32;
        let mut scan = |start: f32, end: f32| {
            let first = first_index(start);
            let last = first_index(end).max(first);
            for index in first..=last.min(count - 1) {
                for probe in [index.wrapping_sub(1), index, index + 1] {
                    if let Some(&curvature) = self.curvatures.get(probe)
                        && curvature.abs() > best.abs()
                    {
                        best = curvature;
                    }
                }
            }
        };
        if to >= from {
            scan(from, to);
        } else {
            scan(from, lap);
            scan(0.0, to);
        }
        best
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

/// 水平曲率在每个点上的带符号值，`1/m`，左转为正。
///
/// 三点圆公式：相邻两段切线之间的转角除以弦长。正负号留着，因为「倾斜
/// 帮你过弯还是拖后腿」取决于弯的方向——正着倾斜的左弯白送抓地，反着倾斜
/// 的右弯就是缺口。
///
/// 逐点计算才能让规划器看见真实点间距——按固定网格采样会把尖弯（例如
/// Daytona 半径 26 m 的回头弯）整段漏掉，限速因此偏到该速度的两倍。
fn point_curvatures(points: &[AiPoint]) -> Vec<f32> {
    let count = points.len();
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return out;
    }
    if count < 3 {
        out.resize(count, 0.0);
        return out;
    }
    for index in 0..count {
        let a = points[(index + count - 1) % count].position;
        let b = points[index].position;
        let c = points[(index + 1) % count].position;
        let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
        let bc = Vec3::new(c.x - b.x, 0.0, c.z - b.z);
        let ca = Vec3::new(a.x - c.x, 0.0, a.z - c.z);
        let denominator = ab.length() * bc.length() * ca.length();
        let curvature = if denominator > 1.0e-6 {
            (2.0 * ab.cross(bc).y / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        out.push(curvature);
    }
    out
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

fn format_error(message: String) -> TrackError {
    TrackError::Format(message)
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

    fn take(&mut self, length: usize) -> TrackResult<&'a [u8]> {
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

    fn read_i32(&mut self) -> TrackResult<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u32(&mut self) -> TrackResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_f32(&mut self) -> TrackResult<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    fn read_vec3(&mut self) -> TrackResult<Vec3> {
        Ok(Vec3::new(
            self.read_f32()?,
            self.read_f32()?,
            self.read_f32()?,
        ))
    }

    fn read_count(&mut self, what: &str, record_size: usize) -> TrackResult<usize> {
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
    /// 把一组物理帧点写成 AI spline（含统一的 extras）。
    fn spline_from_points(points: &[Vec3], speed: f32, normal_ac: Vec3) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, VERSION);
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

    /// 直道 → 半径 25 m 的回头弯 → 直道。尖弯前后都要有直道，才能看出
    /// 曲率极大值没有被直道平均掉。
    fn hairpin_spline() -> Vec<u8> {
        let radius = 25.0f32;
        let center = Vec3::new(radius, 0.0, -10.0);
        let mut points: Vec<Vec3> = vec![
            Vec3::new(0.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -30.0),
            Vec3::new(0.0, 0.0, -20.0),
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::new(0.0, 0.0, -5.0),
        ];
        for step in 0..=20 {
            let theta = std::f32::consts::PI + std::f32::consts::PI * step as f32 / 20.0;
            points.push(center + radius * Vec3::new(theta.cos(), 0.0, theta.sin()));
        }
        points.push(Vec3::new(2.0 * radius, 0.0, -5.0));
        points.push(Vec3::new(2.0 * radius, 0.0, -10.0));
        points.push(Vec3::new(2.0 * radius, 0.0, -20.0));
        points.push(Vec3::new(2.0 * radius, 0.0, -30.0));
        points.push(Vec3::new(2.0 * radius, 0.0, -40.0));
        spline_from_points(&points, 30.0, Vec3::new(0.0, 1.0, 0.0))
    }
    #[test]
    fn curvature_of_a_circle_is_the_curvature_that_was_authored() {
        for radius in [15.0f32, 25.0, 40.0, 80.0] {
            let spline = AiSpline::parse(&circle_spline(radius)).unwrap();
            let measured = spline.max_curvature_between(0.0, 20.0).abs();
            let expected = 1.0 / radius;
            assert!(
                (measured - expected).abs() < 0.12 * expected,
                "radius {radius} measured κ {measured:.5} (expected {expected:.5})"
            );
        }
    }

    #[test]
    fn hairpin_curvature_is_not_diluted_by_the_straights_around_it() {
        let spline = AiSpline::parse(&hairpin_spline()).unwrap();
        // 入直道中段的 4 m 区间曲率必须接近 0（远离闭合处的接缝）。
        let straight = spline.max_curvature_between(21.0, 29.0).abs();
        assert!(
            straight < 1.0e-3,
            "a 8 m straight should be flat, measured {straight:.5}"
        );
        // 回头弯里最紧的那 4 m 必须接近 1/25，而不是被前后直道平均掉。
        let tight = spline.max_curvature_between(58.0, 62.0).abs();
        assert!(
            (tight - 1.0 / 25.0).abs() < 0.15 / 25.0,
            "hairpin apex κ {tight:.5} (expected {expected:.5})",
            expected = 1.0 / 25.0
        );
        assert!(
            tight > straight * 20.0,
            "the hairpin must dominate its surroundings: {tight:.5} vs {straight:.5}"
        );
    }
}
