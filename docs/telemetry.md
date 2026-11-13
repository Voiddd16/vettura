# 实时遥测

把车辆 / 赛道 / 计时三个方向的数据打成 JSON，经网络（UDP 或 TCP）按帧发出去，
供外部工具（叠加显示、数据记录、调校分析）消费。

- **配置来源**：`user://telemetry.cfg`（Godot `ConfigFile` / INI 格式），运行中可改，
  约 0.5 秒内热生效；
- **实现位置**：核心在 [`world/src/telemetry.rs`](../world/src/telemetry.rs)
  （配置解析、帧组装、socket 输出，保持 headless）；读文件与逐帧驱动在
  `gdext/src/world/mod.rs` 的 `WorldBody`（唯一推进 `World::step` 与
  `LapTimer::update` 的地方，采样在其后）；
- **依赖**：`serde` + `serde_json`（workspace 级依赖）。

## 快速开始

在 Godot 用户目录放 `telemetry.cfg`（首次运行不存在 = 静默禁用）：

```ini
[telemetry]
enabled=true
protocol="udp"        ; "udp" | "tcp"
host="127.0.0.1"
port=9999
rate_hz=20            ; 采样频率上限；0 = 每个物理帧都采
scope="ego"           ; "ego"（仅主车）| "all"（全场）| "indices"
; indices="0,2,3"     ; scope="indices" 时生效，下标 = 发车顺序

[vehicle]
enabled=true          ; 车辆方向总开关
pose=true             ; 位置/姿态/速度/角速度
controls=true         ; 油门/刹车/离合/转向/手刹/DRS/ABS/TC/ERS 模式
powertrain=true       ; 转速/挡位/涡轮/冷却液/机油/ERS
wheels=true           ; 四轮：接触/载荷/滑移/滑角/外倾/悬架/表面
tyres=true            ; 轮胎温度/压力(psi)/磨损
brakes=true           ; 刹车盘温度
fuel=true             ; 燃油质量/油量比例

[track]
enabled=true          ; 赛道方向总开关
weather=true          ; 湿度/雨强/气温/路温/风/抓地/雾
groove=true           ; 每辆车位置处的橡胶线进度
impacts=true          ; 碰撞事件增量（只发新增）
max_impacts=64        ; 单帧碰撞事件上限，0 = 不限

[timing]
enabled=true          ; 计时方向总开关
laps=true             ; 圈数/当前圈速/上圈/最佳圈/有效性
sectors=true          ; 当前分段与本圈分段成绩
last_lap=true         ; 上圈分段成绩与有效性
```

所有键都可省略，缺省值见上（`enabled` 默认 `false`）。启动后看 Godot 输出：

```
WorldBody: telemetry enabled -> udp 127.0.0.1:9999 @ 20 Hz, scope Ego
```

快速验证（另一个终端）：

```bash
nc -u -l 9999          # UDP
nc -l 9999             # TCP（protocol="tcp"）
```

## 帧格式

一行一个 JSON 对象（UDP = 一个数据报；TCP = 连续流，每帧一行语义上独立）：

```json
{
  "v": 1,
  "t": 12.35,
  "vehicles": [
    {
      "id": 1, "index": 0, "primary": true,
      "pos": [10.2, 0.4, -3.1], "rot": [0, 0, 0, 1],
      "vel": [42.0, 0.0, 1.3], "ang_vel": [0, 0.01, 0],
      "speed_mps": 42.02,
      "controls": {"throttle": 1.0, "brake": 0, "clutch": 0, "steering": -0.12,
                   "handbrake": false, "drs": false, "abs": true, "tc": true,
                   "ers": "Deploy"},
      "rpm": 11200, "gear": 5, "turbo_boost_bar": 1.1,
      "coolant_c": 98.0, "oil_c": 110.0,
      "ers": {"soc": 0.62, "energy_j": 1.4e6, "lap_deploy_j": 8.2e5, "temperature_c": 41.0},
      "wheels": [
        {"contact": true, "surface": 1, "load_n": 3100.0,
         "slip_ratio": 0.02, "slip_angle": 0.03, "camber_deg": -3.1,
         "suspension_m": 0.06, "water_mm": 0.0},
        "... ×4 (LF, RF, LR, RR)"
      ],
      "tyre_temp_c": [88.0, 91.0, 85.0, 87.0],
      "tyre_pressure_psi": [27.1, 27.3, 26.8, 27.0],
      "tyre_wear": [0.01, 0.01, 0.02, 0.02],
      "brake_temp_c": [420.0, 415.0, 380.0, 377.0],
      "fuel_kg": 42.5, "fuel_fraction": 0.61,
      "groove": 0.73
    }
  ],
  "weather": {"wetness": 0, "rain": 0, "air_c": 24.0, "track_c": 38.0,
               "wind": [1.2, 0, 0.4], "grip": 1.0, "fog": 0.01},
  "impacts": [
    {"a": "vehicle:1", "b": "track:3", "pos": [12, 0.3, -4],
     "impulse_ns": 8500.0, "speed_mps": 3.2, "severity": 0.18}
  ],
  "timing": {"laps": 3, "lap_s": 71.4, "last_lap_s": 70.9,
              "best_lap_s": 70.9, "sector": 2,
              "sector_s": [22.1, 24.6], "last_sector_s": [22.0, 24.5],
              "lap_valid": true, "last_lap_valid": true}
}
```

约定：

- **缺失的段 / 字段 = 该开关关闭或本帧无数据**，消费者按"字段存在性"判断，
  不要假设固定结构；`"v"` 是协议版本，破坏性变更时递增；
- `id` 是 `VehicleHandle::raw()`，跨帧稳定；`index` 是发车顺序下标（也是
  `scope="indices"` 用的下标）；
- 非有限浮点（NaN/Inf）会被 `serde_json` 写成 `null`（而不是让整帧失败）；
- 轮序固定 **LF, RF, LR, RR**（与 `VehicleSnapshot.wheels` 一致）。

## 关键行为

- **范围（scope）**：`ego` 只发主车（第一个 `spawn_vehicle*` 成为 primary）；
  `all` 发场上全部车；`indices` 按发车顺序下标过滤（可配 `indices="0,2,3"`）。
- **节流（rate_hz）**：首帧立即发，之后按 `1/rate_hz` 节流；`0` = 每个物理帧都发
  （默认 60 Hz，可到 `rate_hz=60`）。物理步进与计时更新之后采样，帧里是本帧终态。
- **碰撞增量**：`World::impacts()` 是累积缓冲且从不 drain，遥测用只读游标取
  "上次发送之后"的新增事件，不会破坏其它消费者；配置里 `max_impacts` 防止单帧膨胀。
- **热更新**：每 0.5 秒比一次配置文件 mtime，变了就重载并重建 socket（协议 /
  地址变化时）。文件被删除 = 禁用（带 warning）。GDScript 也可手动触发：
  `world.reload_telemetry_config()` / `world.set_telemetry_enabled(bool)` /
  `world.is_telemetry_enabled()`。
- **传输**：UDP 无连接，丢帧无所谓（每帧独立）；TCP 断线自动重连（连上后补发
  当前帧），发送失败按指数退避记 warning，不刷屏。
- **缺省状态**：`user://telemetry.cfg` 不存在 = 整体禁用、不建 socket、零开销
  （每次物理帧只查一次开关）。

## 验证

```bash
cargo fmt --all && cargo check --workspace && cargo test --workspace
cargo test -p vettura-world telemetry          # 配置解析 / 节流 / 帧序列化单测
```

真机联动（需 `mods/`，跑起来后改 `user://telemetry.cfg` 观察热更新日志）：

```bash
/Applications/Godot.app/Contents/MacOS/Godot --path . res://test_visual/main.tscn
```
