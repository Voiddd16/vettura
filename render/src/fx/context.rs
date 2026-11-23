//! 每帧传给 FX 模块的只读快照 + 预算。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxQuality {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl Default for FxQuality {
    fn default() -> Self {
        Self::Medium
    }
}

impl FxQuality {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "ultra" => Some(Self::Ultra),
            _ => None,
        }
    }

    pub fn at_least(self, minimum: Self) -> bool {
        self >= minimum
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Ultra => "ultra",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WeatherState {
    /// 0..1 的降雨强度。
    pub rain: f32,
    /// 0..1 的路面湿滑程度。
    pub wetness: f32,
    /// 风速，m/s。
    pub wind_speed: f32,
    /// 风向，度。
    pub wind_direction_deg: f32,
    /// 当地时间，小时。
    pub time_of_day: f32,
    /// 0..1 的雾强度。
    pub fog: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CameraState {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub fov_deg: f32,
    pub in_cockpit: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CarVisualState {
    pub index: i32,
    pub speed_kmh: f32,
    pub brake_temp_c: f32,
    pub tyre_slip: f32,
    pub is_player: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct FxCost {
    pub draw_calls: u32,
    pub particles: u32,
    pub lights: u32,
    pub post_passes: u32,
    pub milliseconds: f32,
}

impl Default for FxCost {
    fn default() -> Self {
        Self {
            draw_calls: 0,
            particles: 0,
            lights: 0,
            post_passes: 0,
            milliseconds: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct FxBudget {
    pub max_draw_calls: u32,
    pub max_particles: u32,
    pub max_lights: u32,
    pub max_post_passes: u32,
    pub max_ms: f32,
}

impl Default for FxBudget {
    fn default() -> Self {
        Self {
            max_draw_calls: 2_000,
            max_particles: 20_000,
            max_lights: 64,
            max_post_passes: 8,
            max_ms: 4.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum FxEvent {
    SceneLoaded,
    TrackLoaded,
    CarSpawned { index: i32 },
    CarDespawned { index: i32 },
    WeatherChanged,
    LapCompleted { index: i32, lap_ms: i64 },
    CameraChanged,
    QualityChanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct FxContext {
    pub dt: f32,
    pub time: f32,
    pub quality: FxQuality,
    pub weather: WeatherState,
    pub camera: CameraState,
    pub cars: Vec<CarVisualState>,
    pub budget: FxBudget,
    pub events: Vec<FxEvent>,
}

impl Default for FxContext {
    fn default() -> Self {
        Self {
            dt: 0.0,
            time: 0.0,
            quality: FxQuality::Medium,
            weather: WeatherState::default(),
            camera: CameraState::default(),
            cars: Vec::new(),
            budget: FxBudget::default(),
            events: Vec::new(),
        }
    }
}
