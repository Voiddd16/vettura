//! 真正的 FMOD 实现：[`FmodEngine`] 与 libfmod 类型转换。
//!
//! 整个模块只在 build.rs 找到本地 SDK（`fmod_sdk_found`）时编译；没有 SDK 时
//! [`super::AudioEngine`] 保留完整 API，但所有运行时操作返回 `NotLinked`。

use std::collections::HashMap;

use libfmod::{Attributes3d, EventInstance, Init, LoadBank, StopMode, Studio, StudioInit, Vector};

use super::plugins::load_bank_plugins;
use super::{AudioTransform, AudioVec3, VehicleParams};
use crate::error::{AudioError, Result};
use crate::manifest::SoundManifest;

/// 真正的 FMOD 实现：只在 build.rs 找到本地 SDK 时编译。
#[derive(Debug)]
pub(super) struct FmodEngine {
    studio: Studio,
    bank: Option<libfmod::Bank>,
    manifest: Option<SoundManifest>,
    plugins: Vec<u32>,
    param_cache: HashMap<String, ParamNames>,
    /// 命名事件实例：循环播放时保持句柄，便于 stop / set_parameter。
    instances: HashMap<String, EventInstance>,
}

#[derive(Debug, Clone, Default)]
struct ParamNames {
    rpm: Option<String>,
    throttle: Option<String>,
    load: Option<String>,
    speed: Option<String>,
    view: Option<String>,
}

impl ParamNames {
    const RPM_CANDIDATES: &'static [&'static str] = &["rpms", "rpm", "RPM", "engine_rpm"];
    const THROTTLE_CANDIDATES: &'static [&'static str] = &["throttle", "Throttle", "gas"];
    const LOAD_CANDIDATES: &'static [&'static str] = &["load", "Load", "engine_load"];
    const SPEED_CANDIDATES: &'static [&'static str] = &["speed", "Speed", "kmh"];
    const VIEW_CANDIDATES: &'static [&'static str] = &["view", "View", "camera"];

    fn probe(description: &libfmod::EventDescription) -> Self {
        fn first(description: &libfmod::EventDescription, candidates: &[&str]) -> Option<String> {
            let count = description.get_parameter_description_count().ok()?;
            for index in 0..count {
                let name = description
                    .get_parameter_description_by_index(index)
                    .ok()?
                    .name;
                if candidates.iter().any(|candidate| *candidate == name) {
                    return Some(name);
                }
            }
            None
        }

        Self {
            rpm: first(description, Self::RPM_CANDIDATES),
            throttle: first(description, Self::THROTTLE_CANDIDATES),
            load: first(description, Self::LOAD_CANDIDATES),
            speed: first(description, Self::SPEED_CANDIDATES),
            view: first(description, Self::VIEW_CANDIDATES),
        }
    }
}

impl FmodEngine {
    pub(super) fn start(max_channels: u32) -> Result<Self> {
        // 调试构建默认链接 fmodL，会把每次样本加载写入终端；先压掉日志再建系统。
        unsafe {
            libfmod::ffi::FMOD_Debug_Initialize(
                libfmod::ffi::FMOD_DEBUG_LEVEL_ERROR,
                libfmod::ffi::FMOD_DEBUG_MODE_TTY,
                None,
                std::ptr::null(),
            );
        }

        let max_channels = i32::try_from(max_channels).unwrap_or(0);
        let studio = Studio::create().map_err(fmod_error)?;
        // Godot、Vettura 物理帧都是右手系；FMOD 也支持右手 3D。
        studio
            .initialize(
                max_channels,
                StudioInit::NORMAL,
                Init::NORMAL | Init::RIGHTHANDED_3D,
                None,
            )
            .map_err(fmod_error)?;

        // 部分老 bank（AC/mod）会引用 FMOD Designer 时代的 DSP 插件；
        // 没有插件，load_bank 会返回 ERR_PLUGIN_MISSING。这里先注册插件再载 bank。
        let plugins = load_bank_plugins(&studio)?;

        Ok(Self {
            studio,
            bank: None,
            manifest: None,
            plugins,
            param_cache: HashMap::new(),
            instances: HashMap::new(),
        })
    }

    pub(super) fn car_id(&self) -> Option<&str> {
        self.manifest.as_ref().map(SoundManifest::car_id)
    }

    pub(super) fn event_names(&self) -> Vec<&str> {
        self.manifest
            .as_ref()
            .map(SoundManifest::event_names)
            .unwrap_or_default()
    }

    pub(super) fn load_manifest(&mut self, manifest: &SoundManifest) -> Result<()> {
        // FMOD copies the buffer when using `load_bank_memory`，所以这里读完即可丢弃。
        let bank_bytes = std::fs::read(manifest.bank_path())?;
        let bank = self
            .studio
            .load_bank_memory(&bank_bytes, LoadBank::NORMAL)
            .map_err(fmod_error)
            // bank 加载失败是"没声音但车还能跑"的降级点；在这里记一条，
            // 之后错误只是向上传播，别处不再重复记。
            .inspect_err(|err| {
                tracing::error!(
                    bank = %manifest.bank_path().display(),
                    "fmod: bank load failed: {err}"
                );
            })?;
        bank.load_sample_data()
            .map_err(fmod_error)
            .inspect_err(|err| {
                tracing::error!(
                    bank = %manifest.bank_path().display(),
                    "fmod: bank sample data load failed: {err}"
                );
            })?;

        // 先清旧事件句柄，再卸载旧 bank，避免 Use-After-Free。
        for event in self.instances.keys().cloned().collect::<Vec<_>>() {
            let _ = self.stop(&event);
        }
        if let Some(old_bank) = self.bank.take() {
            let _ = old_bank.unload();
        }

        self.bank = Some(bank);
        self.manifest = Some(manifest.clone());
        Ok(())
    }

    pub(super) fn play(&mut self, event: &str) -> Result<()> {
        let description = self.description(event)?;
        let instance = description.create_instance().map_err(fmod_error)?;

        if let Some(previous) = self.instances.insert(event.to_owned(), instance) {
            let _ = previous.stop(StopMode::AllowFadeout);
            let _ = previous.release();
        }

        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance.start().map_err(fmod_error)
    }

    pub(super) fn play_once(&mut self, event: &str) -> Result<()> {
        let description = self.description(event)?;
        let instance = description.create_instance().map_err(fmod_error)?;
        instance.start().map_err(fmod_error)?;
        instance.release().map_err(fmod_error)
    }

    pub(super) fn stop(&mut self, event: &str) -> Result<()> {
        let Some(instance) = self.instances.remove(event) else {
            return Ok(());
        };
        instance.stop(StopMode::AllowFadeout).map_err(fmod_error)?;
        instance.release().map_err(fmod_error)
    }

    pub(super) fn set_parameter(&mut self, event: &str, name: &str, value: f32) -> Result<()> {
        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance
            .set_parameter_by_name(name, value, false)
            .map_err(fmod_error)
    }

    pub(super) fn set_event_transform(
        &mut self,
        event: &str,
        transform: AudioTransform,
    ) -> Result<()> {
        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance
            .set_3d_attributes(to_fmod_attributes(transform))
            .map_err(fmod_error)
    }

    pub(super) fn set_listener(&mut self, transform: AudioTransform) -> Result<()> {
        self.studio
            .set_listener_attributes(0, to_fmod_attributes(transform), None)
            .map_err(fmod_error)
    }

    pub(super) fn update(&mut self) -> Result<()> {
        self.studio.update().map_err(fmod_error)
    }

    pub(super) fn parameter_names(&self, event: &str) -> Result<Vec<String>> {
        let description = self.description(event)?;
        let count = description
            .get_parameter_description_count()
            .map_err(fmod_error)?;
        let mut names = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            names.push(
                description
                    .get_parameter_description_by_index(index)
                    .map_err(fmod_error)?
                    .name,
            );
        }
        Ok(names)
    }

    pub(super) fn set_vehicle_parameters(
        &mut self,
        event: &str,
        params: VehicleParams,
    ) -> Result<()> {
        let aliases = match self.param_cache.get(event) {
            Some(aliases) => aliases.clone(),
            None => {
                let description = self.description(event)?;
                let aliases = ParamNames::probe(&description);
                self.param_cache.insert(event.to_owned(), aliases.clone());
                aliases
            }
        };

        let Some(instance) = self.instances.get(event).copied() else {
            return Ok(());
        };
        for (name, value) in [
            (aliases.rpm.as_ref(), params.rpm),
            (aliases.throttle.as_ref(), params.throttle),
            (aliases.load.as_ref(), params.load),
            (aliases.speed.as_ref(), params.speed),
            (aliases.view.as_ref(), params.view_blend),
        ] {
            if let Some(name) = name {
                let _ = instance.set_parameter_by_name(name, value, false);
            }
        }
        Ok(())
    }

    /// 事件名 → FMOD `EventDescription`。
    ///
    /// 用 `GUIDs.txt` 里的 GUID 而不是事件路径：不同 mod 会用相同事件路径，
    /// GUID 才是 AC 内容自己的命名空间；FMOD 的 `get_event` 同时接受两者。
    fn description(&self, event: &str) -> Result<libfmod::EventDescription> {
        let manifest = self
            .manifest
            .as_ref()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        let entry = manifest
            .events()
            .iter()
            .find(|entry| entry.name == event)
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        self.studio.get_event(&entry.guid).map_err(fmod_error)
    }
}

impl Drop for FmodEngine {
    fn drop(&mut self) {
        for event in self.instances.keys().cloned().collect::<Vec<_>>() {
            let _ = self.stop(&event);
        }
        if let Some(bank) = self.bank.take() {
            let _ = bank.unload();
        }
        // 先卸载 bank，再卸载 bank 引用的 DSP 插件。
        if let Ok(core) = self.studio.get_core_system() {
            for handle in std::mem::take(&mut self.plugins) {
                let _ = core.unload_plugin(handle);
            }
        }
        // FMOD invalidates every handle when the system is released.
        let _ = self.studio.release();
    }
}

fn to_fmod_attributes(transform: AudioTransform) -> Attributes3d {
    Attributes3d {
        position: to_fmod_vector(transform.position),
        velocity: to_fmod_vector(transform.velocity),
        forward: to_fmod_vector(transform.forward),
        up: to_fmod_vector(transform.up),
    }
}

fn to_fmod_vector(vector: AudioVec3) -> Vector {
    Vector {
        x: vector.x,
        y: vector.y,
        z: vector.z,
    }
}

pub(super) fn fmod_error<E: std::fmt::Display>(error: E) -> AudioError {
    AudioError::Fmod(error.to_string())
}
