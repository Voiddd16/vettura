//! Assetto Corsa car data formats.
//!
//! * [`acd`] — decrypt and unpack the `data.acd` container.
//! * [`ini`] — permissive INI reader for `car.ini`, `engine.ini`, ...
//! * [`lut`] — `x|y` lookup tables (`power.lut`, torque curves, ...).
//!
//! The ACD key is derived from the car folder name, so files cannot be
//! decrypted after being moved out of their original folder. [`AcdArchive::from_path`]
//! takes care of that automatically.

pub mod acd;
pub mod ini;
pub mod lut;

pub use acd::{AcdArchive, AcdEntry, derive_key};
pub use ini::{Ini, IniEntry, IniSection};
pub use lut::{Lut, LutPoint};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid ACD container: {0}")]
    InvalidAcd(String),

    #[error("ACD key must not be empty")]
    EmptyKey,
}
