//! GDScript 侧日志 API（`VetturaLogger`）：运行期调级别、读最近日志、导出转储。
//!
//! RefCounted，不进场景树：`var logger := VetturaLogger.new()` 即用。全局状态
//! 都在 logging 模块（`runtime::state`），本类是薄封装；subscriber 未安装
//! （`log.cfg enabled=false`）时所有方法安静地返回空值 / false，不报错。
//!
//! ```gdscript
//! var logger := VetturaLogger.new()
//! logger.set_level("vettura_world", "debug")   # 运行期级别覆盖
//! var tail := logger.recent(50)                 # 最近 50 行（崩溃排查入口）
//! logger.dump_to_file("user://log-dump.txt")    # 全量导出
//! ```

use godot::prelude::*;

use super::runtime;
use crate::paths::native_path;

/// GDScript 侧日志入口（热重载 / 级别覆盖 / 环形缓冲读取）。
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub(crate) struct VetturaLogger;

#[godot_api]
impl VetturaLogger {
    /// 立即重读 `user://log.cfg` 并热应用级别类键（与 mtime 轮询同一实现；
    /// sink 结构变化只 warn 不重建）。返回 `false` = 日志被禁用、未安装。
    #[func]
    pub fn reload(&self) -> bool {
        let Some(state) = runtime::state() else {
            return false;
        };
        runtime::reload_now(state);
        true
    }

    /// 运行期级别覆盖：`target` 为空 = 全局；`level` 取 `trace..error`。
    /// 覆盖在热重载后仍保留；返回 `false` = 级别名不认识或日志被禁用。
    #[func]
    pub fn set_level(&self, target: GString, level: GString) -> bool {
        let Some(parsed) = super::parse_level(&level.to_string()) else {
            return false;
        };
        let Some(state) = runtime::state() else {
            return false;
        };
        runtime::apply_override(state, &target.to_string(), parsed);
        true
    }

    /// 日志目录（Godot 虚拟路径，如 `user://logs`）。
    #[func]
    pub fn log_dir(&self) -> GString {
        runtime::state()
            .map(|state| GString::from(runtime::lock(&state.cfg).dir.as_str()))
            .unwrap_or_default()
    }

    /// 环形缓冲最近 `count` 条（时间正序；`count <= 0` = 全部）。
    #[func]
    pub fn recent(&self, count: i32) -> PackedStringArray {
        let Some(state) = runtime::state() else {
            return PackedStringArray::new();
        };
        let take = if count <= 0 {
            usize::MAX
        } else {
            count as usize
        };
        state
            .ring
            .recent(take)
            .iter()
            .map(|line| GString::from(line.as_str()))
            .collect::<PackedStringArray>()
    }

    /// 环形缓冲全量转储（一行一条）。
    #[func]
    pub fn dump(&self) -> GString {
        runtime::state()
            .map(|state| GString::from(state.ring.dump().as_str()))
            .unwrap_or_default()
    }

    /// 转储写到 Godot 虚拟路径（父目录自动创建）；返回是否成功。
    #[func]
    pub fn dump_to_file(&self, path: GString) -> bool {
        let Some(state) = runtime::state() else {
            return false;
        };
        let native = native_path(&path);
        if let Some(parent) = std::path::Path::new(&native).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&native, state.ring.dump()).is_ok()
    }

    /// 仅测试用：触发一个 Rust panic，验证 panic hook 的 `last-crash.log` 转储。
    /// panic 会被 godot-rust 在调用边界捕获（进程不崩），但调用它的 GDScript
    /// 函数会被中止——想继续验证就先把 quit 定时器布置好再调它。
    #[func]
    pub fn simulate_panic(&self) {
        panic!("vettura-log: simulated panic (last-crash.log verification)");
    }
}
