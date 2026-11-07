//! 直接调用 FMOD Studio runtime 的音频引擎。
//!
//! 这里不做任何 bank / FSB 解析：bank 文件由 FMOD 自己加载，我们只通过
//! `sfx/GUIDs.txt` 找到事件 GUID，然后：
//!
//! 1. `Studio::create()` / `initialize()` 启动 FMOD Engine 2.02.22；
//! 2. `Studio::load_bank_memory()` 把 `.bank` 字节交给 FMOD；
//! 3. `Studio::get_event()` 用事件路径或 GUID 取 `EventDescription`；
//! 4. `EventDescription::create_instance()` / `EventInstance::start()` 播放；
//! 5. `Studio::update()` 每帧推进。
//!
//! 兼容规则（见 `docs/audio.md`）：2.02.22 runtime 只保证加载 2.02.x 编出来的
//! bank；2.03 / 2.04 编出来的 bank 不保证能读。
//!
//! 链接方式：动态链接，SDK 二进制不入库；本地 SDK 由主工程 `scripts/fetch_fmod.py`
//! 拉到 `.cache/fmod/`，build.rs 只做定位与 `-l` 链接。

#[cfg(fmod_sdk_found)]
use std::collections::HashMap;
#[cfg(fmod_sdk_found)]
use std::env;
use std::path::Path;
#[cfg(fmod_sdk_found)]
use std::path::PathBuf;

#[cfg(fmod_sdk_found)]
use libfmod::{Attributes3d, EventInstance, Init, LoadBank, StopMode, Studio, StudioInit, Vector};

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
/// 找不到本地 FMOD SDK（build.rs 输出 `fmod_sdk_missing`）时保留完整 API，但所有
/// 运行时操作都会返回 [`AudioError::NotLinked`]，这样没有 SDK 的贡献者也能编译。
#[derive(Debug)]
pub struct AudioEngine {
    #[cfg(fmod_sdk_found)]
    engine: Option<FmodEngine>,
}

/// 每帧推进给 FMOD 的车辆声浪参数。
///
/// 不同 mod 的 FMOD 事件参数名不一致；`AudioEngine` 会用候选名探测，缺失的参数
/// 直接跳过，不会让整个音频子系统报错。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleParams {
    pub rpm: f32,
    pub throttle: f32,
    pub load: f32,
    pub speed: f32,
    pub view_blend: f32,
}

impl Default for VehicleParams {
    fn default() -> Self {
        Self {
            rpm: 0.0,
            throttle: 0.0,
            load: 0.0,
            speed: 0.0,
            view_blend: 0.0,
        }
    }
}

impl AudioEngine {
    /// 启动 FMOD runtime，并把坐标系设为 Godot 的右手系。
    ///
    /// `max_channels <= 0` 时交给 FMOD 默认值。
    pub fn start(max_channels: u32) -> Result<Self> {
        #[cfg(fmod_sdk_found)]
        {
            Ok(Self {
                engine: Some(FmodEngine::start(max_channels)?),
            })
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = max_channels;
            Err(AudioError::NotLinked)
        }
    }

    /// runtime 是否已经启动。
    pub fn is_ready(&self) -> bool {
        #[cfg(fmod_sdk_found)]
        {
            self.engine.is_some()
        }
        #[cfg(not(fmod_sdk_found))]
        {
            false
        }
    }

    /// 当前加载的 bank 对应车 ID。
    pub fn car_id(&self) -> Option<&str> {
        #[cfg(fmod_sdk_found)]
        {
            self.engine.as_ref().and_then(|engine| engine.car_id())
        }
        #[cfg(not(fmod_sdk_found))]
        {
            None
        }
    }

    /// 当前 manifest 里可用的事件名。
    pub fn event_names(&self) -> Vec<&str> {
        #[cfg(fmod_sdk_found)]
        {
            self.engine
                .as_ref()
                .map(|engine| engine.event_names())
                .unwrap_or_default()
        }
        #[cfg(not(fmod_sdk_found))]
        {
            Vec::new()
        }
    }

    /// 加载一个车模的 `.bank`；随后可用事件名查事件。
    pub fn load_manifest(&mut self, manifest: &SoundManifest) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.load_manifest(manifest)
        }
        #[cfg(not(fmod_sdk_found))]
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
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.play(event)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 一次性播放事件实例，播放完自动回收（适合碰撞、换挡、回火）。
    pub fn play_once(&mut self, event: &str) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.play_once(event)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 停止并释放 [`AudioEngine::play`] 创建的循环事件。
    pub fn stop(&mut self, event: &str) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.stop(event)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 设置事件参数，例如 `rpm` / `load` / `surface`。
    pub fn set_parameter(&mut self, event: &str, name: &str, value: f32) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_parameter(event, name, value)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = (event, name, value);
            Err(AudioError::NotLinked)
        }
    }

    /// 设置一个事件的 3D 属性。
    pub fn set_event_transform(&mut self, event: &str, transform: AudioTransform) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_event_transform(event, transform)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = (event, transform);
            Err(AudioError::NotLinked)
        }
    }

    /// 设置听者（通常传 Godot 里的相机或驾驶位）。
    pub fn set_listener(&mut self, transform: AudioTransform) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_listener(transform)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = transform;
            Err(AudioError::NotLinked)
        }
    }

    /// 每帧推进 FMOD；Godot `_process` / `_physics_process` 调用一次。
    pub fn update(&mut self) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.update()
        }
        #[cfg(not(fmod_sdk_found))]
        {
            Ok(())
        }
    }

    /// 查询一个事件的参数名；不同 mod 命名不统一，主项目/调试器需要它做适配。
    pub fn parameter_names(&self, event: &str) -> Result<Vec<String>> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_ref().ok_or(AudioError::NotLinked)?;
            engine.parameter_names(event)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = event;
            Err(AudioError::NotLinked)
        }
    }

    /// 用常见参数名适配不同 mod 的车辆声浪事件；不存在的参数会被跳过。
    pub fn set_vehicle_parameters(&mut self, event: &str, params: VehicleParams) -> Result<()> {
        #[cfg(fmod_sdk_found)]
        {
            let engine = self.engine.as_mut().ok_or(AudioError::NotLinked)?;
            engine.set_vehicle_parameters(event, params)
        }
        #[cfg(not(fmod_sdk_found))]
        {
            let _ = (event, params);
            Err(AudioError::NotLinked)
        }
    }
}

/// 真正的 FMOD 实现：只在 build.rs 找到本地 SDK 时编译。
#[cfg(fmod_sdk_found)]
#[derive(Debug)]
struct FmodEngine {
    studio: Studio,
    bank: Option<libfmod::Bank>,
    manifest: Option<SoundManifest>,
    plugins: Vec<u32>,
    param_cache: HashMap<String, ParamNames>,
    /// 命名事件实例：循环播放时保持句柄，便于 stop / set_parameter。
    instances: HashMap<String, EventInstance>,
}

#[cfg(fmod_sdk_found)]
#[derive(Debug, Clone, Default)]
struct ParamNames {
    rpm: Option<String>,
    throttle: Option<String>,
    load: Option<String>,
    speed: Option<String>,
    view: Option<String>,
}

#[cfg(fmod_sdk_found)]
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

#[cfg(fmod_sdk_found)]
impl FmodEngine {
    fn start(max_channels: u32) -> Result<Self> {
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
        // FMOD copies the buffer when using `load_bank_memory`，所以这里读完即可丢弃。
        let bank_bytes = std::fs::read(manifest.bank_path())?;
        let bank = self
            .studio
            .load_bank_memory(&bank_bytes, LoadBank::NORMAL)
            .map_err(fmod_error)?;
        bank.load_sample_data().map_err(fmod_error)?;

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

    fn play(&mut self, event: &str) -> Result<()> {
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
        instance.stop(StopMode::AllowFadeout).map_err(fmod_error)?;
        instance.release().map_err(fmod_error)
    }

    fn set_parameter(&mut self, event: &str, name: &str, value: f32) -> Result<()> {
        let instance = self
            .instances
            .get(event)
            .copied()
            .ok_or_else(|| AudioError::UnknownEvent(event.to_owned()))?;
        instance
            .set_parameter_by_name(name, value, false)
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
        self.studio
            .set_listener_attributes(0, to_fmod_attributes(transform), None)
            .map_err(fmod_error)
    }

    fn update(&mut self) -> Result<()> {
        self.studio.update().map_err(fmod_error)
    }

    fn parameter_names(&self, event: &str) -> Result<Vec<String>> {
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

    fn set_vehicle_parameters(&mut self, event: &str, params: VehicleParams) -> Result<()> {
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

#[cfg(fmod_sdk_found)]
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

/// 注册本地 runtime 目录里的 bank 插件（当前是 FMOD Distance Filter）。
///
/// SDK 里只给插件源码，不发布二进制；`scripts/fetch_fmod.py` 会把它编到
/// `.cache/fmod/runtime/<os>/`。找不到插件时不报错：老 bank 需要它，普通 bank 不需要。
#[cfg(fmod_sdk_found)]
fn load_bank_plugins(studio: &Studio) -> Result<Vec<u32>> {
    let Some(runtime) = runtime_dir() else {
        return Ok(vec![]);
    };
    let core = studio.get_core_system().map_err(fmod_error)?;
    let mut handles = vec![];

    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(&runtime)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                let name = name.to_ascii_lowercase();
                                name.contains("fmod") && name.contains("distance")
                            })
                })
                .collect()
        })
        .unwrap_or_default();
    candidates.sort();

    for plugin in candidates {
        let path = plugin.to_string_lossy().to_string();
        match core.load_plugin(&path, None) {
            Ok(handle) => {
                handles.push(handle);
            }
            Err(_) => {
                // 某些环境可能已注册同名插件；这里不阻断 bank 加载。
            }
        }
    }

    Ok(handles)
}

#[cfg(fmod_sdk_found)]
fn runtime_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("VETTURA_FMOD_RUNTIME_PATH"));
    dir.exists().then_some(dir)
}

#[cfg(fmod_sdk_found)]
fn to_fmod_attributes(transform: AudioTransform) -> Attributes3d {
    Attributes3d {
        position: to_fmod_vector(transform.position),
        velocity: to_fmod_vector(transform.velocity),
        forward: to_fmod_vector(transform.forward),
        up: to_fmod_vector(transform.up),
    }
}

#[cfg(fmod_sdk_found)]
fn to_fmod_vector(vector: AudioVec3) -> Vector {
    Vector {
        x: vector.x,
        y: vector.y,
        z: vector.z,
    }
}

#[cfg(fmod_sdk_found)]
fn fmod_error<E: std::fmt::Display>(error: E) -> AudioError {
    AudioError::Fmod(error.to_string())
}
