#!/usr/bin/env python3
"""KN5 加载链路性能探针：生成合成车模 + 跑 Godot 无头探针场景。

回答“加载慢在哪”：同一台合成车（默认 256 网格 × 4096 顶点 × 32 张 512 KiB
DXT1 贴图 ≈ 66 MiB，接近真实 mod 车）分别测量

1. `Kn5File.load()`——文件读取 + 解析；
2. 第一次 `instantiate()`——贴图解压、`ArrayMesh` 上传、节点树创建（冷路径）；
3. 新建 `Kn5File` 再加载一次——缓存条目失效后的重新加载；
4. 同一个 `Kn5File` 第二次实例化——解析/贴图/材质全部命中缓存；
5. `Kn5SceneBuilder` 分帧加载——进度反馈路径与每帧预算。

用法：

    python3 scripts/load_probe.py                 # 默认规模
    python3 scripts/load_probe.py --meshes 64 --vertices 1024
    python3 scripts/load_probe.py --keep           # 只跑探针，复用已生成的模型

只需 Python 3.10+（纯标准库）；Godot 可执行文件用 `--godot` 指定，
默认按平台探测 `/Applications/Godot.app/Contents/MacOS/Godot`。
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = ROOT / "target" / "probe"
MODEL = TARGET_DIR / "car.kn5"
PROBE_SCENE = "res://gdext/load_probe.tscn"

# 平台默认的 Godot 可执行文件；`$GODOT` 优先，都找不到时用 `--godot` 指定。
GODOT_CANDIDATES = (
    os.environ.get("GODOT"),
    shutil.which("godot"),
    shutil.which("godot4"),
    "/Applications/Godot.app/Contents/MacOS/Godot",
    "/usr/local/bin/godot",
)


def default_godot() -> str:
    for candidate in GODOT_CANDIDATES:
        if candidate and Path(candidate).exists():
            return str(candidate)
    sys.exit("找不到 Godot 可执行文件，请用 --godot 指定")


def run(command: list[str], *, cwd: Path = ROOT) -> None:
    print(f"$ {' '.join(str(part) for part in part_list(command))}")
    result = subprocess.run(command, cwd=cwd)
    if result.returncode != 0:
        sys.exit(f"命令失败（exit {result.returncode}）：{' '.join(map(str, command))}")


def part_list(command: list[str]) -> list[str]:
    return [str(part) for part in command]


def generate_model(godot: str, args: argparse.Namespace) -> None:
    """先构建 kn5-inspect，再由它生成合成 .kn5。"""
    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    run(["cargo", "build", "-p", "kn5-inspect"])
    inspect = ROOT / "target" / "debug" / "kn5-inspect"
    run(
        [
            inspect,
            "--synth",
            MODEL,
            "--synth-meshes",
            args.meshes,
            "--synth-vertices",
            args.vertices,
            "--synth-textures",
            args.textures,
            "--synth-texture-bytes",
            args.texture_bytes,
            f"--synth-truncated-mips={'true' if args.truncated_mips else 'false'}",
        ]
    )
    if not MODEL.exists():
        sys.exit(f"合成模型没有生成：{MODEL}")


def run_probe(godot: str) -> None:
    """跑无头探针场景；输出 `[load-probe]` 前缀的测量结果。"""
    command = [godot, "--headless", "--path", str(ROOT), PROBE_SCENE]
    result = subprocess.run(command, cwd=ROOT)
    if result.returncode != 0:
        sys.exit(f"探针失败（exit {result.returncode}）")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--godot", default=default_godot(), help="Godot 可执行文件路径")
    parser.add_argument("--meshes", type=int, default=256, help="合成模型网格数")
    parser.add_argument("--vertices", type=int, default=4096, help="每个网格的顶点数")
    parser.add_argument("--textures", type=int, default=32, help="合成模型贴图数")
    parser.add_argument("--texture-bytes", type=int, default=262144, help="每张贴图基础层字节数")
    parser.add_argument(
        "--truncated-mips",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="贴图写成 KN5 风格的截断 mip 链（Godot 需要修补）",
    )
    parser.add_argument("--keep", action="store_true", help="复用已生成的模型，不重新生成")
    args = parser.parse_args()

    if args.keep and MODEL.exists():
        print(f"复用 {MODEL}")
    else:
        generate_model(args.godot, args)

    size_mib = MODEL.stat().st_size / (1024 * 1024)
    print(f"合成模型：{MODEL}（{size_mib:.1f} MiB）")
    run_probe(args.godot)


if __name__ == "__main__":
    main()
