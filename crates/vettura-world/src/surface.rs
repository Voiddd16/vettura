//! Track surfaces and the surface map.
//!
//! The physics ground is a single static Jolt body. Grip and rolling resistance
//! are looked up from a lightweight 2-D map, so a track can mix asphalt, kerbs,
//! grass, gravel, sand, snow and standing water without needing one collider per
//! surface or per-triangle materials.
//!
//! The map is intentionally simple: rectangles and circles, evaluated
//! back-to-front (last patch wins). It is enough for a first playable track and
//! for tests; a mesh-based track loader can later fill the same map or replace
//! [`SurfaceMap::sample`] with a triangle lookup.

use std::sync::Arc;

use glam::Vec3;

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// Kinds of surface shipped with the default table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    #[default]
    Asphalt,
    Concrete,
    Curb,
    Grass,
    Gravel,
    Sand,
    Dirt,
    Snow,
    Ice,
    Water,
}

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 10] = [
        SurfaceKind::Asphalt,
        SurfaceKind::Concrete,
        SurfaceKind::Curb,
        SurfaceKind::Grass,
        SurfaceKind::Gravel,
        SurfaceKind::Sand,
        SurfaceKind::Dirt,
        SurfaceKind::Snow,
        SurfaceKind::Ice,
        SurfaceKind::Water,
    ];

    /// Baseline properties. Grip values are dimensionless friction
    /// coefficients relative to a dry racing slick on asphalt.
    pub const fn properties(self) -> SurfaceProperties {
        let (dry_grip, wet_grip, rolling_resistance, roughness) = match self {
            SurfaceKind::Asphalt => (1.00, 0.72, 0.012, 0.10),
            SurfaceKind::Concrete => (0.95, 0.68, 0.014, 0.15),
            SurfaceKind::Curb => (0.95, 0.65, 0.020, 0.80),
            SurfaceKind::Grass => (0.42, 0.28, 0.090, 0.70),
            SurfaceKind::Gravel => (0.50, 0.36, 0.140, 1.00),
            SurfaceKind::Sand => (0.32, 0.22, 0.200, 1.00),
            SurfaceKind::Dirt => (0.48, 0.32, 0.110, 0.80),
            SurfaceKind::Snow => (0.22, 0.16, 0.160, 0.90),
            SurfaceKind::Ice => (0.08, 0.06, 0.005, 0.00),
            SurfaceKind::Water => (0.25, 0.18, 0.220, 0.30),
        };
        SurfaceProperties {
            kind: self,
            dry_grip,
            wet_grip,
            rolling_resistance,
            roughness,
        }
    }
}

/// One entry in a [`SurfaceTable`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceProperties {
    pub kind: SurfaceKind,
    /// Friction coefficient when dry.
    pub dry_grip: f32,
    /// Friction coefficient when fully wet (before standing-water effects).
    pub wet_grip: f32,
    /// Rolling-resistance coefficient, multiplied by the wheel load.
    pub rolling_resistance: f32,
    /// Visual/audio roughness hint. The first version uses it only in reporting;
    /// a future suspension model can feed it into vertical excitation.
    pub roughness: f32,
}

impl SurfaceProperties {
    pub fn new(kind: SurfaceKind) -> Self {
        kind.properties()
    }

    /// Clamp values into physically meaningful ranges.
    pub fn clamped(mut self) -> Self {
        self.dry_grip = finite_clamp(self.dry_grip, 0.0, 2.5, 0.0);
        self.wet_grip = finite_clamp(self.wet_grip, 0.0, 2.5, 0.0);
        self.rolling_resistance = finite_clamp(self.rolling_resistance, 0.0, 1.0, 0.0);
        self.roughness = finite_clamp(self.roughness, 0.0, 2.0, 0.0);
        self
    }
}

/// Stable identifier of an entry in a [`SurfaceTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(pub u16);

impl SurfaceId {
    pub const ASPHALT: Self = Self(SurfaceKind::Asphalt as u16);
    pub const CONCRETE: Self = Self(SurfaceKind::Concrete as u16);
    pub const CURB: Self = Self(SurfaceKind::Curb as u16);
    pub const GRASS: Self = Self(SurfaceKind::Grass as u16);
    pub const GRAVEL: Self = Self(SurfaceKind::Gravel as u16);
    pub const SAND: Self = Self(SurfaceKind::Sand as u16);
    pub const DIRT: Self = Self(SurfaceKind::Dirt as u16);
    pub const SNOW: Self = Self(SurfaceKind::Snow as u16);
    pub const ICE: Self = Self(SurfaceKind::Ice as u16);
    pub const WATER: Self = Self(SurfaceKind::Water as u16);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An immutable table of surface properties.
///
/// The default table holds one entry per [`SurfaceKind`], in enum order, so
/// [`SurfaceId::ASPHALT`] and friends are stable. Custom entries can be appended
/// before vehicles are created.
#[derive(Clone, Debug)]
pub struct SurfaceTable {
    entries: Vec<SurfaceProperties>,
}

impl SurfaceTable {
    pub fn standard() -> Self {
        let entries = SurfaceKind::ALL
            .iter()
            .copied()
            .map(SurfaceProperties::new)
            .collect();
        Self { entries }
    }

    pub fn entries(&self) -> &[SurfaceProperties] {
        &self.entries
    }

    pub fn get(&self, id: SurfaceId) -> Option<&SurfaceProperties> {
        self.entries.get(id.index())
    }

    /// First entry of the given kind, if the table has it.
    pub fn id_of(&self, kind: SurfaceKind) -> Option<SurfaceId> {
        self.entries
            .iter()
            .position(|p| p.kind == kind)
            .map(|index| SurfaceId(index as u16))
    }

    /// Appends a custom surface.
    pub fn add(&mut self, properties: SurfaceProperties) -> SurfaceId {
        let id = SurfaceId(self.entries.len() as u16);
        self.entries.push(properties.clamped());
        id
    }

    /// The standard entry for `kind`, panicking only if the invariant that the
    /// standard table contains every kind was broken.
    pub fn standard_id(&self, kind: SurfaceKind) -> SurfaceId {
        self.id_of(kind)
            .unwrap_or_else(|| panic!("standard surface table is missing {kind:?}"))
    }
}

impl Default for SurfaceTable {
    fn default() -> Self {
        Self::standard()
    }
}

/// A 2-D shape on the track plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceShape {
    Rect {
        min_x: f32,
        max_x: f32,
        min_z: f32,
        max_z: f32,
    },
    Circle {
        center_x: f32,
        center_z: f32,
        radius: f32,
    },
}

impl SurfaceShape {
    pub fn rect(min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> Self {
        Self::Rect {
            min_x: min_x.min(max_x),
            max_x: min_x.max(max_x),
            min_z: min_z.min(max_z),
            max_z: min_z.max(max_z),
        }
    }

    pub fn circle(center_x: f32, center_z: f32, radius: f32) -> Self {
        Self::Circle {
            center_x,
            center_z,
            radius: radius.abs(),
        }
    }

    pub fn contains(self, x: f32, z: f32) -> bool {
        match self {
            Self::Rect {
                min_x,
                max_x,
                min_z,
                max_z,
            } => (min_x..=max_x).contains(&x) && (min_z..=max_z).contains(&z),
            Self::Circle {
                center_x,
                center_z,
                radius,
            } => {
                let dx = x - center_x;
                let dz = z - center_z;
                dx * dx + dz * dz <= radius * radius
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SurfacePatch {
    shape: SurfaceShape,
    surface: SurfaceId,
}

#[derive(Clone, Copy, Debug)]
struct PuddlePatch {
    shape: SurfaceShape,
    water_depth_m: f32,
}

/// The result of sampling the track at one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSample {
    pub surface: SurfaceId,
    /// Standing water at the point, metres. Zero on a dry track.
    pub water_depth_m: f32,
}

/// A 2-D map of track surfaces and puddles.
#[derive(Clone, Debug)]
pub struct SurfaceMap {
    table: Arc<SurfaceTable>,
    patches: Vec<SurfacePatch>,
    puddles: Vec<PuddlePatch>,
    default_surface: SurfaceId,
}

impl SurfaceMap {
    /// A dry asphalt map with no patches.
    pub fn new() -> Self {
        let table = SurfaceTable::standard();
        let default_surface = table.standard_id(SurfaceKind::Asphalt);
        Self {
            table: Arc::new(table),
            patches: Vec::new(),
            puddles: Vec::new(),
            default_surface,
        }
    }

    /// A map whose whole plane is `kind`.
    pub fn all(kind: SurfaceKind) -> Self {
        let mut map = Self::new();
        let id = map.table.standard_id(kind);
        map.default_surface = id;
        map
    }

    pub fn table(&self) -> &SurfaceTable {
        &self.table
    }

    pub(crate) fn table_arc(&self) -> Arc<SurfaceTable> {
        Arc::clone(&self.table)
    }

    pub fn default_surface(&self) -> SurfaceId {
        self.default_surface
    }

    /// Set the fallback surface for points outside every patch.
    pub fn set_default_surface(&mut self, id: SurfaceId) {
        if self.table.get(id).is_some() {
            self.default_surface = id;
        }
    }

    /// Append a custom surface. Do this before creating vehicles: each vehicle
    /// keeps a snapshot of the table for its tyre callback.
    pub fn add_surface(&mut self, properties: SurfaceProperties) -> SurfaceId {
        Arc::make_mut(&mut self.table).add(properties)
    }

    /// Add a rectangular patch.
    pub fn rect(
        &mut self,
        kind: SurfaceKind,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> &mut Self {
        let surface = self.table.standard_id(kind);
        self.patch(SurfaceShape::rect(min_x, min_z, max_x, max_z), surface)
    }

    /// Add a circular patch.
    pub fn circle(
        &mut self,
        kind: SurfaceKind,
        center_x: f32,
        center_z: f32,
        radius: f32,
    ) -> &mut Self {
        let surface = self.table.standard_id(kind);
        self.patch(SurfaceShape::circle(center_x, center_z, radius), surface)
    }

    /// Add a patch with an explicit surface id (including custom entries).
    pub fn patch(&mut self, shape: SurfaceShape, surface: SurfaceId) -> &mut Self {
        assert!(
            self.table.get(surface).is_some(),
            "surface id {:?} is not in the table",
            surface
        );
        self.patches.push(SurfacePatch { shape, surface });
        self
    }

    /// Add a rectangular puddle. Puddle depth is the maximum of all puddles
    /// covering a point, so overlapping puddles do not stack.
    pub fn puddle_rect(
        &mut self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        water_depth_m: f32,
    ) -> &mut Self {
        self.puddles.push(PuddlePatch {
            shape: SurfaceShape::rect(min_x, min_z, max_x, max_z),
            water_depth_m: water_depth_m.max(0.0),
        });
        self
    }

    /// Add a circular puddle.
    pub fn puddle_circle(
        &mut self,
        center_x: f32,
        center_z: f32,
        radius: f32,
        water_depth_m: f32,
    ) -> &mut Self {
        self.puddles.push(PuddlePatch {
            shape: SurfaceShape::circle(center_x, center_z, radius),
            water_depth_m: water_depth_m.max(0.0),
        });
        self
    }

    /// Sample the map at a world-space point (only `x` and `z` are used).
    pub fn sample(&self, position: Vec3) -> SurfaceSample {
        let (x, z) = (position.x, position.z);
        let surface = self
            .patches
            .iter()
            .rev()
            .find(|patch| patch.shape.contains(x, z))
            .map_or(self.default_surface, |patch| patch.surface);

        let water_depth_m = self
            .puddles
            .iter()
            .filter(|puddle| puddle.shape.contains(x, z))
            .map(|puddle| puddle.water_depth_m)
            .fold(0.0f32, f32::max);

        SurfaceSample {
            surface,
            water_depth_m,
        }
    }

    /// Convenience: just the surface id at a point.
    pub fn surface_at(&self, position: Vec3) -> SurfaceId {
        self.sample(position).surface
    }
}

impl Default for SurfaceMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_table_ids_are_stable() {
        let table = SurfaceTable::standard();
        for kind in SurfaceKind::ALL {
            let id = table.standard_id(kind);
            assert_eq!(table.get(id), Some(&kind.properties()));
        }
        assert_eq!(SurfaceId::ASPHALT, SurfaceId(0));
        assert_eq!(SurfaceId::WATER, SurfaceId(9));
    }

    #[test]
    fn last_patch_wins_and_default_fills_gaps() {
        let mut map = SurfaceMap::new();
        map.rect(SurfaceKind::Grass, -10.0, -10.0, 10.0, 10.0).rect(
            SurfaceKind::Gravel,
            -2.0,
            -2.0,
            2.0,
            2.0,
        );

        assert_eq!(map.surface_at(Vec3::new(0.0, 0.0, 0.0)), SurfaceId::GRAVEL);
        assert_eq!(map.surface_at(Vec3::new(5.0, 0.0, 0.0)), SurfaceId::GRASS);
        assert_eq!(
            map.surface_at(Vec3::new(20.0, 0.0, 0.0)),
            SurfaceId::ASPHALT
        );
    }

    #[test]
    fn puddles_use_max_depth_not_sum() {
        let mut map = SurfaceMap::new();
        map.puddle_rect(-5.0, -5.0, 5.0, 5.0, 0.01)
            .puddle_circle(0.0, 0.0, 2.0, 0.03);

        let at_center = map.sample(Vec3::ZERO);
        assert!((at_center.water_depth_m - 0.03).abs() < 1e-6);
        let at_edge = map.sample(Vec3::new(4.0, 0.0, 0.0));
        assert!((at_edge.water_depth_m - 0.01).abs() < 1e-6);
        assert_eq!(map.sample(Vec3::new(20.0, 0.0, 0.0)).water_depth_m, 0.0);
    }

    #[test]
    fn custom_surface_can_be_added() {
        let mut map = SurfaceMap::new();
        let id = map.add_surface(SurfaceProperties {
            kind: SurfaceKind::Asphalt,
            dry_grip: 1.6,
            wet_grip: 1.0,
            rolling_resistance: 0.02,
            roughness: 0.1,
        });
        map.patch(SurfaceShape::circle(0.0, 0.0, 1.0), id);
        assert_eq!(map.surface_at(Vec3::ZERO), id);
        assert!((map.table().get(id).unwrap().dry_grip - 1.6).abs() < 1e-6);
    }
}
