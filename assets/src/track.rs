//! Assetto Corsa track modules and their physics geometry.
//!
//! A track is more than a mesh: `models.ini` places the visual/physical KN5
//! files, `data/surfaces.ini` defines what each surface key means, and every
//! node whose name starts with `1` (`1ROAD_...`, `1GRASS_...`, `1WALL_...`)
//! is collidable. Physical geometry is not limited to `collision.kn5`: tracks
//! commonly keep drivable road surfaces in `*mesh*.kn5`.
//!
//! [`TrackModule::load`] turns that directory structure into
//!
//! - the placed [`TrackModelEntry`] list (visual models stay in the renderer),
//! - a [`vettura_world::SurfaceMap`] whose mesh index samples the collision triangles,
//! - one compact triangle mesh that [`vettura_world::World`] can turn into static Jolt bodies.
//!
//! The AC authoring frame is `+X` left / `+Y` up / `+Z` forward. Everything
//! returned by this module is already converted to the Vettura physics frame
//! (`+X` right / `+Y` up / `-Z` forward); callers should not convert twice.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};
use thiserror::Error;
use vettura_acdata::ini::Ini;
use vettura_core::coords::ac_to_godot;
use vettura_kn5::{Kn5, Node, NodeKind, transform_to_mat4};
use vettura_physics::Pose;
use vettura_world::{
    AiDriver, AiSpline, ColliderTriangle, DEFAULT_GROOVE_CELL_M, GrooveMap, GrooveSettings,
    LapTimer, MeshSurfaceIndex, SPAWN_CLEARANCE_M, SurfaceId, SurfaceKind, SurfaceMap,
    SurfaceProperties, TrackObjectId, TrackSection, World, parse_groove_ini, triangle_height,
};

/// 赛道加载错误：文件 / INI / KN5 解析，以及底层世界装配。
#[derive(Debug, Error)]
pub enum TrackError {
    #[error("track i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid track data: {0}")]
    Format(String),
    #[error("KN5 error: {0}")]
    Kn5(#[from] vettura_kn5::Error),
    #[error(transparent)]
    World(#[from] vettura_world::WorldError),
}

/// 赛道加载结果别名。
pub type TrackResult<T> = std::result::Result<T, TrackError>;

/// Horizontal radius of the cylinder used to check spawn clearance, metres.
pub const SPAWN_CLEARANCE_RADIUS_M: f32 = 2.2;
/// Height above the ground checked for overhead collider geometry, metres.
pub const SPAWN_CLEARANCE_HEIGHT_M: f32 = 2.0;
/// Maximum residual of the four spawn samples against the fitted road plane,
/// metres. Banked corners are allowed; kerbs and seams are not.
pub const SPAWN_MAX_HEIGHT_VARIATION_M: f32 = 0.15;
/// Maximum road slope at a spawn, as height change per horizontal metre.
/// Daytona's oval banking is about 0.33.
pub const SPAWN_MAX_SLOPE: f32 = 0.45;
/// Maximum horizontal distance a spawn may snap away from the AI line, metres.
pub const SPAWN_SNAP_RADIUS_M: f32 = 2.5;
/// Maximum vertical distance between the AI line and the collision surface.
pub const SPAWN_MAX_LINE_HEIGHT_M: f32 = 2.0;
/// Default distance between starting-grid slots, metres.
pub const DEFAULT_SPAWN_SPACING_M: f32 = 8.0;
const SPAWN_SNAP_STEP_M: f32 = 0.5;

/// One static entry from `models.ini` / `models_<layout>.ini`.
///
/// `position` and `rotation_deg` are kept in the AC authoring frame exactly as
/// written in the INI, so callers can round-trip or display the original data.
/// Use [`TrackModelEntry::transform`] for a physics-frame model matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackModelEntry {
    /// KN5 file name, relative to the track root.
    pub file: String,
    /// AC-frame position (`+X` left, `+Z` forward).
    pub position: Vec3,
    /// AC-frame Euler rotation in degrees, XYZ order.
    pub rotation_deg: Vec3,
}

impl TrackModelEntry {
    /// Model-to-world matrix in the Vettura physics frame.
    pub fn transform(&self) -> Mat4 {
        euler_deg_to_mat4(self.position, self.rotation_deg)
    }

    /// Whether the file name follows AC's `collision*.kn5` convention.
    ///
    /// This is only a hint for diagnostics. Physical geometry is identified by
    /// a leading `1` on node names and may live in any placed model, including
    /// visual `*mesh*.kn5` files.
    pub fn is_collider(&self) -> bool {
        let name = self.file.to_ascii_lowercase();
        name.contains("collision") || name.contains("collider")
    }
}

/// Parse the static `[MODEL_n]` sections of an AC `models*.ini`.
///
/// Dynamic objects (`[DYNAMIC_OBJECT_n]`) are intentionally ignored: they are
/// visual props and are not part of the deterministic physics world.
pub fn parse_models_ini(text: &str) -> Vec<TrackModelEntry> {
    let ini = Ini::parse(text);
    let mut models = Vec::new();

    for section in &ini.sections {
        if !section.name.to_ascii_uppercase().starts_with("MODEL_") {
            continue;
        }
        let Some(file) = section.get("FILE").map(unquote).filter(|s| !s.is_empty()) else {
            continue;
        };
        let position = section
            .get("POSITION")
            .map(parse_vec3)
            .unwrap_or(Vec3::ZERO);
        let rotation_deg = section
            .get("ROTATION")
            .map(parse_vec3)
            .unwrap_or(Vec3::ZERO);
        models.push(TrackModelEntry {
            file: file.to_owned(),
            position,
            rotation_deg,
        });
    }

    models
}

/// Build a physics-frame model matrix from AC `models.ini` position/rotation.
///
/// `rotation_deg` is interpreted as an XYZ Euler triple (degrees). The result
/// conjugates the AC transform by the 180° yaw between the AC and physics
/// frames, so a KN5 whose vertices were converted with
/// [`vettura_core::coords::ac_to_godot`] can be transformed by this matrix
/// directly.
pub fn euler_deg_to_mat4(position: Vec3, rotation_deg: Vec3) -> Mat4 {
    let position = finite_vec3(position);
    let rotation_deg = finite_vec3(rotation_deg);
    let rotation = Mat4::from_euler(
        EulerRot::XYZ,
        rotation_deg.x.to_radians(),
        rotation_deg.y.to_radians(),
        rotation_deg.z.to_radians(),
    );
    let ac = Mat4::from_translation(position) * rotation;

    // C = 180° yaw: (x, y, z) -> (-x, y, -z), C⁻¹ = C.
    // p_physics = C * M_ac * C⁻¹ * p_physics_local.
    let c = Mat4::from_scale(Vec3::new(-1.0, 1.0, -1.0));
    c * ac * c
}

/// One surface definition from `data/surfaces.ini`.
///
/// The fields mirror the INI keys most useful to physics and telemetry. Values
/// that AC leaves empty or as `NULL` are represented by `None`/zero.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDef {
    /// The authored `SURFACE_n` index, useful for debugging and telemetry.
    pub index: u16,
    /// AC surface key (`ROAD`, `GRASS`, `KERB`, ...).
    pub key: String,
    /// Dry friction coefficient from AC.
    pub friction: f32,
    /// Suspension damping hint. Not yet used by the Jolt tyre model.
    pub damping: f32,
    /// Surface sound file, if authored.
    pub wav: Option<String>,
    /// Sound pitch multiplier.
    pub wav_pitch: f32,
    /// Force-feedback effect strength; `None` for `NULL`.
    pub ff_effect: Option<f32>,
    /// How much dirt this surface adds to the tyres.
    pub dirt_additive: f32,
    /// Black-flag time in seconds when cutting through this surface.
    pub black_flag_time_s: i32,
    /// Whether the surface counts as valid track.
    pub is_valid_track: bool,
    /// Vertical sine-excitation height, metres.
    pub sin_height_m: f32,
    /// Vertical sine-excitation wavelength, metres.
    pub sin_length_m: f32,
    /// Whether this surface belongs to the pit lane.
    pub is_pitlane: bool,
    /// Vibration gain for force feedback.
    pub vibration_gain: f32,
    /// Vibration wavelength, metres.
    pub vibration_length_m: f32,
}

impl SurfaceDef {
    /// Physics properties derived from the AC definition.
    ///
    /// AC only authors dry friction. The wet value keeps the standard
    /// wet/dry ratio of the closest [`SurfaceKind`], so wet weather still
    /// behaves sensibly on mod tracks.
    pub fn properties(&self) -> SurfaceProperties {
        let kind = surface_kind(&self.key);
        let base = kind.properties();
        let dry = if self.friction.is_finite() {
            self.friction.clamp(0.0, 2.5)
        } else {
            base.dry_grip
        };
        let wet_ratio = if base.dry_grip > 0.0 {
            (base.wet_grip / base.dry_grip).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // `SIN_HEIGHT` / `SIN_LENGTH` are AC's road undulation: the racing
        // surface is laid flat and the run-off areas are not, so the authored
        // value replaces the kind's default.
        let bump_height_m = if self.sin_height_m.is_finite() {
            self.sin_height_m.clamp(0.0, 0.2)
        } else {
            base.bump_height_m
        };
        let bump_length_m = if self.sin_length_m.is_finite() && self.sin_length_m > 0.0 {
            self.sin_length_m.clamp(0.25, 50.0)
        } else {
            base.bump_length_m
        };
        SurfaceProperties {
            kind,
            dry_grip: dry,
            wet_grip: dry * wet_ratio,
            rolling_resistance: base.rolling_resistance,
            roughness: base.roughness.max(if self.vibration_gain.is_finite() {
                self.vibration_gain.clamp(0.0, 2.0)
            } else {
                0.0
            }),
            bump_height_m,
            bump_length_m,
        }
    }
}

/// Parse all `[SURFACE_n]` sections of an AC `surfaces.ini`.
pub fn parse_surfaces_ini(text: &str) -> Vec<SurfaceDef> {
    let ini = Ini::parse(text);
    let mut surfaces = Vec::new();

    for (position, section) in ini.sections.iter().enumerate() {
        let upper = section.name.to_ascii_uppercase();
        if !upper.starts_with("SURFACE_") {
            continue;
        }
        let Some(key) = section.get("KEY").map(unquote).filter(|s| !s.is_empty()) else {
            continue;
        };
        let index = upper
            .strip_prefix("SURFACE_")
            .and_then(|suffix| suffix.parse::<u16>().ok())
            .unwrap_or(position as u16);
        let kind = surface_kind(key);

        surfaces.push(SurfaceDef {
            index,
            key: key.to_owned(),
            friction: section
                .get_f32("FRICTION")
                .unwrap_or(kind.properties().dry_grip),
            damping: section.get_f32("DAMPING").unwrap_or(0.0),
            wav: section
                .get("WAV")
                .map(unquote)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            wav_pitch: section.get_f32("WAV_PITCH").unwrap_or(0.0),
            ff_effect: section.get("FF_EFFECT").and_then(parse_optional_f32),
            dirt_additive: section.get_f32("DIRT_ADDITIVE").unwrap_or(0.0),
            black_flag_time_s: section
                .get("BLACK_FLAG_TIME")
                .and_then(parse_i32)
                .unwrap_or(0),
            is_valid_track: section
                .get("IS_VALID_TRACK")
                .and_then(parse_i32)
                .map(|value| value != 0)
                .unwrap_or(true),
            sin_height_m: section.get_f32("SIN_HEIGHT").unwrap_or(0.0),
            sin_length_m: section.get_f32("SIN_LENGTH").unwrap_or(0.0),
            is_pitlane: section
                .get("IS_PITLANE")
                .and_then(parse_i32)
                .map(|value| value != 0)
                .unwrap_or(false),
            vibration_gain: section.get_f32("VIBRATION_GAIN").unwrap_or(0.0),
            vibration_length_m: section.get_f32("VIBRATION_LENGTH").unwrap_or(0.0),
        });
    }

    surfaces
}

/// Parse all `[SECTION_n]` sections of an AC `sections.ini`.
///
/// The [`TrackSection`] type itself lives in `vettura_world` (the timing
/// plugin owns sector semantics); this module only parses it out of the INI.
///
/// Sections without usable `IN`/`OUT` values are skipped; the result is sorted
/// by `IN` so the caller always gets the sectors in lap order.
pub fn parse_sections_ini(text: &str) -> Vec<TrackSection> {
    let ini = Ini::parse(text);
    let mut sections = Vec::new();

    for section in &ini.sections {
        let name = section.name.to_ascii_uppercase();
        if !(name.starts_with("SECTION_") || name.starts_with("SECTOR_")) {
            continue;
        }
        let start = section.get_f32("IN").filter(|value| value.is_finite());
        let end = section.get_f32("OUT").filter(|value| value.is_finite());
        let (start, end) = match (start, end) {
            (Some(start), Some(end)) => (start, end),
            (Some(start), None) => (start, start),
            (None, Some(end)) => (end, end),
            (None, None) => continue,
        };
        let start = start.clamp(0.0, 1.0);
        let end = end.clamp(0.0, 1.0);
        if (end - start).abs() < 1.0e-5 {
            // A zero-length sector carries no timing information.
            continue;
        }
        sections.push(TrackSection {
            name: section
                .get("TEXT")
                .map(unquote)
                .filter(|name| !name.is_empty())
                .unwrap_or("Sector")
                .to_owned(),
            in_fraction: start.min(end),
            out_fraction: start.max(end),
        });
    }

    sections.sort_by(|a, b| a.in_fraction.total_cmp(&b.in_fraction));
    sections
}

/// Flatten every collidable static mesh under `kn5.root` into physics-frame
/// triangles.
///
/// AC marks physical geometry with a leading `1` on the node name
/// (`1ROAD_...`, `1grass_...`, `1WALL_...`). The marker may sit on a dummy
/// parent, in which case the meshes below it are collidable too. Only those
/// nodes contribute triangles: a track often keeps drivable road surfaces in
/// its visual `*mesh*.kn5` rather than the file named `collision.kn5`.
///
/// `model_transform` is the placement matrix from [`TrackModelEntry::transform`].
/// `default_surface` is used for meshes whose name does not identify a surface.
/// `surface_of` receives node names (for example `1ROAD_003`) and may return a
/// surface from the track's `surfaces.ini`; returning `None` falls back to the
/// parent's surface and finally to `default_surface`.
pub fn collect_collider_triangles(
    kn5: &Kn5,
    model_transform: Mat4,
    default_surface: SurfaceId,
    mut surface_of: impl FnMut(&str) -> Option<SurfaceId>,
) -> Vec<ColliderTriangle> {
    let mut triangles = Vec::new();
    walk_meshes(
        &kn5.root,
        model_transform,
        None,
        false,
        default_surface,
        &mut surface_of,
        &mut triangles,
    );
    triangles
}

fn walk_meshes(
    node: &Node,
    parent_transform: Mat4,
    inherited_surface: Option<SurfaceId>,
    inherited_physical: bool,
    default_surface: SurfaceId,
    surface_of: &mut dyn FnMut(&str) -> Option<SurfaceId>,
    out: &mut Vec<ColliderTriangle>,
) {
    let local = match &node.kind {
        NodeKind::Dummy { transform } => physics_transform(transform_to_mat4(transform)),
        _ => Mat4::IDENTITY,
    };
    let world = parent_transform * local;
    let surface = surface_of(&node.name)
        .or(inherited_surface)
        .unwrap_or(default_surface);
    // A leading `1` on either this node or an ancestor marks AC's collidable
    // geometry. Node names such as `10pits` are not physical: the surface key
    // parser rejects names without a letter after the marker.
    let physical = inherited_physical || node_surface_key(&node.name).is_some();
    // Negative scale mirrors the mesh, so its triangles must be reversed to
    // keep the front faces on the same side after the transform.
    let flip_winding = world.determinant() < 0.0;

    if physical && let NodeKind::Mesh(mesh) = &node.kind {
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let mut vertices = [Vec3::ZERO; 3];
            for (slot, &index) in vertices.iter_mut().zip(triangle) {
                let position = mesh
                    .vertices
                    .get(index as usize)
                    .map(|vertex| Vec3::from_array(vertex.position))
                    .unwrap_or(Vec3::ZERO);
                *slot = world.transform_point3(ac_to_godot(position));
            }
            let area = (vertices[1] - vertices[0])
                .cross(vertices[2] - vertices[0])
                .length_squared();
            if area > 1.0e-12 && vertices.iter().all(|vertex| vertex.is_finite()) {
                if flip_winding {
                    vertices.swap(1, 2);
                }
                out.push(ColliderTriangle { vertices, surface });
            }
        }
    }

    for child in &node.children {
        walk_meshes(
            child,
            world,
            Some(surface),
            physical,
            default_surface,
            surface_of,
            out,
        );
    }
}

fn physics_transform(ac: Mat4) -> Mat4 {
    let c = Mat4::from_scale(Vec3::new(-1.0, 1.0, -1.0));
    c * ac * c
}

/// A loaded AC track: placed models, surfaces, and physics collider.
#[derive(Clone, Debug)]
pub struct TrackModule {
    root: PathBuf,
    layout: Option<String>,
    models: Vec<TrackModelEntry>,
    surfaces: Vec<SurfaceDef>,
    surface_ids: Vec<SurfaceId>,
    surface_map: SurfaceMap,
    collider: Option<TrackCollider>,
    /// Timed sectors from this layout's `sections.ini`, in lap order.
    sections: Vec<TrackSection>,
    /// Track-evolution settings from `groove.ini`, when the track ships one.
    groove: Option<GrooveSettings>,
}

#[derive(Clone, Debug)]
struct TrackCollider {
    path: PathBuf,
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    surface_index: MeshSurfaceIndex,
}

impl TrackCollider {
    /// Conservative overhead check for spawn selection.
    ///
    /// A car footprint is approximated by a vertical cylinder. Any triangle
    /// that crosses the cylinder between `center.y + 0.6 m` and
    /// `center.y + height_m` blocks the spawn. The lower bound ignores road
    /// surface and kerbs; the upper bound keeps low bridges and gantries from
    /// hiding a real clearance problem.
    fn overhead_blocked(&self, center: Vec3, radius_m: f32, height_m: f32) -> bool {
        let radius_sq = radius_m.max(0.0).powi(2);
        let min_y = center.y + 0.6;
        let max_y = center.y + height_m.max(0.6);

        for triangle in &self.triangles {
            let points = triangle.map(|index| self.vertices[index as usize]);
            let min_x = points[0].x.min(points[1].x).min(points[2].x);
            let max_x = points[0].x.max(points[1].x).max(points[2].x);
            let min_z = points[0].z.min(points[1].z).min(points[2].z);
            let max_z = points[0].z.max(points[1].z).max(points[2].z);
            if max_x < center.x - radius_m
                || min_x > center.x + radius_m
                || max_z < center.z - radius_m
                || min_z > center.z + radius_m
            {
                continue;
            }

            let min_tri_y = points[0].y.min(points[1].y).min(points[2].y);
            let max_tri_y = points[0].y.max(points[1].y).max(points[2].y);
            if max_tri_y < min_y || min_tri_y > max_y {
                continue;
            }

            // A horizontal or sloped surface directly above the car.
            if triangle_height(points, center.x, center.z)
                .is_some_and(|height| height > min_y && height < max_y)
            {
                return true;
            }

            // Columns, walls and the edges of overhead geometry.
            for point in points {
                if point.y <= min_y || point.y >= max_y {
                    continue;
                }
                let dx = point.x - center.x;
                let dz = point.z - center.z;
                if dx * dx + dz * dz <= radius_sq {
                    return true;
                }
            }
        }

        false
    }
}

impl TrackModule {
    /// Load a track directory, using `models.ini` when present or the first
    /// `models_*.ini` (alphabetically) otherwise.
    pub fn load(root: impl AsRef<Path>) -> TrackResult<Self> {
        let root = root.as_ref();
        let models_path = find_models_ini(root).ok_or_else(|| {
            TrackError::Format(format!(
                "no models.ini or models_*.ini in {}",
                root.display()
            ))
        })?;
        let layout = models_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix("models_"))
            .map(str::to_owned)
            .filter(|layout| !layout.is_empty() && !layout.eq_ignore_ascii_case("ini"));
        Self::load_files(root, &models_path, layout)
    }

    /// Load a specific layout (`roadcourse` for `models_roadcourse.ini`).
    pub fn load_layout(root: impl AsRef<Path>, layout: &str) -> TrackResult<Self> {
        let root = root.as_ref();
        let layout = layout
            .trim()
            .trim_end_matches(".ini")
            .trim_start_matches("models_");
        let name = if layout.is_empty() || layout.eq_ignore_ascii_case("models") {
            "models.ini".to_owned()
        } else {
            format!("models_{layout}.ini")
        };
        let models_path = root.join(name);
        if !models_path.is_file() {
            return Err(TrackError::Io {
                path: models_path,
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "track layout models file not found",
                ),
            });
        }
        Self::load_files(
            root,
            &models_path,
            (!layout.is_empty() && !layout.eq_ignore_ascii_case("models"))
                .then(|| layout.to_owned()),
        )
    }

    fn load_files(root: &Path, models_path: &Path, layout: Option<String>) -> TrackResult<Self> {
        let models_text = read_text(models_path)?;
        let models = parse_models_ini(&models_text);

        // Standard tracks keep `data/` next to models.ini. Multi-layout tracks
        // (Daytona, Spa, ...) keep `data/` inside the layout folder, e.g.
        // `roadcourse/data/surfaces.ini`.
        let surfaces = if let Some(surfaces_path) = find_surfaces_ini(root, layout.as_deref()) {
            parse_surfaces_ini(&read_text(&surfaces_path)?)
        } else {
            Vec::new()
        };

        // `sections.ini` describes the timed sectors of the layout. It is
        // optional and only affects timing, never geometry.
        let sections = find_sections_ini(root, layout.as_deref())
            .and_then(|path| read_text(&path).ok())
            .as_deref()
            .map(parse_sections_ini)
            .unwrap_or_default();

        // `groove.ini` describes how rubber builds up grip on the racing line
        // within one session. A track without the file has no evolution.
        let groove = find_groove_ini(root, layout.as_deref())
            .and_then(|path| read_text(&path).ok())
            .as_deref()
            .map(parse_groove_ini)
            .and_then(|defs| GrooveSettings::from_defs(&defs));

        let mut surface_map = SurfaceMap::new();
        let mut surface_ids = Vec::with_capacity(surfaces.len());
        for surface in &surfaces {
            let id = surface_map.add_surface(surface.properties());
            surface_ids.push(id);
            if surface.key.eq_ignore_ascii_case("ROAD") {
                surface_map.set_default_surface(id);
            }
        }

        let default_surface = if surfaces.is_empty() {
            SurfaceId::ASPHALT
        } else {
            surface_map.default_surface()
        };

        // AC does not require physical geometry to live in `collision.kn5`:
        // road surfaces are often authored in `*mesh*.kn5` and marked with a
        // leading `1`. Scan every placed model and keep only those physical
        // nodes. A model referenced twice with different transforms is
        // collected twice, which is intentional.
        let mut model_entries: Vec<&TrackModelEntry> = models.iter().collect();
        model_entries.sort_by(|a, b| a.file.cmp(&b.file));

        let mut triangles = Vec::new();
        let mut collider_path = None;
        let mut collider_vertices = Vec::new();
        let mut collider_indices = Vec::new();

        if model_entries.is_empty() {
            for name in ["collider.kn5", "collision.kn5"] {
                let path = root.join(name);
                if path.is_file() {
                    let entry = TrackModelEntry {
                        file: name.to_owned(),
                        position: Vec3::ZERO,
                        rotation_deg: Vec3::ZERO,
                    };
                    let kn5 = Kn5::from_path(&path)?;
                    let surface_of = surface_lookup(&surfaces, &surface_ids, &surface_map);
                    triangles.extend(collect_collider_triangles(
                        &kn5,
                        entry.transform(),
                        default_surface,
                        surface_of,
                    ));
                    collider_path = Some(path);
                    break;
                }
            }
        } else {
            for entry in &model_entries {
                let path = root.join(&entry.file);
                if !path.is_file() {
                    // Layouts often reference optional day/night variants that
                    // are not shipped. Physics should still load from the
                    // models that are present.
                    tracing::warn!(
                        model = %entry.file,
                        "track model listed in models.ini is missing; skipping"
                    );
                    continue;
                }
                let kn5 = Kn5::from_path(&path)?;
                let surface_of = surface_lookup(&surfaces, &surface_ids, &surface_map);
                let before = triangles.len();
                triangles.extend(collect_collider_triangles(
                    &kn5,
                    entry.transform(),
                    default_surface,
                    surface_of,
                ));
                // Keep a conventional collider path for diagnostics, falling
                // back to the first model that actually contributed geometry.
                if triangles.len() > before && (collider_path.is_none() || entry.is_collider()) {
                    collider_path = Some(path);
                }
            }
        }

        let surface_index = MeshSurfaceIndex::new(&triangles);
        if !surface_index.is_empty() {
            surface_map.set_mesh_index(surface_index.clone());
        }
        compact_mesh(&triangles, &mut collider_vertices, &mut collider_indices);

        let collider = collider_path.map(|path| TrackCollider {
            path,
            vertices: collider_vertices,
            triangles: collider_indices,
            surface_index,
        });

        Ok(Self {
            root: root.to_owned(),
            layout,
            models,
            surfaces,
            surface_ids,
            surface_map,
            collider,
            sections,
            groove,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Layout name (`roadcourse`, `oval`, ...), if derived from a
    /// `models_<layout>.ini` file.
    pub fn layout(&self) -> Option<&str> {
        self.layout.as_deref()
    }

    pub fn models(&self) -> &[TrackModelEntry] {
        &self.models
    }

    pub fn surfaces(&self) -> &[SurfaceDef] {
        &self.surfaces
    }

    /// Surface ID assigned to the `index`-th definition in [`Self::surfaces`].
    pub fn surface_ids(&self) -> &[SurfaceId] {
        &self.surface_ids
    }

    /// Surface map including the mesh-based triangle lookup.
    pub fn surface_map(&self) -> &SurfaceMap {
        &self.surface_map
    }

    pub fn collider_path(&self) -> Option<&Path> {
        self.collider
            .as_ref()
            .map(|collider| collider.path.as_path())
    }

    /// Collision triangles usable for surface lookup.
    pub fn surface_index(&self) -> Option<&MeshSurfaceIndex> {
        self.collider
            .as_ref()
            .map(|collider| &collider.surface_index)
    }

    /// The compact collider mesh (vertices + triangle indices).
    pub fn collider_mesh(&self) -> Option<(&[Vec3], &[[u32; 3]])> {
        self.collider
            .as_ref()
            .map(|collider| (collider.vertices.as_slice(), collider.triangles.as_slice()))
    }

    pub fn collider_triangle_count(&self) -> usize {
        self.collider
            .as_ref()
            .map_or(0, |collider| collider.triangles.len())
    }

    /// Look up the surface ID for a `surfaces.ini` key.
    pub fn surface_id(&self, key: &str) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .zip(&self.surface_ids)
            .find(|(surface, _)| surface.key.eq_ignore_ascii_case(key.trim()))
            .map(|(_, id)| *id)
    }

    /// Path to this layout's `ai/fast_lane.ai`, when the track ships one.
    pub fn fast_lane_path(&self) -> Option<PathBuf> {
        find_ai_file(&self.root, self.layout.as_deref(), "fast_lane.ai")
    }

    /// Parse `ai/fast_lane.ai`.
    ///
    /// AI data is optional, so a track without the file returns `Ok(None)`.
    /// Parse errors still propagate: a corrupt AI line should be visible to
    /// the caller instead of silently disabling the racing line.
    pub fn load_fast_lane(&self) -> TrackResult<Option<AiSpline>> {
        self.fast_lane_path()
            .map(AiSpline::from_path)
            .transpose()
            .map_err(TrackError::World)
    }

    /// Create a pure-pursuit AI driver for this track's `fast_lane.ai`.
    ///
    /// Returns `Ok(None)` when the track has no AI line.
    pub fn ai_driver(&self) -> TrackResult<Option<AiDriver>> {
        Ok(self.load_fast_lane()?.map(AiDriver::new))
    }

    /// Path to this layout's `ai/pit_lane.ai`, when the track ships one.
    pub fn pit_lane_path(&self) -> Option<PathBuf> {
        find_ai_file(&self.root, self.layout.as_deref(), "pit_lane.ai")
    }

    /// Whether the track has a pit lane spline that a car can follow.
    pub fn has_pit_lane(&self) -> bool {
        self.pit_lane_path().is_some()
    }

    /// Parse `ai/pit_lane.ai`.
    ///
    /// The pit lane uses the same v7 spline format as `fast_lane.ai`, so the
    /// returned [`AiSpline`] drives through [`AiDriver`] and spawns through
    /// [`Self::spawn_poses_on_line`] exactly like the racing line.
    pub fn load_pit_lane(&self) -> TrackResult<Option<AiSpline>> {
        self.pit_lane_path()
            .map(AiSpline::from_path)
            .transpose()
            .map_err(TrackError::World)
    }

    /// A driver that follows the pit lane instead of the racing line.
    pub fn pit_lane_driver(&self) -> TrackResult<Option<AiDriver>> {
        Ok(self.load_pit_lane()?.map(AiDriver::new))
    }

    /// Timed sectors of this layout, in lap order.
    ///
    /// Empty when the track has no `sections.ini`, which makes a lap a single
    /// untimed sector (see [`vettura_world::LapTimer`]).
    pub fn sections(&self) -> &[TrackSection] {
        &self.sections
    }

    /// Lap and sector timer for the racing line of this track.
    ///
    /// Returns `Ok(None)` when the track has no `fast_lane.ai`.
    pub fn lap_timer(&self) -> TrackResult<Option<LapTimer>> {
        Ok(self
            .load_fast_lane()?
            .map(|line| LapTimer::new(line, &self.sections)))
    }

    /// Spawn pose at the start of `fast_lane.ai`.
    ///
    /// The pose already uses the physics frame and is lifted by
    /// [`SPAWN_CLEARANCE_M`](vettura_world::SPAWN_CLEARANCE_M) above the line.
    pub fn start_pose(&self) -> TrackResult<Option<Pose>> {
        Ok(self.load_fast_lane()?.and_then(|line| line.start_pose()))
    }

    /// Build a safe starting grid behind the first `fast_lane.ai` point.
    ///
    /// Candidates are visited backwards from the start line (then forwards if
    /// the lap does not provide enough clear slots), keeping at least
    /// `spacing_m` between cars. A candidate is accepted only when:
    ///
    /// - the collision mesh has asphalt under all four wheels and the four
    ///   samples form a smooth plane (banking is allowed, kerbs are not);
    /// - the spawn point itself is on asphalt;
    /// - no collider geometry occupies the car's overhead clearance cylinder.
    ///
    /// This avoids the common AC pit-lane/gantry case where the raw AI-line
    /// start sits under overhead geometry. The returned poses are physics-frame
    /// poses with their position lifted by
    /// [`SPAWN_CLEARANCE_M`](vettura_world::SPAWN_CLEARANCE_M). Fewer than `count`
    /// poses may be returned when the available line has no clear slots.
    pub fn spawn_poses(&self, count: usize, spacing_m: f32) -> TrackResult<Vec<Pose>> {
        let Some(line) = self.load_fast_lane()? else {
            return Ok(Vec::new());
        };
        Ok(self.spawn_poses_on_line(&line, count, spacing_m))
    }

    /// [`Self::spawn_poses`] using an already parsed line.
    pub fn spawn_poses_on_line(&self, line: &AiSpline, count: usize, spacing_m: f32) -> Vec<Pose> {
        let count = count.min(1_000);
        if count == 0 || line.is_empty() {
            return Vec::new();
        }
        let spacing = if spacing_m.is_finite() {
            spacing_m.clamp(2.0, 100.0)
        } else {
            DEFAULT_SPAWN_SPACING_M
        };
        let minimum_distance_sq = (spacing * 0.9).powi(2);

        let mut poses: Vec<Pose> = Vec::with_capacity(count);
        let len = line.len();
        // Visit the start line first, then one full lap backwards, then one
        // lap forwards. Two laps cover every index even when the first lap
        // does not contain enough clear slots.
        for step in 0..len.saturating_mul(2) {
            let index = if step < len {
                (len - step) % len
            } else {
                step - len + 1
            };
            let Some(pose) = self.clear_spawn_pose(line, index) else {
                continue;
            };
            if poses.iter().any(|existing| {
                let delta = existing.position - pose.position;
                delta.x * delta.x + delta.z * delta.z < minimum_distance_sq
            }) {
                continue;
            }
            poses.push(pose);
            if poses.len() == count {
                break;
            }
        }
        poses
    }

    /// Check one AI-line index and return a lifted pose if the spot is clear.
    ///
    /// Mod colliders are not always watertight under the AI line, so the exact
    /// point is tried first and then nearby points in a deterministic spiral.
    /// The heading always stays aligned with the AI line.
    fn clear_spawn_pose(&self, line: &AiSpline, index: usize) -> Option<Pose> {
        let pose = line.pose_at(index)?;
        let forward = horizontal_axis(pose.orientation * Vec3::NEG_Z);
        let right = horizontal_axis(pose.orientation * Vec3::X);

        let steps = (SPAWN_SNAP_RADIUS_M / SPAWN_SNAP_STEP_M).ceil() as i32;
        let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(((steps * 2 + 1) as usize).pow(2));
        for lateral in -steps..=steps {
            for longitudinal in -steps..=steps {
                offsets.push((lateral, longitudinal));
            }
        }
        offsets.sort_by_key(|&(lateral, longitudinal)| {
            (
                lateral * lateral + longitudinal * longitudinal,
                lateral.abs() + longitudinal.abs(),
                longitudinal,
                lateral,
            )
        });

        for (lateral, longitudinal) in offsets {
            let base = pose.position
                + right * (lateral as f32 * SPAWN_SNAP_STEP_M)
                + forward * (longitudinal as f32 * SPAWN_SNAP_STEP_M);
            if let Some(spawn) = self.clear_spawn_at(base, pose.orientation) {
                return Some(spawn);
            }
        }
        None
    }

    /// Validate one snapped spawn position and lift it by the spawn clearance.
    fn clear_spawn_at(&self, base: Vec3, orientation: Quat) -> Option<Pose> {
        let Some(collider) = &self.collider else {
            // No collision mesh: trust the surface map. This is useful for
            // synthetic tracks and tests, where patches define the ground.
            return self.is_asphalt(base).then_some(Pose {
                position: base + Vec3::Y * SPAWN_CLEARANCE_M,
                orientation,
            });
        };

        let ground = collider.surface_index.sample_height(base)?;
        // Do not snap onto an overhead bridge when the AI line is on the road
        // below it. A hole in the collider is fine, a different floor is not.
        if (ground - base.y).abs() > SPAWN_MAX_LINE_HEIGHT_M {
            return None;
        }
        let center = Vec3::new(base.x, ground, base.z);
        if !self.is_asphalt(center) {
            return None;
        }

        // Fit a plane to the four wheel positions. A banked oval (Daytona)
        // tilts the whole grid, so only the residual against the plane matters,
        // not the absolute height variation.
        let forward = horizontal_axis(orientation * Vec3::NEG_Z);
        let right = horizontal_axis(orientation * Vec3::X);
        let forward_distance = 1.4;
        let right_distance = 0.9;
        let sample = |offset: Vec3| -> Option<f32> {
            let point = center + offset;
            let height = collider
                .surface_index
                .sample_height(point + Vec3::Y * 0.25)?;
            if !height.is_finite() || !self.is_asphalt(Vec3::new(point.x, height, point.z)) {
                return None;
            }
            Some(height)
        };

        let front = sample(forward * forward_distance)?;
        let back = sample(-forward * forward_distance)?;
        let right_side = sample(right * right_distance)?;
        let left_side = sample(-right * right_distance)?;
        let slope_forward = (front - back) / (forward_distance * 2.0);
        let slope_right = (right_side - left_side) / (right_distance * 2.0);
        if slope_forward.abs() > SPAWN_MAX_SLOPE || slope_right.abs() > SPAWN_MAX_SLOPE {
            return None;
        }

        // A smooth bank passes; a kerb, seam or bridge edge shows up as a
        // residual at one of the four corners.
        for forward_offset in [-forward_distance, forward_distance] {
            for right_offset in [-right_distance, right_distance] {
                let height = sample(forward * forward_offset + right * right_offset)?;
                let expected = ground + slope_forward * forward_offset + slope_right * right_offset;
                if (height - expected).abs() > SPAWN_MAX_HEIGHT_VARIATION_M {
                    return None;
                }
            }
        }

        if collider.overhead_blocked(center, SPAWN_CLEARANCE_RADIUS_M, SPAWN_CLEARANCE_HEIGHT_M) {
            return None;
        }

        // Keep the AI-line yaw but align local up with the fitted surface
        // normal so the car starts level with the banked road.
        let up = Vec3::new(
            -slope_forward * forward.x - slope_right * right.x,
            1.0,
            -slope_forward * forward.z - slope_right * right.z,
        )
        .normalize();
        let right = forward.cross(up).normalize();
        let forward = up.cross(right).normalize();
        let orientation = Quat::from_mat3(&Mat3::from_cols(right, up, -forward)).normalize();

        Some(Pose {
            position: center + up * SPAWN_CLEARANCE_M,
            orientation,
        })
    }

    fn is_asphalt(&self, position: Vec3) -> bool {
        self.surface_map
            .table()
            .get(self.surface_map.surface_at(position))
            .is_some_and(|properties| properties.kind == SurfaceKind::Asphalt)
    }

    /// Install this track's surface map and static collider into `world`.
    ///
    /// The world must not have vehicles yet, because vehicles keep a snapshot
    /// of the surface table for their tyre callback. Add the track first, then
    /// spawn cars on it.
    /// Track-evolution settings, when this layout ships `groove.ini`.
    pub fn groove_settings(&self) -> Option<GrooveSettings> {
        self.groove
    }

    /// Whether the track can build rubber as cars drive over it.
    pub fn has_groove(&self) -> bool {
        self.groove.is_some()
    }

    /// Build a fresh rubber map for this track's collider.
    ///
    /// Returns `None` when the track has no `groove.ini`. The map starts cold,
    /// which is what a new session needs.
    pub fn groove_map(&self) -> Option<GrooveMap> {
        let settings = self.groove?;
        let vertices = self.collider.as_ref()?.vertices.as_slice();
        Some(GrooveMap::new(vertices, settings, DEFAULT_GROOVE_CELL_M))
    }

    pub fn add_to_world(&self, world: &mut World) -> TrackResult<Option<TrackObjectId>> {
        if let Some(groove) = self.groove_map() {
            world.set_groove_map(groove);
        }
        world.set_surface_map(self.surface_map.clone())?;
        let Some(collider) = &self.collider else {
            return Ok(None);
        };
        if collider.vertices.is_empty() || collider.triangles.is_empty() {
            return Ok(None);
        }
        world
            .add_static_mesh(&collider.vertices, &collider.triangles, 1.0, 0.0)
            .map(Some)
            .map_err(TrackError::World)
    }

    /// Convenience: create a world with a safety ground and this track's
    /// collider installed.
    ///
    /// The safety ground catches cars that leave the collider. Use
    /// [`Self::create_world_without_ground`] when the track has geometry below
    /// `y = 0` or when a fall should not be silently caught.
    pub fn create_world(&self) -> TrackResult<World> {
        let mut world = World::new(self.surface_map.clone())?;
        self.add_to_world(&mut world)?;
        Ok(world)
    }

    /// Convenience: create a world whose only static ground is the track
    /// collider.
    pub fn create_world_without_ground(&self) -> TrackResult<World> {
        let mut world = World::without_ground(self.surface_map.clone())?;
        self.add_to_world(&mut world)?;
        Ok(world)
    }
}

fn find_surfaces_ini(root: &Path, layout: Option<&str>) -> Option<PathBuf> {
    if let Some(layout) = layout {
        let path = root.join(layout).join("data").join("surfaces.ini");
        if path.is_file() {
            return Some(path);
        }
    }
    let path = root.join("data").join("surfaces.ini");
    if path.is_file() {
        return Some(path);
    }
    // Last resort for a layout chosen by `load`: any subdirectory.
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path().join("data").join("surfaces.ini");
        if path.is_file() {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

fn find_groove_ini(root: &Path, layout: Option<&str>) -> Option<PathBuf> {
    if let Some(layout) = layout {
        let path = root.join(layout).join("data").join("groove.ini");
        if path.is_file() {
            return Some(path);
        }
    }
    let path = root.join("data").join("groove.ini");
    if path.is_file() {
        return Some(path);
    }
    None
}

fn find_sections_ini(root: &Path, layout: Option<&str>) -> Option<PathBuf> {
    if let Some(layout) = layout {
        let path = root.join(layout).join("data").join("sections.ini");
        if path.is_file() {
            return Some(path);
        }
    }
    let path = root.join("data").join("sections.ini");
    if path.is_file() {
        return Some(path);
    }
    None
}

fn find_ai_file(root: &Path, layout: Option<&str>, name: &str) -> Option<PathBuf> {
    if let Some(layout) = layout {
        let path = root.join(layout).join("ai").join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    let path = root.join("ai").join(name);
    if path.is_file() {
        return Some(path);
    }
    // Last resort for a layout chosen by `load`.
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path().join("ai").join(name);
        if path.is_file() {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

fn find_models_ini(root: &Path) -> Option<PathBuf> {
    let default = root.join("models.ini");
    if default.is_file() {
        return Some(default);
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("models_") && lower.ends_with(".ini") {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

fn surface_lookup<'a>(
    surfaces: &'a [SurfaceDef],
    surface_ids: &'a [SurfaceId],
    surface_map: &'a SurfaceMap,
) -> impl FnMut(&str) -> Option<SurfaceId> + 'a {
    move |name| {
        let key = node_surface_key(name)?;
        if let Some(id) = surfaces
            .iter()
            .zip(surface_ids)
            .find(|(surface, _)| surface.key.eq_ignore_ascii_case(key))
            .map(|(_, id)| *id)
        {
            return Some(id);
        }
        // `1WALL` / `1PITS` and friends are often not listed in
        // surfaces.ini; fall back to the standard table entry for that kind.
        let kind = surface_kind(key);
        Some(surface_map.table().standard_id(kind))
    }
}

/// Extract the surface key from an AC collision node name.
///
/// AC marks physical meshes with a leading `1`: `1ROAD_003`, `1GRASS_01`,
/// `1WALL_pitwall02`. The key ends at the first `_`, digit or other separator.
fn node_surface_key(name: &str) -> Option<&str> {
    let name = name.trim();
    let stripped = name.strip_prefix('1')?;
    let end = stripped
        .char_indices()
        .find(|(_, c)| *c == '_' || c.is_ascii_digit() || c.is_whitespace())
        .map_or(stripped.len(), |(index, _)| index);
    let key = &stripped[..end];
    (!key.is_empty()).then_some(key)
}

fn surface_kind(key: &str) -> SurfaceKind {
    match key.trim().to_ascii_uppercase().as_str() {
        "ROAD" | "ASPHALT" | "TARMAC" | "TRACK" => SurfaceKind::Asphalt,
        "KERB" | "CURB" => SurfaceKind::Curb,
        "GRASS" | "GREEN" | "LAWN" => SurfaceKind::Grass,
        "GRAVEL" | "GRAV" => SurfaceKind::Gravel,
        "SAND" => SurfaceKind::Sand,
        "DIRT" | "CUT" | "SOIL" => SurfaceKind::Dirt,
        "SNOW" => SurfaceKind::Snow,
        "ICE" => SurfaceKind::Ice,
        "WATER" | "WET" => SurfaceKind::Water,
        "PITS" | "PITLANE" | "PIT" | "CONCRETE" | "WALL" | "BARRIER" => SurfaceKind::Concrete,
        _ => SurfaceKind::Asphalt,
    }
}

fn horizontal_axis(axis: Vec3) -> Vec3 {
    let horizontal = Vec3::new(axis.x, 0.0, axis.z);
    if horizontal.length_squared() > 1.0e-8 {
        horizontal.normalize()
    } else {
        Vec3::NEG_Z
    }
}

fn compact_mesh(
    triangles: &[ColliderTriangle],
    vertices: &mut Vec<Vec3>,
    indices: &mut Vec<[u32; 3]>,
) {
    let mut lookup: HashMap<[u32; 3], u32> = HashMap::new();
    for triangle in triangles {
        let mut index = [0u32; 3];
        for (slot, &vertex) in index.iter_mut().zip(&triangle.vertices) {
            let key = [vertex.x.to_bits(), vertex.y.to_bits(), vertex.z.to_bits()];
            *slot = *lookup.entry(key).or_insert_with(|| {
                let id = vertices.len() as u32;
                vertices.push(vertex);
                id
            });
        }
        if index[0] != index[1] && index[1] != index[2] && index[2] != index[0] {
            indices.push(index);
        }
    }
}

fn read_text(path: &Path) -> TrackResult<String> {
    std::fs::read_to_string(path).map_err(|source| TrackError::Io {
        path: path.to_owned(),
        source,
    })
}

fn parse_vec3(value: &str) -> Vec3 {
    let mut parts = value
        .trim()
        .trim_matches(|c| c == '(' || c == ')')
        .split([',', ' ', '\t'])
        .filter(|part| !part.is_empty())
        .map(parse_f32);
    let x = parts.next().unwrap_or(0.0);
    let y = parts.next().unwrap_or(0.0);
    let z = parts.next().unwrap_or(0.0);
    finite_vec3(Vec3::new(x, y, z))
}

fn parse_f32(value: &str) -> f32 {
    value.trim().parse::<f32>().unwrap_or(0.0)
}

fn parse_i32(value: &str) -> Option<i32> {
    let value = value.trim();
    value
        .parse::<i32>()
        .ok()
        .or_else(|| value.parse::<f32>().ok().map(|value| value as i32))
}

fn parse_optional_f32(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        None
    } else {
        value.parse().ok()
    }
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'').trim()
}

fn finite_vec3(value: Vec3) -> Vec3 {
    Vec3::new(
        if value.x.is_finite() { value.x } else { 0.0 },
        if value.y.is_finite() { value.y } else { 0.0 },
        if value.z.is_finite() { value.z } else { 0.0 },
    )
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::sync::Arc;

    #[test]
    fn parses_models_sections_and_ignores_dynamic_objects() {
        let models = parse_models_ini(
            r#"
[MODEL_0]
FILE=collision.kn5
POSITION=1,2,3
ROTATION=0,90,0

[MODEL_1]
FILE=visual.kn5

[DYNAMIC_OBJECT_0]
PROBABILITY=100
FILE=balloon.kn5
POS_MODE=RANDOM
"#,
        );
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].file, "collision.kn5");
        assert_eq!(models[0].position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(models[0].rotation_deg, Vec3::new(0.0, 90.0, 0.0));
        assert!(models[0].is_collider());
        assert!(!models[1].is_collider());
    }

    #[test]
    fn identity_model_transform_only_flips_the_frame() {
        let matrix = euler_deg_to_mat4(Vec3::new(10.0, 2.0, -3.0), Vec3::ZERO);
        let point = matrix.transform_point3(Vec3::ZERO);
        assert!((point - Vec3::new(-10.0, 2.0, 3.0)).length() < 1.0e-6);
    }

    #[test]
    fn yaw_is_converted_to_the_physics_frame() {
        // AC +Z (forward) rotated +90° about +Y maps to AC +X (left), which
        // becomes physics -X. The input vertex is already AC→physics converted.
        let matrix = euler_deg_to_mat4(Vec3::ZERO, Vec3::new(0.0, 90.0, 0.0));
        let point = matrix.transform_point3(ac_to_godot(Vec3::Z));
        assert!((point - Vec3::NEG_X).length() < 1.0e-5, "got {point:?}");
    }

    #[test]
    fn parses_surfaces_ini() {
        let surfaces = parse_surfaces_ini(
            r#"
[SURFACE_0]
KEY=KERB
FRICTION=0.92
WAV=kerb.wav
FF_EFFECT=1
IS_VALID_TRACK=1
IS_PITLANE=0
VIBRATION_GAIN=0.4

[SURFACE_1]
KEY=ROAD
FRICTION=1
WAV=
FF_EFFECT=NULL
IS_VALID_TRACK=1
"#,
        );
        assert_eq!(surfaces.len(), 2);
        assert_eq!(surfaces[0].key, "KERB");
        assert_eq!(surfaces[0].friction, 0.92);
        assert_eq!(surfaces[0].ff_effect, Some(1.0));
        assert_eq!(surfaces[1].wav, None);
        assert_eq!(surfaces[1].ff_effect, None);
        assert!(surfaces[1].is_valid_track);
        assert!((surfaces[0].properties().dry_grip - 0.92).abs() < 1.0e-6);
    }

    #[test]
    fn only_ac_physical_nodes_contribute_collider_triangles() {
        use vettura_kn5::{Mesh, MeshFlags, Vertex};

        fn triangle_mesh() -> Mesh {
            let vertex = |position| Vertex {
                position,
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: [1.0, 0.0, 0.0],
            };
            Mesh {
                flags: MeshFlags::default(),
                vertices: vec![
                    vertex([0.0, 0.0, 0.0]),
                    vertex([1.0, 0.0, 0.0]),
                    vertex([0.0, 0.0, 1.0]),
                ],
                indices: vec![0, 1, 2],
                material: 0,
                layer: 0,
                lod_in: 0.0,
                lod_out: 0.0,
                bounding_sphere_center: [0.0; 3],
                bounding_sphere_radius: 1.0,
                is_renderable: true,
            }
        }

        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mesh_node = |name: &str| Node {
            name: name.to_owned(),
            active: true,
            kind: NodeKind::Mesh(Arc::new(triangle_mesh())),
            children: Vec::new(),
        };
        let dummy = Node {
            name: "1ROAD_parent".to_owned(),
            active: true,
            kind: NodeKind::Dummy {
                transform: identity,
            },
            children: vec![mesh_node("road_surface")],
        };
        let root = Node {
            name: "_root".to_owned(),
            active: true,
            kind: NodeKind::Dummy {
                transform: identity,
            },
            children: vec![
                dummy,
                mesh_node("1grass_direct"),
                mesh_node("TREES"),
                mesh_node("10pits"),
            ],
        };
        let kn5 = Kn5 {
            version: 5,
            header_extra: None,
            textures: Vec::new(),
            materials: Vec::new(),
            root,
        };

        let triangles =
            collect_collider_triangles(&kn5, Mat4::IDENTITY, SurfaceId::ASPHALT, |_| None);
        assert_eq!(triangles.len(), 2, "only `1`-prefixed nodes are physical");
    }

    #[test]
    fn parses_sections_ini_in_lap_order() {
        let sections = parse_sections_ini(
            r#"
[SECTION_2]
IN=0.5
OUT=0.75
TEXT=Back straight

[SECTION_0]
IN=0
OUT=0.25
TEXT=Main straight

[SECTION_9]
IN=0.25
OUT=0.5
TEXT=Bus Stop

[IGNORE_ME]
IN=0
OUT=1
"#,
        );
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].name, "Main straight");
        assert_eq!(sections[1].name, "Bus Stop");
        assert_eq!(sections[2].name, "Back straight");
        assert_eq!(sections[0].in_fraction, 0.0);
        assert_eq!(sections[0].out_fraction, 0.25);
        assert!((sections[2].length_m(1000.0) - 250.0).abs() < 1.0e-4);
        assert!(sections[0].contains(0.1));

        // Sections without usable timings are dropped rather than producing
        // zero-length sectors that never complete.
        let empty = parse_sections_ini("[SECTION_0]\nTEXT=No timings\n");
        assert!(empty.is_empty());
        let zero = parse_sections_ini("[SECTION_0]\nIN=0.5\nOUT=0.5\n");
        assert!(zero.is_empty());
    }

    #[test]
    fn node_names_map_to_surface_keys() {
        assert_eq!(node_surface_key("1ROAD_003"), Some("ROAD"));
        assert_eq!(node_surface_key("1grass_2infield06"), Some("grass"));
        assert_eq!(node_surface_key("1wall_pitwall01"), Some("wall"));
        assert_eq!(node_surface_key("road_cone"), None);
        assert_eq!(node_surface_key("1"), None);
    }

    #[test]
    fn mesh_surface_index_samples_a_sloped_triangle() {
        let triangles = [ColliderTriangle {
            vertices: [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 10.0),
            ],
            surface: SurfaceId::GRASS,
        }];
        let index = MeshSurfaceIndex::with_cell_size(&triangles, 4.0);
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.sample(Vec3::new(1.0, 0.1, 1.0)),
            Some(SurfaceId::GRASS)
        );
        assert!((index.sample_height(Vec3::new(1.0, 0.1, 1.0)).unwrap() - 0.1).abs() < 1.0e-5);
        assert_eq!(index.sample(Vec3::new(50.0, 0.0, 50.0)), None);
    }

    #[test]
    fn spawn_overhead_check_rejects_low_roofs_only() {
        let ground = [
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(-10.0, 0.0, 10.0),
        ];
        let roof = [
            Vec3::new(-3.0, 1.5, -3.0),
            Vec3::new(3.0, 1.5, -3.0),
            Vec3::new(3.0, 1.5, 3.0),
            Vec3::new(-3.0, 1.5, 3.0),
        ];
        let vertices: Vec<Vec3> = ground.into_iter().chain(roof).collect();
        let triangles = vec![[0, 2, 1], [0, 3, 2], [4, 6, 5], [4, 7, 6]];
        let mut collider = TrackCollider {
            path: PathBuf::new(),
            vertices,
            triangles: triangles.clone(),
            surface_index: MeshSurfaceIndex::default(),
        };

        assert!(collider.overhead_blocked(Vec3::ZERO, 2.2, 2.0));
        assert!(!collider.overhead_blocked(Vec3::new(20.0, 0.0, 0.0), 2.2, 2.0));

        // Raise the same roof above the clearance cylinder.
        for index in 4..8 {
            collider.vertices[index].y = 5.0;
        }
        assert!(!collider.overhead_blocked(Vec3::ZERO, 2.2, 2.0));
    }

    #[test]
    fn vertical_walls_are_not_surface_candidates() {
        let triangles = [ColliderTriangle {
            vertices: [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(5.0, 0.0, 0.0),
            ],
            surface: SurfaceId::CONCRETE,
        }];
        let index = MeshSurfaceIndex::new(&triangles);
        assert!(index.is_empty());
        assert_eq!(index.skipped(), 1);
    }
}
