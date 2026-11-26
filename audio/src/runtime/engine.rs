//! 真正的 FMOD 实现：[`FmodEngine`] 与 libfmod 类型转换。
//!
//! 整个模块只在 build.rs 找到本地 SDK（`fmod_sdk_found`）时编译；没有 SDK 时
//! [`super::AudioEngine`] 保留完整 API，但所有运行时操作返回 `NotLinked`。

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_void;

use libfmod::{
    Attributes3d, Channel, EventInstance, Init, LoadBank, Mode, OutputType, Sound, StopMode,
    Studio, StudioInit, Vector,
};

use super::plugins::load_bank_plugins;
use super::roles::{
    SamplePosition, SampleRole, SampleView, band_rpm, classify, reference_rpm, sample_position,
    sample_view,
};
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
    /// 命名事件实例：循环播放时保持句柄，便于 stop / set_parameter。
    instances: HashMap<String, EventInstance>,
    /// WAV 输出模式下保持文件名存活（FMOD 在 initialize 时读取这块内存）。
    _wav_path: Option<CString>,
    /// WAV 模式下用 bank 内嵌 FSB 的真实采样驱动引擎声（Studio 事件缺 bus 时）。
    fallback: Option<FallbackAudio>,
}

/// 直接从 AC bank 内嵌 FSB 里取出的真实车声采样。
///
/// Studio 事件引用的 bus 不在 bank 里时（AC 老 mod 没有 master bank），事件本身
/// 完全静音；这层用 Core API 直接播放 bank 内嵌 FSB 的采样，是真正能听到的
/// 引擎 / 风 / 胎 / 打滑 / 换挡声源。
#[derive(Debug)]
struct FallbackAudio {
    /// FSB 容器：子采样是容器的句柄，容器必须先于它们存活。
    _containers: Vec<Sound>,
    engine_idle: Vec<LoopSample>,
    engine_on: Vec<LoopSample>,
    engine_off: Vec<LoopSample>,
    wind: Option<LoopSample>,
    tyre: Option<LoopSample>,
    skid: Option<LoopSample>,
    turbo: Option<LoopSample>,
    /// 换挡一次性采样：只收 `gearup*` / `geardn*` 这类短音效。
    /// `upsh*` / `dwsh*` / `transmission_*` 是几十秒的循环层，不能当一次性播。
    gear_up: Vec<OneShot>,
    gear_down: Vec<OneShot>,
    backfire: Vec<OneShot>,
    /// TC 介入声是循环采样（不是一次性音效），按打滑量调音量即可。
    traction: Option<LoopSample>,
    /// 变速箱啸叫循环：按 `transmission_4th` 这类名字里的挡位匹配当前挡位，
    /// 只在小音量下播放，避免盖过引擎。
    transmission: Vec<(i32, LoopSample)>,
    /// 一次性采样的轮转索引，避免连续换挡都播同一个 backfire。
    next_one_shot: usize,
    last_gear: Option<i32>,
    /// 换挡音效冷却帧数，避免挡位在 0/目标挡之间抖动时连播多次。
    shift_cooldown: u32,
    last_throttle: f32,
    /// 平滑后的 rpm。物理起步时 rpm 可能从 0 直接跳到红线，采样 pitch
    /// 跟着瞬变会发出很尖的“啾”声；这里做一阶低通。
    last_rpm: Option<f32>,
}

/// 一次性音效（换挡、回火）。保留视角信息，车外只选车外录音，避免
/// `gearup` 与 `gearupEXT` 这类同角色不同麦克风的采样轮流触发。
#[derive(Debug, Clone, Copy)]
struct OneShot {
    sound: Sound,
    view: SampleView,
}

/// 一个循环播放的采样。
#[derive(Debug, Clone)]
struct LoopSample {
    name: String,
    /// 子采样句柄：容器已经持有它，这里只是防止未来改动时误释放。
    _sound: Sound,
    channel: Channel,
    /// 采样录制时的转速，用来把车辆 rpm 映射成播放速率。
    reference_rpm: f32,
    view: SampleView,
    /// 车头 / 车尾麦克风位置。车外视角只选其中一侧，避免前后采样同时叠加
    /// 产生梳状滤波和尖锐感。
    position: SamplePosition,
}

/// 从 FSB 里收集到的一个采样。
#[derive(Debug, Clone)]
struct FsbSample {
    name: String,
    sound: Sound,
    view: SampleView,
    reference_rpm: Option<f32>,
    /// 采样时长（毫秒）。分类时用它区分"循环层"和"一次性音效"，
    /// 避免把几十秒的 transmission/upsh 长循环当成换挡短音效。
    duration_ms: u32,
}

struct CollectedSamples {
    containers: Vec<Sound>,
    samples: Vec<FsbSample>,
}

impl Drop for FallbackAudio {
    fn drop(&mut self) {
        for sample in self
            .engine_idle
            .iter()
            .chain(&self.engine_on)
            .chain(&self.engine_off)
            .chain(self.wind.iter())
            .chain(self.tyre.iter())
            .chain(self.skid.iter())
            .chain(self.turbo.iter())
            .chain(self.traction.iter())
            .chain(self.transmission.iter().map(|(_, sample)| sample))
        {
            let _ = sample.channel.stop();
        }
    }
}

impl FallbackAudio {
    /// 每帧把车辆状态映射到各层采样。
    ///
    /// 关键点：Core API 的 `Channel::set_frequency` 单位是 **Hz**，不是倍率；
    /// 这里必须用 `set_pitch`（1.0 = 原速），否则引擎声会被压到 1–3 Hz 的
    /// 次声波，听感上就是"没有引擎声"。
    fn update(&mut self, core: &libfmod::System, event: &str, params: VehicleParams) -> Result<()> {
        let in_cockpit = event.to_ascii_lowercase().contains("_int");
        let rpm_target = params.rpm.max(0.0);
        let rpm = match self.last_rpm {
            Some(previous) => {
                let alpha = if rpm_target > previous { 0.35 } else { 0.20 };
                previous + (rpm_target - previous) * alpha
            }
            None => rpm_target,
        };
        self.last_rpm = Some(rpm);
        let throttle = params.throttle.clamp(0.0, 1.0);
        let speed_norm = (params.speed.max(0.0) / 250.0).clamp(0.0, 1.0);

        let idle_indices = Self::usable_indices(&self.engine_idle, in_cockpit);
        let idle_index = idle_indices.first().copied();
        let idle_ref = idle_index
            .and_then(|index| self.engine_idle.get(index))
            .map_or(1000.0, |sample| sample.reference_rpm);
        let rpm_norm = ((rpm - idle_ref) / (9000.0 - idle_ref).max(1.0)).clamp(0.0, 1.0);

        for (index, sample) in self.engine_idle.iter().enumerate() {
            let volume = if idle_index == Some(index) {
                // 高转时 idle 必须完全淡出：它只有一个录制转速，继续拉高
                // pitch 会变成尖锐的啸叫，盖过真正的 on/off 采样。
                let pitch = (rpm / sample.reference_rpm.max(1.0)).clamp(0.8, 1.20);
                sample.channel.set_pitch(pitch).map_err(fmod_error)?;
                0.50 * (1.0 - 0.45 * throttle) * (1.0 - rpm_norm).powf(0.8)
            } else {
                0.0
            };
            sample.channel.set_volume(volume).map_err(fmod_error)?;
        }

        // 引擎 on/off 层按 rpm 只交叉淡化相邻的两个采样。以前把所有落在
        // 三角权重里的采样同时叠加，多份不同录音相加会产生梳状滤波和明显的
        // 尖锐感（Porsche 没有转速标注时尤其严重，5 条采样会一起响）。
        self.mix_engine_layer(
            &self.engine_on,
            in_cockpit,
            rpm,
            0.60 * throttle * (0.35 + 0.65 * rpm_norm),
        )?;
        self.mix_engine_layer(
            &self.engine_off,
            in_cockpit,
            rpm,
            0.45 * (1.0 - throttle) * (0.20 + 0.80 * rpm_norm),
        )?;

        if let Some(wind) = &self.wind {
            wind.channel
                .set_pitch(0.9 + 0.3 * speed_norm)
                .map_err(fmod_error)?;
            wind.channel
                .set_volume(0.32 * speed_norm.powf(1.4))
                .map_err(fmod_error)?;
        }
        if let Some(tyre) = &self.tyre {
            tyre.channel
                .set_pitch(0.85 + 0.4 * speed_norm)
                .map_err(fmod_error)?;
            tyre.channel
                .set_volume(0.28 * speed_norm)
                .map_err(fmod_error)?;
        }
        if let Some(skid) = &self.skid {
            let slip = ((params.slip - 0.05) / 0.30).clamp(0.0, 1.0);
            skid.channel.set_volume(0.70 * slip).map_err(fmod_error)?;
        }
        if let Some(turbo) = &self.turbo {
            turbo
                .channel
                .set_volume(0.30 * throttle * rpm_norm)
                .map_err(fmod_error)?;
        }

        // 变速箱啸叫：只播当前挡位对应的 `transmission_Nth` 循环，音量远低于
        // 引擎（10%），pitch 固定 1.0。它本身能量集中在 1–6 kHz，跟着 rpm 拉
        // pitch 会立刻变成刺耳的高频啸叫。
        for (gear, sample) in &self.transmission {
            let volume = if *gear == params.gear {
                0.08 * (0.25 + 0.75 * speed_norm)
            } else {
                0.0
            };
            sample.channel.set_pitch(1.0).map_err(fmod_error)?;
            sample.channel.set_volume(volume).map_err(fmod_error)?;
        }

        // 换挡：挡位变化时播一次对应方向的一次性采样（Porsche 等新 bank 有
        // gearup/geardn；F1 2008 的 bank 没有换挡采样，只能留空）。
        // 注意不能把 `upsh*` / `dwsh*` 当换挡声：它们是 30–90 秒的循环层，
        // 以前每次换挡都会启动一条几十秒的长采样，听起来像变速箱一直在响。
        self.shift_cooldown = self.shift_cooldown.saturating_sub(1);
        if let Some(previous) = self.last_gear
            && params.gear != previous
            && previous != 0
            && params.gear != 0
            && self.shift_cooldown == 0
        {
            let upshift = params.gear > previous;
            // 音色完全来自 mod 自己的 `gearup*` / `geardn*` 采样；没有这类
            // 采样就不出声，不用程序化声音去“补”。pitch 只做统一的小幅转速
            // 映射（升挡略高、降挡略低），不改变采样本身的音色。
            let shift_rpm = (rpm / 9000.0).clamp(0.0, 1.0);
            let shift_pitch = if upshift {
                0.96 + 0.08 * shift_rpm
            } else {
                0.88 + 0.06 * shift_rpm
            };
            let sounds = if upshift {
                self.gear_up.clone()
            } else {
                self.gear_down.clone()
            };
            self.play_one_shot(core, &sounds, 0.55, shift_pitch, in_cockpit)?;
            self.shift_cooldown = 10;
        }
        self.last_gear = Some(params.gear);

        // 松油门时的高转回火：AC bank 里 backfire 采样通常很多，轮转播放。
        if self.last_throttle > 0.35 && throttle < 0.10 && rpm > 5000.0 {
            let sounds = self.backfire.clone();
            self.play_one_shot(core, &sounds, 0.45, 1.0, in_cockpit)?;
        }
        self.last_throttle = throttle;

        // 牵引力控制：AC 的 `traction_control` 是循环采样，按打滑量调音量。
        // 之前逐帧 play_sound 会在 60 Hz 下堆出成百上千个重叠 channel。
        if let Some(traction) = &self.traction {
            let slip = ((params.slip - 0.20) / 0.35).clamp(0.0, 1.0);
            traction
                .channel
                .set_volume(0.35 * slip * throttle)
                .map_err(fmod_error)?;
        }

        Ok(())
    }

    /// 当前视角可用的采样索引，按 `reference_rpm` 已排序。
    ///
    /// 车外视角优先车尾（排气侧）采样；同时有车头/车尾两套录音时，把车头
    /// 那条排除掉，避免两个麦克风位置的声音叠在一起。
    fn usable_indices(layer: &[LoopSample], in_cockpit: bool) -> Vec<usize> {
        let view_ok = |sample: &LoopSample| sample.view.matches(in_cockpit);
        let prefer_rear = !in_cockpit
            && layer
                .iter()
                .any(|sample| view_ok(sample) && sample.position == SamplePosition::Rear);
        layer
            .iter()
            .enumerate()
            .filter(|(_, sample)| {
                view_ok(sample) && (!prefer_rear || sample.position != SamplePosition::Front)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn mix_engine_layer(
        &self,
        layer: &[LoopSample],
        in_cockpit: bool,
        rpm: f32,
        volume: f32,
    ) -> Result<()> {
        let usable = Self::usable_indices(layer, in_cockpit);
        let mut selection: Vec<(usize, f32)> = Vec::with_capacity(2);
        if let (Some(first), Some(last)) = (usable.first().copied(), usable.last().copied()) {
            match (
                rpm <= layer[first].reference_rpm,
                rpm >= layer[last].reference_rpm,
            ) {
                (true, _) => selection.push((first, 1.0)),
                (false, true) => selection.push((last, 1.0)),
                (false, false) => {
                    for pair in usable.windows(2) {
                        let (a, b) = (pair[0], pair[1]);
                        let (low, high) = (layer[a].reference_rpm, layer[b].reference_rpm);
                        if rpm >= low && rpm <= high {
                            let t = ((rpm - low) / (high - low).max(1.0)).clamp(0.0, 1.0);
                            selection.push((a, 1.0 - t));
                            selection.push((b, t));
                            break;
                        }
                    }
                    if selection.is_empty() {
                        selection.push((last, 1.0));
                    }
                }
            }
        }

        for (index, sample) in layer.iter().enumerate() {
            let share = selection
                .iter()
                .find(|(selected, _)| *selected == index)
                .map_or(0.0, |(_, weight)| *weight);
            // 单个采样的变调限制在 ±5 个半音左右；以前允许 0.5–2.5 倍，
            // 高转时会把采样拉成刺耳的高频啸叫。
            let pitch = (rpm / sample.reference_rpm.max(1.0)).clamp(0.75, 1.20);
            sample.channel.set_pitch(pitch).map_err(fmod_error)?;
            sample
                .channel
                .set_volume(volume * share)
                .map_err(fmod_error)?;
        }
        Ok(())
    }

    /// 播放一次性音效，并优先选与当前视角一致的录音。
    ///
    /// 同一个角色在 bank 里常有 `gearup`（通用）和 `gearupEXT`（车外）两条；
    /// 以前共用一个轮转索引，换挡声会在两条不同麦克风的录音之间来回跳。现在
    /// 车外只选 `EXT`，驾驶位优先 `INT`，都没有才退回通用/任意采样。
    fn play_one_shot(
        &mut self,
        core: &libfmod::System,
        sounds: &[OneShot],
        volume: f32,
        pitch: f32,
        in_cockpit: bool,
    ) -> Result<()> {
        if sounds.is_empty() {
            return Ok(());
        }
        let wanted = if in_cockpit {
            SampleView::Int
        } else {
            SampleView::Ext
        };
        let specific: Vec<&OneShot> = sounds.iter().filter(|sound| sound.view == wanted).collect();
        let both: Vec<&OneShot> = sounds
            .iter()
            .filter(|sound| sound.view == SampleView::Both)
            .collect();
        let pool = match (!specific.is_empty(), !both.is_empty()) {
            (true, _) => specific,
            (false, true) => both,
            (false, false) => sounds.iter().collect(),
        };
        let index = self.next_one_shot % pool.len();
        self.next_one_shot = self.next_one_shot.wrapping_add(1);
        let channel = core
            .play_sound(pool[index].sound, None, false)
            .map_err(fmod_error)?;
        channel.set_pitch(pitch).map_err(fmod_error)?;
        channel.set_volume(volume).map_err(fmod_error)?;
        Ok(())
    }

    fn summary(&self) -> String {
        fn names(samples: &[LoopSample]) -> String {
            samples
                .iter()
                .map(|sample| sample.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
        fn one(sample: &Option<LoopSample>) -> String {
            sample
                .as_ref()
                .map_or_else(|| "-".to_owned(), |sample| sample.name.clone())
        }

        let transmission = self
            .transmission
            .iter()
            .map(|(gear, sample)| format!("{gear}:{}", sample.name))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "idle=[{}] on=[{}] off=[{}] wind={} tyre={} skid={} turbo={} traction={} transmission=[{}] gear_up={} gear_down={} backfire={}",
            names(&self.engine_idle),
            names(&self.engine_on),
            names(&self.engine_off),
            one(&self.wind),
            one(&self.tyre),
            one(&self.skid),
            one(&self.turbo),
            one(&self.traction),
            transmission,
            self.gear_up.len(),
            self.gear_down.len(),
            self.backfire.len(),
        )
    }
}

/// AC 各 mod 引擎事件的参数名不统一，运行期用这些候选名逐个尝试。
///
/// 不要用 `EventDescription::get_parameter_description_*` 去"探测"：实测这些
/// 调用会让 `EventInstance` 变成 invalid handle（连之后新建的实例也受影响）。
struct ParamCandidates;

impl ParamCandidates {
    const RPM: &'static [&'static str] = &["rpms", "rpm", "RPM", "engine_rpm"];
    const THROTTLE: &'static [&'static str] = &["throttle", "Throttle", "gas"];
    const LOAD: &'static [&'static str] = &["load", "Load", "engine_load"];
    const SPEED: &'static [&'static str] = &["speed", "Speed", "kmh"];
    const VIEW: &'static [&'static str] = &["view", "View", "camera"];
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

        // 可选：把 FMOD 混音输出直接写成 WAV（录视频时音画分开采集，最后混流）。
        // VETTURA_AUDIO_WAV 未设置时完全不影响正常播放路径。
        let wav_path = std::env::var("VETTURA_AUDIO_WAV")
            .ok()
            .filter(|path| !path.is_empty())
            .and_then(|path| CString::new(path).ok());
        let wav_ptr = wav_path.as_ref().map(|path| path.as_ptr() as *mut c_void);
        if wav_ptr.is_some() {
            let core = studio.get_core_system().map_err(fmod_error)?;
            core.set_output(OutputType::WavWriterNrt)
                .map_err(fmod_error)?;
        }

        // Godot、Vettura 物理帧都是右手系；FMOD 也支持右手 3D。
        studio
            .initialize(
                max_channels,
                StudioInit::NORMAL,
                Init::NORMAL | Init::RIGHTHANDED_3D,
                wav_ptr,
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
            instances: HashMap::new(),
            _wav_path: wav_path,
            fallback: None,
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

        // AC 老 bank 的事件引用的 bus 在 bank 外，Studio 事件实际静音（FMOD 会
        // 打 "Bus not found"）。所有播放模式都从 bank 内嵌的 FSB5 取真实采样，
        // 用 Core API 按车辆参数驱动——这才是玩家真正听到的引擎声 / 风噪 / 胎噪。
        // WAV 录制只是把同一份混音写到文件，不是另一套声源。
        if let Err(err) = self.setup_fallback(&bank_bytes) {
            eprintln!("[vettura-audio] FSB fallback unavailable: {err:?}");
        } else {
            eprintln!("[vettura-audio] FSB fallback active");
        }
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
        instance.start().map_err(fmod_error)?;
        Ok(())
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
        // 逐字段填充后直接走 FFI，绕开 libfmod 的封装层。
        let mut attributes: libfmod::ffi::FMOD_3D_ATTRIBUTES = unsafe { std::mem::zeroed() };
        attributes.position.x = transform.position.x;
        attributes.position.y = transform.position.y;
        attributes.position.z = transform.position.z;
        attributes.velocity.x = transform.velocity.x;
        attributes.velocity.y = transform.velocity.y;
        attributes.velocity.z = transform.velocity.z;
        attributes.forward.x = transform.forward.x;
        attributes.forward.y = transform.forward.y;
        attributes.forward.z = transform.forward.z;
        attributes.up.x = transform.up.x;
        attributes.up.y = transform.up.y;
        attributes.up.z = transform.up.z;
        let result = unsafe {
            libfmod::ffi::FMOD_Studio_EventInstance_Set3DAttributes(
                instance.as_mut_ptr(),
                &mut attributes,
            )
        };
        if result != 0 {
            return Err(fmod_error(format!(
                "FMOD_Studio_EventInstance_Set3DAttributes: {result}"
            )));
        }
        Ok(())
    }

    pub(super) fn set_listener(&mut self, transform: AudioTransform) -> Result<()> {
        self.studio
            .set_listener_attributes(0, to_fmod_attributes(transform), None)
            .map_err(fmod_error)
    }

    pub(super) fn update(&mut self) -> Result<()> {
        // `Studio::System::update` 内部已经推进 low-level system；Core API 直接
        // 播放的 fallback channel 也走同一个 system。再调一次 `System::update`
        // 会把同一个混音缓冲推进两次（实测表现为声音断续 / 卡顿）。
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

    /// 从 bank 内嵌的所有 FSB5 块里取真实采样，用 Core API 循环播放。
    fn setup_fallback(&mut self, bank_bytes: &[u8]) -> Result<()> {
        let core = self.studio.get_core_system().map_err(fmod_error)?;
        let CollectedSamples {
            containers,
            samples,
        } = collect_fsb_samples(&core, bank_bytes)?;
        if samples.is_empty() {
            return Err(AudioError::UnknownEvent("FSB5 samples".to_owned()));
        }

        // 采样名里能解析出录制转速的（`6halfK` / `12halfK` / `14Kish`）优先；
        // 解析不出的老 mod 用最低 on/off 参考转速倒推怠速。
        let min_engine_ref = samples
            .iter()
            .filter(|sample| {
                matches!(
                    classify(&sample.name, sample.duration_ms),
                    SampleRole::EngineOn | SampleRole::EngineOff
                )
            })
            .filter_map(|sample| sample.reference_rpm)
            .reduce(f32::min);
        let idle_ref = samples
            .iter()
            .filter(|sample| classify(&sample.name, sample.duration_ms) == SampleRole::Idle)
            .find_map(|sample| sample.reference_rpm)
            .or_else(|| min_engine_ref.map(|rpm| rpm * 0.6))
            .unwrap_or(1000.0);

        // 没有转速标注的 on/off 采样按数量均分到怠速..9000 rpm，保证交叉淡化
        // 覆盖整个转速区间。
        let unlabeled = |role: SampleRole| {
            samples
                .iter()
                .filter(|sample| {
                    classify(&sample.name, sample.duration_ms) == role
                        && sample
                            .reference_rpm
                            .or_else(|| band_rpm(&sample.name.to_ascii_lowercase(), idle_ref))
                            .is_none()
                })
                .count()
        };
        let unlabeled_on_total = unlabeled(SampleRole::EngineOn);
        let unlabeled_off_total = unlabeled(SampleRole::EngineOff);
        let mut unlabeled_on_index = 0;
        let mut unlabeled_off_index = 0;

        let mut engine_idle = Vec::new();
        let mut engine_on = Vec::new();
        let mut engine_off = Vec::new();
        let mut wind_candidates = Vec::new();
        let mut tyre_candidates = Vec::new();
        let mut skid_candidates = Vec::new();
        let mut turbo_candidates = Vec::new();
        let mut gear_up = Vec::new();
        let mut gear_down = Vec::new();
        let mut backfire = Vec::new();
        let mut traction_candidates = Vec::new();
        let mut transmission_candidates = Vec::new();

        for sample in &samples {
            let name = sample.name.to_ascii_lowercase();
            match classify(&sample.name, sample.duration_ms) {
                SampleRole::Idle => {
                    engine_idle.push(loop_sample(&core, sample, idle_ref)?);
                }
                SampleRole::EngineOn => {
                    let reference = match sample.reference_rpm.or_else(|| band_rpm(&name, idle_ref))
                    {
                        Some(reference) => reference,
                        None => {
                            let reference = synthetic_reference(
                                idle_ref,
                                unlabeled_on_index,
                                unlabeled_on_total,
                                9000.0,
                            );
                            unlabeled_on_index += 1;
                            reference
                        }
                    };
                    engine_on.push(loop_sample(&core, sample, reference)?);
                }
                SampleRole::EngineOff => {
                    let reference = match sample.reference_rpm.or_else(|| band_rpm(&name, idle_ref))
                    {
                        Some(reference) => reference,
                        None => {
                            let reference = synthetic_reference(
                                idle_ref,
                                unlabeled_off_index,
                                unlabeled_off_total,
                                7500.0,
                            );
                            unlabeled_off_index += 1;
                            reference
                        }
                    };
                    engine_off.push(loop_sample(&core, sample, reference)?);
                }
                SampleRole::Wind => wind_candidates.push(sample),
                SampleRole::Tyre => tyre_candidates.push(sample),
                SampleRole::Skid => skid_candidates.push(sample),
                SampleRole::Turbo => turbo_candidates.push(sample),
                SampleRole::GearUp => gear_up.push(OneShot {
                    sound: sample.sound,
                    view: sample.view,
                }),
                SampleRole::GearDown => gear_down.push(OneShot {
                    sound: sample.sound,
                    view: sample.view,
                }),
                SampleRole::Backfire => backfire.push(OneShot {
                    sound: sample.sound,
                    view: sample.view,
                }),
                SampleRole::Traction => traction_candidates.push(sample),
                SampleRole::Transmission(gear) => {
                    transmission_candidates.push((gear, sample));
                }
                SampleRole::Ignored => {}
            }
        }

        // `mix_engine_layer` 依赖按录制转速排序来选相邻采样。
        engine_idle.sort_by(|a, b| a.reference_rpm.total_cmp(&b.reference_rpm));
        engine_on.sort_by(|a, b| a.reference_rpm.total_cmp(&b.reference_rpm));
        engine_off.sort_by(|a, b| a.reference_rpm.total_cmp(&b.reference_rpm));

        if engine_idle.is_empty() && engine_on.is_empty() && engine_off.is_empty() {
            return Err(AudioError::UnknownEvent("engine samples in FSB".to_owned()));
        }

        let wind = pick(&wind_candidates, &["wind"])
            .map(|sample| loop_sample(&core, sample, 1.0))
            .transpose()?;
        let tyre = pick(&tyre_candidates, &["tyre_rolling", "tire_rolling"])
            .map(|sample| loop_sample(&core, sample, 1.0))
            .transpose()?;
        let skid = pick(&skid_candidates, &["Skid", "skid"])
            .map(|sample| loop_sample(&core, sample, 1.0))
            .transpose()?;
        let turbo = pick(&turbo_candidates, &["turbo"])
            .map(|sample| loop_sample(&core, sample, 1.0))
            .transpose()?;
        let traction = pick(&traction_candidates, &["traction_control"])
            .map(|sample| loop_sample(&core, sample, 1.0))
            .transpose()?;
        let mut transmission = Vec::new();
        for (gear, sample) in transmission_candidates {
            transmission.push((gear, loop_sample(&core, sample, 1.0)?));
        }

        let fallback = FallbackAudio {
            _containers: containers,
            engine_idle,
            engine_on,
            engine_off,
            wind,
            tyre,
            skid,
            turbo,
            gear_up,
            gear_down,
            backfire,
            traction,
            transmission,
            next_one_shot: 0,
            last_gear: None,
            shift_cooldown: 0,
            last_throttle: 0.0,
            last_rpm: None,
        };
        eprintln!("[vettura-audio] FSB fallback: {}", fallback.summary());
        self.fallback = Some(fallback);
        Ok(())
    }

    /// 用车辆参数驱动 FSB 采样。
    fn update_fallback(&mut self, event: &str, params: VehicleParams) {
        let core = match self.studio.get_core_system() {
            Ok(core) => core,
            Err(err) => {
                eprintln!("[vettura-audio] fallback core unavailable: {err}");
                return;
            }
        };
        let Some(fallback) = self.fallback.as_mut() else {
            return;
        };
        if let Err(err) = fallback.update(&core, event, params) {
            eprintln!("[vettura-audio] fallback update failed: {err:?}");
        }
    }

    pub(super) fn fallback_summary(&self) -> Option<String> {
        self.fallback.as_ref().map(FallbackAudio::summary)
    }

    pub(super) fn set_vehicle_parameters(
        &mut self,
        event: &str,
        params: VehicleParams,
    ) -> Result<()> {
        self.update_fallback(event, params);
        let Some(instance) = self.instances.get(event).copied() else {
            return Ok(());
        };
        // 逐个候选名尝试，命中即停。不同 mod 的参数名不统一，但不能用
        // EventDescription::get_parameter_description_* 去探测：实测这些调用
        // 会让 EventInstance 变成 invalid handle（连之后新建的实例也受影响）。
        for (names, value) in [
            (ParamCandidates::RPM, params.rpm),
            (ParamCandidates::THROTTLE, params.throttle),
            (ParamCandidates::LOAD, params.load),
            (ParamCandidates::SPEED, params.speed),
            (ParamCandidates::VIEW, params.view_blend),
        ] {
            for name in names {
                if instance.set_parameter_by_name(name, value, false).is_ok() {
                    break;
                }
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
        // 先停 fallback 的循环 channel，再卸载 bank / release system。
        self.fallback = None;
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

/// 从候选里优先取精确名，否则取名字最短的（旁支采样通常名字更长）。
fn pick<'a>(candidates: &[&'a FsbSample], preferred: &[&str]) -> Option<&'a FsbSample> {
    preferred
        .iter()
        .find_map(|wanted| {
            candidates
                .iter()
                .find(|sample| sample.name.eq_ignore_ascii_case(wanted))
                .copied()
        })
        .or_else(|| {
            candidates
                .iter()
                .min_by_key(|sample| sample.name.len())
                .copied()
        })
}

/// 把一个 FSB 子采样变成循环 channel。
fn loop_sample(
    core: &libfmod::System,
    sample: &FsbSample,
    reference_rpm: f32,
) -> Result<LoopSample> {
    // `Channel::set_loop_count(-1)` 要求采样 mode 带 LOOP_NORMAL；FSB 里的
    // 采样默认不一定是循环点，先显式打开。
    sample
        .sound
        .set_mode(Mode::LOOP_NORMAL)
        .map_err(fmod_error)?;
    let channel = core
        .play_sound(sample.sound, None, true)
        .map_err(fmod_error)?;
    channel.set_loop_count(-1).map_err(fmod_error)?;
    channel.set_volume_ramp(true).map_err(fmod_error)?;
    channel.set_volume(0.0).map_err(fmod_error)?;
    channel.set_paused(false).map_err(fmod_error)?;
    Ok(LoopSample {
        name: sample.name.clone(),
        _sound: sample.sound,
        channel,
        reference_rpm: reference_rpm.max(1.0),
        view: sample.view,
        position: sample_position(&sample.name.to_ascii_lowercase()),
    })
}

/// 从 bank 里找出所有 FSB5 块，逐块创建容器并收集子采样。
///
/// AC bank 里经常不止一个 FSB5（F1 2008 的第二个块只放了一条
/// `f2008_int_off_hi`）；只取第一个会漏采样。
fn collect_fsb_samples(core: &libfmod::System, bank_bytes: &[u8]) -> Result<CollectedSamples> {
    let mut containers = Vec::new();
    let mut samples = Vec::new();
    for (index, chunk) in fsb_chunks(bank_bytes).into_iter().enumerate() {
        // libfmod 的 Core `create_sound` 只接受文件路径；FMOD 对 OPENMEMORY 的
        // FSB 容器要求 exinfo 显式给长度，写临时文件更稳。
        let temp = std::env::temp_dir().join(format!(
            "vettura_car_audio_{}_{index}.fsb",
            std::process::id()
        ));
        std::fs::write(&temp, chunk)?;
        let container = core
            .create_sound(temp.to_string_lossy().as_ref(), Mode::DEFAULT, None)
            .map_err(fmod_error)?;
        let count = container.get_num_sub_sounds().map_err(fmod_error)?;
        for sub_index in 0..count {
            let sound = container.get_sub_sound(sub_index).map_err(fmod_error)?;
            let name = sound.get_name(256).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            let duration_ms = sound
                .get_length(libfmod::ffi::FMOD_TIMEUNIT_MS)
                .unwrap_or(0);
            samples.push(FsbSample {
                view: sample_view(&lower),
                reference_rpm: reference_rpm(&lower),
                duration_ms,
                name,
                sound,
            });
        }
        containers.push(container);
    }

    // 同一辆车的 FSB 里可能有重名采样（例如 Porsche 的 `idle` 出现两次），
    // 保留第一个即可，避免创建重复 channel。
    let mut seen = std::collections::HashSet::new();
    samples.retain(|sample| seen.insert(sample.name.to_ascii_lowercase()));

    if !samples.is_empty() {
        eprintln!(
            "[vettura-audio] FSB samples ({}): {:?}",
            samples.len(),
            samples
                .iter()
                .map(|sample| sample.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    Ok(CollectedSamples {
        containers,
        samples,
    })
}

/// 按 FSB5 头里的长度切出 bank 内的所有 FSB5 块。
fn fsb_chunks(bank_bytes: &[u8]) -> Vec<&[u8]> {
    let mut chunks = Vec::new();
    let mut search = 0;
    while search + 4 <= bank_bytes.len() {
        let Some(relative) = bank_bytes[search..]
            .windows(4)
            .position(|window| window == b"FSB5")
        else {
            break;
        };
        let start = search + relative;
        if start + 0x30 <= bank_bytes.len() {
            let header = &bank_bytes[start..start + 0x30];
            let sample_headers = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
            let name_table = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
            let data = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
            let length = 0x30usize
                .saturating_add(sample_headers)
                .saturating_add(name_table)
                .saturating_add(data);
            if length > 0x30 && start + length <= bank_bytes.len() {
                chunks.push(&bank_bytes[start..start + length]);
                search = start + length;
                continue;
            }
        }
        search = start + 4;
    }
    chunks
}

/// 没有转速标注时把这一类采样均分到怠速..`top` rpm。
fn synthetic_reference(idle_ref: f32, index: usize, total: usize, top: f32) -> f32 {
    let low = (idle_ref * 1.4).max(1200.0);
    let high = top.max(low + 1000.0);
    if total <= 1 {
        (low + high) * 0.5
    } else {
        low + (high - low) * index as f32 / (total - 1) as f32
    }
}

pub(super) fn fmod_error<E: std::fmt::Display>(error: E) -> AudioError {
    AudioError::Fmod(error.to_string())
}
