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
        self.files
            .get(name)
            .or_else(|| self.files.get(&normalize_key(name)))
            .map(Vec::as_slice)
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

/// Normalize a lookup key so that Windows-style backslash paths can be found
/// in the forward-slash map produced by [`read_directory`].
///
/// The in-memory map always stores relative paths with `/` separators; the
/// caller may still pass a Windows path (e.g. from an INI list), so we try the
/// original key first and only fall back to this normalized form.
fn normalize_key(name: &str) -> String {
    name.replace('\\', "/")
}

/// Whether `path` is an existing regular file with a `.kn5` extension
/// (case-insensitive, so `body.KN5` also counts).
fn is_kn5_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kn5"))
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

    /// 主模型 `.kn5` 的路径（渲染用车模，不含 `collider.kn5`）。
    ///
    /// 规则与 AcTools 的 `AcPaths.GetMainCarFilename`（Content Manager）一致：
    ///
    /// 1. `lods.ini` 的 `[LOD_HR] FILE` 或 `[LOD_0] FILE` 优先；
    /// 2. 否则取车目录下体积最大的 `*.kn5`——`collider.kn5` 与 `*_lodb.kn5`
    ///    通常远小于完整车模，所以按体积取最大值刚好挑中主模型。
    ///
    /// `lods.ini` 条目存在但不指向一个真实的 `*.kn5` 文件时（指向别的扩展名、
    /// 文件缺失）继续走第 2 步，让被写坏的 mod 仍能显示。
    ///
    /// 找不到任何 `.kn5` 时返回 `None`（例如只剩 collider 的残包）。
    ///
    /// 参见：<https://github.com/ac-custom-shaders/actools/blob/master/src/Paths.cs>
    pub fn main_model_path(&self) -> Option<PathBuf> {
        if let Ok(lods) = self.data.ini("lods.ini") {
            for (section, key) in [("LOD_HR", "FILE"), ("LOD_0", "FILE")] {
                if let Some(file) = lods
                    .get(section, key)
                    .map(str::trim)
                    .filter(|file| !file.is_empty())
                {
                    let candidate = self.root.join(file);
                    if is_kn5_file(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }

        std::fs::read_dir(&self.root)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("kn5"))
            })
            // 读不到 metadata 的文件（坏符号链接、权限问题）直接跳过，
            // 而不是当成 0 字节参与比较——否则它会顶掉真正的最小模型。
            .filter_map(|path| {
                let len = std::fs::metadata(&path).ok()?.len();
                Some((path, len))
            })
            .max_by_key(|(_, len)| *len)
            .map(|(path, _)| path)
    }

    /// Map `car.ini` / `engine.ini` / `tyres.ini` onto the backend-neutral spec.
    ///
    /// This is intentionally conservative: values that are not understood yet
    /// fall back to `VehicleSpec::default()` and are logged at debug level.
    pub fn vehicle_spec(&self) -> Result<vettura_physics::VehicleSpec> {
        physics_mapping::spec_from_car(&self.root, &self.data)
    }
}
