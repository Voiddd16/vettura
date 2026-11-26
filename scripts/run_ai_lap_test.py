#!/usr/bin/env python3
"""整圈 AI 测试全流程：全量重编译 → 测试 → 两段录像 → 混流 → 合并 → 报告。

默认配置就是项目常用的 Daytona roadcourse / Porsche 992 GT3 R / skill 110：

    python3 scripts/run_ai_lap_test.py

流程：
1. （默认）`cargo clean` + `cargo build --workspace` + 带 fmod 的 GDExtension；
2. `cargo test --workspace` 与 `cargo fmt --check`；
3. Godot 分别以车后/驾驶位镜头跑整圈：JPEG 抓帧 + FMOD WAV 音轨（分开采集）；
4. ffmpeg 编码视频，按音轨/视频时长用 atempo 对齐后混流；
5. 两段合并成一个视频；
6. 抽帧与音量检查，写 REPORT.md。

常用参数：

    --no-clean        不 clean，增量编译
    --skip-build      不编译，直接用现有 dylib
    --skip-tests      跳过测试与 fmt
    --views chase     只录车后（默认 chase,cockpit；也可重复 --view）
    --view drone      其它视角：hood/bumper/wheel/drone/tcam/orbit
    --no-audio        只录视频、不采集/混流音频
    --parallel        两段并行跑（不推荐：抢 GPU，实时窗口容易卡顿）
    --keep-frames     保留 JPEG 帧
    --out-dir PATH    指定输出目录（默认 build/ai_lap_test/<时间戳>）

赛道 / 车辆 / 视角都可参数化：

    --track daytona_2017 --layout roadcourse
    --car res://mods/trr_porsche_992_gt3r
    --view chase --view cockpit
    --view drone --view tcam
    --weather clear|overcast|rain|fog|night
    --eye-x/-y/-z ...                 驾驶位相机偏移（米）
    --chase-distance/-height/-lookahead/-fov
    --cockpit-fov

纯标准库实现，无第三方依赖；需要本机有 cargo / Godot / ffmpeg。
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCENE = "res://gdext/ai_lap.tscn"
CAR_DEFAULT = "res://mods/trr_porsche_992_gt3r"
TRACK_DEFAULT = "daytona_2017"
LAYOUT_DEFAULT = "roadcourse"
SKILL_DEFAULT = 110
RESOLUTION_DEFAULT = "1920x1080"
EVERY_DEFAULT = 2          # 每 2 个渲染帧抓一张 → 60fps / 2 = 30fps
FPS_DEFAULT = 30
CRF_DEFAULT = 24
MAX_S_DEFAULT = 400.0

VIEW_ORDER = {"chase": 1, "cockpit": 2, "hood": 3, "bumper": 4,
              "wheel": 5, "drone": 6, "tcam": 7, "orbit": 8}


# ---------------------------------------------------------------------------
# 基础工具
# ---------------------------------------------------------------------------

def log(message: str) -> None:
    print(message, flush=True)


def step(title: str) -> None:
    log(f"\n=== {title} ===")


def default_godot() -> str:
    env = os.environ.get("GODOT")
    if env:
        return env
    candidates = [
        "/Applications/Godot.app/Contents/MacOS/Godot",
        "/Applications/Godot_mono.app/Contents/MacOS/Godot",
        shutil.which("godot"),
        shutil.which("godot4"),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists():
            return candidate
    return "godot"


def find_tool(name: str, override: str | None = None) -> str:
    if override:
        return override
    for candidate in (name, f"/opt/homebrew/bin/{name}", f"/usr/local/bin/{name}"):
        found = shutil.which(candidate)
        if found:
            return found
    raise SystemExit(f"{name} not found; 安装后重试或显式传入路径")


def run(cmd: list[str], *, env: dict[str, str] | None = None,
        log_path: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    """运行命令；日志写文件（同时错误时把尾部打出来）。"""
    sink = None
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        sink = open(log_path, "w", encoding="utf-8", errors="replace")
    try:
        proc = subprocess.run(
            cmd, cwd=ROOT, env=env, stdout=sink, stderr=subprocess.STDOUT, text=True
        )
    finally:
        if log_path is not None:
            sink.close()
    if check and proc.returncode != 0:
        tail = ""
        if log_path is not None and log_path.exists():
            tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-25:])
        raise SystemExit(f"命令失败（exit {proc.returncode}）：{' '.join(cmd)}\n{tail}")
    return proc


def media_duration(path: Path, ffprobe: str) -> float:
    out = subprocess.run(
        [ffprobe, "-v", "error", "-show_entries", "format=duration",
         "-of", "json", str(path)],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        raise SystemExit(f"ffprobe 读取时长失败：{path}\n{out.stderr[-800:]}")
    return float(json.loads(out.stdout)["format"]["duration"])


def detect_duplicate_run(path: str, ffmpeg: str) -> tuple[int, int]:
    """逐帧 framemd5，返回 (最长连续相同帧数, 起始帧号)。

    比 `freezedetect` 可靠：赛道大片同色区域会让基于像素容差的检测误报，
    而完全相同的解码帧才会命中同一个 MD5。
    """
    proc = subprocess.run(
        [ffmpeg, "-v", "error", "-i", str(path), "-map", "0:v:0", "-f", "framemd5", "-"],
        capture_output=True, text=True,
    )
    hashes = [
        line.split(",")[-1].strip()
        for line in proc.stdout.splitlines()
        if line and not line.startswith("#")
    ]
    best = current = 1
    best_at = 0
    for index in range(1, len(hashes)):
        if hashes[index] == hashes[index - 1]:
            current += 1
            if current > best:
                best, best_at = current, index - current + 1
        else:
            current = 1
    return (best, best_at) if hashes else (0, 0)


def atempo_chain(factor: float) -> str:
    """把任意 atempo 比例拆成若干 0.5..2.0 的段（兼容旧 ffmpeg）。"""
    parts: list[float] = []
    while factor > 2.0:
        parts.append(2.0)
        factor /= 2.0
    while factor < 0.5:
        parts.append(0.5)
        factor /= 0.5
    parts.append(factor)
    return ",".join(f"atempo={p:.6f}" for p in parts)


def tail_lines(path: Path, count: int = 12) -> str:
    if not path.exists():
        return ""
    return "\n".join(path.read_text(errors="replace").splitlines()[-count:])


def parse_lap(path: Path) -> dict[str, str]:
    """从 ai_lap 日志里抓最后一圈的关键数据。"""
    result: dict[str, str] = {}
    if not path.exists():
        return result
    text = path.read_text(errors="replace")
    finished = re.findall(
        r"\[ai-lap\] finished t=([\d.]+).*?max_speed=([\d.]+).*?laps=(\d+).*?"
        r"last=([-\d.]+).*?best=([-\d.]+).*?max_airborne=([\d.]+)",
        text,
    )
    if finished:
        t, speed, laps, last, best, airborne = finished[-1]
        result.update(t=t, max_speed=speed, laps=laps, last=last,
                      best=best, max_airborne=airborne)
    lap = re.findall(r"\[ai-lap\] LAP \d+ done: last=([\d.]+) s .*?valid\(last\)=(\w+)", text)
    if lap:
        result["last"], result["valid"] = lap[-1]
    result["pass"] = "PASS" if "[ai-lap] PASS" in text else "FAIL"
    return result


# ---------------------------------------------------------------------------
# 编译与测试
# ---------------------------------------------------------------------------

def cargo_env() -> dict[str, str]:
    """cargo 子进程环境：macOS 上兜底 CLT 缺失的 libc++ 头。

    个别机器上 CommandLineTools 的 `usr/include/c++/v1` 只剩几个旧头，
    Jolt 之类的 C++ 依赖 `#include <new>` 会失败；SDK 里有完整一套，
    用 `CPLUS_INCLUDE_PATH` 补上。正常情况下不改动环境。
    """
    env = dict(os.environ)
    if sys.platform != "darwin":
        return env
    try:
        cxx = subprocess.run(["xcrun", "--find", "c++"], capture_output=True,
                             text=True, check=True).stdout.strip()
        toolchain_inc = Path(cxx).parent.parent / "include" / "c++" / "v1"
        if (toolchain_inc / "new").exists():
            return env
        sdk = subprocess.run(["xcrun", "--show-sdk-path"], capture_output=True,
                             text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return env
    sdk_inc = Path(sdk) / "usr" / "include" / "c++" / "v1"
    if not (sdk_inc / "new").exists():
        return env
    existing = env.get("CPLUS_INCLUDE_PATH")
    env["CPLUS_INCLUDE_PATH"] = f"{sdk_inc}:{existing}" if existing else str(sdk_inc)
    log(f"  [环境] CLT libc++ 头不完整，已用 SDK 兜底：{sdk_inc}")
    return env


def build_all(godot_build: bool, tests: bool, clean: bool) -> dict[str, str]:
    info: dict[str, str] = {}
    env = cargo_env()
    if clean:
        step("cargo clean（全量）")
        started = time.time()
        run(["cargo", "clean"])
        # cargo clean 会清空整个 target/，包括被跟踪的 .gdignore，补回来。
        gdignore = ROOT / "target" / ".gdignore"
        if not gdignore.exists():
            subprocess.run(["git", "checkout", "--", "target/.gdignore"],
                           cwd=ROOT, capture_output=True)
        info["clean_s"] = f"{time.time() - started:.0f}"

    if godot_build:
        step("cargo build --workspace")
        started = time.time()
        run(["cargo", "build", "--workspace"], env=env)
        info["workspace_s"] = f"{time.time() - started:.0f}"

        step("cargo build -p vettura-godot --features fmod")
        started = time.time()
        run(["cargo", "build", "-p", "vettura-godot", "--features", "fmod"], env=env)
        info["gdext_s"] = f"{time.time() - started:.0f}"

    if tests:
        step("cargo test --workspace")
        started = time.time()
        log_path = ROOT / "build" / "ai_lap_test" / "cargo_test.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        run(["cargo", "test", "--workspace"], env=env, log_path=log_path)
        passed = failed = ignored = 0
        for line in log_path.read_text(errors="replace").splitlines():
            m = re.match(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", line)
            if m:
                passed += int(m.group(1))
                failed += int(m.group(2))
                ignored += int(m.group(3))
        info["tests"] = f"{passed} passed / {failed} failed / {ignored} ignored"
        info["tests_s"] = f"{time.time() - started:.0f}"

        step("cargo fmt --all -- --check")
        run(["cargo", "fmt", "--all", "--", "--check"], env=env)
        info["fmt"] = "ok"
    return info


# ---------------------------------------------------------------------------
# 录制
# ---------------------------------------------------------------------------

def recording_env(args: argparse.Namespace, view: str, out_dir: Path,
                  audio: bool) -> dict[str, str]:
    env = dict(os.environ)
    env.update({
        "AI_LAP_TRACK": args.track,
        "AI_LAP_LAYOUT": args.layout,
        "AI_LAP_CAR": args.car,
        "AI_LAP_SKILL": str(args.skill),
        "AI_LAP_PACE": str(args.ai_pace),
        "AI_LAP_SPAWN_GRID": str(args.spawn_grid),
        "AI_LAP_VIEW": view,
        "AI_LAP_MAX_S": str(args.max_s),
        "AI_LAP_LAPS": "1",
        "AI_LAP_REQUIRE_LAP": "1",
        "AI_LAP_CAPTURE": "1",
        "AI_LAP_CAM": "1",
        "AI_LAP_CAPTURE_EVERY": str(args.every),
        "AI_LAP_CAPTURE_DIR": str(out_dir / f"frames_{view}"),
    })
    # 冲突时清掉旧值，避免外层 shell 的环境变量串进来。
    for name in ("AI_LAP_WEATHER", "AI_LAP_EYE_X", "AI_LAP_EYE_Y", "AI_LAP_EYE_Z",
                 "AI_LAP_CHASE_DISTANCE", "AI_LAP_CHASE_HEIGHT",
                 "AI_LAP_CHASE_LOOKAHEAD", "AI_LAP_CHASE_FOV", "AI_LAP_COCKPIT_FOV"):
        env.pop(name, None)
    optional = {
        "AI_LAP_WEATHER": args.weather,
        "AI_LAP_EYE_X": args.eye_x,
        "AI_LAP_EYE_Y": args.eye_y,
        "AI_LAP_EYE_Z": args.eye_z,
        "AI_LAP_CHASE_DISTANCE": args.chase_distance,
        "AI_LAP_CHASE_HEIGHT": args.chase_height,
        "AI_LAP_CHASE_LOOKAHEAD": args.chase_lookahead,
        "AI_LAP_CHASE_FOV": args.chase_fov,
        "AI_LAP_COCKPIT_FOV": args.cockpit_fov,
    }
    for name, value in optional.items():
        if value is not None:
            env[name] = str(value)
    if audio:
        env["VETTURA_AUDIO_WAV"] = str(out_dir / f"audio_{view}.wav")
    return env


def record(args: argparse.Namespace, views: list[str], out_dir: Path) -> dict[str, dict]:
    step(f"录制 {', '.join(views)}（{args.resolution}, skill {args.skill}）")
    godot = args.godot
    results: dict[str, dict] = {}

    def launch(view: str) -> subprocess.Popen:
        env = recording_env(args, view, out_dir, audio=args.audio)
        scenario_log = out_dir / f"{view}_run.log"
        scenario_log.parent.mkdir(parents=True, exist_ok=True)
        sink = open(scenario_log, "w", encoding="utf-8", errors="replace")
        cmd = [godot, "--path", str(ROOT), SCENE,
               "--fixed-fps", "60", "--resolution", args.resolution]
        proc = subprocess.Popen(cmd, cwd=ROOT, env=env,
                                stdout=sink, stderr=subprocess.STDOUT)
        proc._vettura_log = scenario_log  # type: ignore[attr-defined]
        proc._vettura_sink = sink  # type: ignore[attr-defined]
        return proc

    procs: dict[str, subprocess.Popen] = {}
    # 默认串行：两个 Godot 各自做 1080p GPU 回读，并行会互相抢 GPU，
    # 渲染停住、实时窗口卡顿。成片不受影响（异步写帧 + 帧号连续）。
    if args.parallel:
        log("  [警告] 并行录制会抢 GPU，实时窗口可能卡顿")
        for view in views:
            procs[view] = launch(view)
        for view in views:
            procs[view].wait()
    else:
        for view in views:
            procs[view] = launch(view)
            procs[view].wait()

    for view in views:
        proc = procs[view]
        log_path: Path = proc._vettura_log  # type: ignore[attr-defined]
        proc._vettura_sink.close()  # type: ignore[attr-defined]
        info = parse_lap(log_path)
        frames = out_dir / f"frames_{view}"
        info["frames"] = str(len(list(frames.glob("f*.jpg")))) if frames.is_dir() else "0"
        info["log"] = str(log_path)
        results[view] = info
        log(f"  {view}: {info.get('pass')} last={info.get('last')}s "
            f"frames={info['frames']}")
        if info.get("pass") != "PASS":
            log(tail_lines(log_path, 25))
            raise SystemExit(f"{view} 录制未通过（见 {log_path}）")
        if int(info["frames"]) < 30:
            raise SystemExit(f"{view} 抓帧太少（{info['frames']}）")
    return results


# ---------------------------------------------------------------------------
# 编码 / 混流 / 合并
# ---------------------------------------------------------------------------

def encode_and_mux(args: argparse.Namespace, views: list[str],
                   out_dir: Path, ffmpeg: str, ffprobe: str) -> dict[str, dict]:
    videos = out_dir / "videos"
    videos.mkdir(parents=True, exist_ok=True)
    encoded: dict[str, dict] = {}
    for view in views:
        idx = VIEW_ORDER.get(view, len(encoded) + 1)
        frames = out_dir / f"frames_{view}"
        video = videos / f"{idx:02d}_{view}_video.mp4"
        step(f"编码 {view}")
        run([
            ffmpeg, "-y", "-framerate", str(args.fps),
            "-i", str(frames / "f%05d.jpg"),
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-crf", str(args.crf), "-preset", "medium",
            "-movflags", "+faststart", str(video),
        ], log_path=out_dir / f"encode_{view}.log")

        record = {"video": str(video), "duration": media_duration(video, ffprobe)}
        if args.audio:
            audio = out_dir / f"audio_{view}.wav"
            if not audio.exists():
                raise SystemExit(f"{view} 音轨缺失：{audio}")
            audio_s = media_duration(audio, ffprobe)
            tempo = audio_s / record["duration"]
            final = videos / f"{idx:02d}_{view}.mp4"
            step(f"混流 {view}（atempo {tempo:.4f}）")
            run([
                ffmpeg, "-y", "-i", str(video), "-i", str(audio),
                "-filter:a", atempo_chain(tempo),
                "-c:v", "copy", "-c:a", "aac", "-b:a", "192k",
                "-shortest", "-movflags", "+faststart", str(final),
            ], log_path=out_dir / f"mux_{view}.log")
            record.update(audio=str(audio), audio_duration=audio_s, atempo=tempo)
            record["final"] = str(final)
        else:
            record["final"] = str(video)
        encoded[view] = record
        log(f"  {view}: {record['duration']:.1f}s -> {Path(record['final']).name}")
    return encoded


def concat(args: argparse.Namespace, views: list[str], encoded: dict[str, dict],
           out_dir: Path, ffmpeg: str, ffprobe: str) -> tuple[str | None, float]:
    videos = out_dir / "videos"
    track_tag = args.track.rstrip("/").split("/")[-1]
    car_tag = args.car.rstrip("/").split("/")[-1]
    res_tag = f"{args.resolution.split('x')[0]}p"
    suffix = "_two_cameras_with_audio" if args.audio else "_two_cameras"
    combined = videos / f"{track_tag}_{car_tag}_skill{args.skill}_{res_tag}{suffix}.mp4"
    list_file = videos / "concat.txt"
    list_file.write_text(
        "".join(f"file '{encoded[v]['final']}'\n" for v in views), encoding="utf-8"
    )
    step("合并两段")
    run([
        ffmpeg, "-y", "-f", "concat", "-safe", "0", "-i", str(list_file),
        "-c", "copy", "-movflags", "+faststart", str(combined),
    ], log_path=out_dir / "encode_concat.log")
    duration = media_duration(combined, ffprobe)
    log(f"  合并版：{combined.name}（{duration:.1f}s）")
    return str(combined), duration


# ---------------------------------------------------------------------------
# QA / 报告
# ---------------------------------------------------------------------------

def qa(args: argparse.Namespace, views: list[str], encoded: dict[str, dict],
       combined: str | None, out_dir: Path, ffmpeg: str) -> dict[str, str]:
    info: dict[str, str] = {}
    # 不合并时用第一段成片做 QA 源。
    source = combined or encoded[views[0]]["final"]
    if args.audio:
        proc = subprocess.run(
            [ffmpeg, "-i", source, "-vn", "-af", "volumedetect", "-f", "null", "-"],
            capture_output=True, text=True,
        )
        for line in proc.stderr.splitlines():
            for key in ("mean_volume", "max_volume"):
                if key in line:
                    info[key] = line.split(":")[-1].strip()
        log(f"  合并版音轨：mean={info.get('mean_volume')} max={info.get('max_volume')}")

    # 重复帧检测：GPU 停住时抓帧会重复读到同一张图，成片表现为连续相同帧。
    targets = [(view, encoded[view]["final"]) for view in views]
    if combined and len(views) > 1:
        targets.append(("combined", combined))
    worst = 0.0
    for name, path in targets:
        frames, at_frame = detect_duplicate_run(path, ffmpeg)
        seconds = frames / max(args.fps, 1)
        info[f"dup_{name}"] = (frames, seconds, at_frame / max(args.fps, 1))
        worst = max(worst, seconds)
        log(f"  重复帧检测 {name}: 最长 {frames} 帧（{seconds:.2f}s）"
            f" @ {at_frame / max(args.fps, 1):.1f}s")
    if worst > 0.5:
        log(f"  [警告] 检测到连续重复帧（最长 {worst:.2f}s），检查 GPU 负载或录制方式")

    # 抽两帧做视觉检查：起点 + 第二段中段（或第一段中段）。
    first = encoded[views[0]]["duration"]
    picks = [(1.0, "qa_start")]
    if combined and len(views) > 1:
        picks.append((first + 1.0, "qa_second"))
    else:
        picks.append((first * 0.5, "qa_mid"))
    for ts, name in picks:
        out = out_dir / f"{name}.jpg"
        subprocess.run(
            [ffmpeg, "-y", "-ss", f"{ts:.1f}", "-i", source,
             "-frames:v", "1", "-map", "0:v:0", str(out), "-loglevel", "error"],
            capture_output=True,
        )
        if out.exists():
            info[name] = str(out)
    return info


def write_report(args: argparse.Namespace, views: list[str], build: dict[str, str],
                 recordings: dict[str, dict], encoded: dict[str, dict],
                 combined: str | None, combined_duration: float,
                 qa_info: dict[str, str], out_dir: Path) -> Path:
    lines = [
        "# AI 整圈测试报告",
        "",
        f"- 生成时间：{dt.datetime.now().isoformat(timespec='seconds')}",
        f"- 赛道：`{args.track}`（{args.layout}）",
        f"- 车辆：`{args.car}`",
        f"- AI 难度：{args.skill}；外部配速倍率：{args.ai_pace}；分辨率：{args.resolution}；"
        f"抓帧间隔：每 {args.every} 帧",
        f"- 视角：{', '.join(views)}；天气：{args.weather or 'clear（默认）'}；"
        f"发车格：{args.spawn_grid}",
        "",
        "## 相机 / 场景参数",
        "",
        f"- 车后：距离 {args.chase_distance or 4.5} m / 高度 {args.chase_height or 2.0} m / "
        f"前视 {args.chase_lookahead or 14.0} m / FOV {args.chase_fov or 72.0}",
        f"- 驾驶位：眼位 ({args.eye_x if args.eye_x is not None else '-0.367'}, "
        f"{args.eye_y if args.eye_y is not None else '1.000'}, "
        f"{args.eye_z if args.eye_z is not None else '0.157'}) m / "
        f"FOV {args.cockpit_fov or 70.0}",
        "",
        "## 编译与测试",
        "",
    ]
    labels = {
        "clean_s": "cargo clean",
        "workspace_s": "cargo build --workspace",
        "gdext_s": "cargo build -p vettura-godot --features fmod",
        "tests": "cargo test --workspace",
        "fmt": "cargo fmt --check",
    }
    if not any(key in build for key in labels):
        lines.append("- 跳过（--skip-build / --skip-tests）")
    for key, label in labels.items():
        if key in build:
            value = build[key] + ("s" if key.endswith("_s") else "")
            lines.append(f"- {label}：{value}")
    lines += ["", "## 整圈结果", "",
              "| 视角 | 圈速 | 有效性 | 最高速 | 抓帧 | 结果 |",
              "| --- | --- | --- | --- | --- | --- |"]
    for view in views:
        r = recordings[view]
        lines.append(
            f"| {view} | {r.get('last', '-')} s | {r.get('valid', '-')} | "
            f"{r.get('max_speed', '-')} km/h | {r.get('frames', '-')} | {r.get('pass', '-')} |"
        )
    lines += ["", "## 成片", "",
              "| 文件 | 时长 | 说明 |", "| --- | --- | --- |"]
    for view in views:
        e = encoded[view]
        note = "带声音" if args.audio else "无声"
        lines.append(f"| `{Path(e['final']).name}` | {e['duration']:.1f} s | {note} |")
    if combined and len(views) > 1:
        lines.append(
            f"| `{Path(combined).name}` | {combined_duration:.1f} s | 两段合并 |"
        )
    if qa_info:
        lines += ["", "## QA", ""]
        if "mean_volume" in qa_info:
            lines.append(
                f"- 合并版音轨：mean `{qa_info.get('mean_volume')}` / "
                f"max `{qa_info.get('max_volume')}`"
            )
        dup = {k: v for k, v in qa_info.items() if k.startswith("dup_")}
        if dup:
            lines += ["", "## 重复帧检测", ""]
            for key, (frames, seconds, at) in dup.items():
                name = key[len("dup_"):]
                lines.append(
                    f"- {name}：最长连续相同帧 {frames} 帧"
                    f"（{seconds:.2f} s，约 {at:.1f} s 处）"
                )
        for key, value in qa_info.items():
            if key.startswith("qa_"):
                lines.append(f"- 抽帧：`{Path(value).name}`")
    report = out_dir / "REPORT.md"
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return report


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--godot", default=default_godot(), help="Godot 可执行文件")
    parser.add_argument("--ffmpeg", default=None, help="ffmpeg 路径（默认自动查找）")
    parser.add_argument("--ffprobe", default=None, help="ffprobe 路径（默认自动查找）")
    parser.add_argument("--track", default=TRACK_DEFAULT)
    parser.add_argument("--layout", default=LAYOUT_DEFAULT)
    parser.add_argument("--car", default=CAR_DEFAULT)
    parser.add_argument("--skill", type=int, default=SKILL_DEFAULT)
    parser.add_argument("--ai-pace", type=float, default=1.0,
                        help="AI 外部配速倍率（0.5..3.0，默认 1.0；110 档自带约 1.65）")
    parser.add_argument("--spawn-grid", type=int, default=1,
                        help="发车格序号（0=起跑线，1=线后 8 m，默认 1；-1 用起跑线）")
    parser.add_argument("--resolution", default=RESOLUTION_DEFAULT)
    parser.add_argument("--every", type=int, default=EVERY_DEFAULT)
    parser.add_argument("--fps", type=int, default=FPS_DEFAULT)
    parser.add_argument("--crf", type=int, default=CRF_DEFAULT)
    parser.add_argument("--max-s", type=float, default=MAX_S_DEFAULT)
    parser.add_argument("--views", default=None,
                        help="逗号分隔：chase,cockpit（默认两段）；"
                             "可选 hood/bumper/wheel/drone/tcam/orbit")
    parser.add_argument("--view", action="append", default=None,
                        metavar="VIEW", help="视角，可重复：--view chase --view drone")
    parser.add_argument("--weather", default=None,
                        choices=["clear", "overcast", "rain", "fog", "night"],
                        help="天气预设（默认 clear）")
    parser.add_argument("--eye-x", type=float, default=None, help="驾驶位相机 X 偏移（米）")
    parser.add_argument("--eye-y", type=float, default=None, help="驾驶位相机 Y 偏移（米）")
    parser.add_argument("--eye-z", type=float, default=None, help="驾驶位相机 Z 偏移（米）")
    parser.add_argument("--chase-distance", type=float, default=None,
                        help="车后镜头距离（默认 4.5 m）")
    parser.add_argument("--chase-height", type=float, default=None,
                        help="车后镜头高度（默认 2.0 m）")
    parser.add_argument("--chase-lookahead", type=float, default=None,
                        help="车后镜头前视距离（默认 14 m）")
    parser.add_argument("--chase-fov", type=float, default=None, help="车后镜头 FOV（默认 72）")
    parser.add_argument("--cockpit-fov", type=float, default=None,
                        help="驾驶位镜头 FOV（默认 70）")
    parser.add_argument("--out-dir", type=Path, default=None)
    parser.add_argument("--clean", dest="clean", action="store_true", default=True,
                        help="编译前 cargo clean（默认）")
    parser.add_argument("--no-clean", dest="clean", action="store_false")
    parser.add_argument("--skip-build", action="store_true",
                        help="跳过 cargo build，直接用现有 dylib")
    parser.add_argument("--skip-tests", action="store_true", help="跳过 test 与 fmt")
    parser.add_argument("--no-audio", dest="audio", action="store_false", default=True,
                        help="只录视频，不采集/混流音频")
    parser.add_argument("--parallel", action="store_true",
                        help="两段并行录制（不推荐：抢 GPU，实时窗口容易卡顿）")
    parser.add_argument("--keep-frames", action="store_true", help="保留 JPEG 帧")
    parser.add_argument("--no-concat", action="store_true", help="不合并两段")
    args = parser.parse_args()

    if args.view:
        views = [v.strip() for v in args.view if v.strip()]
    elif args.views:
        views = [v.strip() for v in args.views.split(",") if v.strip()]
    else:
        views = ["chase", "cockpit"]
    for view in views:
        if view not in VIEW_ORDER:
            raise SystemExit(
                f"未知视角：{view}（可选 {'/'.join(VIEW_ORDER)}）")

    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = (args.out_dir or (ROOT / "build" / "ai_lap_test" / stamp)).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    ffmpeg = find_tool("ffmpeg", args.ffmpeg)
    ffprobe = find_tool("ffprobe", args.ffprobe)

    log(f"[run_ai_lap_test] 输出目录：{out_dir}")
    build = build_all(
        godot_build=not args.skip_build,
        tests=not args.skip_tests,
        clean=args.clean and not args.skip_build,
    )

    recordings = record(args, views, out_dir)
    encoded = encode_and_mux(args, views, out_dir, ffmpeg, ffprobe)

    combined = None
    combined_duration = 0.0
    if len(views) > 1 and not args.no_concat:
        combined, combined_duration = concat(args, views, encoded, out_dir, ffmpeg, ffprobe)
    elif len(views) == 1:
        combined = encoded[views[0]]["final"]
        combined_duration = encoded[views[0]]["duration"]

    qa_info = qa(args, views, encoded, combined, out_dir, ffmpeg)
    report = write_report(args, views, build, recordings, encoded, combined,
                          combined_duration, qa_info, out_dir)

    if not args.keep_frames:
        for view in views:
            shutil.rmtree(out_dir / f"frames_{view}", ignore_errors=True)

    step("完成")
    for view in views:
        log(f"  {view}: {encoded[view]['final']}")
    if combined:
        log(f"  合并版: {combined}")
    log(f"  报告:   {report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
