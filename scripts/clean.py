#!/usr/bin/env python3
"""清理构建产物与缓存：先看（默认预览），再加 `--run` 才真删。

分层（可多选，`all` = 全部）：

    quick      可再生且代价最小的缓存：cargo 增量缓存、`target/probe`、
               全仓 `__pycache__`
    rust       整个 `target/`（下次编译全量重编；已经编好的 Android .so
               也会被清掉，导 APK 前要重跑 `python3 scripts/build.py android`）
    godot      `.godot/` 导入缓存 + `.cache/godot/` 模板下载缓存
               （已安装的导出模板本体不动）
    artifacts  `build/` 导出产物（APK / 签名等）
    all        以上全部

典型用法：

    python3 scripts/clean.py                 # 预览：各项大小 + 会删什么
    python3 scripts/clean.py quick --run     # 只清快缓存（最常用）
    python3 scripts/clean.py all --run       # 全清

刻意不碰的东西：
- `~/Library/Application Support/Godot/export_templates/`——删了等于卸载
  导出模板，要重新 `python3 scripts/android.py templates` 才能导出；
- `user://`（含车库面板导入的车模库）与 `editor_settings-4.7.tres`；
- `.cache/` 里的 FMOD SDK——重新下载要 fmod.com 账号，想清直接手动删。
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEVEL_ORDER = ["quick", "rust", "godot", "artifacts"]
# os.walk 时直接跳过的目录（大、且不可能藏 __pycache__）。
WALK_SKIP = {".git", "target", ".godot", ".cache", "build", "dist", "node_modules"}
MIB = 1024 * 1024


def level_paths(level: str) -> list[Path]:
    target = ROOT / "target"
    if level == "quick":
        incremental = {target / "debug" / "incremental",
                       target / "release" / "incremental"}
        incremental |= set(target.glob("*/incremental"))
        return sorted(incremental) + [target / "probe", *pycache_dirs()]
    if level == "rust":
        return [target]
    if level == "godot":
        return [ROOT / ".godot", ROOT / ".cache" / "godot"]
    if level == "artifacts":
        return [ROOT / "build"]
    raise ValueError(f"未知清理层：{level}")


def pycache_dirs() -> list[Path]:
    found: list[Path] = []
    for parent, dirs, _files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in WALK_SKIP]
        found.extend(Path(parent) / d for d in dirs if d == "__pycache__")
    return found


def resolve(levels: list[str]) -> list[Path]:
    """多选去重 + 剔除被祖先覆盖的条目（`all` 里 quick 被 rust 吸收）。"""
    chosen: list[Path] = []
    for level in levels:
        if level == "all":
            chosen.extend(roots for name in LEVEL_ORDER
                          for roots in level_paths(name))
        else:
            chosen.extend(level_paths(level))

    unique: list[Path] = []
    for path in chosen:
        if path in unique:
            continue
        if any(ancestor != path and path.is_relative_to(ancestor)
               for ancestor in chosen):
            continue  # 祖先要整树删，子条目不用单独列
        unique.append(path)
    return unique


def tree_size(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    total = 0
    for child in path.rglob("*"):
        try:
            if child.is_file():
                total += child.stat().st_size
        except OSError:
            pass
    return total


def delete(path: Path) -> None:
    if path.is_dir():
        shutil.rmtree(path, ignore_errors=True)
    else:
        path.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("levels", nargs="*", metavar="LEVEL",
                        choices=[*LEVEL_ORDER, "all"],
                        help="要清理的层（quick/rust/godot/artifacts/all，可多选）")
    parser.add_argument("--run", action="store_true",
                        help="真的执行删除；缺省只预览")
    args = parser.parse_args()

    levels = args.levels or ["quick"]
    plan = resolve(levels)
    if not plan:
        print("没有可清理的条目")
        return

    total = 0
    for path in plan:
        if not path.exists():
            print(f"  --  （不存在）         {path.relative_to(ROOT)}")
            continue
        size = tree_size(path)
        total += size
        print(f"  {'删除' if args.run else '将删'} {size / MIB:8.1f} MB  "
              f"{path.relative_to(ROOT)}")

    print(f"\n合计 {total / 1e9:.2f} GB（{total / MIB:.0f} MiB），"
          f"{'已执行' if args.run else '预览模式，加 --run 才真删'}")
    if args.run:
        for path in plan:
            delete(path)
        print("清理完成；`target/android/<abi>/` 的 .so 如被清除，"
              "导 APK 前先跑：python3 scripts/build.py android")


if __name__ == "__main__":
    main()
