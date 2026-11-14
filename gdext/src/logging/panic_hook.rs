//! panic hook：崩溃时把现场固化到 `user://logs/last-crash.log`。
//!
//! 链式挂载：`take_hook()` 拿到前一个 hook（通常是 godot-rust 自己装的，负责
//! 把 panic 打进 Godot 控制台），本 hook 先写转储、再转交，原行为不变。
//!
//! 转储内容（同步写，进程正在死，阻塞可接受；ring 本来就在内存里）：
//!
//! 1. 时间 / 线程 / panic 消息 / 位置；
//! 2. `Backtrace::force_capture()`——`force` 不看 `RUST_BACKTRACE`，崩溃现场
//!    不该依赖用户环境变量；
//! 3. 环形缓冲全量 dump（最近 N 条，含 panic 前的级别/上下文线索）。
//!
//! 追加写；超过 1MiB 先清空重建（上一次崩溃的现场已经过期）。全程不 panic
//! （hook 里再 panic 直接 abort），所有 IO 失败静默放弃——没有比转储本身更要紧。
//!
//! 日志被禁用（`log.cfg enabled=false`）时不安装本 hook，panic 走原路径。

use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::PathBuf;

use tracing_subscriber::fmt::time::{FormatTime, SystemTime};

use super::runtime;

/// 转储文件超过该大小则重建。
const MAX_CRASH_LOG: u64 = 1024 * 1024;

/// 挂 panic hook（幂等性由调用方保证：只在 subscriber 安装成功后调一次）。
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        write_crash_dump(info);
        previous(info);
    }));
}

fn write_crash_dump(info: &PanicHookInfo<'_>) {
    let Some(state) = runtime::state() else {
        return;
    };
    let Some(dir) = &state.native_dir else {
        return;
    };
    let path = PathBuf::from(dir).join("last-crash.log");

    let mut out = String::new();
    let mut timestamp = String::new();
    {
        let mut writer = tracing_subscriber::fmt::format::Writer::new(&mut timestamp);
        let _ = SystemTime.format_time(&mut writer);
    }
    let thread = std::thread::current();
    let _ = write!(
        out,
        "=== vettura crash @ {} ===\nthread: {}\npanic: {}\n",
        timestamp,
        thread.name().unwrap_or("<unnamed>"),
        payload_message(info.payload()),
    );
    if let Some(location) = info.location() {
        let _ = writeln!(
            out,
            "location: {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    }
    out.push_str("backtrace:\n");
    out.push_str(&format!("{}\n", std::backtrace::Backtrace::force_capture()));
    out.push_str("--- 最近日志（环形缓冲）---\n");
    out.push_str(&state.ring.dump());
    out.push('\n');

    // 超限重建：追加会让每次崩溃都翻一倍大小，现场过期就不值得保留。
    let oversized = std::fs::metadata(&path)
        .map(|metadata| metadata.len() > MAX_CRASH_LOG)
        .unwrap_or(false);
    if oversized {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(out.as_bytes());
    }
}

/// panic payload 的文本：宏字面量是 `&'static str`，`panic!("{}", x)` 是
/// `String`，自定义 payload 只能说明类型。
fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "(non-string panic payload)".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_message_covers_common_kinds() {
        let literal: &(dyn std::any::Any + Send) = &"literal";
        assert_eq!(payload_message(literal), "literal");
        let owned: &(dyn std::any::Any + Send) = &String::from("owned");
        assert_eq!(payload_message(owned), "owned");
    }
}
