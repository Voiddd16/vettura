# Vettura 脚本目录

本项目所有工程脚本都用 Python 编写，并用 [uv](https://docs.astral.sh/uv/) 统一管理。

## 当前脚本

| 脚本 | 职责 |
| --- | --- |
| `fetch_fmod.py` | 从 fmod.com 下载 FMOD Engine SDK 2.02.22，解压到本地 `.cache/`，并编译 AC/mod bank 可能依赖的 `FMOD Distance Filter` DSP 插件 |

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

## 依赖策略

`fetch_fmod.py` 只用 Python 内置库；确有额外依赖时统一加到本目录 `pyproject.toml`，
执行时统一 `uv run`。
