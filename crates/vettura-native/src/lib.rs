// `#[derive(GodotClass)]` generates `base: base` style initializers that this
// lint flags at the field declaration site.
#![allow(clippy::redundant_field_names)]

use godot::prelude::*;

mod ac_car;
mod kn5_file;
mod vehicle_body;

struct VetturaNative;

#[gdextension]
unsafe impl ExtensionLibrary for VetturaNative {}
