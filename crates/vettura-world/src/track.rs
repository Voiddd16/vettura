//! AC track module loading
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use glam::{Mat4, Vec3};
use vettura_acdata::ini::Ini;
use vettura_core::coords::ac_to_godot;
use vettura_kn5::{Kn5, Material, Mesh, Node, NodeKind, Vertex};
use vettura_kn5::model::transform_to_mat4;
use crate::surface::{SurfaceId, SurfaceKind, SurfaceProperties};
use crate::world::{WorldError, WorldResult, TrackObjectId};
