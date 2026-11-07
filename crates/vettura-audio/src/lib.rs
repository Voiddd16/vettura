//! Assetto Corsa 车模音频发现 + FMOD Studio runtime 直达。
//!
//! 这个 crate **不解析 `.bank`**：bank 里的 FMOD 封装由 FMOD Engine 自己解码。
//! Rust 侧只做三件事：
//!
//! 1. 读 `sfx/GUIDs.txt`，拿到事件名与 GUID（[`guids`]）；
//! 2. 定位 `sfx/<car_id>.bank`（[`manifest`]）；
//! 3. 调 FMOD Engine 2.02.22 加载 bank / 播放事件 / 每帧 update（[`runtime`]）。
//!
//! SDK 版本与兼容规则（`docs/audio.md`）：
//!
//! - 锁定 FMOD Engine SDK **2.02.22**，`libfmod` 锁 **~2.222**；
//! - 2.02.22 runtime 只保证加载 **2.02.x** 编出来的 bank；2.03 / 2.04 不保证；
//! - 只动态链接，SDK 二进制不入库；本地由主工程脚本下载到 `.cache/fmod/`。
//!
//! 没找到本地 SDK 时，build.rs 会输出 `fmod_sdk_missing`，`AudioEngine` 保留 API
//! 但 `start()` 返回 [`AudioError::NotLinked`]，因此 workspace 仍能编译/测试。
//!
//! 启用真机音频：
//!
//! ```bash
//! cd /path/to/vettura/scripts && uv run fetch_fmod.py
//! cargo build -p vettura-audio --features fmod   # 或 -p vettura-godot --features fmod
//! ```

pub mod error;
pub mod guids;
pub mod manifest;
pub mod runtime;

pub use error::{AudioError, Result};
pub use guids::{SoundEvent, parse_events, parse_events_for};
pub use manifest::{GUIDS_NAME, SFX_DIR, SoundManifest};
pub use runtime::{AudioEngine, AudioTransform, AudioVec3, VehicleParams};
