#!/usr/bin/env python3
"""移动端（Android）初始化与导出：把散落在文档里的手工步骤变成一条命令。

典型用法：

    python3 scripts/android.py doctor          # 只看缺什么，不改任何东西
    python3 scripts/android.py all             # 模板 + 设置 + NDK + 编译 + 导出
    python3 scripts/android.py export          # 只重跑导出

每一步都可单独执行（`templates` / `settings` / `sdk` / `lib` / `export`），
脚本是幂等的：已装好的模板、已写好的设置、已编译的库都会跳过。

`--godot` 指定 Godot 可执行文件；默认按平台探测
`/Applications/Godot.app/Contents/MacOS/Godot`。
"""

from __future__ import annotations

import argparse
import os
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PRESET_NAME = "Android"
APK_PATH = "build/android/Vettura.apk"
# Android ABI → (Rust target, NDK clang 前缀)。注意 32 位 ARM 的 NDK 前缀是
# `armv7a-linux-androideabi`（Rust target 却是 `armv7-linux-androideabi`）
# ——但 oxijolt-sys 拒绝非 64 位目标，所以默认只构建 64 位 ABI。
ALL_ABIS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
}
# strip_mcp_autoload() 出手前保存的 project.godot 原文，导出结束后原样恢复。
ORIGINAL_PROJECT_GODOT = ""
ABIS = {
    abi: triple for abi, triple in ALL_ABIS.items()
    if abi != "armeabi-v7a"
}
ANDROID_SDK_ENV = "ANDROID_HOME"
ANDROID_SDK_DEFAULT = Path.home() / "Library" / "Android" / "sdk"
GODOT_VERSION_RE = re.compile(r"(\d+\.\d+(?:\.\d+)?)\.(\w+)")


class Step:
    """一个带标题的步骤，失败时统一报错退出。"""

    def __init__(self, title: str) -> None:
        self.title = title

    def __enter__(self) -> "Step":
        print(f"\n== {self.title}")
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        if exc_type is not None:
            print(f"!! {self.title} 失败：{exc}", file=sys.stderr)
            raise SystemExit(1)
        print(f"== {self.title} 完成")
        return False


def run(command: list[str], *, env: dict[str, str] | None = None,
        check: bool = True, capture: bool = False) -> subprocess.CompletedProcess:
    shown = " ".join(str(part) for part in command)
    print(f"$ {shown}")
    result = subprocess.run(
        [str(part) for part in command],
        cwd=ROOT,
        env={**os.environ, **(env or {})},
        capture_output=capture,
        text=True,
    )
    if check and result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise RuntimeError(f"命令退出码 {result.returncode}\n{detail[-2000:]}")
    return result


# ---------------------------------------------------------------- 平台与路径

def default_godot() -> Path:
    candidates = [
        Path("/Applications/Godot.app/Contents/MacOS/Godot"),
        Path("/usr/local/bin/godot"),
    ]
    which = shutil.which("godot")
    if which:
        candidates.insert(0, Path(which))
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise SystemExit("找不到 Godot 可执行文件，请用 --godot 指定")


def macos_godot_app(godot: Path) -> Path:
    """`Godot.app/Contents/MacOS/Godot` → `Godot.app`。"""
    return godot.parent.parent if godot.name == "Godot" else godot


_VERSION_CACHE: dict[str, str] = {}


def godot_full_config(godot: Path) -> str:
    """`4.7.2.stable.official.xxx` → `4.7.2.stable`（导出模板目录名）。"""
    cached = _VERSION_CACHE.get(str(godot))
    if cached:
        return cached
    result = run([godot, "--version"], capture=True)
    raw = result.stdout.strip()
    match = GODOT_VERSION_RE.search(raw)
    if not match:
        raise SystemExit(f"无法从 '{raw}' 解析 Godot 版本")
    config = f"{match.group(1)}.{match.group(2)}"
    _VERSION_CACHE[str(godot)] = config
    return config


def godot_release_tag(godot: Path) -> str:
    """godot-builds 的 release tag：`4.7.2-stable`。"""
    config = godot_full_config(godot)
    version, flavour = config.rsplit(".", 1)
    return f"{version}-{flavour}"


def godot_user_dir() -> Path:
    """Godot 的用户目录（编辑器设置 / 导出模板所在）。"""
    system = platform.system()
    if system == "Darwin":
        return Path.home() / "Library" / "Application Support" / "Godot"
    if system == "Windows":
        base = os.environ.get("APPDATA")
        return Path(base) / "Godot" if base else Path.home() / "AppData" / "Roaming" / "Godot"
    return Path.home() / ".local" / "share" / "godot"


def templates_dir(godot: Path) -> Path:
    return godot_user_dir() / "export_templates" / godot_full_config(godot)


def editor_settings_file() -> Path:
    return godot_user_dir() / "editor_settings-4.7.tres"


def android_sdk() -> Path | None:
    override = os.environ.get(ANDROID_SDK_ENV) or os.environ.get("ANDROID_SDK_ROOT")
    if override:
        return Path(override)
    return ANDROID_SDK_DEFAULT if ANDROID_SDK_DEFAULT.exists() else None


def java_home() -> Path | None:
    result = subprocess.run(["/usr/libexec/java_home", "-v", "17"], capture_output=True, text=True)
    if result.returncode == 0 and result.stdout.strip():
        return Path(result.stdout.strip())
    return None


def ndk_dir(sdk: Path | None) -> Path | None:
    if not sdk:
        return None
    candidates = sorted((sdk / "ndk").glob("*"), reverse=True) if (sdk / "ndk").exists() else []
    return candidates[0] if candidates else None


def clang_path(ndk: Path, abi: str) -> Path:
    """NDK clang 包装器的完整路径，例如 `.../bin/aarch64-linux-android24-clang`。"""
    host = "darwin-x86_64" if platform.system() == "Darwin" else "linux-x86_64"
    prefix = ABIS[abi][1]
    return ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin" / f"{prefix}24-clang"


# ------------------------------------------------------------------- 各步骤

def step_doctor(args: argparse.Namespace) -> None:
    godot = Path(args.godot)
    print(f"Godot        : {godot} ({godot_full_config(godot)})")
    print(f"模板目录     : {templates_dir(godot)}")
    for marker in ("android_debug.apk", "android_release.apk", "android_source.zip"):
        path = templates_dir(godot) / marker
        print(f"  {marker:22} {'OK' if path.exists() else '缺失'}")
    print(f"Java SDK     : {java_home() or '缺失（需要 OpenJDK 17）'}")
    print(f"Android SDK  : {android_sdk() or '缺失'}")
    print(f"NDK          : {ndk_dir(android_sdk()) or '缺失（编译扩展 .so 需要）'}")
    for abi in ABIS:
        target = ROOT / "target" / "android" / abi / "libvettura_native.so"
        print(f"  扩展 {abi:12} {'OK' if target.exists() else '未编译'}")
    print(f"导出 preset  : {(ROOT / 'export_presets.cfg').exists() and PRESET_NAME or '缺失'}")


def step_templates(args: argparse.Namespace) -> None:
    godot = Path(args.godot)
    destination = templates_dir(godot)
    marker = destination / "android_source.zip"
    if marker.exists() and not args.force:
        print(f"已安装（{destination}），跳过；`--force` 可强制重装")
        return

    tag = godot_release_tag(godot)
    url = (f"https://github.com/godotengine/godot-builds/releases/download/"
           f"{tag}/Godot_v{tag}_export_templates.tpz")
    download = Path(args.cache) / f"Godot_v{tag}_export_templates.tpz"
    Path(args.cache).mkdir(parents=True, exist_ok=True)

    with Step(f"下载导出模板 {url}"):
        if download.exists() and not args.force:
            print(f"复用已下载文件：{download}")
        else:
            with urllib.request.urlopen(url, timeout=120) as response, \
                    open(download, "wb") as handle:
                total = int(response.headers.get("Content-Length") or 0)
                done = 0
                while chunk := response.read(1 << 20):
                    handle.write(chunk)
                    done += len(chunk)
                    if total:
                        print(f"\r  {done / 1e6:7.1f} / {total / 1e6:.1f} MB", end="")
            print()

    with Step("解压到导出模板目录"):
        destination.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(download) as archive:
            archive.extractall(destination)
        # Godot 官方 tpz 的内容在一个 `templates/` 子目录里（`version.txt` 也
        # 在里面），而引擎直接读 `<模板目录>/<版本>/android_debug.apk`，所以要
        # 把子目录内容上提一层。
        nested = destination / "templates"
        if nested.is_dir():
            for entry in nested.iterdir():
                shutil.move(str(entry), destination / entry.name)
            nested.rmdir()
        expected = ["android_debug.apk", "android_release.apk", "android_source.zip"]
        missing = [name for name in expected if not (destination / name).exists()]
        if missing:
            raise RuntimeError(f"模板包缺少 {missing}（下载可能不完整，用 --force 重下）")
        print(f"解压完成：{destination}")


def step_settings(args: argparse.Namespace) -> None:
    sdk = android_sdk()
    java = java_home()
    if not sdk or not java:
        raise RuntimeError(
            f"缺少 Java SDK（{java}）或 Android SDK（{sdk}）；"
            "请先装 OpenJDK 17 与 Android SDK"
        )
    target = editor_settings_file()
    settings = {
        "export/android/android_sdk_path": str(sdk),
        "export/android/java_sdk_path": str(java),
        # 无头 CI 不装 adb 设备，但保留默认调试端口设置。
        "export/android/debug_adb_port": 5037,
    }
    with Step("写入编辑器设置"):
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('[gd_resource type="EditorSettings" format=3]\n\n[resource]\n')
        lines = target.read_text().splitlines()
        written: dict[str, bool] = dict.fromkeys(settings, False)
        for index, line in enumerate(lines):
            for key, value in settings.items():
                prefix = f"{key} = "
                if line.startswith(prefix):
                    lines[index] = f"{prefix}{json_ish(value)}"
                    written[key] = True
        for key, value in settings.items():
            if not written[key]:
                lines.append(f"{key} = {json_ish(value)}")
        target.write_text("\n".join(lines) + "\n")
        print(f"{target}")


def json_ish(value: str | int) -> str:
    return f'"{value}"' if isinstance(value, str) else str(value)


def step_sdk(args: argparse.Namespace) -> None:
    sdk = android_sdk()
    if not sdk:
        raise RuntimeError("找不到 Android SDK（ANDROID_HOME 或 ~/Library/Android/sdk）")
    manager = sdk / "cmdline-tools" / "latest" / "bin" / "sdkmanager"
    if not manager.exists():
        # Homebrew 的 android-commandlinetools 把 sdkmanager 放在 PATH 里，
        # 而没有完整 SDK 目录结构时回落到它。
        manager = Path(shutil.which("sdkmanager") or manager)
    if not manager.exists():
        raise RuntimeError("找不到 sdkmanager：请安装 Android 命令行工具"
                           "（sdkmanager）")

    wanted = ["platform-tools", "build-tools;36.1.0", "platforms;android-36"]
    if args.ndk:
        wanted.append("ndk;28.1.13356709")

    sdk_root = {"ANDROID_HOME": str(sdk), "ANDROID_SDK_ROOT": str(sdk)}
    with Step("补齐 Android SDK 组件"):
        # sdkmanager 装 NDK / build-tools 前必须先接受所有许可；`yes |` 喂给它，
        # 已经接受过的会直接跳过（幂等）。这里是真正的 shell 管道（之前把 `|`
        # 当字面参数传给 subprocess 是错的）；`yes` 因 SIGPIPE 退出码 141 属
        # 正常现象，只要 sdkmanager 本身成功即可。
        licenses = subprocess.run(
            f"yes | '{manager}' --licenses --sdk_root={sdk}",
            shell=True, cwd=ROOT, env={**os.environ, **sdk_root},
            capture_output=True, text=True,
        )
        if licenses.returncode not in (0, 141):
            detail = (licenses.stderr or licenses.stdout or "").strip()
            raise RuntimeError(f"接受 SDK 许可失败（退出码 {licenses.returncode}）"
                               f"\n{detail[-2000:]}")
        run([manager, f"--sdk_root={sdk}", *wanted], env=sdk_root)


def step_lib(args: argparse.Namespace) -> None:
    sdk = android_sdk()
    ndk = ndk_dir(sdk)
    if not ndk:
        raise RuntimeError("缺少 NDK，先运行：python3 scripts/android.py sdk --ndk")

    targets = sorted({target for target, _ in ABIS.values()})
    with Step("安装 Rust Android 目标"):
        run(["rustup", "target", "add", *targets])

    for abi in ABIS:
        target, _ = ABIS[abi]
        clang = clang_path(ndk, abi)
        if not clang.exists():
            raise RuntimeError(f"NDK clang 不存在：{clang}")
        with Step(f"编译扩展 {abi}（{target}）"):
            build = ROOT / "target" / target / "release"
            env = {
                # oxijolt-sys 的 build.rs 用这三个变量之一定位 NDK。
                "ANDROID_NDK_HOME": str(ndk),
                "ANDROID_NDK_ROOT": str(ndk),
                "ANDROID_NDK": str(ndk),
                # cc crate（Jolt 的 C++ 源码）与 Rust 链接器都走 NDK clang。
                "CC_aarch64-linux-android": str(clang),
                "CXX_aarch64-linux-android": str(clang.with_name(f"{clang.name}++")),
                "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER": str(clang),
                # FMOD 默认不参与 Android 构建（运行期库见 scripts/pack_fmod.py --android）。
            }
            if abi == "armeabi-v7a":
                arm_clang = clang
                env.update({
                    "CC_armv7-linux-androideabi": str(arm_clang),
                    "CXX_armv7-linux-androideabi": str(
                        arm_clang.with_name("armv7a-linux-androideabi24-clang++")),
                    "CARGO_TARGET_ARMV7_LINUX_ANDROIDABI_LINKER": str(arm_clang),
                })
            elif abi == "x86_64":
                x86_clang = clang
                env.update({
                    "CC_x86_64-linux-android": str(x86_clang),
                    "CXX_x86_64-linux-android": str(
                        x86_clang.with_name("x86_64-linux-android24-clang++")),
                    "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER": str(x86_clang),
                })
            run(["cargo", "build", "-p", "vettura-godot", "--release", "--target", target], env=env)
            source = build / "libvettura_native.so"
            if not source.exists():
                raise RuntimeError(f"编译产物缺失：{source}")
            destination = ROOT / "target" / "android" / abi
            destination.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination / source.name)
            print(f"  已拷贝 -> {destination / source.name}")


# 只删 `MCPRuntimeProbe` 这一行，不依赖它在 `[autoload]` 里的位置或前后空行。
# 大厅的 `GameSettings` 也是 autoload，原先“整段匹配”的正则会因它插在中间而
# 静默失效，导致 APK 启动报 Unrecognized UID。
MCP_AUTOLOAD_RE = re.compile(r"^MCPRuntimeProbe=[^\n]*\n", re.MULTILINE)


def strip_mcp_autoload() -> tuple[bool, str]:
    """导出前临时摘掉 godot_mcp 插件在编辑器里注册的 autoload 与插件开关。

    该插件在编辑器启动时会：
      1. 用 `add_autoload_singleton` 把运行期探针写进 `[autoload]`；
      2. 自己在 `[editor_plugins]` 里启用（否则插件根本不会跑）。
    而 Android preset 的 exclude_filter 排除了 `addons/*`，探针的 UID 在包里
    不存在，真机启动就刷一条：
        ERROR: Unrecognized UID: "uid://bsg12huaf1u5i"
    关键是：只摘 autoload 没用——导出用的又是 `--editor`，插件会当场把它写回来。
    所以导出期间把 `[editor_plugins]` 里的 godot_mcp 条目一并摘掉，导完再原样恢复。
    返回 (是否改动, 原文) 供恢复用。
    """
    path = ROOT / "project.godot"
    original = path.read_text(encoding="utf-8")
    stripped = MCP_AUTOLOAD_RE.sub("", original, count=1)
    stripped, plugin_removed = _drop_editor_plugin(stripped, "res://addons/godot_mcp/plugin.cfg")
    if stripped == original and not plugin_removed:
        return False, original
    path.write_text(stripped, encoding="utf-8")
    global ORIGINAL_PROJECT_GODOT
    ORIGINAL_PROJECT_GODOT = original
    return True, original


def _drop_editor_plugin(text: str, plugin_cfg: str) -> tuple[str, bool]:
    """从 `[editor_plugins] enabled` 数组里删掉一个插件；删空则连 section 一起删。"""
    marker = "enabled=PackedStringArray("
    cursor = text.find(marker)
    while cursor != -1:
        end = text.find(")", cursor)
        if end == -1:
            break
        body = text[cursor + len(marker):end]
        items = [chunk.strip() for chunk in re.split(r",\s*(?=\")", body) if chunk.strip()]
        kept = [item for item in items if plugin_cfg not in item]
        if len(kept) != len(items):
            if kept:
                text = text[: cursor + len(marker)] + ", ".join(kept) + ")" + text[end + 1:]
                return text, True
            section = text.rfind("[editor_plugins]", 0, cursor)
            line_end = text.find("\n", end)
            next_line = text.find("\n", line_end + 1)
            if section != -1 and next_line != -1:
                return text[:section] + text[next_line + 1:], True
            return text[: section] + text[line_end + 1:], True
        cursor = text.find(marker, end)
    return text, False


def restore_project_godot() -> bool:
    """把 strip_mcp_autoload 改动的 project.godot 原样写回。"""
    original = ORIGINAL_PROJECT_GODOT
    if not original:
        return False
    path = ROOT / "project.godot"
    if path.read_text(encoding="utf-8") == original:
        return True
    path.write_text(original, encoding="utf-8")
    return True


def step_export(args: argparse.Namespace) -> None:
    godot = Path(args.godot)
    if not (ROOT / "export_presets.cfg").exists():
        raise RuntimeError("缺少 export_presets.cfg（Android preset）；请先创建")
    command = [
        godot, "--headless", "--path", str(ROOT), "--editor",
        "--install-android-build-template",
        "--export-debug", PRESET_NAME, str(ROOT / APK_PATH),
    ]
    changed, _ = strip_mcp_autoload()
    if changed:
        print("  已临时移除 MCPRuntimeProbe autoload 与 godot_mcp 编辑器插件"
              "（addons 不随包导出，插件启动还会把 autoload 写回来）")
    try:
        with Step("导出 APK"):
            (ROOT / APK_PATH).parent.mkdir(parents=True, exist_ok=True)
            run(command)
        apk = ROOT / APK_PATH
        if not apk.exists():
            raise RuntimeError("导出命令成功但 APK 不存在")
        print(f"APK: {apk} ({apk.stat().st_size / 1e6:.1f} MB)")
    finally:
        if changed:
            restore = restore_project_godot()
            print(f"  已恢复 project.godot MCPRuntimeProbe autoload：{restore}")


STEPS = {
    "doctor": (step_doctor, "只看缺什么，不改任何东西"),
    "templates": (step_templates, "下载并安装 Godot 导出模板（含 android_source.zip）"),
    "settings": (step_settings, "写入 Java/Android SDK 编辑器设置"),
    "sdk": (step_sdk, "补齐 Android SDK 组件（--ndk 装 NDK）"),
    "lib": (step_lib, "为 Android ABI 编译 GDExtension 的 .so"),
    "export": (step_export, "导出调试 APK"),
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--godot", default=str(default_godot()), help="Godot 可执行文件")
    parser.add_argument("--cache", default=str(ROOT / ".cache" / "godot"),
                        help="模板下载缓存目录")
    parser.add_argument("--force", action="store_true", help="忽略缓存，强制重装")
    parser.add_argument("--ndk", action="store_true", help="连同 NDK 一起安装")
    parser.add_argument("step", choices=[*STEPS.keys(), "all"], help="要执行的步骤")
    args = parser.parse_args()

    if args.step == "doctor":
        step_doctor(args)
        return
    order = ["templates", "settings", "lib", "export"] if args.step == "all" else [args.step]
    if args.step == "all":
        step_sdk(args)   # SDK 组件先就位
    for name in order:
        STEPS[name][0](args)


if __name__ == "__main__":
    main()
