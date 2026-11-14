//! 运行期可变部分：动态 filter、warn 聚合、全局状态与配置热重载。
//!
//! # 为什么自研 [`DynamicFilter`]
//!
//! `tracing-subscriber 0.3.23` 还没有 `filter::reloading` 模块（那是 0.4 的事），
//! `fmt_layer.with_filter_reloading` 底层的 `reloading` 类型也未导出。自研很薄：
//! [`DynamicFilter`] 把 [`EnvFilter`] 塞进 `RwLock`，`Filter::enabled` 每次读锁
//! 拿最新指令；`callsite_enabled` 保持默认 `Interest::sometimes()`，tracing 的
//! 调用点缓存就不会把过滤结果钉死——换指令后下一条事件立即按新级别走。
//!
//! # warn 聚合（Gate）
//!
//! 每帧都可能刷的 warn（如 world 的 dropped contacts）会把 ring 和文件刷爆。
//! 每个 sink 一把 [`WarnGate`]：同一 callsite 在 5s 窗口内首条放行、重复只
//! 计数；窗口结束后由 [`poll_config_reload`] 低频 flush 出一条聚合事件（只从
//! "聚合者"那把 gate 发——file 优先，其次 console，最后 ring——其余 sink 会
//! 通过 bypass 收到同一份聚合行，不重复报数）。`vettura_log` 前缀 target 是
//! 我们自己的诊断事件，直接 bypass，防递归。
//!
//! # 状态与热重载
//!
//! [`LogState`] 是安装期创建的全局单例：当前配置、运行期级别覆盖
//! （`VetturaLogger.set_level`）、各 sink 的动态 filter、聚合者 gate、ring、
//! 配置文件 mtime。`lib.rs` 的 `on_main_loop_frame` 每帧调
//! [`poll_config_reload`]（内部 0.5s 节流）：flush 聚合 + mtime 变了就重读
//! `user://log.cfg`。热重载只应用级别类键（level/console_level/ring_capacity）；
//! sink 结构（enabled/file/dir/rotate/max_files/console）变化只能重启生效，
//! 只 warn 不重建。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use tracing::Level;
use tracing::callsite::Identifier;
use tracing::metadata::Metadata;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Context, Filter};

use super::ring::RingState;
use super::{LOG_CONFIG_PATH, LogConfig, env_directives, env_filter, strictest};

/// 聚合/诊断事件的专用 target 前缀；gate 见到它直接放行（防递归）。
pub(crate) const DIAGNOSTIC_TARGET: &str = "vettura_log";

/// 同类 warn 的聚合窗口。
const WARN_WINDOW: Duration = Duration::from_secs(5);

/// 配置轮询 / 聚合 flush 的间隔（与 telemetry 的 mtime 轮询同一节奏）。
const POLL_INTERVAL: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------- 动态 filter

/// `RwLock<EnvFilter>`：运行期可换指令串的 filter（见模块文档）。
pub(crate) struct DynamicFilter {
    inner: RwLock<EnvFilter>,
}

impl DynamicFilter {
    pub(crate) fn new(directives: &str, fallback: Level) -> Self {
        Self {
            inner: RwLock::new(env_filter(directives, fallback)),
        }
    }

    /// 换指令串；解析失败降级到 `fallback`（别让热重载打死日志）。
    pub(crate) fn replace(&self, directives: &str, fallback: Level) {
        *self.lock_write() = env_filter(directives, fallback);
    }

    fn lock_write(&self) -> std::sync::RwLockWriteGuard<'_, EnvFilter> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<S> Filter<S> for DynamicFilter {
    fn enabled(&self, metadata: &Metadata<'_>, ctx: &Context<'_, S>) -> bool {
        // 全限定调用：`EnvFilter` 有个同名固有方法（ctx 按值收），会被优先解析。
        Filter::enabled(
            &*self
                .inner
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            metadata,
            ctx,
        )
    }
}

/// sink 的完整 filter：先过级别指令（动态），再过 warn 聚合 gate。
pub(crate) struct CombinedFilter {
    dynamic: Arc<DynamicFilter>,
    gate: Option<Arc<WarnGate>>,
}

impl CombinedFilter {
    pub(crate) fn new(dynamic: Arc<DynamicFilter>, gate: Option<Arc<WarnGate>>) -> Self {
        Self { dynamic, gate }
    }
}

impl<S> Filter<S> for CombinedFilter {
    fn enabled(&self, metadata: &Metadata<'_>, ctx: &Context<'_, S>) -> bool {
        // 顺序很重要：级别先挡（被挡的事件不该计入聚合），gate 后挡。
        self.dynamic.enabled(metadata, ctx)
            && self.gate.as_ref().is_none_or(|gate| gate.allow(metadata))
    }
}

// ---------------------------------------------------------------- warn 聚合

/// 同类 warn/error 聚合 gate（每个 sink 一把，语义见模块文档）。
pub(crate) struct WarnGate {
    /// 是否是聚合者（唯一发聚合事件的 gate）。
    aggregator: bool,
    inner: Mutex<GateInner>,
}

struct GateInner {
    entries: HashMap<Identifier, GateEntry>,
    /// 窗口结束待 flush 的聚合（只有 aggregator 会积累）。
    pending: Vec<Aggregate>,
}

#[derive(Clone)]
struct GateEntry {
    /// 窗口内是否已放行首条。
    shown: bool,
    /// 窗口内已抑制的条数。
    suppressed: u32,
    window_start: Instant,
    snapshot: MetaSnapshot,
}

/// 调用点元数据快照：聚合行里指认"是谁在刷"。
/// `Metadata::target/file` 的生命周期绑定在 `&Metadata<'_>` 上，进不了
/// 长生命周期表——拷成 String（表项数量 = 独立 warn 调用点数，很小）。
#[derive(Clone)]
struct MetaSnapshot {
    target: String,
    level: Level,
    file: Option<String>,
    line: Option<u32>,
}

/// 一条待 flush 的聚合。
pub(crate) struct Aggregate {
    snapshot: MetaSnapshot,
    suppressed: u32,
}

impl WarnGate {
    pub(crate) fn new(aggregator: bool) -> Self {
        Self {
            aggregator,
            inner: Mutex::new(GateInner {
                entries: HashMap::new(),
                pending: Vec::new(),
            }),
        }
    }

    /// 单条事件的 gate 决定：true = 放行。只聚合 WARN/ERROR。
    pub(crate) fn allow(&self, metadata: &Metadata<'_>) -> bool {
        if !matches!(*metadata.level(), Level::WARN | Level::ERROR) {
            return true;
        }
        if metadata.target().starts_with(DIAGNOSTIC_TARGET) {
            return true;
        }
        self.allow_at(metadata, Instant::now())
    }

    fn allow_at(&self, metadata: &Metadata<'_>, now: Instant) -> bool {
        let mut inner = lock(&self.inner);
        let snapshot = MetaSnapshot {
            target: metadata.target().to_owned(),
            level: *metadata.level(),
            file: metadata.file().map(str::to_owned),
            line: metadata.line(),
        };
        let key = metadata.callsite();
        // 先把现有表项 clone 出来再做决定，避免同时持有 entries 与 pending 的可变借用。
        let existing = inner.entries.get(&key).cloned();
        match existing {
            None => {
                inner.entries.insert(
                    key,
                    GateEntry {
                        shown: true,
                        suppressed: 0,
                        window_start: now,
                        snapshot,
                    },
                );
                true
            }
            Some(entry) => {
                let step = window_step(entry.shown, entry.window_start, now);
                match step {
                    WindowStep::Allow => {
                        inner.entries.insert(
                            key,
                            GateEntry {
                                shown: true,
                                ..entry.clone()
                            },
                        );
                        true
                    }
                    WindowStep::Suppress => {
                        inner.entries.insert(
                            key,
                            GateEntry {
                                suppressed: entry.suppressed + 1,
                                ..entry.clone()
                            },
                        );
                        false
                    }
                    WindowStep::AllowAndFlush => {
                        if self.aggregator && entry.suppressed > 0 {
                            inner.pending.push(Aggregate {
                                snapshot: entry.snapshot.clone(),
                                suppressed: entry.suppressed,
                            });
                        }
                        inner.entries.insert(
                            key,
                            GateEntry {
                                shown: true,
                                suppressed: 0,
                                window_start: now,
                                snapshot,
                            },
                        );
                        true
                    }
                }
            }
        }
    }

    /// poll 每跳调一次：aggregator 把"窗口已过但还没新事件触发 flush"的存量
    /// 收进 pending（否则刷完就停的 warn 永远等不到聚合行）；所有 gate 顺手
    /// 清掉过期表项，HashMap 不随时间无限增长。
    pub(crate) fn poll_tick(&self, now: Instant) {
        let mut inner = lock(&self.inner);
        let expired: Vec<Identifier> = inner
            .entries
            .iter()
            .filter(|(_, entry)| now.duration_since(entry.window_start) >= WARN_WINDOW)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            // if-let 链里一并判聚合者与存量，避免 clippy collapse 提示。
            if let Some(entry) = inner.entries.remove(&key)
                && self.aggregator
                && entry.suppressed > 0
            {
                inner.pending.push(Aggregate {
                    snapshot: entry.snapshot,
                    suppressed: entry.suppressed,
                });
            }
        }
    }

    /// 取走 pending 聚合（锁已释放后再发事件，避免重入死锁）。
    pub(crate) fn take_pending(&self) -> Vec<Aggregate> {
        let mut inner = lock(&self.inner);
        std::mem::take(&mut inner.pending)
    }
}

/// 窗口状态机的纯逻辑（可测，不依赖 callsite）。
enum WindowStep {
    /// 窗口内首条：放行。
    Allow,
    /// 窗口内重复：抑制。
    Suppress,
    /// 窗口已过的新一条：flush 旧窗口计数，重开窗口并放行本条。
    AllowAndFlush,
}

fn window_step(shown: bool, window_start: Instant, now: Instant) -> WindowStep {
    let elapsed = now.saturating_duration_since(window_start);
    if elapsed < WARN_WINDOW {
        return if shown {
            WindowStep::Suppress
        } else {
            WindowStep::Allow
        };
    }
    WindowStep::AllowAndFlush
}

// ---------------------------------------------------------------- 全局状态

/// 安装期创建的全局日志状态（见模块文档）。
pub(crate) struct LogState {
    pub(crate) cfg: Mutex<LogConfig>,
    /// `VetturaLogger.set_level` 的运行期覆盖（`target` 空 = 全局级别）。
    overrides: Mutex<Vec<LevelOverride>>,
    file_filter: Option<Arc<DynamicFilter>>,
    console_filter: Option<Arc<DynamicFilter>>,
    /// 聚合者 gate（file > console > ring 中存在的那把）。
    aggregator: Option<Arc<WarnGate>>,
    pub(crate) ring: Arc<RingState>,
    /// 日志目录的原生路径（panic hook 直接用，绕开 Godot API）。
    pub(crate) native_dir: Option<String>,
    /// `log.cfg` 的原生路径（poll 用，避免每跳调 Godot）。
    cfg_path: String,
    cfg_mtime: Mutex<Option<SystemTime>>,
    next_poll: Mutex<Option<Instant>>,
}

pub(crate) struct LevelOverride {
    target: String,
    level: Level,
}

static STATE: OnceLock<Arc<LogState>> = OnceLock::new();

pub(crate) fn state() -> Option<&'static Arc<LogState>> {
    STATE.get()
}

/// Mutex 取锁，毒化也取回数据（诊断路径不因毒化中断）。
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 安装成功后建全局状态（幂等：已存在则忽略，与 subscriber 的幂等一致）。
pub(crate) fn create_state(
    cfg: &LogConfig,
    ring: Arc<RingState>,
    file_filter: Option<Arc<DynamicFilter>>,
    console_filter: Option<Arc<DynamicFilter>>,
    aggregator: Option<Arc<WarnGate>>,
    native_dir: Option<String>,
) {
    let cfg_path = native_path_of(LOG_CONFIG_PATH);
    let cfg_mtime = std::fs::metadata(&cfg_path)
        .and_then(|metadata| metadata.modified())
        .ok();
    let state = Arc::new(LogState {
        cfg: Mutex::new(cfg.clone()),
        overrides: Mutex::new(Vec::new()),
        file_filter,
        console_filter,
        aggregator,
        ring,
        native_dir,
        cfg_path,
        cfg_mtime: Mutex::new(cfg_mtime),
        next_poll: Mutex::new(None),
    });
    let _ = STATE.set(state);
}

/// 配置文件的原生路径。安装期调用（有 Godot 单例）；
/// `state.cfg_path` 存下后 poll 不再碰 Godot。
fn native_path_of(virtual_path: &str) -> String {
    use crate::paths::native_path;
    use godot::prelude::GString;
    native_path(&GString::from(virtual_path))
}

/// 当前生效的 filter 指令串：环境变量 > 全局覆盖（set_level）> 配置级别，
/// per-target 覆盖排在最后（EnvFilter 后写的指令赢）。
/// 返回 (指令串, 指令串解析失败时的降级级别)。
pub(crate) fn build_directives(
    env: Option<String>,
    cfg: &LogConfig,
    overrides: &[LevelOverride],
    console: bool,
) -> (String, Level) {
    let global_override = overrides
        .iter()
        .find(|entry| entry.target.is_empty())
        .map(|entry| entry.level);
    let fallback = global_override.unwrap_or(cfg.level);

    let mut directives = match env {
        // 环境变量优先（与文件/控制台的既定规则一致）。
        Some(env) => env,
        None => match global_override {
            Some(level) => level.to_string(),
            None => {
                if console {
                    // 控制台取更严者（与安装期规则一致）。
                    strictest(cfg.level, cfg.console_level).to_string()
                } else {
                    cfg.level.to_string()
                }
            }
        },
    };
    for entry in overrides.iter().filter(|entry| !entry.target.is_empty()) {
        directives.push_str(&format!(",{}={}", entry.target, entry.level));
    }
    (directives, fallback)
}

/// 把当前指令串应用到两个 sink 的动态 filter。
fn refresh_filters(state: &LogState) {
    let (directives, fallback) = {
        let overrides = lock(&state.overrides);
        let cfg = lock(&state.cfg);
        build_directives(env_directives(), &cfg, &overrides, false)
    };
    if let Some(filter) = &state.file_filter {
        filter.replace(&directives, fallback);
    }
    let (console_directives, console_fallback) = {
        let overrides = lock(&state.overrides);
        let cfg = lock(&state.cfg);
        build_directives(env_directives(), &cfg, &overrides, true)
    };
    if let Some(filter) = &state.console_filter {
        filter.replace(&console_directives, console_fallback);
    }
}

/// 运行期级别覆盖（`VetturaLogger.set_level`）：`target` 空 = 全局。
/// 覆盖是运行期状态，热重载后仍保留。
pub(crate) fn apply_override(state: &LogState, target: &str, level: Level) {
    {
        let mut overrides = lock(&state.overrides);
        if let Some(entry) = overrides.iter_mut().find(|entry| entry.target == target) {
            entry.level = level;
        } else {
            overrides.push(LevelOverride {
                target: target.to_owned(),
                level,
            });
        }
    }
    refresh_filters(state);
    tracing::info!(
        target: DIAGNOSTIC_TARGET,
        set_target = target,
        set_level = %level,
        "运行期级别覆盖已应用"
    );
}

/// 立即重读 `user://log.cfg` 并热应用级别类键（mtime 轮询与 `reload()` 共用）。
/// 缺文件 / 读失败：warn 一次并保留当前配置。
pub(crate) fn reload_now(state: &LogState) {
    let text = match std::fs::read_to_string(&state.cfg_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let had_file = lock(&state.cfg_mtime).is_some();
            *lock(&state.cfg_mtime) = None;
            if had_file {
                tracing::warn!(
                    target: DIAGNOSTIC_TARGET,
                    "{LOG_CONFIG_PATH} 已删除，保留当前配置"
                );
            }
            return;
        }
        Err(err) => {
            tracing::warn!(
                target: DIAGNOSTIC_TARGET,
                "cannot read {LOG_CONFIG_PATH}: {err} (keeping current config)"
            );
            return;
        }
    };

    let new_cfg = LogConfig::parse(&text);
    for issue in &new_cfg.issues {
        tracing::warn!(target: DIAGNOSTIC_TARGET, "log config: {issue}");
    }

    let old = lock(&state.cfg).clone();
    // sink 结构（enabled/file/dir/rotate/max_files/console）安装期定死，
    // 热重载只换级别；变化的部分 warn 提醒重启。
    let structure_changed = old.enabled != new_cfg.enabled
        || old.file != new_cfg.file
        || old.dir != new_cfg.dir
        || old.rotate != new_cfg.rotate
        || old.max_files != new_cfg.max_files
        || old.console != new_cfg.console;
    if structure_changed {
        tracing::warn!(
            target: DIAGNOSTIC_TARGET,
            "log config: sink 结构变化（enabled/file/dir/rotate/max_files/console）需重启生效，本次只热更级别"
        );
    }

    if new_cfg.ring_capacity != old.ring_capacity {
        state.ring.set_capacity(new_cfg.ring_capacity);
    }
    *lock(&state.cfg) = new_cfg.clone();
    refresh_filters(state);
    *lock(&state.cfg_mtime) = std::fs::metadata(&state.cfg_path)
        .and_then(|metadata| metadata.modified())
        .ok();

    tracing::info!(
        target: DIAGNOSTIC_TARGET,
        file_level = %new_cfg.level,
        console_level = %new_cfg.console_level,
        ring_capacity = new_cfg.ring_capacity,
        "log config reloaded"
    );
}

/// `on_main_loop_frame` 的入口（0.5s 节流）：聚合 flush + 配置 mtime 轮询。
pub(crate) fn poll_config_reload() {
    let Some(state) = STATE.get() else {
        return;
    };
    let now = Instant::now();
    {
        let mut next = lock(&state.next_poll);
        if next.is_some_and(|due| now < due) {
            return;
        }
        *next = Some(now + POLL_INTERVAL);
    }

    // 聚合 flush：先收过期窗口，再取 pending 发事件（锁已释放，无重入死锁）。
    if let Some(gate) = &state.aggregator {
        gate.poll_tick(now);
        for aggregate in gate.take_pending() {
            emit_aggregate(aggregate);
        }
    }

    // 配置热重载：mtime 变了才重读（与 telemetry 同模式）。
    let mtime = std::fs::metadata(&state.cfg_path)
        .and_then(|metadata| metadata.modified())
        .ok();
    if mtime != *lock(&state.cfg_mtime) {
        reload_now(state);
    }
}

/// 发一条聚合事件：`vettura_log` 前缀 target 保证各 sink 的 gate 都 bypass。
fn emit_aggregate(aggregate: Aggregate) {
    let snapshot = aggregate.snapshot;
    tracing::warn!(
        target: DIAGNOSTIC_TARGET,
        original_target = snapshot.target.as_str(),
        original_level = %snapshot.level,
        file = snapshot.file.as_deref().unwrap_or(""),
        line = snapshot.line.unwrap_or(0),
        suppressed = aggregate.suppressed,
        "同类日志聚合：5s 窗口内抑制了 {} 条", aggregate.suppressed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(level: Level, console_level: Level) -> LogConfig {
        let mut config = LogConfig::default();
        config.level = level;
        config.console_level = console_level;
        config
    }

    fn override_of(target: &str, level: Level) -> LevelOverride {
        LevelOverride {
            target: target.to_owned(),
            level,
        }
    }

    /// 文件级别：环境变量 > 全局覆盖 > 配置级别；per-target 追加在后面。
    #[test]
    fn file_directives_priority() {
        // 基本路径：无环境变量、无覆盖 → 配置级别。
        let (d, f) = build_directives(None, &cfg(Level::INFO, Level::INFO), &[], false);
        assert_eq!(d, "INFO");
        assert_eq!(f, Level::INFO);

        // 环境变量最高，且作为 EnvFilter 整串生效（含 per-target）。
        let (d, f) = build_directives(
            Some("warn,vettura_kn5=trace".to_owned()),
            &cfg(Level::DEBUG, Level::DEBUG),
            &[],
            false,
        );
        assert_eq!(d, "warn,vettura_kn5=trace");
        assert_eq!(f, Level::DEBUG);

        // 全局覆盖压过配置级别，per-target 追加在后（后写的赢）。
        let (d, f) = build_directives(
            None,
            &cfg(Level::INFO, Level::INFO),
            &[
                override_of("", Level::ERROR),
                override_of("vettura_world", Level::DEBUG),
            ],
            false,
        );
        assert_eq!(d, "ERROR,vettura_world=DEBUG");
        assert_eq!(f, Level::ERROR);
    }

    /// 控制台级别：无环境变量/覆盖时取 level 与 console_level 更严者。
    #[test]
    fn console_directives_take_stricter() {
        let (d, _) = build_directives(None, &cfg(Level::DEBUG, Level::WARN), &[], true);
        assert_eq!(d, "WARN");
        let (d, _) = build_directives(None, &cfg(Level::DEBUG, Level::WARN), &[], false);
        assert_eq!(d, "DEBUG");
        // 有环境变量时控制台不再取严（环境变量整串生效）。
        let (d, _) = build_directives(
            Some("info".to_owned()),
            &cfg(Level::DEBUG, Level::WARN),
            &[],
            true,
        );
        assert_eq!(d, "info");
    }

    /// 窗口状态机：首条放行、重复抑制、过期 flush 后重开窗口。
    #[test]
    fn window_step_first_allow_repeat_suppress() {
        let now = Instant::now();
        assert!(matches!(window_step(false, now, now), WindowStep::Allow));
        assert!(matches!(window_step(true, now, now), WindowStep::Suppress));
    }

    #[test]
    fn window_step_expires_and_flushes() {
        let now = Instant::now();
        let expired = now - WARN_WINDOW - Duration::from_millis(1);
        assert!(matches!(
            window_step(true, expired, now),
            WindowStep::AllowAndFlush
        ));
        // 窗口过期但没有抑制存量：重开窗口即可。
        assert!(matches!(
            window_step(true, expired, now),
            WindowStep::AllowAndFlush
        ));
        // saturating：window_start 在"未来"（时钟回拨）也当没过期处理。
        let future = now + WARN_WINDOW;
        assert!(matches!(
            window_step(true, future, now),
            WindowStep::Suppress
        ));
    }
}
