//! Assetto Corsa 车模音频发现 + FMOD runtime 直达。
//!
//! 这个 crate **不解析 `.bank`**：bank 里的 FSB 数据是 FMOD 自己的封装，交给
//! FMOD Engine 解码。Rust 侧只做三件事：
//!
//! 1. 读 `sfx/GUIDs.txt`，拿到事件名与 GUID（[`guids`]）；
//! 2. 定位 `sfx/<car_id>.bank`（[`manifest`]）；
//! 3. 调 FMOD Studio runtime 加载 bank / 播放事件 / 每帧 update（[`runtime`]）。
//!
//! 默认不链接 FMOD；构建 Godot 扩展时显式启用：
//!
//! ```bash
//! FMOD_SYS_FMOD_DIRECTORY=/path/to/FMODEngine cargo build -p vettura-godot --features fmod
//! ```

pub mod error;
pub mod guids;
pub mod manifest;
pub mod runtime;

pub use error::{AudioError, Result};
pub use guids::{SoundEvent, parse_events, parse_events_for};
pub use manifest::{GUIDS_NAME, SFX_DIR, SoundManifest};
pub use runtime::{AudioEngine, AudioTransform, AudioVec3};
