//! Shared primitives for every Vettura crate.
//!
//! Keep this crate dependency-light. It defines the conventions that all
//! other crates agree on: units, coordinate frames, and the math library.

pub mod coords;
pub mod units;

pub use glam;
pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Version of the workspace, useful for diagnostics in the Godot extension.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
