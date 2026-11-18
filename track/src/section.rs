//! 计时分段（AC `sections.ini`）。

/// One timed sector of an AC `sections.ini`.
///
/// AC stores section boundaries as fractions of the lap length, measured from
/// the first point of `fast_lane.ai`. `IN` is where the sector starts and
/// `OUT` where it ends; crossing `OUT` completes the sector for timing.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSection {
    /// Authored name, for example `Bus Stop`.
    pub name: String,
    /// Normalised lap position where the sector starts, `0..=1`.
    pub in_fraction: f32,
    /// Normalised lap position where the sector ends, `0..=1`.
    pub out_fraction: f32,
}

impl TrackSection {
    /// Sector length for a lap of `lap_length_m` metres.
    pub fn length_m(&self, lap_length_m: f32) -> f32 {
        if !lap_length_m.is_finite() {
            return 0.0;
        }
        (self.out_fraction - self.in_fraction).max(0.0) * lap_length_m
    }

    /// Whether a normalised lap position falls inside this sector.
    pub fn contains(&self, fraction: f32) -> bool {
        fraction >= self.in_fraction && fraction <= self.out_fraction
    }
}
