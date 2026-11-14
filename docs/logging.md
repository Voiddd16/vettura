# 日志模块

全系统运行状态记录 + 出错定位线索。设计决策过程见本文"取舍"一节；实现已落地
（Phase 1：骨架可用；Phase 2：环形缓冲 / `VetturaLogger` / panic 转储 / 热重载 /
warn 聚合）。

- **实现位置**：[`gdext/src/logging/`](../gdext/src/logging/mod.rs)（不单独开
  crate，理由见下）；
- **触发点**：GDExtension `InitStage::Scene`（`gdext/src/lib.rs` 的
  `on_stage_init`）安装全局 `tracing` subscriber；
- **配置**：`user://log.cfg`（Godot `ConfigFile` / INI）；优先级
  `VetturaLogger.set_level` > `VETTURA_LOG` / `RUST_LOG` > 配置文件 > 默认值，
  级别类键支持热重载；
- **依赖**：`tracing` + `tracing-subscriber` + `tracing-appender`（workspace 级）。

## 1. 目标与非目标

**目标**

1. 记录运行状态：加载（KN5/ACD/INI）→ 组装（CarFolder/TrackModule）→ 模拟
   （World/step）→ 渲染桥接（gdext）→ 音频（FMOD）；
2. 出错时给线索：span 上下文（哪条赛道 / 哪个 layout / 哪个文件）、文件名 +
   行号、线程名、启动 banner；
3. 运行期可调：级别 / 开关通过配置与环境变量调整，不重编译；
4. 接近零成本：600 Hz 物理主循环不等 IO。

**非目标（后续阶段）**

- 远程日志收集（留 sink 扩展位）；
- Metrics 时序（`telemetry` 负责）；
- 接管 GDScript 侧 `print`；
- JSON 格式、Android logcat（见 §8，Phase 3 可选项）。

## 2. 架构与边界

```
业务 crate：use tracing::*（只依赖 workspace 的 tracing，不加其它依赖）
   │ events + spans
   ▼
gdext/src/logging（filter + 多 sink fan-out；唯一认识 Godot 的地方）
   ├── FileSink     轮转落盘 user://logs/vettura.YYYY-MM-DD.log（非阻塞 writer 线程）
   ├── ConsoleSink  转 stdout（Godot 编辑器 Output / headless 终端可见）
   └── RingSink     有界环形缓冲（默认 4096 条，ring_capacity 可调）：
                    崩溃转储 / VetturaLogger.recent 的数据源；
                    panic 时 hook 同步追加写 user://logs/last-crash.log
```

三个 sink 各挂一把 warn 聚合 gate（见 §3 / §5）；FileSink / ConsoleSink 的级别
由自研 `DynamicFilter`（`RwLock<EnvFilter>`）驱动，运行期可热换指令串。

**为什么不新建 crate**（评审结论）：插件本身只 `use tracing` 宏，不需要依赖我们
的安装层；只有主程序侧（`gdext`）需要"装 subscriber / 落盘 / 读 `user://log.cfg`",
而这些本来就必须放在唯一认识 Godot 的 crate 里。另开 crate 只会多一个 workspace
成员和一条所有插件都要加的依赖。

**边界**：`logging/` 不解析资产、不懂物理；`tools/` CLI 不加载 GDExtension，
各自 `println!` + anyhow，不走本模块。

## 3. 配置

`user://log.cfg`（文件不存在 = 静默用默认值，与 telemetry 一致）：

```ini
[log]
enabled=true
level="info"          ; trace|debug|info|warn|error，全局默认
file=true             ; 落盘开关
dir="user://logs"     ; 日志目录（Godot 虚拟路径）
rotate="daily"        ; daily|never（每天一个文件 / 单文件追加；size 轮转后续阶段）
max_files=7           ; 最多保留几个日志文件
console=true          ; 是否同时打到 Godot 控制台（stdout）
console_level="info"  ; 控制台级别下限；实际取 level 与它之间更严的一个
ring_capacity=4096    ; 环形缓冲条数（64..=65536，越界回默认并记 issue）
```

- **优先级**：`VetturaLogger.set_level`（运行期覆盖）> 环境变量
  `VETTURA_LOG`（或 `RUST_LOG`）> 配置文件 > 默认值。set_level 里 `target`
  为空 = 全局；覆盖在热重载后仍保留。环境变量支持 per-target 覆盖，例如：

  ```bash
  VETTURA_LOG="info,vettura_world=debug,vettura_assets::track=trace" \
    /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/baseline.tscn
  ```

  默认值：`enabled=true`、`level=info`、`file=true`、`rotate=daily`、
  `max_files=7`、`console=true`、`console_level=info`、`ring_capacity=4096`
  （落盘默认开，见 §9）。
- 解析失败的值（未知键 / 非法级别 / 越界数字）不打死配置：记一条
  `warn: log config: line N: key=value 无法识别，已用默认值`，其余照常生效。
- 环境变量里的 filter 串非法时降级到 `level` 默认值，不让配置错误杀掉日志。

**热重载**（Phase 2）：级别类键（level / console_level / ring_capacity）运行期
可热更——`on_main_loop_frame` 每 0.5s 比一次 mtime，变了就重读（缺文件 warn
一次并保留当前配置）；`VetturaLogger.reload()` 可手动触发同一实现。sink 结构
（enabled / file / dir / rotate / max_files / console）安装期定死，热重载只
warn 提醒需重启生效。

**warn 聚合**（Phase 2）：WARN/ERROR 按 callsite 在 5s 窗口内聚合——每个 sink
一把 gate，首条放行、重复只计数；聚合行只从"聚合者"那把 gate 发（file 优先，
其次 console，最后 ring），其余 sink 经 bypass 收到同一份，不重复报数：

```
WARN vettura_native::logging: log config: line 2: rotate="hourly" 无法识别，已用默认值
WARN vettura_log: 同类日志聚合：5s 窗口内抑制了 4 条 original_target="vettura_native::logging" original_level=WARN file="..." line=368 suppressed=4
```

`vettura_log` 前缀 target（我们自己的诊断事件）bypass 所有 gate，防递归。

## 4. 输出格式

文件 / 控制台一行一条，无 ANSI（Godot Output 面板不渲染转义序列），带线程名 /
target / 文件名 / 行号 / span 上下文：

```
INFO vettura-godot::logging: vettura.2026-11-14.log dir="user://logs" file=true ...: vettura log online
INFO track_load{root="/mods/daytona_2017", layout="roadcourse"}:vettura_assets::track: track module loaded models=5 surfaces=6 triangles=28611 sections=3 groove=true
WARN vettura_assets::track: track model listed in models.ini is missing; skipping model=xxx
```

RingSink 用自写的紧凑格式（时间 / 级别 / target / 消息 / 字段 / span 链），
`VetturaLogger.recent/dump` 与 last-crash.log 都输出这种行：

```
2026-11-14T05:17:03.634195Z INFO vettura_native::logging vettura log online version=0.0.1 ...
2026-11-14T05:17:03.735060Z INFO vettura_log 运行期级别覆盖已应用 set_target=vettura_kn5 set_level=DEBUG
```

崩溃时 panic hook 追加写 `user://logs/last-crash.log`（超 1MiB 重建；链式调用
前一个 hook，godot-rust 的控制台输出不变）：时间 / 线程 / panic 消息 / 位置 /
`Backtrace::force_capture()`（不看 `RUST_BACKTRACE`）/ 环形缓冲全量：

```
=== vettura crash @ 2026-11-14T05:17:03.735298Z ===
thread: <unnamed>
panic: vettura-log: simulated panic (last-crash.log verification)
location: gdext/src/logging/logger.rs:104:9
backtrace:
   0: std::backtrace_rs::backtrace::libunwind::trace ...
--- 最近日志（环形缓冲）---
2026-11-14T05:17:03.634195Z INFO vettura_native::logging vettura log online ...
```

## 5. 插桩约定

- **target** 默认 = crate 路径（`vettura_world` / `vettura_assets::track`），
  per-target 过滤就按它写；**不要**手改 target 名；
- **message 用中文**（与仓库一致），字段名 / 枚举值英文；
- **span 只包阶段**，不包每帧：`track_load`（assets::track）、`car_load`
  （后续，assets::car）。物理 600 Hz 主循环**不**建 span，采样类事件带
  `step_index` 字段即可；
- **错误只在"决定 degrade / 返回"的点记一次**，不在每层重复（例：
  `audio` bank 加载失败在 `load_manifest` 记 `error!`，向上只传播）；
- 每帧级事件默认 Debug 及以下；热路径 warn（world dropped contacts 等）直接
  `warn!` 即可——日志层按 callsite 做 5s 窗口聚合（§3），不会刷爆文件与 ring。

## 6. 性能

- 文件层走 `tracing_appender::non_blocking`：独立 writer 线程 + 有界队列
  （默认 128k 行后开始丢旧行并计数），调用线程只做内存序列化，**永不阻塞**；
- `WorkerGuard` 存全局表活到进程退出，退出时 flush  writer 线程；
- 控制台层直接 `stdout`（自带锁，跨线程安全；不用 `godot_print!`——那需要
  线程上下文，而物理 / 加载线程也会发日志）；
- 级别过滤由 `DynamicFilter`（`RwLock<EnvFilter>`，读锁一次原子开销级）完成，
  未启用的事件只付一次 atomic 读；warn 聚合 gate 是 per-sink 一把 `Mutex`
  HashMap，只对放行的 WARN/ERROR 事件付一次查表；
- 热重载轮询每 0.5s 一次（文件 mtime + 聚合 flush），不落在每帧热路径上。

## 7. Godot 集成

- `gdext/src/lib.rs`：`on_stage_init(InitStage::Scene)` → `logging::init_from_user_config()`。
  选 Scene 而非更早的 Core/Servers：业务日志都发生在场景加载后，且保持默认
  `min_level = Scene` 不动扩展加载级别；
- `on_main_loop_frame()`（每帧，内部 0.5s 节流）→ `logging::poll_config_reload()`：
  warn 聚合 flush + 配置 mtime 轮询。注意**不要**给 override 加
  `#[cfg(since_api)]`——那是 godot-ffi 内部 build.rs 的 cfg，不会传播到本
  crate，加了会被静默裁掉；
- `gdext/src/logging/` 导出：`LOG_CONFIG_PATH` / `LogConfig::parse` /
  `install` / `InstallOutcome` / `init_from_user_config` / `poll_config_reload`；
- **`VetturaLogger`**（`RefCounted`，`logger.rs`）：GDScript 侧入口——
  `reload() -> bool`（重读 log.cfg）、`set_level(target, level) -> bool`（空
  target = 全局；非法级别返回 false）、`log_dir() -> GString`、
  `recent(count) -> PackedStringArray`（`<=0` = 全部）、`dump() -> GString`、
  `dump_to_file(path) -> bool`、`simulate_panic()`（仅测试用：触发一个 Rust
  panic 验证 last-crash.log 链路；panic 被 godot-rust 捕获，进程不崩）。
  日志被禁用时全部安静返回空值/false；
- 启动 banner（第一条 info）含：版本 / 平台 / 架构 / 目录 / 各级 filter 串 /
  ring_capacity。

## 8. 分阶段

**Phase 1（本次已落地）**

- [x] `user://log.cfg` 解析（纯字符串，headless 单测覆盖默认值 / 注释引号 /
      坏值降级 / 未知键 / console 取严 / level 别名）；
- [x] 文件层（daily/never 轮转、`max_files`）+ 控制台层、`EnvFilter` 两位级
      （文件=全局级别，控制台=更严者）；
- [x] 环境变量 `VETTURA_LOG`/`RUST_LOG` 优先；
- [x] `InitStage::Scene` 安装 + banner；
- [x] 插桩：`kn5` 解析统计（debug）、`assets::track_load` span + 摘要（info）、
      `audio` bank 失败 `error!`；
- [x] 文档 + crate 依赖更新。

**Phase 2（本次已落地：线索增强）**

- [x] RingSink（有界环形缓冲，`ring_capacity` 可调，超长行截断）+
      `VetturaLogger` GDScript API（`reload/set_level/log_dir/recent/dump/
      dump_to_file`；设计稿里的 `flush` 没做——崩溃安全改由 panic hook 同步写
      转储保证，非崩溃场景的落盘本来就异步且低风险）；
- [x] panic hook：message + `Backtrace::force_capture()` + ring dump 追加写
      `user://logs/last-crash.log`（超 1MiB 重建；链式调用前一个 hook）；
- [x] 配置热重载（mtime 轮询 0.5s，级别类键热更，sink 结构变化只 warn；
      缺文件 warn 一次并保留当前配置）；
- [x] warn 聚合计数（per-sink gate，5s 窗口，单聚合者发聚合行）；
- [x] 冒烟页 `test_visual/log_smoke.tscn`（VetturaLogger API + reload 链路 +
      `LOG_SMOKE_PANIC=1` 验证 last-crash.log）。

**Phase 3（可选）**

- `rotate="size"`；`format="json"`；Android logcat；UDP sink；FMOD 报错打通；
- `tools/` CLI 接 `VETTURA_LOG`。

## 9. 取舍记录

| 决策 | 选择 | 理由 |
| --- | --- | --- |
| 放置 | `gdext/src/logging`，不开 crate | 插件只用 tracing 宏，安装层必须认识 Godot，只此一处 |
| 落盘默认 | 开（daily / 7 天） | 出问题时用户机器上必须有日志，磁盘有上限兜底 |
| 文件级别 vs 控制台级别 | 控制台取更严者 | 文件留全量线索，控制台保可读性 |
| 环境变量优先级 | 高于配置文件 | 本地 / CI 调试免改文件 |
| size 轮转 | 延后 | `tracing-appender` 只有时间轮转；daily+max_files 已够用 |
| `godot_print!` vs stdout | stdout | 事件可能来自非引擎线程 |
| 运行期换 filter | 自研 `DynamicFilter`（`RwLock<EnvFilter>` + `Interest::sometimes()`） | 0.3.23 无 `filter::reloading` 模块；自研很薄 |
| flush() API | 不做 | 崩溃安全由 panic hook 同步写转储保证；平时落盘异步且低风险 |
| warn 聚合 | per-sink gate + 单聚合者（file > console > ring）发聚合行 | 其余 sink 经 bypass 收同一份，不重复报数 |
| `on_main_loop_frame` cfg | 不加 `#[cfg(since_api)]` | 该 cfg 是 godot-ffi 内部 build.rs 的，不传播到用户 crate |
| 热重载范围 | 级别类键热更，sink 结构只 warn | writer/appender 是安装期资源，运行期重建复杂度不值 |

## 10. 验证

```bash
cargo test -p vettura-godot          # logging 的 parse/ring/gate 单测（headless，不依赖引擎）

# 日志冒烟：VetturaLogger API + reload 链路（ring 内容来自启动 banner）
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://test_visual/log_smoke.tscn
# 期望最后一行 [log-smoke] PASS、退出码 0

# panic 转储链路：真实触发一次 Rust panic（进程由布好的定时器退出）
LOG_SMOKE_PANIC=1 /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://test_visual/log_smoke.tscn
grep "simulated panic" "$HOME/Library/Application Support/Godot/app_userdata/Vettura/logs/last-crash.log"

# 端到端：headless 跑任意场景，检查 user 目录日志落盘
/Applications/Godot.app/Contents/MacOS/Godot --headless --path . --quit-after 1500 res://test_visual/garage_smoke.tscn
ls ~/Library/Application\ Support/Godot/app_userdata/Vettura/logs/   # macOS

# 热重载 / warn 聚合（手动 e2e，需要进程活过 5s 窗口，用 main.tscn 这类不自行退出的场景）
# 1) 写一个 user://log.cfg → 启动 main.tscn → 3s 后改 level/ring_capacity → 日志出现 "log config reloaded"
# 2) 配置里放几个坏键 → 同一 callsite 的 warn 首条放行，5s 后出现 "同类日志聚合：5s 窗口内抑制了 N 条"
```

> 已知坑：本机 CLT 的 C++ 头路径使 `oxijolt-sys` 编译失败（`<new> file not
> found`），与日志无关；绕过：`CXXFLAGS="-isystem
> /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/c++/v1"`
> 再跑需要编译 Jolt 的 cargo 命令。