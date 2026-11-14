//! 环形缓冲 sink：把最近 N 条日志留在内存里。
//!
//! 用途是崩溃上下文：panic hook 把缓冲内容追加写 `user://logs/last-crash.log`
//! （见 [`super::panic_hook`]），`VetturaLogger.recent/dump` 也能在运行期读。
//! 容量有限（默认 4096 条，`log.cfg` 的 `ring_capacity` 可调），旧的先滚出；
//! 单行超长截断，防止一个巨型字段把整个缓冲吃掉。
//!
//! ring **不做级别过滤**（越全越好，容量才是上限），但走同一套 warn 聚合
//! gate（见 [`super::runtime::WarnGate`]），免得每帧刷的 warn 把有价值的旧
//! 日志滚出去。
//!
//! 行格式（紧凑、机器可读，供 recent/dump/last-crash.log）：
//!
//! ```text
//! 2026-11-14T09:30:12.345678Z WARN vettura_assets::track 缺模型 model=xxx  [track_load{root="/mods/daytona"}]
//! ```

use std::collections::VecDeque;
use std::fmt;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use tracing::{
    Subscriber,
    event::Event,
    field::{Field, Visit},
};
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::fmt::format::{DefaultFields, Writer as FmtWriter};
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::layer::Context;
use tracing_subscriber::{Layer, registry::LookupSpan};

use super::runtime::WarnGate;

/// 单行上限（字节）：超过就截断加省略标记，别让一个字段吃掉整个缓冲。
const MAX_LINE_BYTES: usize = 8 * 1024;

/// 环形缓冲本体（全局共享：RingLayer 写，panic hook 与 `VetturaLogger` 读）。
pub(crate) struct RingState {
    inner: Mutex<RingInner>,
}

struct RingInner {
    lines: VecDeque<String>,
    capacity: usize,
}

impl RingState {
    pub(crate) fn new(capacity: usize) -> Self {
        // with_capacity 别照单全收大容量，普通规模先够用。
        Self {
            inner: Mutex::new(RingInner {
                lines: VecDeque::with_capacity(capacity.min(4096)),
                capacity,
            }),
        }
    }

    /// 热重载换容量：保留最近的 `capacity` 条。
    pub(crate) fn set_capacity(&self, capacity: usize) {
        let mut inner = lock(&self.inner);
        inner.capacity = capacity;
        while inner.lines.len() > capacity {
            inner.lines.pop_front();
        }
    }

    pub(crate) fn push(&self, line: String) {
        let mut inner = lock(&self.inner);
        inner.lines.push_back(line);
        while inner.lines.len() > inner.capacity {
            inner.lines.pop_front();
        }
    }

    /// 最近 `count` 条，按时间正序（旧在前）。`usize::MAX` = 全部。
    pub(crate) fn recent(&self, count: usize) -> Vec<String> {
        let inner = lock(&self.inner);
        inner
            .lines
            .iter()
            .rev()
            .take(count)
            .rev()
            .cloned()
            .collect()
    }

    /// 全量转储，一行一条（不含结尾换行）。
    pub(crate) fn dump(&self) -> String {
        let inner = lock(&self.inner);
        inner
            .lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Mutex 取锁，毒化也取回数据（转储比"锁状态一致性"重要）。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// ring 的 `Layer`：只记事件（span 建立时无需缓存），级别全收，gate 挡聚合。
pub(crate) struct RingLayer {
    state: Arc<RingState>,
    gate: Arc<WarnGate>,
}

impl RingLayer {
    pub(crate) fn new(state: Arc<RingState>, gate: Arc<WarnGate>) -> Self {
        Self { state, gate }
    }
}

impl<S> Layer<S> for RingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if !self.gate.allow(event.metadata()) {
            return;
        }
        self.state.push(format_ring_line(event, &ctx));
    }
}

/// 一条紧凑行：`时间 LEVEL target 消息 字段…  [span{字段} > span…]`。
fn format_ring_line<S>(event: &Event<'_>, ctx: &Context<'_, S>) -> String
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let meta = event.metadata();
    let mut line = String::new();
    {
        let mut writer = FmtWriter::new(&mut line);
        // SystemTime 内部取 now()；写失败（时间倒流等）就留空，不因格式化打断记录。
        let _ = SystemTime.format_time(&mut writer);
    }
    let _ = write!(line, " {} {}", meta.level(), meta.target());

    // 事件字段：message 单独做消息，其余 `key=value` 跟在后面。
    let mut visitor = LineVisitor::default();
    event.record(&mut visitor);
    if let Some(message) = visitor.message {
        line.push(' ');
        line.push_str(&message);
    }
    if !visitor.fields.is_empty() {
        line.push(' ');
        line.push_str(&visitor.fields);
    }

    // span 上下文：格式化好的字段值在 extensions 里（fmt 层 on_new_span 时写入），
    // 只读现成的，ring 自己不重复格式化。fmt 层因级别过滤没见过的 span 只有名字。
    if let Some(scope) = ctx.event_scope(event) {
        let mut spans = scope.from_root().peekable();
        if spans.peek().is_some() {
            line.push_str("  [");
            for span in spans {
                if !line.ends_with('[') {
                    line.push_str(" > ");
                }
                line.push_str(span.name());
                if let Some(fields) = span.extensions().get::<FormattedFields<DefaultFields>>()
                    && !fields.fields.is_empty()
                {
                    let _ = write!(line, "{{{}}}", fields.fields);
                }
            }
            line.push(']');
        }
    }

    cap_line(line)
}

/// 超长行截断（按 UTF-8 字符边界，别切出半个汉字）。
fn cap_line(mut line: String) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line;
    }
    let mut cut = MAX_LINE_BYTES;
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    line.push_str("…[截断]");
    line
}

/// 事件字段收集器：`message` 提出来做消息文本，其余拼成 `key=value`。
#[derive(Default)]
struct LineVisitor {
    message: Option<String>,
    fields: String,
}

impl LineVisitor {
    fn push_field(&mut self, field: &Field, value: String) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(field.name());
        self.fields.push('=');
        self.fields.push_str(&value);
    }
}

impl Visit for LineVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.push_field(field, value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            // `format_args!` 的 Debug 就是格式化后的文本本身（无引号）。
            self.message.get_or_insert_with(|| format!("{value:?}"));
        } else {
            self.push_field(field, format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let ring = RingState::new(3);
        for index in 0..5 {
            ring.push(format!("line-{index}"));
        }
        assert_eq!(ring.recent(usize::MAX).len(), 3);
        assert_eq!(ring.recent(10), vec!["line-2", "line-3", "line-4"]);
    }

    /// recent 的顺序：旧在前、新在后（dump/recent 都按时间正序读）。
    #[test]
    fn recent_returns_last_in_chronological_order() {
        let ring = RingState::new(10);
        for index in 0..4 {
            ring.push(format!("line-{index}"));
        }
        assert_eq!(ring.recent(2), vec!["line-2", "line-3"]);
        assert_eq!(ring.recent(0), Vec::<String>::new());
    }

    #[test]
    fn dump_joins_all_lines() {
        let ring = RingState::new(10);
        ring.push("a".to_owned());
        ring.push("b".to_owned());
        assert_eq!(ring.dump(), "a\nb");
    }

    /// 热重载换容量：保留最新的一段，旧的滚出。
    #[test]
    fn set_capacity_keeps_newest() {
        let ring = RingState::new(5);
        for index in 0..5 {
            ring.push(format!("line-{index}"));
        }
        ring.set_capacity(2);
        assert_eq!(ring.recent(usize::MAX).len(), 2);
        assert_eq!(ring.recent(10), vec!["line-3", "line-4"]);
        ring.set_capacity(8);
        ring.push("new".to_owned());
        assert_eq!(ring.recent(10), vec!["line-3", "line-4", "new"]);
    }

    #[test]
    fn caps_long_lines_on_char_boundary() {
        let long = "汉".repeat(5000); // 15000 字节，超 8KiB
        let capped = cap_line(long);
        assert!(capped.len() <= MAX_LINE_BYTES + "…[截断]".len());
        assert!(capped.ends_with("…[截断]"));
        // 截断点不能落在多字节字符中间（String::truncate 会 panic，这里没有）。
        assert!(!capped.starts_with('\u{fffd}'));
    }

    #[test]
    fn short_lines_pass_through() {
        assert_eq!(cap_line("ok".to_owned()), "ok");
    }
}
