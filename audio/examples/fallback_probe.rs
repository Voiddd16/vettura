//! FSB fallback 诊断：加载真实 AC bank，按 ai_lap 的方式驱动采样，输出 WAV。
//!
//! ```bash
//! cargo run -q -p vettura-audio --features fmod --example fallback_probe -- \
//!     "mods/<car dir>" /tmp/fallback.wav
//! ```
//!
//! 前半段是低油门 / 4000 rpm，后半段切到全油门 / 12000 rpm 并升一挡，
//! 用来确认引擎 on/off 层、换挡一次性采样、风/胎噪层都在混音里。
//! 可用 Python + numpy 检查频谱：<20 Hz 应为 0%，主能量落在 200–1000 Hz。
use std::time::Duration;

use vettura_audio::{AudioEngine, VehicleParams};

fn main() {
    let car = std::env::args()
        .nth(1)
        .expect("用法: fallback_probe <car dir> <out.wav>");
    let out = std::env::args()
        .nth(2)
        .expect("用法: fallback_probe <car dir> <out.wav>");

    // SAFETY: 单线程诊断程序，启动 FMOD 前设置。
    unsafe { std::env::set_var("VETTURA_AUDIO_WAV", &out) };
    let mut engine = match AudioEngine::start(64) {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("FMOD 不可用（需要 --features fmod 和本地 SDK）：{err}");
            return;
        }
    };
    println!("loaded car: {:?}", engine.load_car(&car));
    engine.play("engine_ext").expect("play");
    for frame in 0..240 {
        let throttle = if frame < 120 { 0.0 } else { 0.8 };
        let rpm = if frame < 120 { 4000.0 } else { 12000.0 };
        let params = VehicleParams {
            rpm,
            throttle,
            load: throttle,
            speed: 200.0,
            view_blend: 0.0,
            gear: if frame < 120 { 2 } else { 3 },
            slip: if frame < 120 { 0.0 } else { 0.2 },
        };
        engine.set_vehicle_parameters("engine_ext", params).unwrap();
        engine.update().unwrap();
        std::thread::sleep(Duration::from_millis(8));
    }
    println!(
        "fallback: {:?}",
        engine
            .fallback_summary()
            .unwrap_or_else(|| "N/A".to_owned())
    );
    println!("done -> {out}");
}
