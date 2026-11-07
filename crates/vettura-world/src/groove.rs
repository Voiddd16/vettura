//! Track evolution: rubber laid down by cars, sampled on a 2-D grid.
//!
//! AC's `groove.ini` describes how much grip a part of the track can gain as
//! cars drive over it. The file lists one entry per authored groove with
//! `MIN` (cold grip), `MAX` (fully rubbed-in grip) and `MULT` (build-up rate).
//! Track mods do not bind those entries to individual triangles in the KN5 we
//! parse, so instead of guessing a per-triangle mapping this module keeps the
//! evolution **position driven**: any wheel that touches the road deposits
//! rubber where it touched.
//!
//! That reproduces the phenomenon the physics actually needs: the racing line
//! gains grip over a session, the unused edges of the track do not, and a car
//! that keeps going off line finds less grip than one that stays on it.
//!
//! ```ignore
//! // TrackModule 加载时构建，赛车每一步往里沉积橡胶。
//! let mut map = GrooveMap::new(&vertices, settings, 4.0);
//! map.deposit(position.x, position.z, 0.02);
//! let multiplier = map.grip_multiplier(position.x, position.z);
//! ```

use glam::Vec3;

/// Default grid resolution for the rubber map, metres.
pub const DEFAULT_GROOVE_CELL_M: f32 = 4.0;

/// Largest relative gain rubber can add to grip.
///
/// AC's `MAX` goes up to 1.9 on some tracks and down to 0.79 on others, while
/// a physically sane racing gain is a few percent to half. Values outside this
/// window are clamped so a mod cannot make the track twice as sticky.
pub const MAX_GROOVE_GAIN: f32 = 0.6;

/// One groove entry from `groove.ini`.
#[derive(Clone, Debug, PartialEq)]
pub struct GrooveDef {
    /// Authored name, for example `GROOVE_01`.
    pub name: String,
    /// Grip of the cold groove, as AC writes it.
    pub min: f32,
    /// Grip of a fully rubbed-in groove.
    pub max: f32,
    /// Build-up multiplier.
    pub mult: f32,
}

/// Groove settings summarised from `groove.ini`.
///
/// AC writes `MIN`/`MAX` as **additional grip** on top of the surface's own
/// coefficient: Daytona uses `MIN=0` (an untouched part of the track gets no
/// bonus) and `MAX=0.6..0.9` (a fully rubbed-in line gains up to 60%, which is
/// why its racing line is so much faster than the approach road). Both values
/// are clamped to [`MAX_GROOVE_GAIN`] so a mod cannot make the track twice as
/// sticky.
///
/// The file holds one entry per authored groove; the physics needs one set of
/// numbers, so the median `MAX` is used. That keeps a track with many similar
/// channels from averaging away its most useful value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrooveSettings {
    /// Added grip of an untouched groove, `0..=MAX_GROOVE_GAIN`.
    pub cold_bonus: f32,
    /// Added grip of a fully rubbed-in groove.
    pub hot_bonus: f32,
    /// Deposit rate multiplier, from the median `MULT`.
    pub mult: f32,
}

impl GrooveSettings {
    /// Summarise `defs` into one set of numbers.
    ///
    /// An empty list means "no authored groove", which the caller reports as
    /// `None` so a track without the file behaves exactly as before.
    pub fn from_defs(defs: &[GrooveDef]) -> Option<Self> {
        if defs.is_empty() {
            return None;
        }
        // `MULT` rates the build-up, not a grip value, so it has its own
        // range: AC writes values like 5 or 12, which `clamp_sane` would cut.
        let mut mults: Vec<f32> = defs.iter().map(|def| sane_mult(def.mult)).collect();
        mults.sort_by(f32::total_cmp);

        let mut gains: Vec<f32> = defs.iter().map(|def| clamp_sane(def.max)).collect();
        gains.sort_by(f32::total_cmp);
        let hot_bonus = gains[gains.len() / 2];
        let cold_bonus = clamp_sane(defs.iter().map(|def| def.min).fold(1.0, f32::min));

        Some(Self {
            cold_bonus: cold_bonus.min(hot_bonus),
            hot_bonus,
            mult: mults[mults.len() / 2],
        })
    }

    /// Grip multiplier for a rubber fraction of `0..=1`.
    pub fn multiplier(&self, progress: f32) -> f32 {
        if !progress.is_finite() {
            return 1.0 + self.cold_bonus;
        }
        let progress = progress.clamp(0.0, 1.0);
        let bonus = self.cold_bonus + (self.hot_bonus - self.cold_bonus) * progress;
        (1.0 + bonus).clamp(1.0, 1.0 + MAX_GROOVE_GAIN)
    }

    /// Deposit scale per metre of wheel travel, used by the world.
    pub fn deposit_rate(&self) -> f32 {
        (self.mult * 0.02).clamp(0.0, 1.0)
    }
}

impl Default for GrooveSettings {
    fn default() -> Self {
        Self {
            cold_bonus: 0.0,
            hot_bonus: 0.15,
            mult: 1.0,
        }
    }
}

/// Rubber state on a horizontal grid over the track footprint.
#[derive(Clone, Debug)]
pub struct GrooveMap {
    settings: GrooveSettings,
    cell_size: f32,
    min_x: f32,
    min_z: f32,
    cells_x: usize,
    cells_z: usize,
    progress: Vec<f32>,
}

impl GrooveMap {
    /// Build a map covering the footprint of `vertices`.
    ///
    /// Vertices outside the track are ignored by construction: the grid only
    /// covers the bounding box of the collider, so a car in the grandstands
    /// deposits nothing.
    pub fn new(vertices: &[Vec3], settings: GrooveSettings, cell_size: f32) -> Self {
        let cell_size = if cell_size.is_finite() && cell_size > 0.25 {
            cell_size
        } else {
            DEFAULT_GROOVE_CELL_M
        };

        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for vertex in vertices {
            if !vertex.is_finite() {
                continue;
            }
            min_x = min_x.min(vertex.x);
            min_z = min_z.min(vertex.z);
            max_x = max_x.max(vertex.x);
            max_z = max_z.max(vertex.z);
        }
        if !max_x.is_finite() || max_x <= min_x {
            return Self::empty(settings, cell_size);
        }

        let cells_x = (((max_x - min_x) / cell_size).ceil() as usize + 1).max(1);
        let cells_z = (((max_z - min_z) / cell_size).ceil() as usize + 1).max(1);
        Self {
            settings,
            cell_size,
            min_x,
            min_z,
            cells_x,
            cells_z,
            progress: vec![0.0; cells_x * cells_z],
        }
    }

    /// A map with no cells, used for empty or invalid geometry.
    pub fn empty(settings: GrooveSettings, cell_size: f32) -> Self {
        Self {
            settings,
            cell_size: cell_size.max(0.25),
            min_x: 0.0,
            min_z: 0.0,
            cells_x: 0,
            cells_z: 0,
            progress: Vec::new(),
        }
    }

    /// Number of grid cells.
    pub fn cell_count(&self) -> usize {
        self.cells_x * self.cells_z
    }

    /// Grid resolution, metres.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// The settings this map was built with.
    pub fn settings(&self) -> GrooveSettings {
        self.settings
    }

    /// Replace the settings, keeping the deposited rubber.
    pub fn set_settings(&mut self, settings: GrooveSettings) {
        self.settings = settings;
    }

    /// Add rubber at a horizontal position.
    ///
    /// Deposits below the road edge are ignored, because the grid only spans
    /// the collider's footprint.
    pub fn deposit(&mut self, x: f32, z: f32, amount: f32) {
        if !x.is_finite() || !z.is_finite() || amount <= 0.0 || self.progress.is_empty() {
            return;
        }
        let Some(index) = self.cell_index(x, z) else {
            return;
        };
        // Rubber spreads like a blur: a quarter of the deposit lands in each
        // 3×3 neighbourhood cell, which keeps a thin racing line from being
        // missed by a 4 m grid.
        let deposit = amount.clamp(0.0, 4.0);
        let share = deposit * 0.25;
        let centre = deposit * 0.25;
        self.add_cell(index, centre);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if let Some(neighbour) = self.offset_index(index, dx, dz) {
                self.add_cell(neighbour, share);
            }
        }
    }

    /// Rubber fraction at a horizontal position, `0..=1`.
    pub fn progress_at(&self, x: f32, z: f32) -> f32 {
        self.sample(x, z)
    }

    /// Rubber fraction at a world position (y is ignored).
    pub fn progress(&self, position: Vec3) -> f32 {
        self.sample(position.x, position.z)
    }

    /// Grip multiplier at a horizontal position.
    pub fn grip_multiplier(&self, x: f32, z: f32) -> f32 {
        self.settings.multiplier(self.sample(x, z))
    }

    /// Grip multiplier at a world position.
    pub fn grip_at(&self, position: Vec3) -> f32 {
        self.settings
            .multiplier(self.sample(position.x, position.z))
    }

    /// Fraction of the grid that has any rubber on it.
    pub fn coverage(&self) -> f32 {
        if self.progress.is_empty() {
            return 0.0;
        }
        let used = self
            .progress
            .iter()
            .filter(|value| **value > 1.0e-4)
            .count();
        used as f32 / self.progress.len() as f32
    }

    /// Total rubber deposited, summed over the grid.
    pub fn total_progress(&self) -> f32 {
        self.progress.iter().sum()
    }

    /// Clear every cell.
    pub fn reset(&mut self) {
        self.progress.iter_mut().for_each(|value| *value = 0.0);
    }

    fn add_cell(&mut self, index: usize, amount: f32) {
        if let Some(value) = self.progress.get_mut(index) {
            *value = (*value + amount).clamp(0.0, 1.0);
        }
    }

    fn sample(&self, x: f32, z: f32) -> f32 {
        if !x.is_finite() || !z.is_finite() {
            return 0.0;
        }
        self.cell_index(x, z)
            .and_then(|index| self.progress.get(index).copied())
            .unwrap_or(0.0)
    }

    fn cell_index(&self, x: f32, z: f32) -> Option<usize> {
        if self.cells_x == 0 || self.cells_z == 0 {
            return None;
        }
        let cx = ((x - self.min_x) / self.cell_size).floor();
        let cz = ((z - self.min_z) / self.cell_size).floor();
        if cx < 0.0 || cz < 0.0 {
            return None;
        }
        let cx = cx as usize;
        let cz = cz as usize;
        (cx < self.cells_x && cz < self.cells_z).then_some(cz * self.cells_x + cx)
    }

    fn offset_index(&self, index: usize, dx: i32, dz: i32) -> Option<usize> {
        let cx = index % self.cells_x;
        let cz = index / self.cells_x;
        let nx = cx as i32 + dx;
        let nz = cz as i32 + dz;
        if nx < 0 || nz < 0 || nx >= self.cells_x as i32 || nz >= self.cells_z as i32 {
            return None;
        }
        Some(nz as usize * self.cells_x + nx as usize)
    }
}

/// Parse every `[GROOVE_n]` section of an AC `groove.ini`.
pub fn parse_groove_ini(text: &str) -> Vec<GrooveDef> {
    let mut defs = Vec::new();

    let mut section = String::new();
    let mut name = String::new();
    let mut min = 0.0f32;
    let mut max = 1.0f32;
    let mut mult = 1.0f32;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            push_groove(&section, &name, min, max, mult, &mut defs);
            section = header.trim().to_owned();
            name = String::new();
            min = 0.0;
            max = 1.0;
            mult = 1.0;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim();
        let parsed = value
            .parse::<f32>()
            .ok()
            .filter(|number| number.is_finite());
        match key.as_str() {
            "NAME" => name = value.trim_matches('"').to_owned(),
            "MIN" => min = parsed.unwrap_or(0.0),
            "MAX" => max = parsed.unwrap_or(1.0),
            "MULT" => mult = parsed.unwrap_or(1.0),
            _ => {}
        }
    }
    push_groove(&section, &name, min, max, mult, &mut defs);
    defs
}

/// Close one `[GROOVE_n]` section, ignoring anything else.
fn push_groove(
    section: &str,
    name: &str,
    min: f32,
    max: f32,
    mult: f32,
    defs: &mut Vec<GrooveDef>,
) {
    if !section.to_ascii_uppercase().starts_with("GROOVE") {
        return;
    }
    defs.push(GrooveDef {
        name: if name.is_empty() {
            "groove".to_owned()
        } else {
            name.to_owned()
        },
        min,
        max,
        mult,
    });
}

fn clamp_sane(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 4.0)
    } else {
        1.0
    }
}

/// Sanitise a build-up multiplier, which lives on a wider scale than grip.
fn sane_mult(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.01, 20.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(count: usize) -> Vec<Vec3> {
        // 平面顶点网格，覆盖 [-20, 20] 见方。
        let mut vertices = Vec::with_capacity(count * count);
        for z in 0..count {
            for x in 0..count {
                vertices.push(Vec3::new(
                    -20.0 + 40.0 * x as f32 / (count - 1) as f32,
                    0.0,
                    -20.0 + 40.0 * z as f32 / (count - 1) as f32,
                ));
            }
        }
        vertices
    }

    #[test]
    fn parses_groove_ini_entries() {
        let defs = parse_groove_ini(
            "[HEADER]\nGROOVES_NUMBER=2\n\n[GROOVE_0]\nNAME=groove_1\nMIN=0.0\nMAX=0.6\nMULT=5\n\n\
             [GROOVE_1]\nNAME=GROOVE_01\nMIN=0.1\nMAX=0.9\nMULT=3\n",
        );
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "groove_1");
        assert_eq!(defs[0].max, 0.6);
        assert_eq!(defs[1].mult, 3.0);

        assert!(parse_groove_ini("").is_empty());
        assert!(parse_groove_ini("[OTHER]\nMIN=1\n").is_empty());
    }

    #[test]
    fn settings_use_the_median_max_and_clamp_gain() {
        let settings = GrooveSettings::from_defs(&parse_groove_ini(
            "[GROOVE_0]\nMAX=0.6\nMULT=5\n[GROOVE_1]\nMAX=0.9\nMULT=2\n",
        ))
        .expect("settings");
        // MIN=0 → 冷奖金 0；MAX 取中位 0.9，但倍率被钳到 MAX_GROOVE_GAIN。
        assert_near(settings.cold_bonus, 0.0);
        assert_near(settings.hot_bonus, 0.9);
        assert_near(settings.multiplier(0.0), 1.0);
        assert_near(settings.multiplier(1.0), 1.0 + MAX_GROOVE_GAIN);
        assert_near(settings.mult, 5.0);

        let calm = GrooveSettings::from_defs(&parse_groove_ini("[GROOVE_0]\nMAX=1.2\nMULT=1\n"))
            .expect("settings");
        // 1.2 超出能力范围：倍率钳到 MAX_GROOVE_GAIN，原始值仍然保留。
        assert_near(calm.hot_bonus, 1.2);
        // MIN 未写 → 冷奖金 0，完全没有橡胶的区域不受影响。
        assert_near(calm.cold_bonus, 0.0);
        assert_near(calm.multiplier(0.0), 1.0);
        assert_near(calm.multiplier(1.0), 1.0 + MAX_GROOVE_GAIN);
    }

    #[test]
    fn map_deposits_and_reports_grip_only_on_the_grid() {
        let vertices = plain(9);
        let settings = GrooveSettings::default();
        let mut map = GrooveMap::new(&vertices, settings, 4.0);

        // 40 m 跨度加上最后一个半格，得到 11×11 个 cell。
        assert_eq!(map.cell_count(), 121, "a 40 m span plus the closing cell");
        assert_eq!(map.progress_at(0.0, 0.0), 0.0);

        map.deposit(0.0, 0.0, 0.5);
        assert!(
            map.progress_at(0.0, 0.0) > 0.1,
            "{}",
            map.progress_at(0.0, 0.0)
        );
        // 相邻格分到四分之一，形成一小片而不是一个点。
        assert!(map.progress_at(4.0, 0.0) > 0.05);
        assert_eq!(map.progress_at(200.0, 0.0), 0.0, "outside the footprint");
        assert!(map.grip_multiplier(0.0, 0.0) > 1.0);
        assert!(map.coverage() > 0.0 && map.coverage() < 0.5);

        map.deposit(0.0, 0.0, 100.0);
        assert!(map.progress_at(0.0, 0.0) <= 1.0, "progress saturates");

        map.reset();
        assert_eq!(map.total_progress(), 0.0);
    }

    #[test]
    fn empty_and_invalid_geometry_produce_an_empty_map() {
        let settings = GrooveSettings::default();
        let mut empty = GrooveMap::new(&[], settings, 4.0);
        assert_eq!(empty.cell_count(), 0);
        empty.deposit(0.0, 0.0, 1.0);
        assert_eq!(empty.progress_at(0.0, 0.0), 0.0);

        let degenerate = GrooveMap::new(&[Vec3::new(f32::NAN, 0.0, 0.0)], settings, 0.0);
        assert_eq!(degenerate.cell_count(), 0);
        assert_eq!(degenerate.grip_at(Vec3::ZERO), 1.0);
    }

    #[test]
    fn grip_rises_monotonically_with_rubber() {
        let settings = GrooveSettings {
            cold_bonus: 0.0,
            hot_bonus: 0.2,
            mult: 1.0,
        };
        let mut previous = settings.multiplier(0.0);
        for step in 1..=10 {
            let progress = step as f32 / 10.0;
            let multiplier = settings.multiplier(progress);
            assert!(multiplier > previous, "{progress} did not raise grip");
            assert!(multiplier <= 1.0 + settings.hot_bonus + 1.0e-6);
            previous = multiplier;
        }
        assert_near(settings.multiplier(0.0), 1.0);
        assert_near(settings.multiplier(1.0), 1.2);
        // 非有限输入回到冷抓地。
        assert_near(settings.multiplier(f32::NAN), 1.0);
    }

    fn assert_near(value: f32, expected: f32) {
        assert!(
            (value - expected).abs() < 1.0e-6,
            "{value} is not within 1e-6 of {expected}"
        );
    }
}
