//! Tyre slip curves: the Magic Formula shape, ported from Project Chrono.
//!
//! Jolt's wheeled vehicle takes a friction *curve* per direction: the friction
//! coefficient over longitudinal slip ratio and over lateral slip angle in
//! degrees. Until now that curve was a hardcoded table, so every car in every
//! mod had the same tyre shape regardless of its own data.
//!
//! The curve built here is the Pacejka Magic Formula whose Project Chrono
//! 10.0.0 implements in
//! `src/chrono_vehicle/wheeled_vehicle/tire/ChPac89Tire.cpp`
//! (<https://github.com/projectchrono/chrono>, BSD-3-Clause):
//!
//! ```text
//! F(x) = D · sin(C · atan(B·x − E·(B·x − atan(B·x))))
//! ```
//!
//! with the *shape factor* `C` fixed (Chrono's own measured tyres use 1.5),
//! the *curvature* `E` fitted so the curve keeps `falloff` of its peak at
//! three times the peak slip, and the *stiffness factor* `B` fitted so the
//! peak lands exactly on the slip the car's data asks for. That is the part of
//! the model that is load and environment independent, which is what a
//! backend curve is for; the load, camber, temperature and pressure terms stay
//! in the seasonal multiplier chain the world applies on top.
//!
//! The formula core is small and analytic; porting it (rather than linking
//! Chrono's C++) keeps the Jolt path free of a second physics engine.

/// Shape factor `C` of the Magic Formula curve.
const SHAPE_FACTOR: f32 = 1.65;

/// One Magic Formula curve, normalised to a peak of 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlipCurve {
    /// Stiffness factor `B`.
    pub b: f32,
    /// Shape factor `C`.
    pub c: f32,
    /// Curvature factor `E`.
    pub e: f32,
    /// Slip at which the curve peaks, in the curve's own units.
    pub peak_slip: f32,
}

impl SlipCurve {
    /// Fit a curve that peaks at `peak_slip` and keeps `falloff` of its peak at
    /// three times that slip.
    ///
    /// Returns `None` only when no curve can satisfy both, which practically
    /// means nonsense data; callers fall back to the previous fixed table.
    pub fn fitted(peak_slip: f32, falloff: f32) -> Option<Self> {
        let peak_slip = if peak_slip.is_finite() && peak_slip > 0.0 {
            peak_slip
        } else {
            return None;
        };
        let falloff = falloff.clamp(0.05, 1.0);
        // The peak sits at `u - E·(u - atan(u)) = tan(pi/(2C))`.
        let target_u = (std::f32::consts::FRAC_PI_2 / SHAPE_FACTOR).tan();

        let u_at_peak = |e: f32| -> f32 {
            // `u - E·(u - atan(u))` is increasing in `u` for E < 1, so bisect.
            let mut low = 0.0f32;
            let mut high = 1.0e6f32;
            for _ in 0..60 {
                let mid = 0.5 * (low + high);
                let value = mid - e * (mid - mid.atan());
                if value > target_u {
                    high = mid;
                } else {
                    low = mid;
                }
            }
            0.5 * (low + high)
        };

        // Bisect the curvature so the value at 3× the peak slip equals the
        // wanted falloff. Larger `E` lifts the tail (flatter peak); more
        // negative `E` drops it.
        let tail_slip = 3.0 * peak_slip;
        let value_at = |b: f32, e: f32, x: f32| -> f32 {
            let u = b * x;
            (SHAPE_FACTOR * (u - e * (u - u.atan())).atan()).sin()
        };
        let mut low = -1.0f32;
        let mut high = 0.99f32;
        let mut best = None;
        for _ in 0..60 {
            let e = 0.5 * (low + high);
            let b = u_at_peak(e) / peak_slip;
            let tail = value_at(b, e, tail_slip);
            if tail > falloff {
                high = e;
            } else {
                low = e;
            }
            best = Some(e);
        }
        let e = best?;
        let b = u_at_peak(e) / peak_slip;
        if !(b.is_finite() && e.is_finite()) {
            return None;
        }
        Some(Self {
            b,
            c: SHAPE_FACTOR,
            e,
            peak_slip,
        })
    }

    /// Normalised friction coefficient at `x` slip units.
    pub fn value(&self, x: f32) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        let u = self.b * x;
        (self.c * (u - self.e * (u - u.atan())).atan()).sin()
    }

    /// Sampling ladder for a physics backend's friction table.
    ///
    /// The ladder is expressed in multiples of the peak slip, so a tyre whose
    /// peak is at 6 degrees is sampled right up to 30 degrees and one whose
    /// peak is at 6 % slip is sampled to 50 %.
    pub fn points(&self, longitudinal: bool) -> Vec<(f32, f32)> {
        let ladder: &[f32] = if longitudinal {
            &[0.0, 0.15, 0.33, 0.5, 0.83, 1.33, 2.0, 3.0, 4.2, 5.8, 8.3]
        } else {
            &[
                0.0, 0.08, 0.17, 0.33, 0.5, 0.67, 0.83, 1.0, 1.33, 2.0, 3.0, 5.0,
            ]
        };
        let mut points: Vec<(f32, f32)> = Vec::with_capacity(ladder.len() + 1);
        for &multiple in ladder {
            let x = (self.peak_slip * multiple).max(0.0);
            let y = self.value(x).clamp(0.0, 1.0);
            if let Some(last) = points.last_mut()
                && x - last.0 <= 0.0
            {
                *last = (x, y);
                continue;
            }
            points.push((x, y));
        }
        // The peak itself is always in the table, so a backend interpolating it
        // sees the exact authored peak slip rather than the nearest ladder
        // point.
        points.push((self.peak_slip, self.value(self.peak_slip)));
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.dedup_by(|a, b| (a.0 - b.0).abs() < 1.0e-6);
        // Normalise so the table's peak is exactly 1.0: the world multiplies
        // the table by the car's own peak friction, so 1.0 must mean "peak".
        let peak = points
            .iter()
            .map(|(_, y)| *y)
            .fold(0.0f32, f32::max)
            .max(1.0e-3);
        for (_, y) in &mut points {
            *y /= peak;
        }
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_magic_formula_peaks_where_it_is_asked_to() {
        let curve = SlipCurve::fitted(0.06, 0.88).expect("a sane tyre is fittable");
        let peak = curve.peak_slip;
        let at_peak = curve.value(peak);
        let neighbours = [0.9 * peak, 0.95 * peak, 1.05 * peak, 1.1 * peak]
            .iter()
            .map(|x| curve.value(*x))
            .collect::<Vec<_>>();
        assert!(
            at_peak >= neighbours.iter().copied().fold(0.0f32, f32::max) - 1e-3,
            "the curve does not peak at {peak}: {at_peak} vs {neighbours:?}"
        );
        // At rest the tyre makes no force.
        assert!(curve.value(0.0).abs() < 1e-6);
        // It is still rising at a quarter of the peak slip.
        assert!(curve.value(0.25 * peak) > 0.5 * at_peak);
        // The tail keeps the requested share of the peak.
        let tail = curve.value(3.0 * peak);
        assert!((tail - 0.88).abs() < 0.03, "tail was {tail}");
        // Nonsense input stays finite.
        assert!(curve.value(f32::NAN).abs() < 1e-6);
    }

    #[test]
    fn fitted_curves_are_rejected_only_on_nonsense() {
        assert!(SlipCurve::fitted(0.0, 0.88).is_none());
        assert!(SlipCurve::fitted(f32::NAN, 0.88).is_none());
        // A flat falloff is still a curve.
        assert!(SlipCurve::fitted(6.0, 1.0).is_some());
        // Very different peaks give very different stiffnesses.
        let lazy = SlipCurve::fitted(12.0, 0.88).unwrap();
        let sharp = SlipCurve::fitted(4.0, 0.88).unwrap();
        assert!(lazy.value(0.5) < sharp.value(0.5));
    }

    #[test]
    fn sampled_tables_are_usable_by_a_backend() {
        let curve = SlipCurve::fitted(6.0, 0.88).unwrap();
        let points = curve.points(false);
        // Strictly increasing x, finite y, peak at 1.0.
        assert!(points.windows(2).all(|pair| pair[1].0 > pair[0].0));
        assert!(points.iter().all(|(_, y)| y.is_finite() && *y >= 0.0));
        assert!(
            points.iter().any(|(_, y)| (*y - 1.0).abs() < 1e-3),
            "the table never reaches 1.0"
        );
        assert!(points[0].0 == 0.0 && points[0].1 == 0.0);
        // The table spans well past the peak, so a backend interpolating beyond
        // the last point still sees the falloff.
        assert!(points.last().unwrap().0 > 3.0 * curve.peak_slip);

        let longitudinal = SlipCurve::fitted(0.06, 0.88).unwrap();
        let points = longitudinal.points(true);
        assert!(points.windows(2).all(|pair| pair[1].0 > pair[0].0));
        assert!(points.last().unwrap().0 > 0.2);
        // 50 % slip still has grip, which is what makes lock-up survivable.
        assert!(points.last().unwrap().1 > 0.5);
    }
}
