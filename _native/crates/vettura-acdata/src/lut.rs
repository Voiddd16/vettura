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

/// 详细解析结果，比 [`Lut::parse`] 多报告跳过了多少行。
#[derive(Debug, Clone)]
pub struct LutParseReport {
    /// 解析得到的查找表。
    pub table: Lut,
    /// 被跳过的格式错误行数（不包含空行和注释）。
    pub skipped: usize,
}

impl Lut {
    /// 宽松解析：跳过空行、注释与无法解析的行。
    pub fn parse(text: &str) -> Self {
        Self::parse_with_report(text).table
    }

    /// 宽松解析，并返回被跳过的行数，便于调用方在调试时输出诊断信息。
    pub fn parse_with_report(text: &str) -> LutParseReport {
        let mut points = Vec::new();
        let mut skipped = 0usize;

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with("//")
            {
                continue;
            }

            let Some((x, y)) = parse_row(line) else {
                skipped += 1;
                continue;
            };
            points.push(LutPoint { x, y });
        }

        points.sort_by(|a, b| a.x.total_cmp(&b.x));
        LutParseReport {
            table: Self { points },
            skipped,
        }
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

/// 解析单行并返回 `(x, y)`。支持的格式：`x|y`、`x,y` 或两个空白分隔的列。
fn parse_row(line: &str) -> Option<(f32, f32)> {
    let (left, right) = if let Some((left, right)) = line.split_once('|') {
        (left, Some(right))
    } else if let Some((left, right)) = line.split_once(',') {
        (left, Some(right))
    } else {
        let mut parts = line.split_whitespace();
        (parts.next().unwrap_or(""), parts.next())
    };

    let right = right?;
    let x = left.trim().parse::<f32>().ok()?;
    let y = right.trim().parse::<f32>().ok()?;
    Some((x, y))
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

    #[test]
    fn report_counts_skipped_rows() {
        let report = Lut::parse_with_report("rpm|torque\n1000|100\nnot a number\n");
        assert_eq!(report.table.len(), 1);
        assert_eq!(report.skipped, 2);
        assert_eq!(report.table.sample(1000.0), Some(100.0));
    }

    #[test]
    fn report_counts_nothing_for_clean_input() {
        let report = Lut::parse_with_report("0|0\n1000|100\n");
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn parse_row_accepts_all_delimiters() {
        assert_eq!(parse_row("1|2"), Some((1.0, 2.0)));
        assert_eq!(parse_row("1,2"), Some((1.0, 2.0)));
        assert_eq!(parse_row(" 1   2 "), Some((1.0, 2.0)));
        assert_eq!(parse_row("1"), None);
        assert_eq!(parse_row("a|2"), None);
    }
}
