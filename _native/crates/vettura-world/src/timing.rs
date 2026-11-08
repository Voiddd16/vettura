//! Lap and sector timing on top of a parsed AI spline.
//!
//! A race timer has to answer three questions every step: where the car is on
//! the lap, which sector it is in, and whether the lap should count. This
//! module keeps that state outside [`World`] and outside [`AiDriver`] so the
//! same timer works for a player car, a replay, or an AI car.
//!
//! ```ignore
//! let mut timer = LapTimer::new(line, track.sections());
//! loop {
//!     let update = timer.update(view.position(), time_s);
//!     if update.lap_completed {
//!         println!("lap {}: {update:?}", update.lap_time_s.unwrap_or_default());
//!     }
//! }
//! ```
//!
//! The timer projects the car onto the racing line. Without a `sections.ini`
//! the lap is a single sector, so only start/finish crossings are reported.
//! Reversing more than [`BACKWARD_TOLERANCE_M`] invalidates the lap being
//! driven, which is the cheap approximation of a track-limit check that most
//! tracks do not author data for.

use glam::Vec3;

use crate::ai::AiSpline;
use crate::track::TrackSection;

/// Default maximum distance from the line for a reading to count, metres.
///
/// A car in the pit lane or on the grass still projects onto the racing line,
/// so this guards the timer against projections onto a far-away part of the
/// circuit while other layouts come close to the road.
pub const DEFAULT_MAX_LATERAL_M: f32 = 25.0;

/// Backtracking beyond this marks the lap being driven invalid, metres.
pub const BACKWARD_TOLERANCE_M: f32 = 10.0;

/// One sector boundary derived from a `sections.ini` entry.
#[derive(Clone, Debug, PartialEq)]
struct SectorBoundary {
    /// Normalised lap position of the boundary, `0..=1`.
    fraction: f32,
    /// Name of the sector that ends at this boundary.
    name: String,
}

/// A lap timer for one AiSpline.
#[derive(Clone, Debug)]
pub struct LapTimer {
    line: AiSpline,
    /// Boundaries in ascending lap order. The last entry is always the
    /// start/finish line at `1.0`, so its index equals `sector_count - 1`.
    boundaries: Vec<SectorBoundary>,
    max_lateral_m: f32,
    /// Index of the next boundary the car has to cross.
    next_boundary: usize,
    /// Absolute distance of the last start/finish crossing, metres.
    lap_offset_m: f32,
    /// Unwrapped distance driven since the timer started, metres.
    total_m: f32,
    /// Furthest `total_m` reached since the current lap started, metres. Used
    /// to detect backtracking, which invalidates the lap in progress.
    furthest_m: f32,
    /// Distance of the previous reading within the lap, metres.
    wrapped_m: Option<f32>,
    running: bool,
    laps: u32,
    lap_start_time_s: f32,
    last_boundary_time_s: f32,
    last_time_s: f32,
    sector_times_s: Vec<f32>,
    /// Splits of the last completed lap, one per sector.
    last_sector_times_s: Vec<f32>,
    last_lap_s: Option<f32>,
    best_lap_s: Option<f32>,
    lap_valid: bool,
    last_lap_valid: bool,
}

/// What a single [`LapTimer::update`] changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LapUpdate {
    /// The car crossed the start/finish line this step.
    pub crossed_line: bool,
    /// A lap was completed this step. The first crossing of the line only
    /// starts the timer and does not complete a lap.
    pub lap_completed: bool,
    /// Time of the lap completed by this step, seconds.
    pub lap_time_s: Option<f32>,
    /// Whether the completed lap counted (no backtracking).
    pub lap_valid: bool,
    /// Index of the sector this step completed, if any.
    pub sector_completed: Option<usize>,
}

impl LapTimer {
    /// Create a timer for `line`, using `sections` as sector boundaries.
    ///
    /// The sections' `OUT` fractions are the boundaries; the start/finish line
    /// at `1.0` always closes the lap. Sections that do not add a distinct
    /// boundary (zero length, duplicated `OUT`) are dropped, so a track with
    /// sloppy timing data still produces a usable sector list.
    pub fn new(line: AiSpline, sections: &[TrackSection]) -> Self {
        Self {
            line,
            boundaries: sector_boundaries(sections),
            max_lateral_m: DEFAULT_MAX_LATERAL_M,
            next_boundary: 0,
            lap_offset_m: 0.0,
            total_m: 0.0,
            furthest_m: 0.0,
            wrapped_m: None,
            running: false,
            laps: 0,
            lap_start_time_s: 0.0,
            last_boundary_time_s: 0.0,
            last_time_s: 0.0,
            sector_times_s: Vec::new(),
            last_sector_times_s: Vec::new(),
            last_lap_s: None,
            best_lap_s: None,
            lap_valid: true,
            last_lap_valid: false,
        }
    }

    /// Override how far off the line a position may be and still count.
    pub fn with_max_lateral_m(mut self, metres: f32) -> Self {
        if metres.is_finite() && metres > 0.0 {
            self.max_lateral_m = metres;
        }
        self
    }

    /// The line this timer measures on.
    pub fn line(&self) -> &AiSpline {
        &self.line
    }

    /// Number of sectors, including the start/finish sector.
    pub fn sector_count(&self) -> usize {
        self.boundaries.len()
    }

    /// Name of the sector that ends at boundary `index`.
    pub fn sector_name(&self, index: usize) -> Option<&str> {
        self.boundaries
            .get(index)
            .map(|boundary| boundary.name.as_str())
    }

    /// Names of all sectors in lap order.
    pub fn sector_names(&self) -> Vec<&str> {
        self.boundaries
            .iter()
            .map(|boundary| boundary.name.as_str())
            .collect()
    }

    /// Distance at which the sector ending at `index` starts, metres.
    pub fn sector_start_m(&self, index: usize) -> Option<f32> {
        let previous = index.checked_sub(1)?;
        let start = self
            .boundaries
            .get(previous)
            .map_or(0.0, |boundary| boundary.fraction);
        Some(start * self.line.lap_length_m())
    }

    /// Distance at which the sector ending at `index` ends, metres.
    pub fn sector_end_m(&self, index: usize) -> Option<f32> {
        self.boundaries
            .get(index)
            .map(|boundary| boundary.fraction * self.line.lap_length_m())
    }

    /// Feed the current car position and the simulation time.
    ///
    /// Positions far from the line, or times that do not advance, are ignored
    /// so a teleporting car (respawn, pit entry from another layout) does not
    /// invent lap times.
    pub fn update(&mut self, position: Vec3, time_s: f32) -> LapUpdate {
        let mut update = LapUpdate::default();
        if !position.is_finite() || !time_s.is_finite() {
            return update;
        }
        let lap_length = self.line.lap_length_m();
        if !lap_length.is_finite() || lap_length <= 1.0e-6 {
            return update;
        }
        let Some(projection) = self.line.project(position) else {
            return update;
        };
        if projection.distance_m > self.max_lateral_m {
            return update;
        }

        let along = projection.along_m.clamp(0.0, lap_length);
        if time_s < self.last_time_s {
            return update;
        }

        let delta = match self.wrapped_m {
            None => {
                self.wrapped_m = Some(along);
                self.total_m = along;
                self.last_time_s = time_s;
                // Skip boundaries the car has already passed at the first
                // reading, so a mid-lap spawn does not complete them late.
                self.next_boundary = self
                    .boundaries
                    .iter()
                    .position(|boundary| boundary.fraction * lap_length > along + 1.0e-3)
                    .unwrap_or(self.boundaries.len().saturating_sub(1));
                return update;
            }
            Some(previous) => {
                let raw = along - previous;
                if raw > 0.5 * lap_length {
                    raw - lap_length
                } else if raw < -0.5 * lap_length {
                    raw + lap_length
                } else {
                    raw
                }
            }
        };

        if delta < -BACKWARD_TOLERANCE_M {
            self.lap_valid = false;
        }

        let total = (self.total_m + delta).max(0.0);
        // Backtracking too far behind the furthest point of the lap means the
        // car cut the circuit, respawned or reversed: the lap is not countable.
        if total < self.furthest_m - BACKWARD_TOLERANCE_M {
            self.lap_valid = false;
        }

        while let Some(index) = self.boundaries.get(self.next_boundary) {
            let boundary = self.lap_offset_m + index.fraction * lap_length;
            if boundary > total + 1.0e-6 {
                break;
            }

            // The start/finish line closes the lap, so a car sitting exactly on
            // it (the AI line is closed: `along` at the start point can come
            // back as `lap_length`) must consume the line instead of being
            // skipped as "already behind"; skipping it loops through the
            // sectors forever.
            if self.is_start_line(self.next_boundary) {
                self.lap_offset_m = boundary;
                self.next_boundary = 0;
                update.crossed_line = true;
                if self.running {
                    // The last split runs from the previous boundary to the
                    // line, so one push per boundary completes the list.
                    self.sector_times_s.push(time_s - self.last_boundary_time_s);
                    let lap_time_s = time_s - self.lap_start_time_s;
                    self.laps += 1;
                    self.last_lap_s = Some(lap_time_s);
                    self.last_lap_valid = self.lap_valid;
                    if self.lap_valid && self.best_lap_s.is_none_or(|best| lap_time_s < best) {
                        self.best_lap_s = Some(lap_time_s);
                    }
                    self.last_sector_times_s = std::mem::take(&mut self.sector_times_s);
                    update.lap_completed = true;
                    update.lap_time_s = Some(lap_time_s);
                    update.lap_valid = self.lap_valid;
                }
                self.running = true;
                self.lap_start_time_s = time_s;
                self.last_boundary_time_s = time_s;
                self.sector_times_s.clear();
                self.lap_valid = true;
                self.furthest_m = boundary;
                continue;
            }

            if boundary <= self.total_m + 1.0e-9 {
                // Already behind us after a rewind; move on without timing.
                self.next_boundary += 1;
                if self.next_boundary >= self.boundaries.len() {
                    self.next_boundary = 0;
                }
                continue;
            }

            if self.running {
                self.sector_times_s.push(time_s - self.last_boundary_time_s);
            }
            update.sector_completed = Some(self.next_boundary);
            self.next_boundary += 1;
            self.last_boundary_time_s = time_s;
        }

        self.wrapped_m = Some(along);
        self.total_m = total;
        self.furthest_m = self.furthest_m.max(total);
        self.last_time_s = time_s;
        update
    }

    /// Number of completed, counted laps.
    pub fn laps(&self) -> u32 {
        self.laps
    }

    /// Whether the car has crossed the start/finish line at least once.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Elapsed time of the lap being driven, seconds. Zero before the first
    /// start/finish crossing.
    pub fn lap_time_s(&self) -> f32 {
        if self.running {
            self.last_time_s - self.lap_start_time_s
        } else {
            0.0
        }
    }

    /// Time of the last completed lap, seconds.
    pub fn last_lap_s(&self) -> Option<f32> {
        self.last_lap_s
    }

    /// Best lap time so far, seconds.
    pub fn best_lap_s(&self) -> Option<f32> {
        self.best_lap_s
    }

    /// Index of the sector currently being driven.
    pub fn sector(&self) -> usize {
        self.next_boundary
    }

    /// Sector splits recorded for the lap in progress.
    pub fn sector_times_s(&self) -> &[f32] {
        &self.sector_times_s
    }

    /// Sector splits of the last completed lap.
    pub fn last_sector_times_s(&self) -> &[f32] {
        &self.last_sector_times_s
    }

    /// Whether the lap in progress is still countable.
    pub fn is_valid_lap(&self) -> bool {
        self.lap_valid
    }

    /// Whether the last completed lap counted.
    pub fn was_last_lap_valid(&self) -> bool {
        self.last_lap_valid
    }

    /// Forget every timing record. The line and sector layout stay.
    pub fn reset(&mut self) {
        self.next_boundary = 0;
        self.lap_offset_m = 0.0;
        self.total_m = 0.0;
        self.furthest_m = 0.0;
        self.wrapped_m = None;
        self.running = false;
        self.laps = 0;
        self.lap_start_time_s = 0.0;
        self.last_boundary_time_s = 0.0;
        self.last_time_s = 0.0;
        self.sector_times_s.clear();
        self.last_sector_times_s.clear();
        self.last_lap_s = None;
        self.best_lap_s = None;
        self.lap_valid = true;
        self.last_lap_valid = false;
    }

    fn is_start_line(&self, index: usize) -> bool {
        index + 1 == self.boundaries.len()
    }
}

/// Turn `sections.ini` data into ascending boundaries that always end with the
/// start/finish line.
fn sector_boundaries(sections: &[TrackSection]) -> Vec<SectorBoundary> {
    let mut boundaries: Vec<SectorBoundary> = sections
        .iter()
        .map(|section| SectorBoundary {
            fraction: section.out_fraction,
            name: section.name.clone(),
        })
        .collect();
    boundaries.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));

    let mut result: Vec<SectorBoundary> = Vec::with_capacity(boundaries.len() + 1);
    for boundary in boundaries {
        if !(0.0..=1.0).contains(&boundary.fraction) {
            continue;
        }
        if let Some(last) = result.last()
            && (boundary.fraction - last.fraction).abs() < 1.0e-4
            && (boundary.fraction - 1.0).abs() > 1.0e-4
        {
            continue;
        }
        result.push(boundary);
    }

    if result
        .last()
        .is_none_or(|boundary| (boundary.fraction - 1.0).abs() > 1.0e-4)
    {
        result.push(SectorBoundary {
            fraction: 1.0,
            name: "Start/Finish".to_owned(),
        });
    }
    result
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

    /// A closed horizontal circle of `radius` metres, `count` samples.
    ///
    /// Points are authored in the AC frame and converted while parsing, so the
    /// test also proves the timer works on physics-frame data.
    fn synthetic_loop(count: usize, radius: f32) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 7);
        push_i32(&mut bytes, count as i32);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, count as i32);
        for index in 0..count {
            let theta = std::f32::consts::TAU * index as f32 / count as f32;
            let physics = Vec3::new(radius * theta.cos(), 0.0, radius * theta.sin());
            let ac = Vec3::new(-physics.x, physics.y, -physics.z);
            push_f32(&mut bytes, ac.x);
            push_f32(&mut bytes, ac.y);
            push_f32(&mut bytes, ac.z);
            push_f32(&mut bytes, index as f32); // length hint, unused
            push_i32(&mut bytes, index as i32);
        }
        push_i32(&mut bytes, 0); // no extras
        push_i32(&mut bytes, 0); // no grid
        bytes
    }

    fn section(name: &str, in_fraction: f32, out_fraction: f32) -> TrackSection {
        TrackSection {
            name: name.to_owned(),
            in_fraction,
            out_fraction,
        }
    }

    /// Sweep a car along the line from 0 to `laps` laps at a constant speed.
    ///
    /// Distances wrap around the lap, and the positions come from the line
    /// itself so the projection always succeeds.
    fn drive(timer: &mut LapTimer, count: usize, speed_mps: f32, laps: f32) -> Vec<LapUpdate> {
        let lap = timer.line().lap_length_m();
        let step_m = (laps * lap) / count as f32;
        let mut updates = Vec::with_capacity(count + 1);
        for step in 0..=count {
            let distance = step as f32 * step_m;
            let position = timer.line().sample_at_distance(distance).unwrap().position;
            updates.push(timer.update(position, step as f32 * step_m / speed_mps));
        }
        updates
    }

    #[test]
    fn first_line_crossing_starts_the_timer_and_the_second_completes_a_lap() {
        let line = AiSpline::parse(&synthetic_loop(120, 60.0)).unwrap();
        let lap_length = line.lap_length_m();
        assert!((lap_length - 2.0 * std::f32::consts::PI * 60.0).abs() < 5.0);

        let mut timer = LapTimer::new(line, &[]);
        assert_eq!(timer.sector_count(), 1, "no sections means one sector");
        assert!(!timer.running());
        let updates = drive(&mut timer, 240, 20.0, 2.0);

        assert!(
            updates.iter().filter(|update| update.crossed_line).count() >= 2,
            "the lap line was never crossed twice"
        );
        assert!(timer.running());
        assert_eq!(timer.laps(), 1, "one completed lap expected");
        let lap = timer.last_lap_s().expect("recorded lap time");
        assert!(
            (lap - lap_length / 20.0).abs() < 0.5,
            "lap time {lap} does not match {speed} m/s over {lap_length} m",
            speed = 20.0
        );
        assert!(timer.was_last_lap_valid());
        assert_eq!(timer.last_sector_times_s().len(), 1);
    }

    #[test]
    fn sections_split_the_lap_into_timed_sectors() {
        let sections = [
            section("A", 0.0, 0.25),
            section("B", 0.25, 0.5),
            section("C", 0.5, 0.75),
            section("D", 0.75, 1.0),
        ];
        let line = AiSpline::parse(&synthetic_loop(120, 60.0)).unwrap();
        let lap_length = line.lap_length_m();
        let mut timer = LapTimer::new(line, &sections);
        assert_eq!(timer.sector_count(), 4);
        // The last authored section reaches the line, so it names the final
        // sector instead of the fallback "Start/Finish".
        assert_eq!(timer.sector_name(3), Some("D"));
        assert!((timer.sector_end_m(2).unwrap() - 0.75 * lap_length).abs() < 1.0e-3);

        let updates = drive(&mut timer, 480, 20.0, 2.0);
        let sector_count = updates
            .iter()
            .filter(|update| update.sector_completed.is_some())
            .count();
        assert!(sector_count >= 6, "only {sector_count} sectors completed");
        assert_eq!(timer.laps(), 1);
        // One split per boundary: the three intermediate sectors plus the
        // start/finish sector that the second line crossing closed.
        assert_eq!(timer.last_sector_times_s().len(), timer.sector_count());
        for (index, time) in timer.last_sector_times_s().iter().enumerate() {
            let expected = 0.25 * lap_length / 20.0;
            assert!(
                (time - expected).abs() < 1.0,
                "sector {index} took {time} s, expected {expected}"
            );
        }
    }

    #[test]
    fn reversing_more_than_a_tolerance_invalidates_the_lap() {
        let line = AiSpline::parse(&synthetic_loop(120, 60.0)).unwrap();
        let lap_length = line.lap_length_m();
        let mut timer = LapTimer::new(line, &[]);

        // Start at the line and drive a bit past it, so the lap is running.
        timer.update(timer.line().sample_at_distance(0.0).unwrap().position, 0.0);
        for step in 1..=40 {
            let distance = step as f32 * 0.5;
            timer.update(
                timer.line().sample_at_distance(distance).unwrap().position,
                step as f32 * 0.1,
            );
        }
        assert!(!timer.running(), "the line was not crossed yet");

        // Cross the line.
        for step in 0..=40 {
            let distance = 20.0 + step as f32 * (lap_length + 5.0 - 20.0) / 40.0;
            timer.update(
                timer.line().sample_at_distance(distance).unwrap().position,
                4.0 + step as f32 * 0.1,
            );
        }
        assert!(timer.running(), "the timer never started");
        assert!(
            timer.is_valid_lap(),
            "the lap was flagged before a backtrack"
        );

        // Reverse back across the line by more than the tolerance.
        for step in 0..6 {
            let distance = lap_length - 6.0 + step as f32 * 2.0;
            timer.update(
                timer.line().sample_at_distance(distance).unwrap().position,
                8.0 + step as f32 * 0.1,
            );
        }
        assert!(
            !timer.is_valid_lap(),
            "backtracking should invalidate the lap: sector={}",
            timer.sector()
        );
    }

    #[test]
    fn readings_far_from_the_line_are_ignored() {
        let line = AiSpline::parse(&synthetic_loop(120, 60.0)).unwrap();
        let mut timer = LapTimer::new(line, &[section("A", 0.0, 0.5)]);
        timer.update(Vec3::new(500.0, 0.0, 500.0), 0.0);
        timer.update(timer.line().sample_at_distance(0.0).unwrap().position, 1.0);
        assert_eq!(timer.laps(), 0);
        assert!(
            timer.is_valid_lap(),
            "ignored readings must not invalidate a lap"
        );
    }

    #[test]
    fn reset_forgets_timing_but_keeps_sectors() {
        let line = AiSpline::parse(&synthetic_loop(120, 60.0)).unwrap();
        let mut timer = LapTimer::new(line, &[section("A", 0.0, 0.5)]);
        drive(&mut timer, 240, 20.0, 2.0);
        assert!(timer.running());
        timer.reset();
        assert!(!timer.running());
        assert_eq!(timer.laps(), 0);
        assert!(timer.last_lap_s().is_none());
        assert_eq!(timer.sector_count(), 2, "sector layout survives reset");
    }
}
