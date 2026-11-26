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
`/vettura/scripts/fetch_fmod.py` 与 `docs/build.md`。

## 本地拉取与链接

```bash
cd scripts
uv run fetch_fmod.py                 # 当前平台
uv run fetch_fmod.py --all           # macOS / Windows / Linux

cd ..                                # 回到工程根（workspace 根）
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

GDExtension 的 rpath 另外带 `@loader_path`、`@loader_path/../Frameworks`（macOS）和
`$ORIGIN`、`$ORIGIN/../lib`（Linux），所以发行包把 dylib 放到 exe 同目录或
`Contents/Frameworks` 就能被找到；DSP 插件的查找顺序见下面「打包 / 发行」。

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
| `set_vehicle_parameters(event, rpm, throttle, load, speed, view, gear, slip)` | 常见名适配 + FSB fallback 驱动，故障容错 |
| `parameter_names(event)` / `sound_event_names()` / `event_guid_by_name()` | 调试与 mod 适配 |

最小探针见 `gdext/fmod_probe.tscn`，它验证：

```text
[fmod] start= true
[fmod] loaded car= cim_2008_ferrari events= [...]
[fmod] play(engine_ext)= true
[fmod] params(engine_ext)= ["throttle", "rpms", "Distance", "Event Cone Angle"]
[fmod] set_vehicle_parameters= true
[fmod] PASS
```

## 单圈测试接入（`gdext/ai_lap.tscn`）

整圈探针用同一个 `FmodSystem` 把声效接进来了：`ai_lap.gd` 起一个节点，加载
当前车的 `sfx/<car_id>.bank`，按视角循环播放 `engine_ext`（车后/绕车/机盖）或
`engine_int`（驾驶位），并在 `_physics_process` 里把 rpm、AI 油门、近似负载、
车速每帧喂进去。所以窗口模式跑一圈能听到随转速变化的引擎声。

声效不设 3D 声源/听者：FMOD 默认听者与事件都在原点，引擎声会以固定音量持续
播放（与 `fmod_probe` 一样）。这是刻意选择——libfmod 的
`EventDescription::get_parameter_description_*` 会让 `EventInstance` 变成 invalid
handle，因此参数名改为固定候选名逐个尝试，不再做运行期探测。

- `AI_LAP_AUDIO=0`：关掉声效（纯圈速/遥测测试用），默认开启；
- 没编 `--features fmod`、没有 SDK、车没 `sfx/` 或 bank 不兼容时只打 warning，
  **不影响**探针的 PASS/FAIL 判定（声效是增强项，不是验证项）；
- 要真出声：`scripts/fetch_fmod.py` 拉到 SDK 后
  `cargo build -p vettura-godot --features fmod`，并保证动态库在 Godot 能找到的
  位置（见上面「本地拉取与链接」/「打包 / 发行」）。

## 老 AC bank 的实际发声路径：FSB5 fallback

AC 老 mod 的 `.bank` 通常**没有 master bus**。Studio 事件能 `play()` 成功，但
FMOD 会报 `Bus not found` / `assertion: 'busModel' failed`，事件实际**完全静音**
（日志里有这两行不等于声效坏了）。所以这里额外从 bank 内嵌的 FSB5 里取出真实
车声采样，用 Core API 直接播放，这才是玩家真正听到的声音：

- 扫描 bank 内**所有** FSB5 块（F1 2008 有两个，第二个只放了一条车内采样），
  用统一的 `classify()` 按“名字 + 时长”分配角色（`>=500 ms` 才可能是循环，
  `<=1000 ms` 才可能是换挡一次性，回火可到 3 s）：`idle` / `on`（`acc*`、
  `onthrottle`、`*_on_*`）/ `off`（`decel*`、`*_off_*`）/ `wind` /
  `tyre_rolling` / `skid` / `turbo` / `gearup*` / `geardn*` /
  `transmission_Nth` / `backfire` / `traction_control`；`upsh*` / `dwsh*` 是长
  循环，识别出来但不参与一次性播放；
- `*_ext_*` / `*_int_*` 区分车外/车内，没有前缀的两种视角共用；
- 名字里带录制转速（`6halfK`=6500、`12halfK`=12500、`14Kish`=14000）的采样按
  rpm 交叉淡化，其余按 `low/mid/high` 粗分或均分到怠速..9000 rpm；
- **同一层最多只播相邻的两个采样**。以前把所有三角权重内的采样同时叠加，
  多份不同录音相加会产生梳状滤波和明显尖锐感（Porsche 没有转速标注时 5 条
  会一起响）；单个采样的变调也限制在 0.75–1.20 倍，避免高转被拉成啸叫；
- 车外视角优先车尾（`back`/`rear`）麦克风，同时有前后两套录音时排除车头，
  不让两个位置的声音叠在一起；
- rpm 做一阶低通（上升快、回落慢），避免起步时 0→红线跳变把采样 pitch
  拉出短促的尖啸；
- 换挡一次性音效只认 mod 自己的 `gearup*` / `geardn*`（按时长和名字双重判断），
  音色原样播放，不做合成、不做 EQ。`upsh*` / `dwsh*` 是 30–90 秒的循环层，
  绝不能当一次性播——之前每次换挡都启动一条几十秒的长采样，听感就是
  "变速箱一直在响"；`transmission_4th` 这类循环只在匹配挡位以 8% 音量播放，
  pitch 固定 1.0；
- `bank` 里没有 `gearup`/`geardn` 时就没有换挡声（例如 F1 2008）。这是刻意的：
  音色只取决于 mod 内部定义，不用程序化声音去补；
- `traction_control` 是循环采样，按侧偏角 + 油门调音量。

这一层是**统一的角色混音器**：解析代码在 `audio/src/runtime/roles.rs`，入口
只有 `classify()`，规则固定为“时长 → 通用 AC 名词 → 视角”，不写任何具体
车型分支；交叉淡化、pitch 范围和音量映射对所有车一致。**音色完全来自 mod
自己的采样**，这里不做音色合成、不做 EQ、不替换采样；某个 bank 缺少某个角色
时该层静音。

解析器可以脱离 FMOD SDK 单独测试：

```bash
cargo test -p vettura-audio --lib runtime::roles
```

**关键坑（都踩过）**：Core API 的 `Channel::set_frequency` 单位是 **Hz**，不是
倍率。旧代码把 `rpm / 3000` 当倍率传进去，引擎声被压到 0.35–3 Hz 的次声波，
听感就是"引擎几乎听不到，只有断断续续的风噪"。现在改用 `set_pitch`
（1.0 = 原速）。另外 `Studio::System::update()` 内部已经推进 low-level
system，不要再额外调用 `System::update()`，否则同一混音缓冲被推进两次，声音
会断续/卡顿。

诊断命令（`VETTURA_AUDIO_WAV` 把混音直接写成 WAV）：

```bash
cargo run -q -p vettura-audio --features fmod --example fallback_probe -- \
  "mods/trr_porsche_992_gt3r" /tmp/fallback.wav
# 日志里的 [vettura-audio] FSB fallback: ... 列出实际选中的采样
```

验收指标：WAV 里 <20 Hz 能量应为 0%，主能量落在 200–1000 Hz；引擎层、风噪、
胎噪、打滑层随参数变化都有非零输出。`ai_lap.gd` 启动时也会打印这条 fallback
摘要，跑一圈即可确认哪一层没命中。

## 打包 / 发行：runtime 动态库必须一起发

native 侧只**动态链接** FMOD，SDK 二进制不入库，所以发行包必须自带对应
平台的 `fmod` / `fmodstudio`（以及老 AC/mod bank 依赖的 Distance Filter 插件）。
用主工程脚本一键完成：

```bash
cd /path/to/vettura/scripts

# 单平台：把 runtime 拷到导出的可执行文件同目录
uv run pack_fmod.py --platform macos --dest "../dist/Vettura.app"     # .app 自动落到 Contents/MacOS
uv run pack_fmod.py --platform windows --dest "../dist/windows"
uv run pack_fmod.py --platform linux --dest "../dist/linux"

# 一次打三个平台（每个平台一个子目录），并指定架构
uv run pack_fmod.py --all --dest-root ../dist --arch x86_64

# CI：只用缓存，不联网
uv run pack_fmod.py --platform macos --dest ../dist/Vettura.app --offline
```

行为说明：

- 缓存里没有该平台 SDK 时，会**自动用 `.fmod.env` 的账号去 fmod.com 下载**；
  在 Mac 上也可以直接拉 Windows / Linux 安装包做跨平台打包；
- `--profile release|debug|both`：默认只拷 release 变体（`libfmod` / `fmod.dll`）；
- `--arch x86_64|arm64|x64|x86`：Windows/Linux SDK 里每个架构一套库，跨平台打包
  必须显式指定，否则会把 arm64 的 dll 当成 x64 发出去；
- 插件（`libfmod_distance_filter.*`）只能在**目标平台本机**编译，跨平台打包会跳过
  并提示，由对应平台的机器或 CI 补。

放置位置（`res://` 工程 → 导出目录）：

| 平台 | 位置 | 谁负责找到它 |
| --- | --- | --- |
| macOS | `<App>.app/Contents/MacOS/`（与 exe、GDExtension 同目录）或 `Contents/Frameworks/` | GDExtension 的 `@loader_path` / `@loader_path/../Frameworks` rpath |
| Windows | 与 `Vettura.exe`、`vettura_native.dll` 同目录 | Windows DLL 搜索顺序 |
| Linux | 与可执行文件、`libvettura_native.so` 同目录（或 `../lib`） | `$ORIGIN` / `$ORIGIN/../lib` rpath |

DSP 插件的查找顺序（`vettura-audio/src/runtime/plugins.rs::plugin_dirs`）：
`VETTURA_FMOD_PLUGIN_DIR` → exe 同目录 / `Frameworks` / `fmod` → 工作目录 `fmod/` →
build.rs 烘焙的开发缓存。也就是说**发行包不依赖 `.cache/`**。

`godot/vettura_native.gdextension` 已区分 debug / release 条目：编辑器用
`target/debug`，导出包用 `target/release`（别忘了先
`cargo build -p vettura-godot --features fmod --release`）。

完整流程示例：

```bash
cargo build -p vettura-godot --features fmod --release
godot --headless --path /path/to/vettura --export-release "macOS" ../dist
cd /path/to/vettura/scripts && uv run pack_fmod.py --platform macos --dest ../dist/Vettura.app
```

> 发行提示：FMOD EULA 要求随产品提供 FMOD 的版权/署名信息，pack 时请一起带上。

## 已知限制

- AC/mod 的老 bank 可能引用 bank 外的 bus；FMOD 会打印 `BusInstanceFactory::create` 的
  `Bus not found`，但加载仍然完成。发布包要自己决定是否注入统一 `Master.bank`；
- bank 是 2.03/2.04 编出来的话，2.02.22 runtime 不保证能读，需要换对应 SDK 复测；
- Windows / Linux 的 DSP 插件二进制需要在对应平台编译（本机脚本不交叉编译）；
  Windows 安装包解压还需要 `7z`（`brew install p7zip`）。
