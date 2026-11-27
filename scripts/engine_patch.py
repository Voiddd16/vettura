#!/usr/bin/env python3
"""按 `engine/patches/<version>/series` 检查/应用 Godot 补丁。

这个脚本只做补丁顺序与 `git apply` 包装，不下载/构建 Godot，也不提交改动。

    python3 scripts/engine_patch.py --list
    python3 scripts/engine_patch.py --source ~/src/godot --check
    python3 scripts/engine_patch.py --source ~/src/godot --apply

`series` 文件每行一个补丁文件名（相对 `engine/patches/<version>/`），空行和
`#` 注释忽略。默认 `--version 4.7`，与 `project.godot` 的引擎版本保持一致。
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PATCH_ROOT = ROOT / "engine" / "patches"


def read_series(version: str) -> list[str]:
    series = PATCH_ROOT / version / "series"
    if not series.is_file():
        raise SystemExit(f"缺少补丁序列：{series}")
    names: list[str] = []
    for line in series.read_text(encoding="utf-8").splitlines():
        name = line.strip()
        if not name or name.startswith("#"):
            continue
        names.append(name)
    return names


def resolve_patches(version: str, names: list[str]) -> list[Path]:
    base = PATCH_ROOT / version
    patches: list[Path] = []
    for name in names:
        path = base / name
        if not path.is_file():
            raise SystemExit(f"series 列出的补丁不存在：{path}")
        patches.append(path)
    return patches


def ensure_git_repo(source: Path) -> None:
    if not source.is_dir():
        raise SystemExit(f"Godot 源码目录不存在：{source}")
    proc = subprocess.run(
        ["git", "-C", str(source), "rev-parse", "--is-inside-work-tree"],
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        raise SystemExit(f"不是 git 仓库：{source}")


def apply_patch(source: Path, patch: Path, check_only: bool) -> None:
    args = ["git", "-C", str(source), "apply", "--whitespace=nowarn"]
    if check_only:
        args.append("--check")
    args.append(str(patch))
    proc = subprocess.run(args, text=True, capture_output=True)
    if proc.stdout:
        print(proc.stdout, end="")
    if proc.returncode != 0:
        if proc.stderr:
            print(proc.stderr, end="", file=sys.stderr)
        raise SystemExit(f"补丁失败：{patch.name}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", default="4.7", help="补丁目录版本（默认 4.7）")
    parser.add_argument("--source", type=Path, help="Godot 源码目录")
    parser.add_argument("--list", action="store_true", help="只列出 series 内容")
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--check", action="store_true", help="只做 git apply --check")
    action.add_argument("--apply", action="store_true", help="真正应用补丁")
    args = parser.parse_args()

    names = read_series(args.version)
    patches = resolve_patches(args.version, names)

    if args.list or (not args.check and not args.apply):
        if not patches:
            print(f"engine/patches/{args.version}/series 当前为空。")
        for index, patch in enumerate(patches, start=1):
            print(f"{index:02d}  {patch.relative_to(ROOT)}")
        return

    if args.source is None:
        raise SystemExit("--check/--apply 需要 --source <Godot 源码目录>")
    ensure_git_repo(args.source)
    for patch in patches:
        print(f"[patch] {'check' if args.check else 'apply'} {patch.name}")
        apply_patch(args.source, patch, args.check)


if __name__ == "__main__":
    main()
