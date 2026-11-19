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
mod engine;
#[cfg(fmod_sdk_found)]
mod plugins;

use std::path::Path;

#[cfg(fmod_sdk_found)]
use engine::FmodEngine;

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
