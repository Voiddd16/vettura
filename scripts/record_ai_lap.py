#!/usr/bin/env python3
"""录一整圈 AI 驾驶视频：Godot 抓帧 + ffmpeg 编码成 mp4。

用法::

    uv run scripts/record_ai_lap.py                      # 完整一圈，默认赛道/车
    uv run scripts/record_ai_lap.py --every 3            # 20 fps 视频
    uv run scripts/record_ai_lap.py --max-s 60           # 只录前 60 秒
    uv run scripts/record_ai_lap.py --view cockpit       # 驾驶位视角
    uv run scripts/record_ai_lap.py --view drone         # 无人机追踪
    uv run scripts/record_ai_lap.py --view tcam          # 赛道边定点机位追踪
    uv run scripts/record_ai_lap.py --car res://mods/trr_porsche_992_gt3r
    uv run scripts/record_ai_lap.py --keep-frames        # 保留 JPEG 帧目录

链路：`res://gdext/ai_lap.tscn` 里的相机每渲染帧存一张 JPEG（
`AI_LAP_CAPTURE=1`），跑完一圈后本脚本用 ffmpeg 把帧序列编码成 H.264。
Godot 的 `--fixed-fps` 让物理按固定 1/60 s 步进，所以视频时长就是圈速，
不随机器快慢变化。

环境变量由本脚本转给 Godot：`AI_LAP_TRACK` / `AI_LAP_LAYOUT` / `AI_LAP_CAR` /
`AI_LAP_CAPTURE_DIR`（帧目录）、`AI_LAP_SKILL`（AI 难度 0..=110）与
`AI_LAP_VIEW`（chase / cockpit / hood / bumper / wheel / drone / tcam / orbit）。
"""

from __future__ import annotations

import argparse
import datetime as dt
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GODOT = "/Applications/Godot.app/Contents/MacOS/Godot"
SCENE = "res://gdext/ai_lap.tscn"
FRAME_GLOB = "f%05d.jpg"

# 默认难度档位：AC 的 AI level，100 = 职业。见 docs/architecture.md 的难度表。
DEFAULT_SKILL = 100


def find_ffmpeg() -> str:
    for candidate in ("ffmpeg", "/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"):
        found = shutil.which(candidate)
        if found:
            return found
    raise SystemExit("ffmpeg not found: install it with `brew install ffmpeg`")


def frame_dir_env() -> Path:
    """帧目录：默认 Godot 的 user data 目录（~/.…/app_userdata/Vettura）。"""
    override = os.environ.get("AI_LAP_CAPTURE_DIR")
    if override:
        return Path(override).expanduser().resolve()
    home = Path.home() / "Library/Application Support/Godot/app_userdata/Vettura/ai_lap_frames"
    return home


def main() -> int:
    parser = argparse.ArgumentParser(description="record a full AI lap as mp4")
    parser.add_argument("--every", type=int, default=2, help="每几帧抓一次（默认 2 → 30fps）")
    parser.add_argument("--fps", type=int, default=30, help="输出视频帧率（默认 30）")
    parser.add_argument("--max-s", type=float, default=400.0, help="圈时/最长跑动秒数")
    parser.add_argument("--laps", type=int, default=1, help="目标圈数")
    parser.add_argument("--skill", type=int, default=DEFAULT_SKILL, help="AI 难度 0..=110")
    parser.add_argument("--track", default=os.environ.get("AI_LAP_TRACK", "daytona_2017"))
    parser.add_argument("--layout", default=os.environ.get("AI_LAP_LAYOUT", "roadcourse"))
    parser.add_argument("--car", default=os.environ.get("AI_LAP_CAR", ""))
    parser.add_argument("--view", choices=("chase", "cockpit", "hood", "bumper",
                                          "wheel", "drone", "tcam", "orbit"),
                        default="chase",
                        help="相机视角：chase=车后（默认）、cockpit=驾驶位、"
                             "hood=前机盖、bumper=保险杠、wheel=轮胎、"
                             "drone=无人机、tcam=定点追踪、orbit=环绕")
    parser.add_argument("--out", type=Path, default=None, help="输出 mp4 路径")
    parser.add_argument("--keep-frames", action="store_true", help="编码后保留 JPEG 帧")
    parser.add_argument("--no-run", action="store_true", help="只编码已有帧，不跑 Godot")
    args = parser.parse_args()

    frames = frame_dir_env()
    frames.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    car_tag = args.car.rstrip("/").split("/")[-1] if args.car else "default"
    out = args.out or ROOT / f"build/ai_lap_{args.track}_{args.layout}_{car_tag}_{args.view}_{stamp}.mp4"
    out.parent.mkdir(parents=True, exist_ok=True)

    if not args.no_run:
        env = dict(os.environ)
        env.update(
            {
                "AI_LAP_CAPTURE": "1",
                "AI_LAP_CAM": "1",
                "AI_LAP_CAPTURE_EVERY": str(args.every),
                "AI_LAP_MAX_S": str(args.max_s),
                "AI_LAP_LAPS": str(args.laps),
                "AI_LAP_SKILL": str(args.skill),
                "AI_LAP_TRACK": args.track,
                "AI_LAP_LAYOUT": args.layout,
                "AI_LAP_VIEW": args.view,
            }
        )
        if args.car:
            env["AI_LAP_CAR"] = args.car
        print(f"[record] Godot 抓帧 -> {frames}  (每 {args.every} 帧一张)")
        print(f"[record] 难度 skill={args.skill}  目标 {args.laps} 圈  上限 {args.max_s:.0f}s")
        proc = subprocess.run(
            [GODOT, "--path", str(ROOT), SCENE, "--fixed-fps", "60"],
            env=env,
            capture_output=True,
            text=True,
        )
        tail = proc.stdout.strip().splitlines()[-8:]
        for line in tail:
            print("[godot]", line)
        if proc.stderr.strip():
            for line in proc.stderr.strip().splitlines()[-10:]:
                print("[godot-err]", line)
        if "finished" not in proc.stdout:
            print(f"[record] Godot 进程退出码 {proc.returncode}，没有抓到完整一圈")
            return proc.returncode or 1

    existing = sorted(frames.glob("f*.jpg"))
    print(f"[record] 帧序列 {len(existing)} 张 -> {frames}")
    if len(existing) < 30:
        print("[record] 帧太少，放弃编码")
        return 1

    ffmpeg = find_ffmpeg()
    encoded = subprocess.run(
        [
            ffmpeg,
            "-y",
            "-framerate",
            str(args.fps),
            "-i",
            str(frames / "f%05d.jpg"),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "24",
            "-preset",
            "medium",
            "-movflags",
            "+faststart",
            str(out),
        ],
        capture_output=True,
        text=True,
    )
    if encoded.returncode != 0:
        print(encoded.stderr[-2000:])
        return 1

    size_mb = out.stat().st_size / 1e6
    seconds = len(existing) / args.fps
    print(f"[record] 视频 {out}  ({size_mb:.1f} MB, {seconds:.0f}s @ {args.fps}fps)")
    print("[record] 需要更小/更大可以用 ffmpeg 重编码：-crf 28 更小、-crf 18 接近无损")
    if not args.keep_frames:
        for file in existing:
            file.unlink()
        print(f"[record] 已清理 {len(existing)} 张过渡帧（--keep-frames 可保留）")
    return 0


if __name__ == "__main__":
    sys.exit(main())