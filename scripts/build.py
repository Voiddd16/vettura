#!/usr/bin/env python3
"""一键编译：宿主 GDExtension 库 / Android .so / 导出 APK。

Android 编译与导出的细节（NDK 定位、ABI 环境变量、编辑器模板安装）都复用
`scripts/android.py`，本脚本只负责把"日常要编什么"包装成一条命令：

    python3 scripts/build.py                  # host debug 库（编辑器/无头跑工程用）
    python3 scripts/build.py host --release   # host release 库（macOS 导出包用）
    python3 scripts/build.py android          # arm64-v8a + x86_64 的 .so
    python3 scripts/build.py apk              # 导出 build/android/Vettura.apk
    python3 scripts/build.py all --check      # check + clippy 过了再全链编译
    python3 scripts/build.py check            # 只跑 cargo check + clippy
    python3 scripts/build.py test             # cargo test --workspace

说明：
- `host` 直接驱动 cargo，产物落在 `target/debug|release/`，
  正好是 `gdext/vettura_native.gdextension` 里各平台条目指向的位置；
- `android` / `apk` 是 `scripts/android.py lib|export` 的转发；
- `all` = host + android + apk；FMOD 默认不参与（发行前可选跑
  `scripts/pack_fmod.py`，见 docs/audio.md）。
"""

from __future__ import annotations

import argparse
import platform
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import android as android_script  # noqa: E402

ROOT = android_script.ROOT
CRATE = "vettura-godot"

HOST_PLUGIN_BY_SYSTEM = {
    "Darwin": "libvettura_native.dylib",
    "Linux": "libvettura_native.so",
    "Windows": "vettura_native.dll",
}


def check() -> None:
    """快速把关：类型检查 + lint。失败即止，不浪费时间往下编。"""
    with android_script.Step("cargo check --workspace"):
        android_script.run(["cargo", "check", "--workspace"])
    with android_script.Step("cargo clippy（警告视为错误）"):
        android_script.run([
            "cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings",
        ])


def test() -> None:
    with android_script.Step("cargo test --workspace"):
        android_script.run(["cargo", "test", "--workspace"])


def step_host(release: bool) -> None:
    profile = "release" if release else "debug"
    args = ["build", "-p", CRATE] + (["--release"] if release else [])
    with android_script.Step(f"编译宿主库（{profile}）"):
        android_script.run(["cargo", *args])
        name = HOST_PLUGIN_BY_SYSTEM[platform.system()]
        artifact = ROOT / "target" / profile / name
        if not artifact.exists():
            raise RuntimeError(f"编译产物缺失：{artifact}")
        print(f"  产物：{artifact}（{artifact.stat().st_size / 1e6:.1f} MB）")


def step_android(args: argparse.Namespace) -> None:
    android_script.step_lib(args)


def step_apk(args: argparse.Namespace) -> None:
    android_script.step_export(args)


def android_namespace(args: argparse.Namespace) -> argparse.Namespace:
    """给 android.py 的步骤构造它认识的参数（godot/cache/force 由它读取）。"""
    return argparse.Namespace(
        godot=args.godot,
        cache=str(ROOT / ".cache" / "godot"),
        force=False,
        ndk=False,
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", nargs="?", default="host",
                        choices=["host", "android", "apk", "all", "check", "test"],
                        help="要执行的动作（默认 host）")
    parser.add_argument("--release", action="store_true",
                        help="host/all 的宿主库用 release profile（导出包需要）")
    parser.add_argument("--check", action="store_true",
                        help="正式编译前先跑 check")
    parser.add_argument("--godot", default=str(android_script.default_godot()),
                        help="Godot 可执行文件（apk/all 用）")
    args = parser.parse_args()

    if args.action == "check":
        check()
        return
    if args.action == "test":
        test()
        return

    if args.check:
        check()

    ns = android_namespace(args)
    if args.action == "host":
        step_host(args.release)
    elif args.action == "android":
        step_android(ns)
    elif args.action == "apk":
        step_apk(ns)
    else:  # all
        step_host(args.release)
        step_android(ns)
        step_apk(ns)


if __name__ == "__main__":
    main()
