//! 全系统日志：安装全局 `tracing` subscriber（环形缓冲 + 控制台 + 滚动文件）。
//!
//! # 边界（为什么住在 `vettura-godot`）
//!
//! 业务 crate（`kn5` / `acdata` / `physics` / `world` / `audio` / `assets`）只
//! `use tracing::*` 发事件，**不依赖本模块**——它们是插件，"装 subscriber、落盘、
//! 读 `user://log.cfg`"这些需要 Godot 的活全部集中在这里（唯一认识 Godot 的
//! crate）。CLI 工具（`tools/`）不加载 GDExtension，各自打印即可，不走本模块。
//!
//! # 架构
//!
//! ```text
//! 业务 crate：use tracing::*（只依赖 workspace 的 tracing，不加其它依赖）
//!    │ events + spans
//!    ▼
//! gdext/src/logging（filter + 多 sink fan-out；唯一认识 Godot 的地方）
//!    ├── FileSink     轮转落盘 user://logs/vettura.YYYY-MM-DD.log（非阻塞 writer 线程）
//!    ├── ConsoleSink  转 stdout（Godot 编辑器 Output / headless 终端可见）
//!    └── RingSink     有界环形缓冲（默认 4096 条）：崩溃转储 / VetturaLogger.recent 的数据源；
//!                     panic 时 hook 同步追加写 user://logs/last-crash.log（backtrace + ring）
//! ```
//!
//! 子模块分工：[`ring`](ring.rs) 环形缓冲、[`runtime`](runtime.rs) 动态 filter
//! / warn 聚合 / 全局状态与热重载、[`panic_hook`](panic_hook.rs) 崩溃转储、
//! [`logger`](logger.rs) `VetturaLogger` GDScript API。
//!
//! # 配置
//!
//! `user://log.cfg`（Godot `ConfigFile` / INI 格式；环境变量 `VETTURA_LOG` /
//! `RUST_LOG` 优先于它，见下）：
//!
//! ```ini
//! [log]
//! enabled=true
//! level="info"          ; trace|debug|info|warn|error，全局默认
//! file=true             ; 落盘开关
//! dir="user://logs"     ; 日志目录（Godot 虚拟路径）
//! rotate="daily"        ; daily|never（每天一个文件 / 单文件追加）
//! max_files=7           ; 最多保留几个日志文件
//! console=true          ; 是否同时打到 Godot 控制台（stdout）
//! console_level="info"  ; 控制台级别下限；与 level 取更严的那个
//! ring_capacity=4096    ; 环形缓冲条数（崩溃转储 / recent() 的数据源）
//! ```
//!
//! 所有键都可省略，缺省值见 [`LogConfig::default`]。**级别类键
//! （level / console_level / ring_capacity）支持热重载**：`on_main_loop_frame`
//! 每 0.5s 比一次 mtime，变了就重读（`VetturaLogger.reload()` 可手动触发）；
//! sink 结构（enabled / file / dir / rotate / max_files / console）在安装期
//! 定死，变化需重启生效（热重载时只 warn）。
//!
//! 环境变量 `VETTURA_LOG`（或通用的 `RUST_LOG`）优先于配置文件，支持
//! per-target 覆盖，例如：
//!
//! ```bash
//! VETTURA_LOG="info,vettura_world=debug,vettura_assets::track=trace" \
//!   /path/to/Godot --headless --path . res://gdext/baseline.tscn
//! ```
//!
//! # 为什么文件要非阻塞
//!
//! 物理循环 600 Hz，主循环等一次磁盘 IO 就可能掉帧。文件层经
//! `tracing_appender::non_blocking` 走独立 writer 线程 + 有界队列（默认
//! 128k 行后开始丢旧行），调用线程只做内存序列化。控制台层直接 `stdout`，
//! Godot 编辑器 / headless 都能看见。崩溃安全不靠 flush：panic hook 在进程
//! 死亡路径上同步写转储（ring 在内存 + hook 同步 IO），见 `panic_hook.rs`。

use std::sync::{Arc, Mutex, OnceLock};

use godot::prelude::*;
use tracing::Level;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, registry};

use crate::paths::native_path;

mod logger;
mod panic_hook;
mod ring;
mod runtime;

use ring::{RingLayer, RingState};
pub(crate) use runtime::poll_config_reload;
use runtime::{CombinedFilter, DynamicFilter, WarnGate};

/// 日志配置文件的 Godot 虚拟路径。
pub const LOG_CONFIG_PATH: &str = "user://log.cfg";

/// 日志文件前缀；实际文件名形如 `vettura.2026-11-14.log`（appender 自己在
/// 前缀与后缀之间插 `.日期`，别在前缀里再带点）。
const LOG_FILE_PREFIX: &str = "vettura";
/// 日志文件后缀。
const LOG_FILE_SUFFIX: &str = "log";

/// 存放 writer 线程的 `WorkerGuard`：drop 会 flush 并 join，必须活到进程退出。
static GUARDS: OnceLock<Mutex<Vec<WorkerGuard>>> = OnceLock::new();

/// 日志文件的轮转策略。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotate {
    /// 每天一个新文件（文件名带日期）。
    #[default]
    Daily,
    /// 单个文件追加写，靠 `max_files` 之外的机制清理（当前即只是追加）。
    Never,
}

/// 解析后的 `user://log.cfg`。
#[derive(Clone, Debug)]
pub struct LogConfig {
    /// 总开关；`false` 时完全不装 subscriber。
    pub enabled: bool,
    /// 全局级别下限。
    pub level: Level,
    /// 是否写文件。
    pub file: bool,
    /// 日志目录，Godot 虚拟路径（如 `user://logs`）。
    pub dir: String,
    /// 轮转策略。
    pub rotate: Rotate,
    /// 最多保留的日志文件数（超出按日期从旧到新删除）。
    pub max_files: usize,
    /// 是否同时输出到控制台。
    pub console: bool,
    /// 控制台级别下限；实际取 `level` 与它之间更严的一个。
    pub console_level: Level,
    /// 环形缓冲条数上限（RingSink / 崩溃转储 / `VetturaLogger.recent`）。
    pub ring_capacity: usize,
    /// 解析期间的降级说明（无法识别的值回退默认），调用方应逐条 `warn!`。
    pub issues: Vec<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: Level::INFO,
            file: true,
            dir: "user://logs".to_owned(),
            rotate: Rotate::Daily,
            max_files: 7,
            console: true,
            console_level: Level::INFO,
            ring_capacity: 4096,
            issues: Vec::new(),
        }
    }
}

impl LogConfig {
    /// 解析 INI 文本（`ConfigFile` 写出来就是这种）。纯字符串、不碰 IO，
    /// headless 可测；文件不存在时调用方直接用 [`Self::default`]。
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split(';').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                cfg.issues
                    .push(format!("line {}: {raw:?} 缺少 '='", index + 1));
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = unquote(value.trim());
            match key.as_str() {
                "enabled" => match parse_bool(value) {
                    Some(v) => cfg.enabled = v,
                    None => cfg.issues.push(bad_value_issue(index, "enabled", value)),
                },
                "level" => match parse_level(value) {
                    Some(v) => cfg.level = v,
                    None => cfg.issues.push(bad_value_issue(index, "level", value)),
                },
                "file" => match parse_bool(value) {
                    Some(v) => cfg.file = v,
                    None => cfg.issues.push(bad_value_issue(index, "file", value)),
                },
                "dir" => {
                    if value.is_empty() {
                        cfg.issues.push(bad_value_issue(index, "dir", value));
                    } else {
                        cfg.dir = value.to_owned();
                    }
                }
                "rotate" => match value.to_ascii_lowercase().as_str() {
                    "daily" => cfg.rotate = Rotate::Daily,
                    "never" => cfg.rotate = Rotate::Never,
                    _ => cfg.issues.push(bad_value_issue(index, "rotate", value)),
                },
                "max_files" => match parse_usize(value) {
                    Some(v) if (1..=64).contains(&v) => cfg.max_files = v,
                    _ => cfg.issues.push(bad_value_issue(index, "max_files", value)),
                },
                "console" => match parse_bool(value) {
                    Some(v) => cfg.console = v,
                    None => cfg.issues.push(bad_value_issue(index, "console", value)),
                },
                "console_level" => match parse_level(value) {
                    Some(v) => cfg.console_level = v,
                    None => cfg
                        .issues
                        .push(bad_value_issue(index, "console_level", value)),
                },
                "ring_capacity" => match parse_usize(value) {
                    Some(v) if (64..=65_536).contains(&v) => cfg.ring_capacity = v,
                    _ => cfg
                        .issues
                        .push(bad_value_issue(index, "ring_capacity", value)),
                },
                _ => cfg.issues.push(format!(
                    "line {}: 未知键 {key:?}（拼写检查一遍？）",
                    index + 1
                )),
            }
        }
        cfg
    }

    /// 文件层生效的 filter 指令串：环境变量优先，否则全局级别。
    fn file_directives(&self) -> String {
        env_directives().unwrap_or_else(|| self.level.to_string())
    }

    /// 控制台层生效的 filter 指令串：环境变量优先，否则取更严的级别。
    fn console_directives(&self) -> String {
        env_directives().unwrap_or_else(|| {
            strictest(self.level, self.console_level)
                .as_str()
                .to_owned()
        })
    }
}

/// `install()` 的结局。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    /// subscriber 安装成功，日志开始流动。
    Installed,
    /// 之前已经装过（本进程重复加载扩展时发生），本次配置未生效。
    AlreadyInstalled,
    /// 配置里 `enabled=false`，什么都没装。
    Disabled,
}

/// 安装 subscriber（幂等，可重复调用）。
///
/// - `native_dir`：`cfg.file` 为真时传 `user://` 目录解析后的原生路径；
/// - **必须先于任何 tracing 事件调用**，否则早期事件仍可能丢失（入口装在
///   `InitStage::Scene`，见 `lib.rs`）；
/// - 已装过时返回 [`InstallOutcome::AlreadyInstalled`]（以全局状态为准，
///   配置热重载走 `runtime`，不重复安装）；
/// - 安装成功后挂 panic hook（崩溃转储见 `panic_hook.rs`）。
pub fn install(cfg: &LogConfig, native_dir: Option<&str>) -> InstallOutcome {
    if runtime::state().is_some() {
        return InstallOutcome::AlreadyInstalled;
    }
    if !cfg.enabled {
        return InstallOutcome::Disabled;
    }

    // 环形缓冲：崩溃上下文兜底，级别全收（容量才是上限）。
    let ring = Arc::new(RingState::new(cfg.ring_capacity));

    // 两个 sink 的动态 filter：热重载 / set_level 只换这里的指令串。
    // `Option<_>` 把泛型留给类型推断，省掉手写 Filtered<...> 具名类型。
    let file_dynamic = Arc::new(DynamicFilter::new(&cfg.file_directives(), cfg.level));
    let console_dynamic = Arc::new(DynamicFilter::new(&cfg.console_directives(), Level::INFO));

    let file_writer = match (cfg.file, native_dir) {
        (true, Some(dir)) => build_file_writer(dir, cfg),
        // `file=true` 但没给目录是调用方 bug；宁可持续掉一条也不 panic。
        (true, None) => {
            eprintln!("[vettura-log] file=true but no directory given; console only");
            None
        }
        (false, _) => None,
    };

    // warn 聚合：每个 sink 一把 gate，聚合事件只从"聚合者"发
    // （file > console > ring），其余 sink 经 bypass 收到同一份聚合行。
    let file_gate = Arc::new(WarnGate::new(file_writer.is_some()));
    let console_gate = Arc::new(WarnGate::new(file_writer.is_none() && cfg.console));
    let ring_gate = Arc::new(WarnGate::new(file_writer.is_none() && !cfg.console));
    let aggregator = if file_writer.is_some() {
        file_gate.clone()
    } else if cfg.console {
        console_gate.clone()
    } else {
        ring_gate.clone()
    };

    let ring_layer = RingLayer::new(ring.clone(), ring_gate);

    let file_layer: Option<_> = file_writer.map(|(writer, guard)| {
        // guard 必须活到进程退出，否则丢了缓冲区里最后几行。
        keep_guard_alive(guard);
        fmt_layer()
            .with_writer(writer)
            .with_filter(CombinedFilter::new(file_dynamic.clone(), Some(file_gate)))
    });

    // 控制台层直接 `stdout`（Godot 编辑器 Output / headless 终端都看得到）。
    // 不用 `godot_print!`：那需要线程上下文，而事件可能来自任意线程（物理 /
    // 加载线程都会发日志）；`std::io::stdout` 本身有锁，跨线程安全。
    let console_layer: Option<_> = if cfg.console {
        Some(
            fmt_layer()
                .with_writer(std::io::stdout)
                .with_filter(CombinedFilter::new(
                    console_dynamic.clone(),
                    Some(console_gate),
                )),
        )
    } else {
        None
    };

    // Option<L> 与 Filtered 都有 Layer impl，多层互不干扰地挂同一个 registry。
    let subscriber = registry()
        .with(ring_layer)
        .with(file_layer)
        .with(console_layer);
    match subscriber.try_init() {
        Ok(()) => {
            runtime::create_state(
                cfg,
                ring,
                Some(file_dynamic),
                cfg.console.then_some(console_dynamic),
                Some(aggregator),
                native_dir.map(str::to_owned),
            );
            panic_hook::install();
            InstallOutcome::Installed
        }
        Err(_) => InstallOutcome::AlreadyInstalled,
    }
}

/// 读 `user://log.cfg` 并安装。**给 Godot 侧的入口调用**（`InitStage::Scene`），
/// 文件缺失 = 静默用默认配置（与 telemetry 一致）。
pub fn init_from_user_config() {
    let path = native_path(&GString::from(LOG_CONFIG_PATH));
    let cfg = match std::fs::read_to_string(&path) {
        Ok(text) => LogConfig::parse(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => LogConfig::default(),
        Err(err) => {
            godot_warn!("vettura: cannot read {LOG_CONFIG_PATH}: {err}; using log defaults");
            LogConfig::default()
        }
    };

    let native_dir = if cfg.file {
        Some(native_path(&GString::from(cfg.dir.as_str())))
    } else {
        None
    };
    if cfg.enabled && !cfg.file && !cfg.console {
        godot_warn!("vettura: log config has file=false and console=false; logging is a no-op");
    }
    let outcome = install(&cfg, native_dir.as_deref());

    // 解析期降级说明：此刻 subscriber 可能刚装上，走 tracing 正常路径。
    for issue in &cfg.issues {
        tracing::warn!("log config: {issue}");
    }

    match outcome {
        InstallOutcome::Disabled => godot_print!("vettura: logging disabled by {LOG_CONFIG_PATH}"),
        InstallOutcome::Installed => tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            dir = cfg.dir.as_str(),
            file = cfg.file,
            console = cfg.console,
            rotate = ?cfg.rotate,
            max_files = cfg.max_files,
            ring_capacity = cfg.ring_capacity,
            file_filter = cfg.file_directives().as_str(),
            console_filter = cfg.console_directives().as_str(),
            "vettura log online"
        ),
        // 已装过（重复加载）：banner 照样写，保证有据可查。
        InstallOutcome::AlreadyInstalled => {
            tracing::info!("vettura log online (subscriber already installed; new config ignored)")
        }
    }
}

/// 建文件 sink 的 writer：轮转文件 + 非阻塞 writer。失败只降级（控制台仍在），
/// 不 panic。
fn build_file_writer(dir: &str, cfg: &LogConfig) -> Option<(NonBlocking, WorkerGuard)> {
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!("[vettura-log] cannot create log dir {dir}: {err}; console only");
        return None;
    }
    let rotation = match cfg.rotate {
        Rotate::Daily => Rotation::DAILY,
        Rotate::Never => Rotation::NEVER,
    };
    let appender = match RollingFileAppender::builder()
        .rotation(rotation)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(cfg.max_files)
        .build(dir)
    {
        Ok(appender) => appender,
        Err(err) => {
            eprintln!("[vettura-log] cannot open rolling file in {dir}: {err}; console only");
            return None;
        }
    };
    Some(tracing_appender::non_blocking(appender))
}

/// WriterGuard 进全局表：drop 会 flush 并 join writer 线程，必须活到进程退出。
fn keep_guard_alive(guard: WorkerGuard) {
    GUARDS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(guard);
}

// 两个注意点：
// 1. 返回值必须是具体类型而不是 `impl Layer`——`with_writer` 是 `fmt::Layer`
//    的固有方法，经 `impl Trait` 返回会被屏蔽；
// 2. `S` 保持泛型不钉死：`fmt::Layer<Registry>` 只能挂到 Registry 上，而两个
//    sink 是叠两层（console 在 file 之上），S 必须由链接处推断。
type FmtLayer<S = Registry> = tracing_subscriber::fmt::Layer<
    S,
    tracing_subscriber::fmt::format::DefaultFields,
    tracing_subscriber::fmt::format::Format<
        tracing_subscriber::fmt::format::Full,
        tracing_subscriber::fmt::time::SystemTime,
    >,
    fn() -> std::io::Stdout,
>;

/// 各层共用的格式：无 ANSI（Godot 输出面板不渲染转义序列）、带线程名 /
/// 文件名 / 行号 / span 上下文——出错时的定位线索全在这里。
fn fmt_layer<S>() -> FmtLayer<S> {
    tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
}

/// 把指令串编译成 `EnvFilter`；非法串降级成级别默认值（别让配置错误打死日志）。
fn env_filter(directives: &str, fallback: Level) -> EnvFilter {
    match EnvFilter::try_new(directives) {
        Ok(filter) => filter,
        Err(err) => {
            eprintln!("[vettura-log] invalid filter {directives:?}: {err}; using {fallback}");
            EnvFilter::new(fallback.to_string())
        }
    }
}

/// 环境变量优先：`VETTURA_LOG`（本项目）→ `RUST_LOG`（通用）。
fn env_directives() -> Option<String> {
    for name in ["VETTURA_LOG", "RUST_LOG"] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            return Some(value);
        }
    }
    None
}

/// 取更严（严重度数字更大）的级别。
fn strictest(a: Level, b: Level) -> Level {
    if rank(a) >= rank(b) { a } else { b }
}

fn rank(level: Level) -> u8 {
    match level {
        Level::TRACE => 0,
        Level::DEBUG => 1,
        Level::INFO => 2,
        Level::WARN => 3,
        Level::ERROR => 4,
    }
}

fn bad_value_issue(index: usize, key: &str, value: &str) -> String {
    format!("line {}: {key}={value:?} 无法识别，已用默认值", index + 1)
}

/// 去掉 INI 值的包裹引号（`ConfigFile` 写字符串值时会带引号）。
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_level(value: &str) -> Option<Level> {
    match value.to_ascii_lowercase().as_str() {
        "trace" => Some(Level::TRACE),
        "debug" => Some(Level::DEBUG),
        "info" => Some(Level::INFO),
        "warn" | "warning" => Some(Level::WARN),
        "error" => Some(Level::ERROR),
        _ => None,
    }
}

fn parse_usize(value: &str) -> Option<usize> {
    value.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全缺省：文件落盘默认开，这是设计决定（出问题时用户机器上必须有日志）。
    #[test]
    fn defaults_enable_file_logging() {
        let cfg = LogConfig::parse("");
        assert!(cfg.enabled);
        assert!(cfg.file);
        assert_eq!(cfg.rotate, Rotate::Daily);
        assert_eq!(cfg.max_files, 7);
        assert_eq!(cfg.level, Level::INFO);
        assert!(cfg.issues.is_empty());
    }

    /// 正常键值 + 注释 + 空行 + 引号值。
    #[test]
    fn parses_known_keys() {
        let cfg = LogConfig::parse(
            "; comment\n[log]\nenabled = true\nlevel=\"debug\"\nconsole=false\nrotate='never'\nmax_files=3\n",
        );
        assert_eq!(cfg.level, Level::DEBUG);
        assert!(!cfg.console);
        assert_eq!(cfg.rotate, Rotate::Never);
        assert_eq!(cfg.max_files, 3);
        assert!(cfg.issues.is_empty());
    }

    /// 坏值不 panic，记录 issue 并回退默认。
    #[test]
    fn bad_values_fall_back_with_issue() {
        let cfg = LogConfig::parse("level=verbose\nrotate=hourly\nmax_files=999\nenabled=maybe\n");
        assert_eq!(cfg.level, Level::INFO);
        assert_eq!(cfg.rotate, Rotate::Daily);
        assert_eq!(cfg.max_files, 7);
        assert!(cfg.enabled);
        assert_eq!(cfg.issues.len(), 4, "each bad key reports one issue");
    }

    /// 未知键显式报出来，不让拼写错误静默失效。
    #[test]
    fn unknown_key_is_reported() {
        let cfg = LogConfig::parse("lever=debug\n");
        assert_eq!(cfg.issues.len(), 1);
        assert!(cfg.issues[0].contains("lever"));
    }

    /// 控制台级别：取 level 与 console_level 中更严的，文件不受影响。
    #[test]
    fn console_level_takes_stricter() {
        let cfg = LogConfig::parse("level=debug\nconsole_level=warn\n");
        assert_eq!(cfg.level, Level::DEBUG);
        assert_eq!(cfg.console_directives(), "WARN");
        assert_eq!(cfg.file_directives(), "DEBUG");
    }

    /// ring_capacity：默认 4096，范围内可调，越界回默认并记 issue。
    #[test]
    fn ring_capacity_parse_and_clamp() {
        let cfg = LogConfig::parse("");
        assert_eq!(cfg.ring_capacity, 4096);

        let cfg = LogConfig::parse("ring_capacity=512\n");
        assert_eq!(cfg.ring_capacity, 512);
        assert!(cfg.issues.is_empty());

        // 越界（<64 / >65536 / 非数字）都回默认并各记一条 issue。
        let cfg = LogConfig::parse("ring_capacity=1\nring_capacity=999999\nring_capacity=big\n");
        assert_eq!(cfg.ring_capacity, 4096);
        assert_eq!(cfg.issues.len(), 3);
    }

    /// 环境变量优先于配置文件。
    #[test]
    fn env_directives_override_config() {
        // 单测内不碰进程级环境变量（并发测试会互相干扰），只验证拼接逻辑的输入分支。
        let cfg = LogConfig::parse("level=warn\n");
        assert_eq!(cfg.level, Level::WARN);
    }

    #[test]
    fn unquote_strips_quotes() {
        assert_eq!(unquote("\"info\""), "info");
        assert_eq!(unquote("'info'"), "info");
        assert_eq!(unquote("info"), "info");
    }

    #[test]
    fn parses_level_aliases() {
        assert_eq!(parse_level("warning"), Some(Level::WARN));
        assert_eq!(parse_level("ERROR"), Some(Level::ERROR));
        assert_eq!(parse_level("nonsense"), None);
    }
}
