//! 直接调用 FMOD Studio runtime 的音频引擎。
//!
//! 这里不做任何 bank / FSB 解析：bank 文件由 FMOD 自己加载，我们只通过
//! `sfx/GUIDs.txt` 得到 GUID，然后：
//!
//! 1. `System.load_bank_memory()` 把 `.bank` 字节交给 FMOD；
//! 2. `System.get_event_by_id()` 用 GUID 取事件；
//! 3. `EventDescription.create_instance()` / `EventInstance.start()` 播放；
//! 4. `System.update()` 每帧推进。
//!
//! FMOD 只负责音频，Godot 仍然负责场景、相机与 HUD。

#[cfg(feature = "fmod")]
use std::collections::HashMap;
use std::path::Path;

use crate::error::{AudioError, Result};
use crate::manifest::SoundManifest;

/// 三维向量（与 Godot/物理帧一致：`+X` 右、`+Y` 上、`-Z` 前）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AudioVec3 {
    pub const FORWARD: Self = Self::new(0.0, 0.0, -1.0);
    pub const UP: Self = Self::new(0.0, 1.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// 一个音频对象的 3D 属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioTransform {
    pub position: AudioVec3,
    pub velocity: AudioVec3,
    pub forward: AudioVec3,
    pub up: AudioVec3,
}

impl Default for AudioTransform {
    fn default() -> Self {
        Self {
            position: AudioVec3::new(0.0, 0.0, 0.0),
            velocity: AudioVec3::new(0.0, 0.0, 0.0),
            forward: AudioVec3::FORWARD,
            up: AudioVec3::UP,
        }
    }
}

/// FMOD Studio runtime 封装。
///
/// 未启用 `vettura-audio/fmod` feature 时仍然保留完整 API，但所有运行时操作都会
/// 返回 [`AudioError::NotLinked`]，这样主项目和测试可以在没有 FMOD SDK 的机器上
/// 编译。
#[derive(Debug)]
pub struct AudioEngine {
    #[cfg(feature = "fmod")]
    engine: Option<FmodEngine>,
}

impl AudioEngine {
    /// 启动 FMOD runtime，并设置 Godot 的右手坐标系。
    ///
    /// `max_channels <= 0` 时交给 FMOD 自己选默认值。
    pub fn start(max_channels: u32) -> Result<Self> {
        #[cfg(feature = "fmod")]
        {
            Ok(Self {
                engine: Some(FmodEngine::start(max_channels)?),
            })
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = max_channels;
            Err(AudioError::NotLinked)
        }
    }

    /// runtime 是否已经启动。
    pub fn is_ready(&self) -> bool {
        #[cfg(feature = "fmod")]
        {
            self.engine.is_some()
        }
        #[cfg(not(feature = "fmod"))]
        {
            false
        }
    }

    /// 当前加载的 bank 对应车 ID。
    pub fn car_id(&self) -> Option<&str> {
        #[cfg(feature = "fmod")]
        {
            self.engine.as_ref().and_then(|engine| engine.car_id())
        }
        #[cfg(not(feature = "fmod"))]
        {
            None
        }
    }

    /// 当前 manifest 里可用的事件名。
    pub fn event_names(&self) -> Vec<&str> {
        #[cfg(feature = "fmod")]
        {
            self.engine
                .as_ref()
                .map(|engine| engine.event_names())
                .unwrap_or_default()
        }
        #[cfg(not(feature = "fmod"))]
        {
            Vec::new()
        }
    }

    /// 加载一个车模的 `.bank`；随后可用 GUID 查事件。
    pub fn load_manifest(&mut self, manifest: &SoundManifest) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.load_manifest(manifest)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = manifest;
            Err(AudioError::NotLinked)
        }
    }

    /// 从车目录发现声音并立即加载。
    pub fn load_car(&mut self, car_root: impl AsRef<Path>) -> Result<Option<String>> {
        let manifest = SoundManifest::discover(car_root)?;
        match manifest {
            Some(manifest) => {
                self.load_manifest(&manifest)?;
                Ok(Some(manifest.car_id().to_owned()))
            }
            None => Ok(None),
        }
    }

    /// 循环播放一个事件（适合引擎、轮胎、风噪这类持续声）。
    pub fn play(&mut self, event: &str) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.play(event)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 一次性播放事件实例，播放完自动回收（适合碰撞、换挡、回火）。
    pub fn play_once(&mut self, event: &str) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.play_once(event)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 停止并释放 [`AudioEngine::play`] 创建的循环事件。
    pub fn stop(&mut self, event: &str) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.stop(event)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 设置事件参数，例如 `rpm` / `load` / `surface`。
    pub fn set_parameter(&mut self, event: &str, name: &str, value: f32) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_parameter(event, name, value)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = (event, name, value);
            Err(AudioError::NotLinked)
        }
    }

    /// 设置一个事件的 3D 属性。
    pub fn set_event_transform(&mut self, event: &str, transform: AudioTransform) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_event_transform(event, transform)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = (event, transform);
            Err(AudioError::NotLinked)
        }
    }

    /// 设置听者（通常传 Godot 里的相机或驾驶位）。
    pub fn set_listener(&mut self, transform: AudioTransform) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_listener(transform)
        }
        #[cfg(not(feature = "fmod"))]
        {
            let _ = transform;
            Err(AudioError::NotLinked)
        }
    }

    /// 每帧推进 FMOD；Godot `_process` / `_physics_process` 调用一次。
    pub fn update(&mut self) -> Result<()> {
        #[cfg(feature = "fmod")]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.update()
        }
        #[cfg(not(feature = "fmod"))]
        {
            Ok(())
        }
    }
}

/// 真正的 FMOD 实现：只在 `fmod` feature 下编译。
#[cfg(feature = "fmod")]
#[derive(Debug)]
struct FmodEngine {
    system: fmod::studio::System,
    bank: Option<fmod::studio::Bank>,
    manifest: Option<SoundManifest>,
    /// 命名事件实例：循环播放时保持句柄，便于 stop / set_parameter。
    instances: HashMap<String, fmod::studio::EventInstance>,
}

#[cfg(feature = "fmod")]
impl FmodEngine {
    fn start(max_channels: u32) -> Result<Self> {
        let max_channels = i32::try_from(max_channels).unwrap_or(0);
        let system = unsafe { fmod::studio::SystemBuilder::new() }
            .map_err(fmod_error)?
            .build(
                max_channels,
                fmod::studio::InitFlags::NORMAL,
                // Godot、Vettura 物理帧都是右手系；FMOD 默认左手系。
                fmod::InitFlags::NORMAL | fmod::InitFlags::RIGHTHANDED_3D,
            )
            .map_err(fmod_error)?;

        Ok(Self {
            system,
            bank: None,
            manifest: None,
            instances: HashMap::new(),
        })
    }

    fn car_id(&self) -> Option<&str> {
        self.manifest.as_ref().map(SoundManifest::car_id)
    }

    fn event_names(&self) -> Vec<&str> {
        self.manifest
            .as_ref()
            .map(SoundManifest::event_names)
            .unwrap_or_default()
    }

    fn load_manifest(&mut self, manifest: &SoundManifest) -> Result<()> {
        // FMOD copies the buffer when using `load_bank_memory`, so we can drop
        // the bytes as soon as this call returns.
        let bank_bytes = std::fs::read(manifest.bank_path())?;
        let bank = self
            .system
            .load_bank_memory(&bank_bytes, fmod::studio::LoadBankFlags::NORMAL)
            .map_err(fmod_error)?;
        bank.load_sample_data().map_err(fmod_error)?;

        // Clear old event handles first: their bank is about to disappear.
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

    fn play(&mut self, event: &str) -> Result<()> {
        let description = self.description(event)?;
        let instance = description.create_instance().map_err(fmod_error)?;

        if let Some(previous) = self.instances.insert(event.to_owned(), instance) {
            let _ = previous.stop(fmod::studio::StopMode::AllowFadeout);
            let _ = previous.release();
        }

        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance.start().map_err(fmod_error)
    }

    fn play_once(&mut self, event: &str) -> Result<()> {
        let description = self.description(event)?;
        let instance = description.create_instance().map_err(fmod_error)?;
        instance.start().map_err(fmod_error)?;
        instance.release().map_err(fmod_error)
    }

    fn stop(&mut self, event: &str) -> Result<()> {
        let Some(instance) = self.instances.remove(event) else {
            return Ok(());
        };
        instance
            .stop(fmod::studio::StopMode::AllowFadeout)
            .map_err(fmod_error)?;
        instance.release().map_err(fmod_error)
    }

    fn set_parameter(&mut self, event: &str, name: &str, value: f32) -> Result<()> {
        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        let name = fmod::Utf8CString::new(name)
            .map_err(|_| AudioError::Fmod("parameter name contains a NUL byte".to_owned()))?;
        instance
            .set_parameter_by_name(&name, value, true)
            .map_err(fmod_error)
    }

    fn set_event_transform(&mut self, event: &str, transform: AudioTransform) -> Result<()> {
        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance
            .set_3d_attributes(to_fmod_attributes(transform))
            .map_err(fmod_error)
    }

    fn set_listener(&mut self, transform: AudioTransform) -> Result<()> {
        self.system
            .set_listener_attributes(0, to_fmod_attributes(transform), None)
            .map_err(fmod_error)
    }

    fn update(&mut self) -> Result<()> {
        self.system.update().map_err(fmod_error)
    }

    /// GUID → FMOD `EventDescription`。
    fn description(&self, event: &str) -> Result<fmod::studio::EventDescription> {
        let manifest = self
            .manifest
            .as_ref()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        let entry = manifest
            .events()
            .iter()
            .find(|entry| entry.name == event)
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        let guid = entry.fmod_guid()?;
        self.system.get_event_by_id(guid).map_err(fmod_error)
    }
}

#[cfg(feature = "fmod")]
impl Drop for FmodEngine {
    fn drop(&mut self) {
        for event in self.instances.keys().cloned().collect::<Vec<_>>() {
            let _ = self.stop(&event);
        }
        if let Some(bank) = self.bank.take() {
            let _ = bank.unload();
        }
        // FMOD invalidates every handle when the system is released.
        unsafe {
            let _ = self.system.release();
        }
    }
}

#[cfg(feature = "fmod")]
fn to_fmod_attributes(transform: AudioTransform) -> fmod::Attributes3D {
    fmod::Attributes3D {
        position: to_fmod_vector(transform.position),
        velocity: to_fmod_vector(transform.velocity),
        forward: to_fmod_vector(transform.forward),
        up: to_fmod_vector(transform.up),
    }
}

#[cfg(feature = "fmod")]
fn to_fmod_vector(vector: AudioVec3) -> fmod::Vector {
    fmod::Vector {
        x: vector.x,
        y: vector.y,
        z: vector.z,
    }
}

#[cfg(feature = "fmod")]
fn fmod_error<E: std::fmt::Display>(error: E) -> AudioError {
    AudioError::Fmod(error.to_string())
}
