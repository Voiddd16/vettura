//! Godot 节点：把 FMOD Studio runtime 接到主项目。
//!
//! 职责边界：
//!
//! - `vettura-audio`：解析 `sfx/GUIDs.txt`、定位 bank、直接调 FMOD；
//! - `FmodSystem`：把这个 API 变成 Godot 可调用的节点，负责每帧 `update`；
//! - 主项目 GDScript：决定什么时候播放 / 停止 / 传参数。

use std::fmt;
use std::path::PathBuf;

use godot::classes::{INode3D, Node3D};
use godot::prelude::*;
use vettura_audio::{AudioEngine, AudioTransform, AudioVec3, SoundManifest};

use crate::ac_car::AcCar;
use crate::paths::native_path;

/// 每帧推进 FMOD，并提供播放 / 参数 / 3D 听者接口。
#[derive(GodotClass)]
#[class(base = Node3D, init)]
pub struct FmodSystem {
    base: Base<Node3D>,
    engine: Option<AudioEngine>,
    manifest: Option<SoundManifest>,
    last_error: GString,
}

impl FmodSystem {
    /// 记录错误并统一返回 `false`，让 GDScript 侧调用点简洁。
    fn fail(&mut self, message: impl fmt::Display) -> bool {
        let message = message.to_string();
        self.last_error = GString::from(message.as_str());
        godot_error!("FmodSystem: {}", message);
        false
    }
}

#[godot_api]
impl INode3D for FmodSystem {
    /// 每帧推进 FMOD Studio；不依赖 Godot 物理帧。
    /// WAV 录制模式下改由 GDScript 在物理帧里显式调用 `step_audio()`，
    /// 让混音进度跟随仿真时间而不是墙钟时间。
    fn process(&mut self, _delta: f64) {
        if std::env::var_os("VETTURA_AUDIO_WAV").is_some() {
            return;
        }
        if let Some(engine) = self.engine.as_mut()
            && let Err(err) = engine.update()
        {
            self.last_error = GString::from(err.to_string().as_str());
            godot_error!("FmodSystem update failed: {}", err);
        }
    }
}

#[godot_api]
impl FmodSystem {
    /// WAV 录制模式下由物理帧调用一次，按仿真时间推进 FMOD 混音。
    #[func]
    pub fn step_audio(&mut self) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        match engine.update() {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }

    /// 启动 FMOD runtime。`max_channels <= 0` 表示用 FMOD 默认值。
    ///
    /// 未用 `--features fmod` 编译时返回 `false`，可在 `last_error()` 看到原因。
    #[func]
    pub fn start(&mut self, max_channels: i64) -> bool {
        let channels = u32::try_from(max_channels).unwrap_or(0);
        match AudioEngine::start(channels) {
            Ok(engine) => {
                self.engine = Some(engine);
                true
            }
            Err(err) => self.fail(err),
        }
    }

    /// FMOD runtime 是否可用。
    #[func]
    pub fn is_ready(&self) -> bool {
        self.engine.is_some()
    }

    /// 最近一次错误信息（空字符串表示没有错误）。
    #[func]
    pub fn last_error(&self) -> GString {
        self.last_error.clone()
    }

    /// 加载 `AcCar` 对应车模的 `.bank`；没有声音时返回 `false`。
    #[func]
    pub fn load_car(&mut self, car: Gd<AcCar>) -> bool {
        let root = car.bind().car().map(|folder| folder.root().to_path_buf());
        match root {
            Some(root) => self.load_car_root(&root),
            None => self.fail("AcCar is not open"),
        }
    }

    /// 直接加载车目录（例如 `res://cars/ferrari`）。
    #[func]
    pub fn load_car_path(&mut self, car_path: GString) -> bool {
        let path = native_path(&car_path);
        self.load_car_root(&PathBuf::from(path))
    }

    /// 当前 bank 对应车 ID。
    #[func]
    pub fn car_id(&self) -> GString {
        self.manifest
            .as_ref()
            .map(|manifest| GString::from(manifest.car_id()))
            .unwrap_or_default()
    }

    /// 当前 bank 路径；未加载时为空字符串。
    #[func]
    pub fn soundbank_path(&self) -> GString {
        self.manifest
            .as_ref()
            .map(|manifest| GString::from(manifest.bank_path().to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    /// `GUIDs.txt` 里可用事件数量。
    #[func]
    pub fn event_count(&self) -> i64 {
        self.manifest
            .as_ref()
            .map_or(0, |manifest| manifest.events().len() as i64)
    }

    /// 当前 manifest 里可用的事件名。
    #[func]
    pub fn event_names(&self) -> PackedStringArray {
        self.manifest
            .as_ref()
            .map(|manifest| {
                manifest
                    .event_names()
                    .into_iter()
                    .map(GString::from)
                    .collect::<PackedStringArray>()
            })
            .unwrap_or_default()
    }

    /// 第 `index` 个事件名。
    #[func]
    pub fn event_name(&self, index: i64) -> GString {
        self.manifest
            .as_ref()
            .and_then(|manifest| manifest.events().get(index.max(0) as usize))
            .map(|event| GString::from(event.name.as_str()))
            .unwrap_or_default()
    }

    /// 第 `index` 个事件的 GUID。
    #[func]
    pub fn event_guid(&self, index: i64) -> GString {
        self.manifest
            .as_ref()
            .and_then(|manifest| manifest.events().get(index.max(0) as usize))
            .map(|event| GString::from(event.guid.as_str()))
            .unwrap_or_default()
    }

    /// 查询事件的参数名；不同 mod 命名不统一，主项目可用它做适配。
    #[func]
    pub fn parameter_names(&self, event: GString) -> PackedStringArray {
        let Some(engine) = self.engine.as_ref() else {
            return PackedStringArray::new();
        };
        let event = event.to_string();
        match engine.parameter_names(&event) {
            Ok(names) => names
                .into_iter()
                .map(|name| GString::from(name.as_str()))
                .collect::<PackedStringArray>(),
            Err(_) => PackedStringArray::new(),
        }
    }

    /// 用常见参数名适配车辆声浪事件；不存在的参数会被跳过。
    ///
    /// `gear` / `slip` 用于 FSB fallback 的换挡声与打滑声；`slip` 传四个轮子里
    /// 最大的侧偏角（弧度）即可。
    #[allow(clippy::too_many_arguments)]
    #[func]
    pub fn set_vehicle_parameters(
        &mut self,
        event: GString,
        rpm: f32,
        throttle: f32,
        load: f32,
        speed: f32,
        view_blend: f32,
        gear: i64,
        slip: f32,
    ) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        let event = event.to_string();
        let params = vettura_audio::VehicleParams {
            rpm,
            throttle,
            load,
            speed,
            view_blend,
            gear: gear as i32,
            slip,
        };
        match engine.set_vehicle_parameters(&event, params) {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }

    /// 当前 FSB fallback 选中的采样；Studio 事件静音时这是唯一发声路径。
    #[func]
    pub fn fallback_summary(&self) -> GString {
        self.engine
            .as_ref()
            .and_then(|engine| engine.fallback_summary())
            .map_or_else(GString::new, |summary| GString::from(summary.as_str()))
    }

    /// 按事件名查 GUID。
    #[func]
    pub fn event_guid_by_name(&self, event: GString) -> GString {
        self.manifest
            .as_ref()
            .and_then(|manifest| manifest.event_guid(event.to_string().as_str()))
            .map(GString::from)
            .unwrap_or_default()
    }

    /// 循环播放事件（引擎、轮胎、风噪等持续声）。
    #[func]
    pub fn play(&mut self, event: GString) -> bool {
        self.play_sound(event.to_string().as_str(), false)
    }

    /// 一次性播放事件并自动回收（碰撞、换挡、回火）。
    #[func]
    pub fn play_once(&mut self, event: GString) -> bool {
        self.play_sound(event.to_string().as_str(), true)
    }

    /// 停止 `play()` 启动的循环事件。
    #[func]
    pub fn stop(&mut self, event: GString) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        match engine.stop(event.to_string().as_str()) {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }

    /// 设置事件参数，例如 `rpm`、`load`、`surface`。
    #[func]
    pub fn set_parameter(&mut self, event: GString, parameter: GString, value: f32) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        match engine.set_parameter(
            event.to_string().as_str(),
            parameter.to_string().as_str(),
            value,
        ) {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }

    /// 设置事件 3D 属性（位置 / 速度 / 前向 / 上向）。
    #[func]
    pub fn set_event_transform(
        &mut self,
        event: GString,
        position: Vector3,
        velocity: Vector3,
        forward: Vector3,
        up: Vector3,
    ) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        let transform = audio_transform(position, velocity, forward, up);
        match engine.set_event_transform(event.to_string().as_str(), transform) {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }

    /// 设置听者；通常传相机或驾驶位变换。
    #[func]
    pub fn set_listener(
        &mut self,
        position: Vector3,
        velocity: Vector3,
        forward: Vector3,
        up: Vector3,
    ) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        let transform = audio_transform(position, velocity, forward, up);
        match engine.set_listener(transform) {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }
}

impl FmodSystem {
    /// 发现 bank / 事件表，并交给 FMOD 加载。
    fn load_car_root(&mut self, root: &std::path::Path) -> bool {
        let manifest = match SoundManifest::discover(root) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => return self.fail(format!("no car sound under {}", root.display())),
            Err(err) => return self.fail(err),
        };

        let loaded = match self.engine.as_mut() {
            Some(engine) => match engine.load_manifest(&manifest) {
                Ok(()) => true,
                Err(err) => return self.fail(err),
            },
            // Keep the manifest even if FMOD is not linked, so the main project can
            // still inspect event names and diagnose the missing runtime.
            None => false,
        };

        self.manifest = Some(manifest);
        if loaded {
            true
        } else {
            self.fail("FMOD runtime is not ready; call start() first")
        }
    }

    fn play_sound(&mut self, event: &str, once: bool) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return self.fail("FMOD runtime is not ready");
        };
        let result = if once {
            engine.play_once(event)
        } else {
            engine.play(event)
        };
        match result {
            Ok(()) => true,
            Err(err) => self.fail(err),
        }
    }
}

/// Godot Vector3 → FMOD 帧（右手系，前向 `-Z`，直接一一对应）。
fn audio_vec3(value: Vector3) -> AudioVec3 {
    AudioVec3::new(value.x, value.y, value.z)
}

fn audio_transform(
    position: Vector3,
    velocity: Vector3,
    forward: Vector3,
    up: Vector3,
) -> AudioTransform {
    AudioTransform {
        position: audio_vec3(position),
        velocity: audio_vec3(velocity),
        forward: audio_vec3(forward),
        up: audio_vec3(up),
    }
}
