# Vettura 脚本目录

本项目所有工程脚本都用 Python 编写，并用 [uv](https://docs.astral.sh/uv/) 统一管理。

## 当前脚本

| 脚本 | 职责 |
| --- | --- |
| `fetch_fmod.py` | 从 fmod.com 下载 FMOD Engine SDK 2.02.22，解压到本地 `.cache/`，并编译 AC/mod bank 可能依赖的 `FMOD Distance Filter` DSP 插件 |
| `pack_fmod.py` | 把 FMOD 运行期动态库拷进 Godot 导出目录（发行包必需）；缓存缺失时自动下载，支持跨平台与 `--arch` |

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

凭证从仓库根目录的 `.fmod.env` 读取（已加入 `.gitignore`，禁止提交）：

```bash
FMOD_USER=<fmod.com 邮箱>
FMOD_PASS=<密码>
```

也可以用环境变量 `FMOD_USER` / `FMOD_PASS` 覆盖。SDK 二进制不入库；
项目用 `libfmod ~2.222` 对应 FMOD Engine 2.02.22，bank 兼容规则见
`vettura-native/docs/audio.md`。

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
