//! Assetto Corsa lookup tables.
//!
//! `.lut` files are usually two columns separated by `|`:
//!
//! ```text
//! 0|0
//! 1000|120
//! 2000|260
//! ```
//!
//! We also accept whitespace/comma separated columns, skip malformed lines,
//! and sort points by their x value.

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LutPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lut {
    points: Vec<LutPoint>,
}

impl Lut {
    pub fn parse(text: &str) -> Self {
        let mut points = Vec::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with("//")
            {
                continue;
            }

            let (left, right) = if let Some((left, right)) = line.split_once('|') {
                (left, Some(right))
            } else if let Some((left, right)) = line.split_once(',') {
                (left, Some(right))
            } else {
                let mut parts = line.split_whitespace();
                (parts.next().unwrap_or(""), parts.next())
            };

            let Some(right) = right else {
                continue;
            };
            let Ok(x) = left.trim().parse::<f32>() else {
                continue;
            };
            let Ok(y) = right.trim().parse::<f32>() else {
                continue;
            };
            points.push(LutPoint { x, y });
        }

        points.sort_by(|a, b| a.x.total_cmp(&b.x));
        Self { points }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn points(&self) -> &[LutPoint] {
        &self.points
    }

    /// Linear interpolation, clamped to the first/last point.
    pub fn sample(&self, x: f32) -> Option<f32> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if x <= first.x {
            return Some(first.y);
        }
        if x >= last.x {
            return Some(last.y);
        }

        for window in self.points.windows(2) {
            let [a, b] = window else { continue };
            if x >= a.x && x <= b.x {
                let span = b.x - a.x;
                if span.abs() < f32::EPSILON {
                    return Some(b.y);
                }
                let t = (x - a.x) / span;
                return Some(a.y + (b.y - a.y) * t);
            }
        }
        Some(last.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_clamps() {
        let lut = Lut::parse("0|0\n1000|100\n2000|300\n");
        assert_eq!(lut.len(), 3);
        assert_eq!(lut.sample(-10.0), Some(0.0));
        assert_eq!(lut.sample(500.0), Some(50.0));
        assert_eq!(lut.sample(1500.0), Some(200.0));
        assert_eq!(lut.sample(9999.0), Some(300.0));
    }

    #[test]
    fn skips_garbage() {
        let lut = Lut::parse("rpm|torque\n1000|100\nnot a number\n");
        assert_eq!(lut.len(), 1);
    }
}
