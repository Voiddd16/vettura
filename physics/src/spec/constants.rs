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
pub const DEFAULT_LONGITUDINAL_CURVE: [(f32, f32); 4] =
    [(0.0, 0.0), (0.06, 1.0), (0.18, 0.93), (0.5, 0.8)];
pub const DEFAULT_LATERAL_CURVE: [(f32, f32); 4] =
    [(0.0, 0.0), (3.0, 1.0), (10.0, 0.95), (20.0, 0.82)];

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
pub(crate) fn sample_curve(points: &[(f32, f32)], x: f32) -> f32 {
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
