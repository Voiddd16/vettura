//! 实时遥测：配置帧组装 + 网络输出。
//!
//! 配置来自 Godot 侧读取的 `user://telemetry.cfg`（`ConfigFile` 格式），在这里
//! 被解析成 [`TelemetryConfig`]；每个物理帧由调用方（`WorldBody`）把采样数据
//! 交给 [`TelemetrySink`]，序列化成一行 JSON 后经 UDP 或 TCP 发出。
//!
//! 本模块保持 headless（只用 `std::net`），不依赖 godot-rust；配置的读取与
//! 热更新在 `vettura-godot` 侧完成。

use std::io::Write;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};

use glam::Vec3;
use serde::Serialize;
use vettura_physics::{Controls, ErsMode};

use crate::timing::LapTimer;
use crate::vehicle::{VehicleSnapshot, WheelSnapshot};
use crate::world::{ContactEntity, ImpactEvent};
use vettura_track::Weather;

/// 配置文件路径（Godot 虚拟路径）。
pub const TELEMETRY_CONFIG_PATH: &str = "user://telemetry.cfg";

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// 采样范围：只采主车，还是场上全部车辆。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TelemetryScope {
    /// 只采 `primary` 主车。
    Ego,
    /// 采 [`World::vehicle_handles`] 里的全部车辆。
    All,
    /// 按发车顺序（`World::spawn` 先后）取给定下标的车辆。
    Indices(Vec<usize>),
}

/// 遥测开关组：总开关 + 三个方向，各自带细粒度子开关。
#[derive(Clone, Debug, PartialEq)]
pub struct TelemetryConfig {
    /// 总开关。`false` 时整个遥测停摆（不建 socket、不发帧）。
    pub enabled: bool,
    /// 采样频率上限，Hz。`0` 表示每个物理帧都采（60 Hz）。
    pub rate_hz: f32,
    /// 输出协议。
    pub protocol: TelemetryProtocol,
    /// 目标地址（主机或 IP）。
    pub host: String,
    /// 目标端口。
    pub port: u16,
    /// 采集范围。
    pub scope: TelemetryScope,
    /// 车辆方向开关。
    pub vehicle: VehicleTelemetry,
    /// 赛道方向开关。
    pub track: TrackTelemetry,
    /// 计时方向开关。
    pub timing: TimingTelemetry,
}

/// 输出协议。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TelemetryProtocol {
    /// 无连接数据报，每帧一个 JSON 行。
    Udp,
    /// 面向连接流，断线自动重连。
    Tcp,
}

/// 车辆方向的细粒度子开关。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleTelemetry {
    pub enabled: bool,
    /// 运动学：位置/姿态/速度/角速度。
    pub pose: bool,
    /// 驾驶员输入：油门/刹车/离合/转向/手刹/挡位请求。
    pub controls: bool,
    /// 动力总成：转速/挡位/涡轮/冷却液/机油/ERS。
    pub powertrain: bool,
    /// 四轮：接触/载荷/滑移/滑角/外倾/悬架/表面。
    pub wheels: bool,
    /// 轮胎：温度/压力/磨损/水膜。
    pub tyres: bool,
    /// 刹车盘温度。
    pub brakes: bool,
    /// 燃油。
    pub fuel: bool,
}

/// 赛道方向的细粒度子开关。
#[derive(Clone, Debug, PartialEq)]
pub struct TrackTelemetry {
    pub enabled: bool,
    /// 天气/环境：湿度/雨强/气温/路温/风/抓地。
    pub weather: bool,
    /// 橡胶线进度（按每辆车位置采样）。
    pub groove: bool,
    /// 碰撞事件增量。
    pub impacts: bool,
    /// 本帧碰撞事件数量上限（0 = 不限）。
    pub max_impacts: usize,
}

/// 计时方向的细粒度子开关。
#[derive(Clone, Debug, PartialEq)]
pub struct TimingTelemetry {
    pub enabled: bool,
    /// 圈数 / 当前圈速 / 上圈 / 最佳圈。
    pub laps: bool,
    /// 当前分段与本圈分段成绩。
    pub sectors: bool,
    /// 上圈分段成绩与有效性。
    pub last_lap: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rate_hz: 10.0,
            protocol: TelemetryProtocol::Udp,
            host: "127.0.0.1".to_string(),
            port: 9999,
            scope: TelemetryScope::Ego,
            vehicle: VehicleTelemetry::default(),
            track: TrackTelemetry::default(),
            timing: TimingTelemetry::default(),
        }
    }
}

impl Default for VehicleTelemetry {
    fn default() -> Self {
        Self {
            enabled: false,
            pose: true,
            controls: true,
            powertrain: true,
            wheels: true,
            tyres: true,
            brakes: true,
            fuel: true,
        }
    }
}

impl Default for TrackTelemetry {
    fn default() -> Self {
        Self {
            enabled: false,
            weather: true,
            groove: true,
            impacts: true,
            max_impacts: 64,
        }
    }
}

impl Default for TimingTelemetry {
    fn default() -> Self {
        Self {
            enabled: false,
            laps: true,
            sectors: true,
            last_lap: true,
        }
    }
}

impl TelemetryConfig {
    /// 任意方向开着就算有内容可发。
    pub fn any_section_enabled(&self) -> bool {
        self.vehicle.enabled || self.track.enabled || self.timing.enabled
    }

    /// 解析 `user://telemetry.cfg` 的内容（`ConfigFile` 文本）。
    ///
    /// 缺失的键一律回落到默认值；类型不匹配或解析失败记 warning 并整体禁用，
    /// 不会让游戏带病运行。
    pub fn parse(text: &str) -> Self {
        let cfg = TelemetryConfigFile::parse(text);
        cfg.into_config()
    }

    /// 采样节流：首次调用立即放行；之后距上次采样不足 `1 / rate_hz` 时返回
    /// `false`。`rate_hz <= 0` 表示不节流（每个物理帧都采）。
    pub fn should_sample(&self, now_s: f32, last_sample_s: &mut Option<f32>) -> bool {
        let Some(prev) = *last_sample_s else {
            *last_sample_s = Some(now_s);
            return true;
        };
        if self.rate_hz > 0.0 && now_s - prev < 1.0 / self.rate_hz - 1e-4 {
            return false;
        }
        *last_sample_s = Some(now_s);
        true
    }
}

// ---------------------------------------------------------------------------
// 配置文件解析（ConfigFile 文本格式）
// ---------------------------------------------------------------------------

/// `user://telemetry.cfg` 的原始键值视图。
///
/// Godot `ConfigFile` 是 INI 风格（`[section]` + `key=value`），这里手写一个
/// 足够小的解析器，避免为了读配置把 godot-rust 拖进 `vettura-world`。
#[derive(Default)]
struct TelemetryConfigFile {
    values: std::collections::HashMap<(String, String), String>,
}

impl TelemetryConfigFile {
    fn parse(text: &str) -> Self {
        let mut values = std::collections::HashMap::new();
        let mut section = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = rest.trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            values.insert(
                (section.clone(), key.trim().to_string()),
                unquote(value.trim()),
            );
        }
        Self { values }
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.values
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
    }

    fn bool(&self, section: &str, key: &str, fallback: bool) -> bool {
        match self.get(section, key) {
            None => fallback,
            Some(v) => match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                other => {
                    tracing::warn!(
                        "telemetry: [{section}]{key}={other:?} 不是布尔值，用默认 {fallback}"
                    );
                    fallback
                }
            },
        }
    }

    fn f32(&self, section: &str, key: &str, fallback: f32) -> f32 {
        match self.get(section, key) {
            None => fallback,
            Some(v) => v.parse::<f32>().unwrap_or_else(|_| {
                tracing::warn!("telemetry: [{section}]{key}={v:?} 不是数字，用默认 {fallback}");
                fallback
            }),
        }
    }

    fn u16(&self, section: &str, key: &str, fallback: u16) -> u16 {
        match self.get(section, key) {
            None => fallback,
            Some(v) => v.parse::<u16>().unwrap_or_else(|_| {
                tracing::warn!("telemetry: [{section}]{key}={v:?} 不是端口号，用默认 {fallback}");
                fallback
            }),
        }
    }

    fn usize(&self, section: &str, key: &str, fallback: usize) -> usize {
        match self.get(section, key) {
            None => fallback,
            Some(v) => v.parse::<usize>().unwrap_or_else(|_| {
                tracing::warn!("telemetry: [{section}]{key}={v:?} 不是整数，用默认 {fallback}");
                fallback
            }),
        }
    }

    fn string(&self, section: &str, key: &str, fallback: &str) -> String {
        self.get(section, key)
            .map(str::to_string)
            .unwrap_or_else(|| fallback.to_string())
    }

    fn into_config(self) -> TelemetryConfig {
        let scope = match self
            .get("telemetry", "scope")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("ego") | Some("primary") => TelemetryScope::Ego,
            Some("all") => TelemetryScope::All,
            Some("indices") => {
                let list = self
                    .get("telemetry", "indices")
                    .unwrap_or("")
                    .split(',')
                    .filter_map(|s| s.trim().parse::<usize>().ok())
                    .collect::<Vec<_>>();
                if list.is_empty() {
                    tracing::warn!("telemetry: scope=indices 但 indices 为空，回退 ego");
                    TelemetryScope::Ego
                } else {
                    TelemetryScope::Indices(list)
                }
            }
            Some(other) => {
                tracing::warn!("telemetry: scope={other:?} 未知，用 ego");
                TelemetryScope::Ego
            }
        };

        let protocol = match self
            .get("telemetry", "protocol")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("udp") => TelemetryProtocol::Udp,
            Some("tcp") => TelemetryProtocol::Tcp,
            Some(other) => {
                tracing::warn!("telemetry: protocol={other:?} 未知，用 udp");
                TelemetryProtocol::Udp
            }
        };

        TelemetryConfig {
            enabled: self.bool("telemetry", "enabled", false),
            rate_hz: self.f32("telemetry", "rate_hz", 10.0),
            protocol,
            host: self.string("telemetry", "host", "127.0.0.1"),
            port: self.u16("telemetry", "port", 9999),
            scope,
            vehicle: VehicleTelemetry {
                enabled: self.bool("vehicle", "enabled", false),
                pose: self.bool("vehicle", "pose", true),
                controls: self.bool("vehicle", "controls", true),
                powertrain: self.bool("vehicle", "powertrain", true),
                wheels: self.bool("vehicle", "wheels", true),
                tyres: self.bool("vehicle", "tyres", true),
                brakes: self.bool("vehicle", "brakes", true),
                fuel: self.bool("vehicle", "fuel", true),
            },
            track: TrackTelemetry {
                enabled: self.bool("track", "enabled", false),
                weather: self.bool("track", "weather", true),
                groove: self.bool("track", "groove", true),
                impacts: self.bool("track", "impacts", true),
                max_impacts: self.usize("track", "max_impacts", 64),
            },
            timing: TimingTelemetry {
                enabled: self.bool("timing", "enabled", false),
                laps: self.bool("timing", "laps", true),
                sectors: self.bool("timing", "sectors", true),
                last_lap: self.bool("timing", "last_lap", true),
            },
        }
    }
}

/// 去掉一对包裹引号（Godot ConfigFile 会给字符串值带引号）。
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// 帧
// ---------------------------------------------------------------------------

/// 一帧遥测的公共外壳。
#[derive(Serialize)]
struct Frame<'a> {
    /// 协议版本，破坏性变更时递增。
    v: u32,
    /// 采样时刻（秒，来自 `WorldBody::elapsed_s`）。
    t: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    vehicles: Option<&'a [VehicleFrame]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weather: Option<WeatherFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    impacts: Option<&'a [ImpactFrame]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timing: Option<TimingFrame>,
}

/// 一辆车的一帧。
#[derive(Serialize)]
struct VehicleFrame {
    /// `VehicleHandle::raw()`，跨帧稳定，可用于关联。
    id: u64,
    /// 发车顺序下标（`World::spawn` 先后）。
    index: usize,
    /// 是否主车。
    primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pos: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rot: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vel: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ang_vel: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_mps: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controls: Option<ControlsFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rpm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gear: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turbo_boost_bar: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coolant_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oil_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ers: Option<ErsFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wheels: Option<[WheelFrame; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tyre_temp_c: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tyre_pressure_psi: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tyre_wear: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brake_temp_c: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fuel_kg: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fuel_fraction: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groove: Option<f32>,
}

#[derive(Serialize)]
struct ControlsFrame {
    throttle: f32,
    brake: f32,
    clutch: f32,
    steering: f32,
    handbrake: bool,
    drs: bool,
    abs: bool,
    tc: bool,
    ers: ErsMode,
}

#[derive(Serialize)]
struct ErsFrame {
    soc: f32,
    /// 焦耳（`VehicleSnapshot::ers_energy_j`）。
    energy_j: f32,
    /// 本圈已部署，焦耳。
    lap_deploy_j: f32,
    temperature_c: f32,
}

#[derive(Serialize, Clone, Copy)]
struct WheelFrame {
    contact: bool,
    /// 表面 id（`SurfaceId`）。
    surface: u16,
    load_n: f32,
    slip_ratio: f32,
    slip_angle: f32,
    /// 外倾角，度（源数据是弧度）。
    camber_deg: f32,
    suspension_m: f32,
    water_mm: f32,
}

#[derive(Serialize)]
struct WeatherFrame {
    wetness: f32,
    rain: f32,
    air_c: f32,
    track_c: f32,
    wind: [f32; 3],
    grip: f32,
    fog: f32,
}

#[derive(Serialize)]
struct ImpactFrame {
    a: String,
    b: String,
    pos: [f32; 3],
    impulse_ns: f32,
    speed_mps: f32,
    severity: f32,
}

#[derive(Serialize)]
struct TimingFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    laps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lap_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_lap_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_lap_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sector: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sector_s: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sector_s: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lap_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_lap_valid: Option<bool>,
}

// ---------------------------------------------------------------------------
// 采样输入
// ---------------------------------------------------------------------------

/// 一辆车在本帧要写进遥测的全部数据（由调用方装配）。
#[derive(Default)]
pub struct VehicleSample<'a> {
    pub handle: u64,
    pub index: usize,
    pub primary: bool,
    pub snapshot: Option<&'a VehicleSnapshot>,
    pub controls: Option<Controls>,
    /// 橡胶线进度（可选，`World` 上按位置采样）。
    pub groove: Option<f32>,
}

/// 一帧的采样输入。
#[derive(Default)]
pub struct TelemetrySample<'a> {
    /// 采样时刻（秒）。
    pub time_s: f32,
    pub vehicles: Vec<VehicleSample<'a>>,
    pub weather: Option<Weather>,
    /// 本帧新增的碰撞（调用方负责游标式去重）。
    pub impacts: &'a [ImpactEvent],
    pub timing: Option<&'a LapTimer>,
    /// 主车句柄（`None` 表示场上无主车）。
    pub primary: Option<u64>,
}

impl TelemetrySample<'_> {
    /// 按配置组装 JSON 帧；没有任何开启的方向时返回 `None`。
    fn build_frame(&self, cfg: &TelemetryConfig) -> Option<String> {
        if !cfg.any_section_enabled() {
            return None;
        }

        let vehicles = if cfg.vehicle.enabled {
            let frames: Vec<VehicleFrame> = self
                .vehicles
                .iter()
                .filter(|sample| sample.snapshot.is_some() || sample.controls.is_some())
                .map(|sample| build_vehicle(sample, cfg))
                .collect();
            Some(frames)
        } else {
            None
        };

        let weather = if cfg.track.enabled && cfg.track.weather {
            self.weather.map(build_weather)
        } else {
            None
        };

        let impacts = if cfg.track.enabled && cfg.track.impacts && !self.impacts.is_empty() {
            let mut frames: Vec<ImpactFrame> = self.impacts.iter().map(build_impact).collect();
            if cfg.track.max_impacts > 0 && frames.len() > cfg.track.max_impacts {
                frames.truncate(cfg.track.max_impacts);
            }
            Some(frames)
        } else {
            None
        };

        let timing = if cfg.timing.enabled {
            self.timing.map(|timer| build_timing(timer, cfg))
        } else {
            None
        };

        // 三个方向都没内容可写就不发帧（比如只有 impacts 且本帧为空）。
        if vehicles.is_none() && weather.is_none() && impacts.is_none() && timing.is_none() {
            return None;
        }

        let frame = Frame {
            v: 1,
            t: self.time_s,
            vehicles: vehicles.as_deref(),
            weather,
            impacts: impacts.as_deref(),
            timing,
        };

        // serde_json 对 NaN/Inf 的**值**写 `null`（只有 map key 会报错），
        // 所以这里直接序列化是安全的。
        match serde_json::to_string(&frame) {
            Ok(text) => Some(text),
            Err(err) => {
                tracing::warn!("telemetry: 帧序列化失败：{err}");
                None
            }
        }
    }
}

fn build_vehicle(sample: &VehicleSample, cfg: &TelemetryConfig) -> VehicleFrame {
    let v = &cfg.vehicle;
    let snapshot = sample.snapshot;
    let pos = snapshot.map(|s| vec3(s.position));
    let rot = snapshot.map(|s| quat(s.orientation));
    let vel = snapshot.map(|s| vec3(s.linear_velocity));
    let ang_vel = snapshot.map(|s| vec3(s.angular_velocity));
    let speed = snapshot.map(|s| s.speed_mps);

    let controls = if v.controls {
        sample.controls.map(|c| ControlsFrame {
            throttle: c.throttle,
            brake: c.brake,
            clutch: c.clutch,
            steering: c.steering,
            handbrake: c.handbrake,
            drs: c.drs,
            abs: c.abs,
            tc: c.tc,
            ers: c.ers,
        })
    } else {
        None
    };

    let (rpm, gear, turbo, coolant, oil, ers) = if v.powertrain {
        (
            snapshot.map(|s| s.engine_rpm),
            snapshot.map(|s| s.gear),
            snapshot.map(|s| s.turbo_boost_bar),
            snapshot.map(|s| s.coolant_c),
            snapshot.map(|s| s.oil_c),
            snapshot.map(|s| ErsFrame {
                soc: s.ers_soc,
                energy_j: s.ers_energy_j,
                lap_deploy_j: s.ers_lap_deploy_j,
                temperature_c: s.ers_temperature_c,
            }),
        )
    } else {
        (None, None, None, None, None, None)
    };

    let wheels = if v.wheels {
        snapshot.map(|s| s.wheels.map(wheel_frame))
    } else {
        None
    };

    let (tyre_temp, tyre_pressure, tyre_wear) = if v.tyres {
        (
            snapshot.map(|s| s.tyre_temperatures_c),
            snapshot.map(|s| s.tyre_pressures_psi),
            snapshot.map(|s| s.tyre_wear),
        )
    } else {
        (None, None, None)
    };

    let brake_temp = if v.brakes {
        snapshot.map(|s| s.brake_temperatures_c)
    } else {
        None
    };

    let (fuel_kg, fuel_fraction) = if v.fuel {
        (
            snapshot.map(|s| s.fuel_kg),
            snapshot.map(|s| s.fuel_fraction),
        )
    } else {
        (None, None)
    };

    // 橡胶线只在 pose 开启时附带（按车辆位置采样）。
    let groove = if v.enabled && cfg.track.groove {
        sample.groove
    } else {
        None
    };

    VehicleFrame {
        id: sample.handle,
        index: sample.index,
        primary: sample.primary,
        pos: v.pose.then_some(pos).flatten(),
        rot: v.pose.then_some(rot).flatten(),
        vel: v.pose.then_some(vel).flatten(),
        ang_vel: v.pose.then_some(ang_vel).flatten(),
        speed_mps: v.pose.then_some(speed).flatten(),
        controls,
        rpm,
        gear,
        turbo_boost_bar: turbo,
        coolant_c: coolant,
        oil_c: oil,
        ers,
        wheels,
        tyre_temp_c: tyre_temp,
        tyre_pressure_psi: tyre_pressure,
        tyre_wear,
        brake_temp_c: brake_temp,
        fuel_kg,
        fuel_fraction,
        groove,
    }
}

fn wheel_frame(wheel: WheelSnapshot) -> WheelFrame {
    WheelFrame {
        contact: wheel.contact,
        surface: wheel.surface.0,
        load_n: wheel.load_n,
        slip_ratio: wheel.slip_ratio,
        slip_angle: wheel.slip_angle,
        camber_deg: wheel.camber_rad.to_degrees(),
        suspension_m: wheel.suspension_length_m,
        water_mm: wheel.water_depth_m * 1000.0,
    }
}

fn build_weather(weather: Weather) -> WeatherFrame {
    WeatherFrame {
        wetness: weather.wetness,
        rain: weather.rain_intensity,
        air_c: weather.air_temperature_c,
        track_c: weather.track_temperature_c,
        wind: vec3(weather.wind),
        grip: weather.grip_scale,
        fog: weather.fog_density,
    }
}

fn build_impact(event: &ImpactEvent) -> ImpactFrame {
    ImpactFrame {
        a: entity_name(event.a),
        b: entity_name(event.b),
        pos: vec3(event.position),
        impulse_ns: event.normal_impulse_ns,
        speed_mps: event.impact_speed_mps,
        severity: event.severity,
    }
}

fn entity_name(entity: ContactEntity) -> String {
    match entity {
        ContactEntity::Vehicle(h) => format!("vehicle:{}", h.raw()),
        ContactEntity::Debris(h) => format!("debris:{}", h.raw()),
        ContactEntity::TrackObject(o) => format!("track:{}", o.raw()),
        ContactEntity::Ground => "ground".to_string(),
        ContactEntity::Other(id) => format!("other:{id}"),
    }
}

fn build_timing(timer: &LapTimer, cfg: &TelemetryConfig) -> TimingFrame {
    let t = &cfg.timing;
    TimingFrame {
        laps: t.laps.then(|| timer.laps()),
        lap_s: t.laps.then(|| timer.lap_time_s()),
        last_lap_s: if t.last_lap { timer.last_lap_s() } else { None },
        best_lap_s: if t.laps { timer.best_lap_s() } else { None },
        sector: t.sectors.then(|| timer.sector()),
        sector_s: t.sectors.then(|| timer.sector_times_s().to_vec()),
        last_sector_s: t.last_lap.then(|| timer.last_sector_times_s().to_vec()),
        lap_valid: t.laps.then(|| timer.is_valid_lap()),
        last_lap_valid: t.last_lap.then(|| timer.was_last_lap_valid()),
    }
}

fn vec3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn quat(q: glam::Quat) -> [f32; 4] {
    [q.x, q.y, q.z, q.w]
}

// ---------------------------------------------------------------------------
// 输出
// ---------------------------------------------------------------------------

/// 网络输出端点：UDP 每帧一个数据报，TCP 一条流（断线自动重连）。
pub struct TelemetrySink {
    config: TelemetryConfig,
    udp: Option<UdpSocket>,
    tcp: Option<TcpStream>,
    address: Option<SocketAddr>,
    /// 地址解析失败 / 连接失败后连续跳过的次数，避免每帧刷日志。
    backoff: u32,
}

impl TelemetrySink {
    pub fn new(config: TelemetryConfig) -> Self {
        Self {
            config,
            udp: None,
            tcp: None,
            address: None,
            backoff: 0,
        }
    }
}

impl Default for TelemetrySink {
    /// 默认即"配置缺失"的状态：整体禁用，不建任何 socket。
    fn default() -> Self {
        Self::new(TelemetryConfig::default())
    }
}

impl TelemetrySink {
    pub fn config(&self) -> &TelemetryConfig {
        &self.config
    }

    /// 换新配置（热更新）。协议或地址变了就重建 socket。
    pub fn reconfigure(&mut self, config: TelemetryConfig) {
        let endpoint_changed = config.protocol != self.config.protocol
            || config.host != self.config.host
            || config.port != self.config.port;
        self.config = config;
        if endpoint_changed {
            self.udp = None;
            self.tcp = None;
            self.address = None;
            self.backoff = 0;
        }
    }

    /// 采样并发送一帧。未启用、无内容或发送失败时静默（失败按退避记 warning）。
    pub fn send(&mut self, sample: &TelemetrySample<'_>) {
        if !self.config.enabled || !self.config.any_section_enabled() {
            return;
        }
        let Some(text) = sample.build_frame(&self.config) else {
            return;
        };
        let bytes = text.as_bytes();

        match self.config.protocol {
            TelemetryProtocol::Udp => self.send_udp(bytes),
            TelemetryProtocol::Tcp => self.send_tcp(bytes),
        }
    }

    fn send_udp(&mut self, bytes: &[u8]) {
        if self.udp.is_none() && !self.prepare_address() {
            return;
        }
        let Some(socket) = self.udp.as_ref() else {
            return;
        };
        let Some(address) = self.address else {
            return;
        };
        match socket.send_to(bytes, address) {
            Ok(_) => self.backoff = 0,
            Err(err) => self.note_failure("UDP 发送", &err),
        }
    }

    fn send_tcp(&mut self, bytes: &[u8]) {
        if self.tcp.is_none() && !self.prepare_address() {
            return;
        }
        let Some(address) = self.address else {
            return;
        };

        let mut need_reconnect = false;
        if let Some(stream) = self.tcp.as_mut() {
            // 先探测对端是否还活着（读到 0 = 对方关闭）。
            match stream.write(bytes) {
                Ok(_) => {
                    let _ = stream.flush();
                    self.backoff = 0;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {
                    need_reconnect = true;
                }
            }
        } else {
            need_reconnect = true;
        }

        if need_reconnect {
            self.tcp = None;
            match TcpStream::connect_timeout(&address, std::time::Duration::from_millis(500)) {
                Ok(mut stream) => {
                    let _ = stream.set_nodelay(true);
                    let _ = stream.set_nonblocking(false);
                    // 连上后立刻补发当前帧，避免首帧丢失。
                    let _ = stream.write_all(bytes);
                    let _ = stream.flush();
                    self.tcp = Some(stream);
                    self.backoff = 0;
                }
                Err(err) => self.note_failure("TCP 连接", &err),
            }
        }
    }

    /// 解析目标地址并准备 UDP socket。失败时按指数退避记日志。
    fn prepare_address(&mut self) -> bool {
        if self.backoff > 0 {
            self.backoff -= 1;
            return false;
        }
        if self.address.is_none() {
            let target = format!("{}:{}", self.config.host, self.config.port);
            let resolved = target
                .to_socket_addrs()
                .ok()
                .and_then(|mut addrs| addrs.next());
            match resolved {
                Some(addr) => self.address = Some(addr),
                None => {
                    self.note_failure("地址解析", &std::io::Error::other(target.as_str()));
                    return false;
                }
            }
        }
        if self.udp.is_none() && self.config.protocol == TelemetryProtocol::Udp {
            match UdpSocket::bind("0.0.0.0:0") {
                Ok(socket) => self.udp = Some(socket),
                Err(err) => {
                    self.note_failure("UDP 绑定", &err);
                    return false;
                }
            }
        }
        self.address.is_some()
    }

    fn note_failure(&mut self, what: &str, err: &std::io::Error) {
        // 1、2、4、8…帧的指数退避，最多约 1 秒（60 Hz）后重试一次。
        let skip = if self.backoff == 0 {
            1
        } else {
            self.backoff * 2
        };
        self.backoff = skip.min(64);
        tracing::warn!("telemetry: {what} 失败（{} 帧后重试）：{err}", self.backoff);
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let text = r#"
[telemetry]
enabled=true
rate_hz=20
protocol="tcp"
host="192.168.1.10"
port=12345
scope="all"

[vehicle]
enabled=true
wheels=false

[track]
enabled=true
max_impacts=8

[timing]
enabled=true
sectors=false
"#;
        let cfg = TelemetryConfig::parse(text);
        assert!(cfg.enabled);
        assert_eq!(cfg.rate_hz, 20.0);
        assert_eq!(cfg.protocol, TelemetryProtocol::Tcp);
        assert_eq!(cfg.host, "192.168.1.10");
        assert_eq!(cfg.port, 12345);
        assert_eq!(cfg.scope, TelemetryScope::All);
        assert!(cfg.vehicle.enabled);
        assert!(!cfg.vehicle.wheels);
        assert!(cfg.track.enabled);
        assert_eq!(cfg.track.max_impacts, 8);
        assert!(cfg.timing.enabled);
        assert!(!cfg.timing.sectors);
    }

    #[test]
    fn missing_file_means_disabled_defaults() {
        let cfg = TelemetryConfig::parse("");
        assert!(!cfg.enabled);
        assert!(!cfg.any_section_enabled());
        assert_eq!(cfg.protocol, TelemetryProtocol::Udp);
        assert_eq!(cfg.scope, TelemetryScope::Ego);
        assert_eq!(cfg.port, 9999);
    }

    #[test]
    fn scope_indices_parses_list() {
        let cfg = TelemetryConfig::parse("[telemetry]\nscope=\"indices\"\nindices=\"0, 2,3\"\n");
        assert_eq!(cfg.scope, TelemetryScope::Indices(vec![0, 2, 3]));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let cfg = TelemetryConfig::parse("# 注释\n\n; 分号注释\n[telemetry]\nenabled=yes\n");
        assert!(cfg.enabled);
    }

    #[test]
    fn rate_throttle_gates_sampling() {
        let cfg = TelemetryConfig {
            rate_hz: 10.0,
            ..TelemetryConfig::default()
        };
        let mut last = None;
        assert!(cfg.should_sample(0.0, &mut last));
        assert!(!cfg.should_sample(0.05, &mut last));
        assert!(cfg.should_sample(0.1, &mut last));
    }

    #[test]
    fn frame_serializes_with_null_for_non_finite() {
        let mut sample = TelemetrySample::default();
        let mut snapshot = VehicleSnapshot::default();
        snapshot.engine_rpm = f32::NAN;
        sample.time_s = 1.5;
        sample.vehicles.push(VehicleSample {
            handle: 7,
            index: 0,
            primary: true,
            snapshot: Some(&snapshot),
            controls: Some(Controls::default()),
            groove: Some(0.5),
        });
        let cfg = TelemetryConfig {
            enabled: true,
            vehicle: VehicleTelemetry {
                enabled: true,
                ..VehicleTelemetry::default()
            },
            ..TelemetryConfig::default()
        };
        let text = sample.build_frame(&cfg).expect("frame");
        assert!(text.contains("\"id\":7"));
        assert!(text.contains("\"rpm\":null"));
        assert!(text.contains("\"t\":1.5"));
    }

    #[test]
    fn disabled_directions_are_omitted() {
        let sample = TelemetrySample::default();
        let cfg = TelemetryConfig {
            enabled: true,
            timing: TimingTelemetry {
                enabled: true,
                ..TimingTelemetry::default()
            },
            ..TelemetryConfig::default()
        };
        // 没有计时器、没有车、没有天气：不发帧。
        assert!(sample.build_frame(&cfg).is_none());
    }
}
