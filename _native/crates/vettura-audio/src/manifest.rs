//! AC 车模声音清单：解析 `GUIDs.txt`，定位 `<car_id>.bank`。
//!
//! 这里**不解析 bank 内容**——FMOD runtime 会自己加载 bank；我们只负责回答：
//!
//! 1. 这个车有没有 `sfx/`；
//! 2. bank 文件在哪、叫什么；
//! 3. 它有哪些事件名和 GUID，方便 `AudioEngine` 直接用 GUID 查事件。

use std::path::{Path, PathBuf};

use crate::error::{AudioError, Result};
use crate::guids::{SoundEvent, car_id_from_root, parse_events_for};

/// AC 每个车模的声音目录。
pub const SFX_DIR: &str = "sfx";
/// GUID → event 映射表。
pub const GUIDS_NAME: &str = "GUIDs.txt";
/// 不参与“自动挑选 bank”检测的公共 bank。
pub const COMMON_BANK: &str = "common.bank";

/// 一台车的声音清单。
#[derive(Debug, Clone)]
pub struct SoundManifest {
    root: PathBuf,
    car_id: String,
    bank_path: PathBuf,
    events: Vec<SoundEvent>,
}

impl SoundManifest {
    /// 从车目录发现声音；**没有声音的车返回 `Ok(None)`**。
    ///
    /// `GUIDs.txt` 缺失不会失败：FMOD 仍然可以 `get_event`，只是主项目拿不到
    /// `sound_event_names()` 这类辅助信息。
    pub fn discover(root: impl AsRef<Path>) -> Result<Option<Self>> {
        let root = root.as_ref().to_path_buf();
        let sfx_dir = root.join(SFX_DIR);
        if !sfx_dir.is_dir() {
            return Ok(None);
        }

        let car_id = car_id_from_root(&root).ok_or_else(|| {
            AudioError::MissingFolder(format!("{}/{}", root.display(), SFX_DIR).into())
        })?;

        // `sfx/<id>.bank` decides the final car id: the directory name is only a
        // hint, and a fallback single bank can legitimately rename it.
        let (bank_path, bank_car_id) = find_bank(&sfx_dir, &car_id)?;
        let car_id = if car_id_dir_matches_bank(&root, &bank_car_id) {
            car_id
        } else {
            bank_car_id
        };

        let events = match std::fs::read_to_string(sfx_dir.join(GUIDS_NAME)) {
            Ok(text) => parse_events_for(&text, &car_id)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err.into()),
        };

        Ok(Some(Self {
            root,
            car_id,
            bank_path,
            events,
        }))
    }

    /// 车目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 车 ID；同时是 `sfx/<car_id>.bank` 的文件名主干。
    pub fn car_id(&self) -> &str {
        &self.car_id
    }

    /// 交给 FMOD 加载的 bank 文件路径。
    pub fn bank_path(&self) -> &Path {
        &self.bank_path
    }

    /// `GUIDs.txt` 里属于这台车的事件。
    pub fn events(&self) -> &[SoundEvent] {
        &self.events
    }

    /// 事件名列表。
    pub fn event_names(&self) -> Vec<&str> {
        self.events
            .iter()
            .map(|event| event.name.as_str())
            .collect()
    }

    /// 按事件名查 GUID。
    pub fn event_guid(&self, event: &str) -> Option<&str> {
        self.events
            .iter()
            .find(|candidate| candidate.name == event)
            .map(|event| event.guid.as_str())
    }

    /// 是否存在这个事件。
    pub fn has_event(&self, event: &str) -> bool {
        self.event_guid(event).is_some()
    }
}

/// 定位车自己的 `.bank`。
///
/// 规则与 Content Manager 一致：优先 `<car_id>.bank`，否则只接受 `sfx/` 下**恰好
/// 一个**非 `common.bank` 的 `.bank`；多个候选时拒绝猜。
fn find_bank(sfx_dir: &Path, car_id: &str) -> Result<(PathBuf, String)> {
    let preferred = sfx_dir.join(format!("{car_id}.bank"));
    if preferred.is_file() {
        return Ok((preferred, car_id.to_owned()));
    }

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(sfx_dir)
        .map_err(|_| AudioError::MissingFolder(sfx_dir.to_path_buf()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.ends_with(".bank")
                            && name != COMMON_BANK
                            && name.matches('.').count() == 1
                    })
        })
        .collect();
    candidates.sort();

    match candidates.as_slice() {
        [] => Err(AudioError::MissingBank(sfx_dir.to_path_buf())),
        [single] => {
            let bank_car_id = single
                .file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| !stem.is_empty())
                .ok_or_else(|| AudioError::MissingBank(single.clone()))?
                .to_owned();
            Ok((single.clone(), bank_car_id))
        }
        _ => Err(AudioError::AmbiguousBank(
            sfx_dir.to_path_buf(),
            candidates
                .iter()
                .filter_map(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                })
                .collect(),
        )),
    }
}

/// 规则：车目录名优先；只有当 bank 名能反推出更合理的车 ID 时才替换。
///
/// 这样 `sfx/<id>.bank` 按车目录命名时不会为了一个不存在的同名 bank 把事件过滤
/// 条件改坏；单个 fallback bank 也能让 `GUIDs.txt` 里的车事件仍然匹配上。
fn car_id_dir_matches_bank(root: &Path, bank_car_id: &str) -> bool {
    car_id_from_root(root).as_deref() == Some(bank_car_id)
}

#[cfg(test)]
mod tests {
    use super::{COMMON_BANK, SFX_DIR, SoundManifest};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vettura-audio-{tag}-{}-{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create fixture dirs");
            }
            fs::write(&path, bytes).expect("write fixture file");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_bank_and_events_for_the_car() {
        let dir = TempDir::new("discover");
        dir.write(
            &format!("{SFX_DIR}/GUIDs.txt"),
            "{79aba212-4247-42af-b172-d4388280680f} event:/cars/cim_2008_ferrari/engine_ext\n\
             {a1d9afa6-0a0c-4ff0-b82b-43edeba0bf8f} event:/cars/other/skid_ext\n",
        );
        dir.write(&format!("{SFX_DIR}/cim_2008_ferrari.bank"), b"binary-bytes");

        let manifest = SoundManifest::discover(dir.path())
            .expect("discover should work")
            .expect("car should have sound");
        assert_eq!(manifest.car_id(), "cim_2008_ferrari");
        assert!(manifest.bank_path().ends_with("cim_2008_ferrari.bank"));
        assert_eq!(manifest.event_names(), vec!["engine_ext"]);
        assert_eq!(
            manifest.event_guid("engine_ext"),
            Some("{79aba212-4247-42af-b172-d4388280680f}")
        );
        assert!(manifest.has_event("engine_ext"));
        assert!(!manifest.has_event("skid_ext"));
    }

    #[test]
    fn multiple_candidate_banks_are_rejected() {
        let dir = TempDir::new("ambiguous");
        dir.write(&format!("{SFX_DIR}/a.bank"), b"");
        dir.write(&format!("{SFX_DIR}/b.bank"), b"");

        let error = SoundManifest::discover(dir.path()).expect_err("must reject");
        assert!(matches!(error, super::AudioError::AmbiguousBank(_, _)));
    }

    #[test]
    fn missing_sfx_is_none_not_an_error() {
        let dir = TempDir::new("missing");
        assert!(SoundManifest::discover(dir.path()).unwrap().is_none());
    }

    #[test]
    fn common_bank_is_not_treated_as_a_car_bank() {
        let dir = TempDir::new("common");
        dir.write(&format!("{SFX_DIR}/{COMMON_BANK}"), b"");

        let error = SoundManifest::discover(dir.path()).expect_err("must reject");
        assert!(matches!(error, super::AudioError::MissingBank(_)));
    }
}
