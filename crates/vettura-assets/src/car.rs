use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vettura_acdata::{AcdArchive, Ini, Lut};

use crate::error::{AssetError, Result};
use crate::physics_mapping;

/// Where a car's data files came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataSource {
    Acd(PathBuf),
    Directory(PathBuf),
}

/// All data files of a car, keyed by relative path (`engine.ini`, `tyres.ini`, ...).
#[derive(Debug, Clone)]
pub struct CarData {
    source: DataSource,
    files: BTreeMap<String, Vec<u8>>,
}

impl CarData {
    pub fn load(car_root: &Path) -> Result<Self> {
        let acd_path = car_root.join("data.acd");
        if acd_path.is_file() {
            let archive = AcdArchive::from_path(&acd_path)?;
            let files = archive
                .entries
                .into_iter()
                .map(|entry| (entry.name, entry.data))
                .collect();
            return Ok(Self {
                source: DataSource::Acd(acd_path),
                files,
            });
        }

        let data_dir = car_root.join("data");
        if data_dir.is_dir() {
            let mut files = BTreeMap::new();
            read_directory(&data_dir, &data_dir, &mut files)?;
            return Ok(Self {
                source: DataSource::Directory(data_dir),
                files,
            });
        }

        Err(AssetError::MissingData(car_root.to_owned()))
    }

    pub fn source(&self) -> &DataSource {
        &self.source
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        if let Some(bytes) = self.files.get(name) {
            return Some(bytes);
        }
        let normalized = name.replace('\\', "/");
        self.files.get(&normalized).map(Vec::as_slice)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn text(&self, name: &str) -> Result<&str> {
        let bytes = self
            .get(name)
            .ok_or_else(|| AssetError::MissingFile(name.to_owned()))?;
        std::str::from_utf8(bytes).map_err(|_| AssetError::NotText(name.to_owned()))
    }

    pub fn ini(&self, name: &str) -> Result<Ini> {
        Ok(Ini::parse(self.text(name)?))
    }

    pub fn lut(&self, name: &str) -> Result<Lut> {
        Ok(Lut::parse(self.text(name)?))
    }
}

fn read_directory(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            read_directory(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, std::fs::read(&path)?);
        }
    }
    Ok(())
}

/// A car folder on disk.
#[derive(Debug, Clone)]
pub struct CarFolder {
    root: PathBuf,
    data: CarData,
}

impl CarFolder {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_owned();
        let data = CarData::load(&root)?;
        Ok(Self { root, data })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn data(&self) -> &CarData {
        &self.data
    }

    pub fn load_kn5(&self, relative: &str) -> Result<vettura_kn5::Kn5> {
        Ok(vettura_kn5::Kn5::from_path(self.root.join(relative))?)
    }

    /// Map `car.ini` / `engine.ini` / `tyres.ini` onto the backend-neutral spec.
    ///
    /// This is intentionally conservative: values that are not understood yet
    /// fall back to `VehicleSpec::default()` and are logged at debug level.
    pub fn vehicle_spec(&self) -> Result<vettura_physics::VehicleSpec> {
        physics_mapping::spec_from_car(&self.root, &self.data)
    }
}
