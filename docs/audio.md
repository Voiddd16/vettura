# FMOD 声浪集成

## 版本与兼容规则

| 项目 | 锁定 |
| --- | --- |
| FMOD Engine SDK | **2.02.22** |
| rust 绑定 | `libfmod = "~2.222"` |
| 链接方式 | **动态链接**，SDK 二进制**不入库** |
| bank 兼容 | 2.02.22 runtime 只保证加载 **2.02.x** 编出来的 bank |

**重要**：bank 兼容规则按 `产品 + major` 判断：

- ✅ 2.02.x 编的 bank → 2.02.22 runtime 保证能加载；
- ⚠️ 2.03 / 2.04 编的 bank → **不保证**能读。遇到老 mod 时先用 `scripts/fetch_fmod.py` 拉对应 SDK，再单独跑一次验证。

FMOD 版本号必须由主工程 `scripts/fetch_fmod.py` 拉到本地缓存，不进本仓库，详见
`/vettura/scripts/fetch_fmod.py` 与 `vettura-native/docs/build.md`。

## 本地拉取与链接

```bash
cd /path/to/vettura/scripts
uv run fetch_fmod.py                 # 当前平台
uv run fetch_fmod.py --all           # macOS / Windows / Linux

cd /path/to/vettura-native
cargo build -p vettura-godot --features fmod
```

脚本会做三件事：

1. 用 fmod.com API 登录并下载 `fmodstudioapi20222{win,mac,linux}` 安装包；
2. 解压并把 `libfmod*.dylib` / `libfmodstudio*.dylib` 复制到 `.cache/fmod/runtime/<os>/`；
3. 编译 AC/mod bank 可能依赖的 `FMOD Distance Filter` DSP 插件。

`build.rs` 会从这些位置找运行期动态库，并给最终 cdylib 加 rpath：

- `.cache/fmod/runtime/<os>/`（macOS / Linux 首选，路径无空格）；
- `FMOD_SDK_PATH`；
- `FMOD_CACHE_DIR`。

没找到 SDK 时不会阻断其它模块：`vettura-audio` 以 stub 编译，`FmodSystem.start()`
返回 `false`，`last_error()` 会说明原因。

## Rust 侧集成

`vettura-audio` 不解析 bank/FSB，只做两件事：

1. `sfx/GUIDs.txt` → 事件名 / GUID（`vettura_audio::guids`）；
2. `sfx/<car_id>.bank` → FMOD `load_bank_memory`（`vettura_audio::manifest` + `runtime::AudioEngine`）。

链路：

```text
GUIDs.txt ──► SoundManifest { bank_path, events }
                │
                ▼
AudioEngine::start()
  Studio::create + initialize(RIGHTHANDED_3D)
  load_bank_plugins()            // FMOD Distance Filter 等
  Studio::load_bank_memory()
  Studio::get_event(guid)
  EventDescription::create_instance()
  EventInstance::start()
  Studio::update()
```

- bank 里的 FMOD 专有音频封装由 FMOD Engine 自己解码，Rust 侧不做 `.bank/.FSB` 解析；
- 每帧在 Godot `_process` 里调用一次 `FmodSystem` 的 `_process`，它会转调 `studio.update()`；
- 事件参数名在不同 mod 里不一致，运行时用候选名探测并跳过缺失项（`rpms` / `throttle` / `load` / `speed` / `view`）。

## Godot 侧

`FmodSystem` 主要方法：

| 方法 | 用途 |
| --- | --- |
| `start(max_channels)` | 启动 FMOD runtime；失败读 `last_error()` |
| `load_car(car: AcCar)` / `load_car_path(path)` | 加载 `sfx/GUIDs.txt` + `.bank` |
| `play(event)` / `play_once(event)` | 循环声 / 一次性声 |
| `stop(event)` | 停止 `play()` 启动的循环事件 |
| `set_parameter(event, name, value)` | 直接写参数 |
| `set_vehicle_parameters(event, rpm, throttle, load, speed, view)` | 常见名适配，故障容错 |
| `parameter_names(event)` / `sound_event_names()` / `event_guid_by_name()` | 调试与 mod 适配 |

最小探针见 `vettura-native/godot/fmod_probe.tscn`，它验证：

```text
[fmod] start= true
[fmod] loaded car= cim_2008_ferrari events= [...]
[fmod] play(engine_ext)= true
[fmod] params(engine_ext)= ["throttle", "rpms", "Distance", "Event Cone Angle"]
[fmod] set_vehicle_parameters= true
[fmod] PASS
```

## 已知限制

- AC/mod 的老 bank 可能引用 bank 外的 bus；FMOD 会打印 `BusInstanceFactory::create` 的
  `Bus not found`，但加载仍然完成。发布包要自己决定是否注入统一 `Master.bank`；
- bank 是 2.03/2.04 编出来的话，2.02.22 runtime 不保证能读，需要换对应 SDK 复测；
- Windows / Linux 的运行期 DLL 打包脚本尚未实现（macOS 已验证）；SDK 二进制不入库，
  由 `.cache/fmod/runtime` 本地加载。
