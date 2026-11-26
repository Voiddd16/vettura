# Vettura 脚本目录

本项目所有工程脚本都用 Python 编写，并用 [uv](https://docs.astral.sh/uv/) 统一管理。

## 当前脚本

| 脚本 | 职责 |
| --- | --- |
| `build.py` | 一键编译链：`host`（宿主 debug/release 库）、`android`（两个 ABI 的 .so）、`apk`（导出 APK）、`all` 全链，另有 `check`（check+clippy）与 `test`。Android 细节内部转发 `android.py` |
| `clean.py` | 清理构建产物与缓存：`quick`（增量缓存等）/ `rust`（整个 target/）/ `godot`（导入缓存+模板下载缓存）/ `artifacts`（build/）/ `all`，默认预览，`--run` 才真删 |
| `android.py` | 移动端初始化与导出全流程：配置诊断、下载模板、写入 SDK 设置、装 NDK、编译 Android .so、导出 APK，每步幂等可单独执行 |
| `fetch_fmod.py` | 从 fmod.com 下载 FMOD Engine SDK 2.02.22，解压到本地 `.cache/`，并编译 AC/mod bank 可能依赖的 `FMOD Distance Filter` DSP 插件 |
| `pack_fmod.py` | 把 FMOD 运行期动态库拷进 Godot 导出目录（发行包必需）；缓存缺失时自动下载，支持跨平台与 `--arch` |
| `load_probe.py` | 生成合成车模（`.kn5` + 合法 DXT1 DDS）并跑 Godot 无头加载探针，输出解析 / 冷实例化 / 缓存命中 / 分帧加载四段耗时。纯标准库，无需 uv 依赖 |
| `run_ai_lap_test.py` | AI 整圈测试全流程：全量重编译 → test/fmt → 车后/驾驶位两段录像（JPEG 帧 + FMOD WAV 分开采集）→ atempo 对齐混流 → 两段合并 → 抽帧/音量 QA → `REPORT.md`。纯标准库 |

## 使用方式

```bash
cd /path/to/vettura/scripts

# 首次/更新 SDK（需要 fmod.com 账号）
uv run fetch_fmod.py

# CI/多平台
uv run fetch_fmod.py --all

# 已下载过，只重新解压 + 重建 runtime 与插件
uv run fetch_fmod.py --extract-only
```

## AI 整圈测试与录像

`run_ai_lap_test.py` 把日常整圈验证固化下来（默认 Daytona roadcourse /
Porsche 992 GT3 R / skill 110 / 1080p）：

```bash
python3 scripts/run_ai_lap_test.py                 # clean + 全量编译 + test + 两段录制 + 混流 + 合并
python3 scripts/run_ai_lap_test.py --no-clean      # 增量编译
python3 scripts/run_ai_lap_test.py --skip-build --skip-tests   # 直接用现有 dylib 重录
python3 scripts/run_ai_lap_test.py --views chase   # 只录车后
python3 scripts/run_ai_lap_test.py --view drone    # 无人机追踪（其它：hood/bumper/wheel/tcam/orbit）
python3 scripts/run_ai_lap_test.py --no-audio      # 只录视频
```

> 两段默认**串行**录制。并行跑两个 Godot 会互相抢 GPU，实时窗口容易卡顿；
> `--parallel` 仅作显式选项（不推荐）。录制时的 JPEG 编码/写盘已放到后台线程，
> 主线程只做 GPU 回读，避免 1080p 下每 2 帧同步写盘卡住画面。
> 每次录完脚本用逐帧 `framemd5` 统计最长连续相同帧，报告里会列出（正常应为 1–2 帧）。

赛道、车辆、视角、相机、天气都是参数：

```bash
python3 scripts/run_ai_lap_test.py \
    --track daytona_2017 --layout roadcourse \
    --car res://mods/trr_porsche_992_gt3r \
    --skill 110 --ai-pace 1.0 --resolution 1920x1080 --spawn-grid 1 \
    --view chase --view cockpit \
    --weather clear \
    --chase-distance 4.5 --chase-height 2.0 --chase-lookahead 14 --chase-fov 72 \
    --eye-x -0.367 --eye-y 1.0 --eye-z 0.157 --cockpit-fov 70
```

`--ai-pace` 是外部配速倍率，乘在难度档位自带的配速上：110 档自带约 1.65，
100 档为 1.0。提高它会同时加快直道授权速度和规划横向抓地（弯速约按 `√pace`
提升），用于逼近人类圈速；过高会突破车辆实际抓地，需看报告的圈速有效性。

产物在 `build/ai_lap_test/<时间戳>/`：`videos/`（两段 + 合并）、`audio_*.wav`
（分开采集的原始音轨）、`<view>_run.log`、`REPORT.md`。码率/帧率/抓帧间隔等
也都能覆盖，见 `--help`。

`load_probe.py` 不依赖任何第三方库（纯标准库）：

```bash
python3 scripts/load_probe.py                 # 默认 66 MiB 合成车模
python3 scripts/load_probe.py --meshes 64     # 小模型快速迭代
python3 scripts/load_probe.py --godot /path/to/Godot
```

## 编译与清理

```bash
python3 scripts/build.py                     # 宿主 debug 库（编辑器/无头用）
python3 scripts/build.py host --release      # 宿主 release 库（例如 macOS 导出包）
python3 scripts/build.py android             # arm64-v8a + x86_64 .so（转发 android.py lib）
python3 scripts/build.py apk                 # 导出 build/android/Vettura.apk（转发 export）
python3 scripts/build.py all --check         # check+clippy 过了再 host+android+apk 全链
python3 scripts/clean.py                     # 预览各项缓存大小（默认 dry-run）
python3 scripts/clean.py quick --run         # 只清增量缓存/探针产物（日常最常用）
python3 scripts/clean.py all --run           # 全清（下次编译全量重编，记得重编 android 库）
```

凭证从仓库根目录的 `.fmod.env` 读取（已加入 `.gitignore`，禁止提交）：

```bash
FMOD_USER=<fmod.com 邮箱>
FMOD_PASS=<密码>
```

也可以用环境变量 `FMOD_USER` / `FMOD_PASS` 覆盖。SDK 二进制不入库；
项目用 `libfmod ~2.222` 对应 FMOD Engine 2.02.22，bank 兼容规则见
[`../docs/audio.md`](../docs/audio.md)。

## 打包 runtime（发行游戏时需要）

游戏只动态链接 FMOD，所以导出后必须把对应平台的 `fmod` / `fmodstudio`
（以及老 bank 需要的 Distance Filter 插件）拷到可执行文件旁边：

```bash
cd /path/to/vettura/scripts

uv run pack_fmod.py --platform macos --dest "../dist/Vettura.app"   # .app 自动落到 Contents/MacOS
uv run pack_fmod.py --platform windows --dest "../dist/windows"
uv run pack_fmod.py --platform linux   --dest "../dist/linux"
uv run pack_fmod.py --all --dest-root ../dist --arch x86_64          # 三个平台一次打完
uv run pack_fmod.py --platform macos --dest ../dist/Vettura.app --offline   # CI：只用缓存
```

要点：

- 缓存里没有该平台 SDK 时会**自动下载**（在 Mac 上也能拉 Windows/Linux 包）；
- `--arch`：Windows/Linux SDK 每架构一套库，跨平台打包必须显式指定；
- `--profile release|debug|both`：默认只发 release 变体；
- 跨平台打包时 DSP 插件编不了（只能在目标平台本机编），会给出 warning；
- Windows 安装包解压需要 `7z`（`brew install p7zip`）。

## 依赖策略

`fetch_fmod.py` / `pack_fmod.py` 只用 Python 内置库；确有额外依赖时统一加到本目录
`pyproject.toml`，执行时统一 `uv run`。
