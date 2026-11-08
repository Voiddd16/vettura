#!/usr/bin/env python3
"""把 FMOD Engine 2.02.22 运行期动态库拷进 Godot 导出目录。

发行版必须自带 FMOD runtime：native 侧（`gdext/`）只**动态链接** FMOD，SDK 二进制
不入库，所以打包时要把对应平台的 `fmod` / `fmodstudio`（以及 AC/mod 老 bank
依赖的 Distance Filter DSP 插件）复制到可执行文件旁边。

    cd scripts
    uv run pack_fmod.py --platform macos --dest "../dist/Vettura.app"
    uv run pack_fmod.py --platform windows --dest "../dist/windows"
    uv run pack_fmod.py --all --dest-root ../dist          # 三个平台各一个子目录

缓存里没有该平台 SDK 时会自动用 `.fmod.env` 里的账号去 fmod.com 下载
（跨平台打包就在本机拉 Windows/Linux 安装包）；加 `--offline` 则只用缓存。
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

import fetch_fmod


def host_platform() -> str:
    return fetch_fmod.host_platform()


def resolve_dest(dest: Path, plat: str) -> Path:
    """Godot 导出的可执行文件所在目录。

    macOS 传 `.app` 时自动落到 `Contents/MacOS`（Godot 把 exe 与 GDExtension
    都放在那里，`@loader_path` rpath 就在同目录找 dylib）。
    """
    if dest.suffix == ".app":
        return dest / "Contents" / "MacOS"
    return dest


def is_debug_library(name: str) -> bool:
    """调试变体会在名字里带大写的 `L`（`libfmodL` / `fmodL.dll`）。"""
    return "fmodL" in name or "fmodstudioL" in name


def select_files(runtime: Path, plat: str, profile: str) -> list[str]:
    """按 release/debug 变体挑出要随游戏分发的文件。

    Linux 的库名带版本号（`libfmod.so.13`），SONAME 也是它，所以按后缀整体拷贝，
    不逐个枚举固定文件名。
    """
    if plat == "windows":
        wanted = lambda name: name.endswith(".dll")
    elif plat == "macos":
        wanted = lambda name: name.endswith(".dylib")
    else:
        wanted = lambda name: ".so" in name

    selected: list[str] = []
    for path in sorted(runtime.iterdir()):
        if not path.is_file():
            continue
        name = path.name
        if name == fetch_fmod.PLUGIN_NAME[plat]:
            selected.append(name)
            continue
        if not wanted(name):
            continue  # 跳过 import library（*.lib）
        debug = is_debug_library(name)
        if profile == "release" and debug:
            continue
        if profile == "debug" and not debug:
            continue
        selected.append(name)
    return selected


def pack(plat: str, dest: Path, profile: str, cache: Path, offline: bool, arch: str | None = None) -> int:
    if fetch_fmod.runtime_ready(plat, cache):
        print(f"[cache] runtime 已齐：{fetch_fmod.runtime_dir_for(cache, plat)}")
        # 插件是单独产物：缓存里没有就尝试用已解压 SDK 补编（Windows 需要在 Windows 上编）。
        runtime = fetch_fmod.runtime_dir_for(cache, plat)
        if not (runtime / fetch_fmod.PLUGIN_NAME[plat]).is_file():
            api_dir = fetch_fmod.unpacked_api(plat, cache)
            if api_dir is not None:
                fetch_fmod.build_plugin(api_dir, plat, runtime)
    else:
        token = user = None
        if not offline:
            user, password = fetch_fmod.load_credentials()
            token = fetch_fmod.login(user, password)
        fetch_fmod.fetch_platform(
            plat, cache, token, user, allow_download=not offline, arch=arch
        )

    runtime = fetch_fmod.runtime_dir_for(cache, plat)
    names = select_files(runtime, plat, profile)
    if not any(not is_debug_library(name) for name in names):
        sys.exit(f"[fail] {plat} 没有可分发的运行期库：{runtime}")

    dest.mkdir(parents=True, exist_ok=True)
    for name in names:
        source = runtime / name
        if not source.is_file():
            sys.exit(f"[fail] {plat} runtime 缺少 {name}：{source}")
        shutil.copy2(source, dest / name)

    if fetch_fmod.PLUGIN_NAME[plat] not in names:
        print(f"[warn] 缺少 DSP 插件 {fetch_fmod.PLUGIN_NAME[plat]}（老 AC/mod bank 会报 ERR_PLUGIN_MISSING）")

    total = sum((dest / name).stat().st_size for name in names) / 1e6
    print(f"[ok] {plat} → {dest}（{len(names)} 个文件，{total:.1f} MB）")
    for name in names:
        print(f"       {name}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Copy the FMOD runtime libraries into a game export")
    parser.add_argument("--platform", action="append", choices=sorted(fetch_fmod.PLATFORMS), help="目标平台，可重复")
    parser.add_argument("--all", action="store_true", help="打包全部三个平台")
    parser.add_argument("--dest", type=Path, help="导出可执行文件所在目录（macOS 可传 .app）")
    parser.add_argument("--dest-root", type=Path, help="与 --all 搭配：每个平台一个子目录")
    parser.add_argument("--profile", choices=["release", "debug", "both"], default="release", help="拷贝哪个变体（默认 release）")
    parser.add_argument("--arch", help="目标架构：x86_64 / x64 / arm64 ...（默认宿主机，跨平台打包建议显式指定）")
    parser.add_argument("--offline", action="store_true", help="不下载，只用本地缓存（CI 用）")
    args = parser.parse_args()

    if args.all and args.dest_root is None:
        sys.exit("--all 需要 --dest-root")
    if not args.all and args.dest is None:
        sys.exit("需要 --dest（导出可执行文件目录或 .app）")
    if args.dest is not None and args.dest_root is not None:
        sys.exit("--dest 与 --dest-root 二选一")

    cache = Path(os.environ.get("FMOD_CACHE_DIR", fetch_fmod.repo_root() / ".cache" / "fmod"))
    plats = list(fetch_fmod.PLATFORMS) if args.all else (args.platform or [host_platform()])

    for plat in plats:
        dest = resolve_dest(args.dest_root / plat if args.all else args.dest, plat)
        pack(plat, dest, args.profile, cache, args.offline, args.arch)

    print("[next] 重新导出 / 启动游戏即可；FMOD runtime 由 dyld / dll 搜索路径在同目录找到")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
