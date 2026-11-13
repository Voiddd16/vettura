//! Track surfaces and the surface map.
//!
//! The physics ground is a single static Jolt body. Grip and rolling resistance
//! are looked up from a lightweight 2-D map, so a track can mix asphalt, kerbs,
//! grass, gravel, sand, snow and standing water without needing one collider per
//! surface or per-triangle materials.
//!
//! The map starts with simple rectangles and circles, evaluated back-to-front
//! (last patch wins), and can additionally hold a [`MeshSurfaceIndex`] built
//! from a track's collision triangles. Patches stay useful as authored
//! overrides (pit lane, wet patches, test fixtures); the mesh supplies the
//! per-triangle surface everywhere else.

use std::collections::HashMap;

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
        // The last two numbers are AC's `SIN_HEIGHT` / `SIN_LENGTH` defaults:
        // racing surfaces are laid flat and the run-off areas are not.
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
        let (bump_height_m, bump_length_m) = match self {
            // A smooth racing surface, exactly as AC lays it.
            SurfaceKind::Asphalt => (0.0, 10.0),
            SurfaceKind::Concrete => (0.002, 4.0),
            SurfaceKind::Curb => (0.012, 1.2),
            SurfaceKind::Grass => (0.03, 0.5),
            SurfaceKind::Gravel => (0.02, 0.35),
            SurfaceKind::Sand => (0.015, 0.6),
            SurfaceKind::Dirt => (0.025, 0.45),
            SurfaceKind::Snow => (0.035, 0.4),
            SurfaceKind::Ice => (0.003, 3.0),
            SurfaceKind::Water => (0.008, 1.0),
        };
        SurfaceProperties {
            kind: self,
            dry_grip,
            wet_grip,
            rolling_resistance,
            roughness,
            bump_height_m,
            bump_length_m,
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
    /// Height of the road undulation, metres. From AC's `SIN_HEIGHT`.
    pub bump_height_m: f32,
    /// Wavelength of that undulation, metres. From AC's `SIN_LENGTH`.
    pub bump_length_m: f32,
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
        self.bump_height_m = finite_clamp(self.bump_height_m, 0.0, 0.2, 0.0);
        self.bump_length_m = finite_clamp(self.bump_length_m, 0.25, 50.0, 10.0);
        self
    }

    /// Height of the road undulation at a point travelled along `heading`.
    ///
    /// This is a deterministic function of position, so it is the same for
    /// every car and on every replay, and it does not need stored state. The
    /// world uses it to excite the suspension and to cost a little grip.
    pub fn bump_m(&self, position: Vec3, heading: Vec3) -> f32 {
        if self.bump_height_m <= 0.0 {
            return 0.0;
        }
        let heading = if heading.is_finite() {
            Vec3::new(heading.x, 0.0, heading.z)
        } else {
            Vec3::NEG_Z
        };
        if heading.length_squared() < 1.0e-8 {
            return 0.0;
        }
        let along = Vec3::new(position.x, 0.0, position.z).dot(heading.normalize());
        if !along.is_finite() {
            return 0.0;
        }
        let phase = std::f32::consts::TAU * along / self.bump_length_m.max(0.25);
        self.bump_height_m * phase.sin()
    }

    /// Vertical slope of the undulation at a point, metres per metre.
    pub fn bump_slope(&self, position: Vec3, heading: Vec3) -> f32 {
        if self.bump_height_m <= 0.0 {
            return 0.0;
        }
        let heading = if heading.is_finite() {
            Vec3::new(heading.x, 0.0, heading.z)
        } else {
            Vec3::NEG_Z
        };
        if heading.length_squared() < 1.0e-8 {
            return 0.0;
        }
        let along = Vec3::new(position.x, 0.0, position.z).dot(heading.normalize());
        if !along.is_finite() {
            return 0.0;
        }
        let wavelength = self.bump_length_m.max(0.25);
        let phase = std::f32::consts::TAU * along / wavelength;
        self.bump_height_m * std::f32::consts::TAU / wavelength * phase.cos()
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
    mesh_index: Option<Arc<MeshSurfaceIndex>>,
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
            mesh_index: None,
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

    /// Install a mesh-based surface lookup (usually the track's collision
    /// triangles). `None` or an empty index restores the patch-only behaviour.
    ///
    /// Explicit patches still take precedence over the mesh; the mesh takes
    /// precedence over [`default_surface`](Self::default_surface).
    pub fn set_mesh_index(&mut self, index: MeshSurfaceIndex) {
        self.mesh_index = (!index.is_empty()).then(|| Arc::new(index));
    }

    pub fn clear_mesh_index(&mut self) {
        self.mesh_index = None;
    }

    pub fn mesh_index(&self) -> Option<&MeshSurfaceIndex> {
        self.mesh_index.as_deref()
    }

    /// Sample the map at a world-space point.
    ///
    /// Surface patches are evaluated back-to-front (last patch wins). If no
    /// patch covers the point, the mesh index (track triangles) is consulted,
    /// and the default surface fills anything outside the track.
    pub fn sample(&self, position: Vec3) -> SurfaceSample {
        let (x, z) = (position.x, position.z);
        let surface = self
            .patches
            .iter()
            .rev()
            .find(|patch| patch.shape.contains(x, z))
            .map(|patch| patch.surface)
            .or_else(|| {
                self.mesh_index
                    .as_ref()
                    .and_then(|index| index.sample(position))
            })
            .unwrap_or(self.default_surface);

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

/// One collision triangle in the physics frame, tagged with its AC surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColliderTriangle {
    pub vertices: [Vec3; 3],
    pub surface: SurfaceId,
}

impl ColliderTriangle {
    pub fn normal(&self) -> Vec3 {
        let [a, b, c] = self.vertices;
        (b - a).cross(c - a)
    }

    pub fn centroid(&self) -> Vec3 {
        (self.vertices[0] + self.vertices[1] + self.vertices[2]) / 3.0
    }

    pub fn area(&self) -> f32 {
        0.5 * self.normal().length()
    }
}

/// A 2-D acceleration structure over the collision triangles.
///
/// [`SurfaceMap`] uses it to answer "what surface is under this wheel?" without
/// iterating all track triangles every step. Triangles that are nearly vertical
/// (walls, barriers) are kept for collision but omitted from surface lookup.
#[derive(Clone, Debug, Default)]
pub struct MeshSurfaceIndex {
    triangles: Vec<IndexedTriangle>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    cell_size: f32,
    skipped: usize,
}

#[derive(Clone, Copy, Debug)]
struct IndexedTriangle {
    vertices: [Vec3; 3],
    surface: SurfaceId,
}

impl MeshSurfaceIndex {
    /// Default uniform-grid cell size, metres.
    pub const DEFAULT_CELL_SIZE: f32 = 10.0;

    pub fn new(triangles: &[ColliderTriangle]) -> Self {
        Self::with_cell_size(triangles, Self::DEFAULT_CELL_SIZE)
    }

    pub fn with_cell_size(triangles: &[ColliderTriangle], cell_size: f32) -> Self {
        let cell_size = if cell_size.is_finite() && cell_size > 0.1 {
            cell_size
        } else {
            Self::DEFAULT_CELL_SIZE
        };
        let mut index = Self {
            triangles: Vec::new(),
            cells: HashMap::new(),
            cell_size,
            skipped: 0,
        };

        for triangle in triangles {
            let [a, b, c] = triangle.vertices;
            let normal = (b - a).cross(c - a);
            let length_squared = normal.length_squared();
            if length_squared <= 1.0e-12 || !length_squared.is_finite() {
                index.skipped += 1;
                continue;
            }
            let normal = normal / length_squared.sqrt();
            // Only near-horizontal triangles can be a surface under a wheel.
            if normal.y.abs() < 0.15 {
                index.skipped += 1;
                continue;
            }

            let id = index.triangles.len() as u32;
            index.triangles.push(IndexedTriangle {
                vertices: triangle.vertices,
                surface: triangle.surface,
            });

            let (min_x, max_x) = min_max(a.x, b.x, c.x);
            let (min_z, max_z) = min_max(a.z, b.z, c.z);
            if !(min_x.is_finite() && max_x.is_finite() && min_z.is_finite() && max_z.is_finite()) {
                continue;
            }
            let x0 = (min_x / cell_size).floor() as i32;
            let x1 = (max_x / cell_size).floor() as i32;
            let z0 = (min_z / cell_size).floor() as i32;
            let z1 = (max_z / cell_size).floor() as i32;
            let cell_count = (x1 as i64 - x0 as i64 + 1) * (z1 as i64 - z0 as i64 + 1);
            if cell_count <= 0 || cell_count > 4096 {
                // Degenerate or enormous projected area: put it in the cell of
                // its centroid so it can still be found, without exploding the
                // grid. Real track triangles are far below this cap.
                let centroid = (a + b + c) / 3.0;
                index
                    .cells
                    .entry(cell_key(centroid.x, centroid.z, cell_size))
                    .or_default()
                    .push(id);
                continue;
            }
            for cell_x in x0..=x1 {
                for cell_z in z0..=z1 {
                    index.cells.entry((cell_x, cell_z)).or_default().push(id);
                }
            }
        }

        index
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Number of triangles usable for surface lookup.
    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    /// Number of triangles skipped because they were vertical or degenerate.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Sample the surface under `position`.
    ///
    /// Uses the highest triangle at or slightly below `position.y`, which
    /// handles bridges and banked sections. If every candidate is above the
    /// query, the closest candidate is returned so an airborne wheel still
    /// reports the surface it is about to land on.
    pub fn sample(&self, position: Vec3) -> Option<SurfaceId> {
        self.sample_entry(position).map(|(_, surface)| surface)
    }

    /// Interpolated height of the surface under `position`.
    ///
    /// This uses the same triangle selection as [`Self::sample`], so callers
    /// can obtain a matching height and surface for spawn or debug queries.
    pub fn sample_height(&self, position: Vec3) -> Option<f32> {
        self.sample_entry(position).map(|(height, _)| height)
    }

    fn sample_entry(&self, position: Vec3) -> Option<(f32, SurfaceId)> {
        if self.triangles.is_empty() || !position.is_finite() {
            return None;
        }
        let key = cell_key(position.x, position.z, self.cell_size);
        let candidates = self.cells.get(&key)?;

        let mut best_below: Option<(f32, SurfaceId)> = None;
        let mut closest: Option<(f32, f32, SurfaceId)> = None;

        for &id in candidates {
            let triangle = &self.triangles[id as usize];
            let Some(height) = triangle_height(triangle.vertices, position.x, position.z) else {
                continue;
            };
            let delta = height - position.y;
            let distance = delta.abs();
            if closest.is_none_or(|(best, _, _)| distance < best) {
                closest = Some((distance, height, triangle.surface));
            }
            if delta <= 0.35 && best_below.is_none_or(|(best, _)| height > best) {
                best_below = Some((height, triangle.surface));
            }
        }

        best_below.or_else(|| closest.map(|(_, height, surface)| (height, surface)))
    }
}

pub fn triangle_height(vertices: [Vec3; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = vertices;
    let denominator = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
    if denominator.abs() < 1.0e-8 {
        return None;
    }
    let wa = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / denominator;
    let wb = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / denominator;
    let wc = 1.0 - wa - wb;
    if wa < -1.0e-4 || wb < -1.0e-4 || wc < -1.0e-4 {
        return None;
    }
    Some(wa * a.y + wb * b.y + wc * c.y)
}

fn cell_key(x: f32, z: f32, cell_size: f32) -> (i32, i32) {
    (
        (x / cell_size).floor() as i32,
        (z / cell_size).floor() as i32,
    )
}

fn min_max(a: f32, b: f32, c: f32) -> (f32, f32) {
    (a.min(b).min(c), a.max(b).max(c))
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
    fn road_undulation_is_a_deterministic_function_of_position() {
        // Gravel from AC's defaults: 3 cm over half a metre.
        let gravel = SurfaceKind::Gravel.properties();
        assert_eq!(gravel.bump_height_m, 0.02);
        assert_eq!(gravel.bump_length_m, 0.35);
        // Asphalt is laid flat, so it never undulates.
        assert_eq!(
            SurfaceKind::Asphalt
                .properties()
                .bump_m(Vec3::ZERO, Vec3::NEG_Z),
            0.0
        );

        // A quarter wavelength in is the peak, half is flat again.
        let heading = Vec3::NEG_Z;
        let peak = gravel.bump_m(Vec3::new(0.0, 0.0, -gravel.bump_length_m * 0.25), heading);
        let flat = gravel.bump_m(Vec3::new(0.0, 0.0, -gravel.bump_length_m * 0.5), heading);
        assert!((peak - gravel.bump_height_m).abs() < 1e-6, "{peak}");
        assert!(flat.abs() < 1e-6, "{flat}");
        // The slope peaks where the height crosses zero.
        let slope_at_peak =
            gravel.bump_slope(Vec3::new(0.0, 0.0, -gravel.bump_length_m * 0.25), heading);
        assert!(slope_at_peak.abs() < 1e-4, "{slope_at_peak}");
        let slope_at_crossing = gravel.bump_slope(Vec3::new(0.0, 0.0, 0.0), heading);
        assert!(slope_at_crossing.abs() > 0.3, "{slope_at_crossing}");
        // Sideways travel leaves the phase alone.
        assert!((gravel.bump_m(Vec3::new(5.0, 0.0, 0.0), heading)).abs() < 1e-6);
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
            bump_height_m: 0.02,
            bump_length_m: 0.6,
        });
        map.patch(SurfaceShape::circle(0.0, 0.0, 1.0), id);
        assert_eq!(map.surface_at(Vec3::ZERO), id);
        assert!((map.table().get(id).unwrap().dry_grip - 1.6).abs() < 1e-6);
    }
}
