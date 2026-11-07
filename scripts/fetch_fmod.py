#!/usr/bin/env python3
"""从 fmod.com 拉取 FMOD Engine SDK 2.02.22，并解压到工作区本地缓存。

只用 Python 标准库；不需要 requests/pip 依赖。由 uv 统一管理：

    cd scripts
    uv run fetch_fmod.py                 # 拉取当前平台
    uv run fetch_fmod.py --all           # 拉取 macOS / Windows / Linux
    uv run fetch_fmod.py --extract-only  # 已下载过，只重新解压

凭证来源（二选一）：
  1) 仓库根目录的本地文件 `.fmod.env`（已 gitignore，禁止提交）
  2) 环境变量 FMOD_USER / FMOD_PASS

产物目录（gitignore，禁止提交）：
  <repo>/.cache/fmod/archives/       # 安装包
  <repo>/.cache/fmod/unpacked/<os>/  # 解压后的 SDK
  <repo>/.cache/fmod_sdk.json        # build.rs 可直接读取的定位文件
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

API_LOGIN = "https://www.fmod.com/api-login"
API_DOWNLOAD = "https://www.fmod.com/api-get-download-link"

# FMOD Engine SDK 2.02.22：本项目的指定版本。
# libfmod 锁 ~2.222 对应该版本；bank 兼容规则见 docs/audio.md。
DEFAULT_VERSION = "20222"

USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
)

# fmod.com 下载 API 使用的固定路径布局。
PLATFORMS = {
    "windows": {
        "path": "files/fmodstudio/api/Windows/",
        "file": "fmodstudioapi{version}win-installer.exe",
        "ext": "exe",
    },
    "macos": {
        "path": "files/fmodstudio/api/Mac/",
        "file": "fmodstudioapi{version}mac-installer.dmg",
        "ext": "dmg",
    },
    "linux": {
        "path": "files/fmodstudio/api/Linux/",
        "file": "fmodstudioapi{version}linux.tar.gz",
        "ext": "tar.gz",
    },
}


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def load_credentials() -> tuple[str, str]:
    user = os.environ.get("FMOD_USER")
    password = os.environ.get("FMOD_PASS")
    if user and password:
        return user, password

    env_file = repo_root() / ".fmod.env"
    if env_file.is_file():
        values = {}
        for line in env_file.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip('"').strip("'")
        user = values.get("FMOD_USER", user)
        password = values.get("FMOD_PASS", password)

    if not user or not password:
        sys.exit(
            "缺少 FMOD 凭证：请在仓库根目录写 `.fmod.env`，"
            "或设置环境变量 FMOD_USER / FMOD_PASS。"
        )
    return user, password


def request_json(url: str, token: str | None = None, method: str = "GET") -> dict:
    headers = {"User-Agent": USER_AGENT, "Accept": "application/json, text/plain, */*"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=b"" if method == "POST" else None, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=60) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return payload


def login(user: str, password: str) -> str:
    # fmod.com 的 api-login 使用 HTTP Basic Auth；浏览器 UA 是必须的。
    import base64

    basic = base64.b64encode(f"{user}:{password}".encode("utf-8")).decode("ascii")
    headers = {
        "User-Agent": USER_AGENT,
        "Accept": "application/json, text/plain, */*",
        "Authorization": f"Basic {basic}",
    }
    request = urllib.request.Request(API_LOGIN, data=b"", headers=headers, method="POST")
    with urllib.request.urlopen(request, timeout=60) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if "token" not in payload:
        sys.exit(f"FMOD 登录失败：{payload}")
    return payload["token"]


def download_url(token: str, user: str, api_path: str, filename: str) -> str:
    params = {"path": api_path, "filename": filename, "user": user}
    payload = request_json(f"{API_DOWNLOAD}?{urllib.parse.urlencode(params)}", token=token)
    url = payload.get("url") if isinstance(payload, dict) else payload
    if not url:
        sys.exit(f"FMOD 没有返回下载链接：{payload}")
    return url


def download(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        print(f"[cache] {destination}")
        return

    temporary = destination.with_suffix(destination.suffix + ".part")
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=300) as response:
        total = int(response.headers.get("Content-Length") or 0)
        downloaded = 0
        with temporary.open("wb") as handle:
            while True:
                chunk = response.read(1 << 16)
                if not chunk:
                    break
                handle.write(chunk)
                downloaded += len(chunk)
                if total:
                    percent = downloaded * 100 // total
                    print(f"\r[download] {percent:3d}% {downloaded / 1e6:.1f}/{total / 1e6:.1f} MB", end="")
    print()
    temporary.replace(destination)


def extract(archive: Path, plat: str, target: Path) -> Path:
    """解压并返回该平台 SDK 的 api/ 目录。"""
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True, exist_ok=True)

    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive) as handle:
            handle.extractall(target)
    elif archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as handle:
            handle.extractall(target)
    elif archive.suffix == ".dmg":
        mount = "/Volumes/FMOD_ENGINE_INSTALL"
        subprocess.run(
            ["hdiutil", "attach", str(archive), "-mountpoint", mount, "-nobrowse", "-quiet"],
            check=True,
        )
        try:
            for item in Path(mount).iterdir():
                destination = target / item.name
                if item.is_dir():
                    shutil.copytree(item, destination, dirs_exist_ok=True)
                else:
                    shutil.copy2(item, destination)
        finally:
            subprocess.run(["hdiutil", "detach", mount, "-quiet"], check=False)
    elif archive.suffix == ".exe":
        subprocess.run(["7z", "x", str(archive), f"-o{target}", "-y"], check=True)
    else:
        sys.exit(f"未知安装包格式：{archive.name}")

    for candidate in target.rglob("api"):
        if candidate.is_dir() and (candidate / "core").exists():
            return candidate
    sys.exit(f"解压完成，但 {target} 下找不到 api/core")


def build_plugin(api_dir: Path, plat: str, runtime_dir: Path) -> None:
    """编译 AC/mod bank 可能依赖的 FMOD Distance Filter DSP 插件。"""
    source = api_dir / "core" / "examples" / "plugins" / "fmod_distance_filter.cpp"
    include = api_dir / "core" / "inc"
    if not source.is_file():
        sys.exit(f"SDK 里缺少 fmod_distance_filter.cpp：{source}")

    if plat == "macos":
        target = runtime_dir / "libfmod_distance_filter.dylib"
        command = [
            "clang++",
            "-std=c++17",
            "-O2",
            "-dynamiclib",
            "-undefined",
            "dynamic_lookup",
            f"-I{include}",
            str(source),
            "-o",
            str(target),
        ]
    elif plat == "linux":
        target = runtime_dir / "libfmod_distance_filter.so"
        command = [
            "g++",
            "-std=c++17",
            "-O2",
            "-fPIC",
            "-shared",
            f"-I{include}",
            str(source),
            "-o",
            str(target),
        ]
    else:
        # Windows 需要 MSVC/clang-cl；本地脚本默认先不编译，交给 CI。
        print(f"[warn] 跳过 {plat} DSP 插件编译（请在 CI 中按目标平台编译）")
        return

    print(f"[build] {' '.join(command)}")
    subprocess.run(command, check=True)
    print(f"[ok] DSP plugin: {target}")


def collect_runtime(api_dir: Path, plat: str, cache: Path) -> str:
    """把动态库复制到无空格路径，供 Rust build.rs 链接和 rpath 使用。"""
    if plat == "windows":
        patterns = ["core/lib/*fmod*", "studio/lib/*fmod*"]
    else:
        patterns = ["core/lib/*fmod*.dylib", "studio/lib/*fmod*.dylib", "core/lib/*fmod*.so", "studio/lib/*fmod*.so"]

    runtime_dir = cache / "runtime" / plat
    runtime_dir.mkdir(parents=True, exist_ok=True)
    for pattern in patterns:
        for library in api_dir.glob(pattern):
            if library.is_file():
                shutil.copy2(library, runtime_dir / library.name)

    for expected in ("fmod", "fmodstudio"):
        if not any(runtime_dir.glob(f"*{expected}*")):
            sys.exit(f"runtime 目录缺少 {expected}: {runtime_dir}")

    build_plugin(api_dir, plat, runtime_dir)
    return str(runtime_dir.resolve())


def write_sdk_json(results: dict[str, str], version: str, runtimes: dict[str, str] | None = None) -> None:
    payload = {
        "version": version,
        "product_major": f"{version[:1]}.{version[1:3]}.x",
        "runtime_link": "dynamic",
        "platforms": results,
        "runtimes": runtimes or {},
    }
    json_path = Path(os.environ.get("FMOD_CACHE_DIR", repo_root() / ".cache" / "fmod")) / "fmod_sdk.json"
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def host_platform() -> str:
    name = platform.system()
    mapping = {"Windows": "windows", "Darwin": "macos", "Linux": "linux"}
    if name not in mapping:
        sys.exit(f"不支持的宿主平台：{name}")
    return mapping[name]


def main() -> int:
    parser = argparse.ArgumentParser(description="Fetch the pinned FMOD Engine SDK")
    parser.add_argument("--all", action="store_true", help="拉取全部三个平台")
    parser.add_argument("--extract-only", action="store_true", help="跳过下载，只重新解压")
    args = parser.parse_args()

    user, password = load_credentials()
    version = os.environ.get("FMOD_VERSION", DEFAULT_VERSION)
    cache = Path(os.environ.get("FMOD_CACHE_DIR", repo_root() / ".cache" / "fmod"))
    archives = cache / "archives"
    unpacked = cache / "unpacked"

    if version != DEFAULT_VERSION:
        print(f"警告：当前锁定版本是 {DEFAULT_VERSION}，正在拉取覆盖版本 {version}。", file=sys.stderr)

    plats = list(PLATFORMS) if args.all else [host_platform()]
    token = None if args.extract_only else login(user, password)

    results: dict[str, str] = {}
    runtimes: dict[str, str] = {}
    for plat in plats:
        info = PLATFORMS[plat]
        filename = info["file"].format(version=version)
        archive = archives / filename
        if not args.extract_only:
            if archive.exists():
                print(f"[cache] {filename}")
            else:
                url = download_url(token, user, info["path"], filename)
                download(url, archive)
        api_dir = extract(archive, plat, unpacked / plat)
        results[plat] = str(api_dir.resolve())
        runtimes[plat] = collect_runtime(api_dir, plat, cache)
        print(f"[ok] {plat}: {api_dir}")
        print(f"[ok] {plat} runtime: {runtimes[plat]}")

    write_sdk_json(results, version, runtimes)
    print(f"[ok] SDK 定位文件：{cache / 'fmod_sdk.json'}")
    print("[next] 构建 vettura-audio：cargo build -p vettura-audio --features fmod")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
