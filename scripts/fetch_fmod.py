#!/usr/bin/env python3
"""从 fmod.com 拉取 FMOD Engine SDK 2.02.22，并解压到工作区本地缓存。

只用 Python 标准库；不需要 requests/pip/uv：

    cd scripts
    python3 fetch_fmod.py                 # 拉取当前平台
    python3 fetch_fmod.py --all           # 拉取 macOS / Windows / Linux
    python3 fetch_fmod.py --extract-only  # 已下载过，只重新解压

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

# 每个平台运行期必须随游戏分发的动态库（发行版默认 release 命名）。
RELEASE_LIBRARIES = {
    "windows": ["fmod.dll", "fmodstudio.dll"],
    "macos": ["libfmod.dylib", "libfmodstudio.dylib"],
    "linux": ["libfmod.so", "libfmodstudio.so"],
}
DEBUG_LIBRARIES = {
    "windows": ["fmodL.dll", "fmodstudioL.dll"],
    "macos": ["libfmodL.dylib", "libfmodstudioL.dylib"],
    "linux": ["libfmodL.so", "libfmodstudioL.so"],
}
# AC/mod 老 bank 依赖的 DSP 插件；Windows 二进制需要 MSVC，本机脚本不编。
PLUGIN_NAME = {
    "windows": "fmod_distance_filter.dll",
    "macos": "libfmod_distance_filter.dylib",
    "linux": "libfmod_distance_filter.so",
}


def expected_libraries(plat: str, profile: str = "release") -> list[str]:
    """发行（或调试）运行期必须出现的动态库文件名。"""
    names = list(RELEASE_LIBRARIES[plat])
    if profile in ("debug", "both"):
        names += DEBUG_LIBRARIES[plat]
    return names


def runtime_dir_for(cache: Path, plat: str) -> Path:
    return cache / "runtime" / plat


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


def is_archive(path: Path) -> bool:
    name = path.name.lower()
    return name.endswith((".tar.gz", ".tgz", ".zip", ".tar"))


def unpack(archive: Path, target: Path) -> None:
    """把单个安装包展开到 target。"""
    if archive.name.lower().endswith((".tar.gz", ".tgz", ".tar")):
        with tarfile.open(archive) as handle:
            handle.extractall(target)
    elif archive.suffix.lower() == ".zip":
        with zipfile.ZipFile(archive) as handle:
            handle.extractall(target)
    elif archive.suffix.lower() == ".dmg":
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
    elif archive.suffix.lower() == ".exe":
        # FMOD Windows 安装包是 NSIS 自解压 exe，需要 7z（brew install p7zip）。
        try:
            subprocess.run(
                ["7z", "x", str(archive), f"-o{target}", "-y"],
                check=True,
                capture_output=True,
            )
        except FileNotFoundError:
            sys.exit("解开 Windows 安装包需要 7z：brew install p7zip")
        except subprocess.CalledProcessError as error:
            detail = (error.stderr or b"").decode(errors="replace")[-400:]
            sys.exit(f"7z 解压失败：{detail}")
    else:
        sys.exit(f"未知安装包格式：{archive.name}")


def find_api(target: Path) -> Path | None:
    for candidate in target.rglob("api"):
        if candidate.is_dir() and (candidate / "core").exists():
            return candidate
    return None


def extract(archive: Path, plat: str, target: Path) -> Path:
    """解压并返回该平台 SDK 的 api/ 目录。"""
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True, exist_ok=True)

    unpack(archive, target)

    # Linux 安装包是“tar.gz 里再包一个同名 tar.gz”，这里把嵌套包也解开。
    for _ in range(3):
        if find_api(target) is not None:
            break
        nested = sorted(path for path in target.iterdir() if path.is_file() and is_archive(path))
        if not nested:
            break
        for inner in nested:
            print(f"[unpack] 嵌套安装包：{inner.name}")
            unpack(inner, target)
            inner.unlink()

    api = find_api(target)
    if api is None:
        sys.exit(f"解压完成，但 {target} 下找不到 api/core")
    return api


def fetch_platform(
    plat: str,
    cache: Path,
    token: str | None = None,
    user: str | None = None,
    version: str = DEFAULT_VERSION,
    allow_download: bool = True,
    arch: str | None = None,
) -> tuple[Path, Path]:
    """下载（如需要）+ 解压 + 收集该平台 runtime，返回 (api_dir, runtime_dir)。

    `pack_fmod.py` 与 `fetch_fmod.py` 共用；`allow_download=False` 用于离线模式。
    """
    info = PLATFORMS[plat]
    filename = info["file"].format(version=version)
    archive = cache / "archives" / filename
    if archive.exists():
        print(f"[cache] {filename}")
    elif allow_download and token and user:
        download(download_url(token, user, info["path"], filename), archive)
    else:
        sys.exit(f"缓存缺少安装包 {filename}；当前模式不允许下载（去掉 --offline 可重下）")

    api_dir = extract(archive, plat, cache / "unpacked" / plat)
    return api_dir, Path(collect_runtime(api_dir, plat, cache, arch))


def runtime_ready(plat: str, cache: Path) -> bool:
    """`.cache/fmod/runtime/<plat>` 是否已经齐了发行 + 调试运行期动态库。"""
    runtime = runtime_dir_for(cache, plat)
    if not runtime.is_dir():
        return False
    return all((runtime / name).is_file() for name in expected_libraries(plat, "both"))


def unpacked_api(plat: str, cache: Path) -> Path | None:
    """已解压 SDK 的 api/ 目录（用来补编插件）。"""
    return find_api(cache / "unpacked" / plat)


def build_plugin(api_dir: Path, plat: str, runtime_dir: Path) -> str:
    """编译 AC/mod bank 可能依赖的 FMOD Distance Filter DSP 插件，返回产物路径。

    只能在**目标平台本机**编译：在 macOS 上给 Linux/Windows 编出来的二进制没法用，
    这种情况只提示，由 CI 或对应平台的机器补。
    """
    source = api_dir / "core" / "examples" / "plugins" / "fmod_distance_filter.cpp"
    include = api_dir / "core" / "inc"
    target = runtime_dir / PLUGIN_NAME[plat]
    if not source.is_file():
        sys.exit(f"SDK 里缺少 fmod_distance_filter.cpp：{source}")

    if host_platform() != plat:
        print(f"[warn] 跨平台打包：跳过 {plat} DSP 插件编译（需在 {plat} 上执行）")
        return str(target)

    if plat == "macos":
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
        # Windows 需要 MSVC / clang-cl；本机脚本不编，交给 Windows CI。
        print(f"[warn] 跳过 {plat} DSP 插件编译（请在 Windows CI 用 MSVC 编出 {PLUGIN_NAME[plat]}）")
        return str(target)

    print(f"[build] {' '.join(command)}")
    subprocess.run(command, check=True)
    print(f"[ok] DSP plugin: {target}")
    return str(target)


def host_machine() -> str:
    """宿主机 CPU 架构，归一化成 FMOD SDK 的 lib/<arch>/ 命名。"""
    machine = platform.machine().lower()
    mapping = {
        "amd64": "x86_64",
        "x86_64": "x86_64",
        "x64": "x86_64",
        "arm64": "arm64",
        "aarch64": "arm64",
        "arm": "arm",
        "armv7l": "arm",
        "i386": "x86",
        "i686": "x86",
        "x86": "x86",
    }
    return mapping.get(machine, machine)


def normalize_arch(name: str | None, plat: str) -> str | None:
    """把用户给的架构名归一化成该平台 SDK 的 lib/<arch>/ 目录名。"""
    if not name:
        return None
    key = name.strip().lower()
    if plat == "windows":
        return {"amd64": "x64", "x86_64": "x64", "x64": "x64", "aarch64": "arm64", "i386": "x86", "i686": "x86"}.get(key, key)
    return {"x64": "x86_64", "amd64": "x86_64"}.get(key, key)


def arch_candidates(plat: str) -> list[str]:
    """该平台的 lib/<arch>/ 目录名：宿主架构优先，再按常见目标兜底。"""
    host = host_machine()
    if plat == "windows":
        # Linux 上的 x86_64 在 Windows SDK 里叫 x64。
        normalized = {"x86_64": "x64"}.get(host, host)
        order = [normalized, "x64", "arm64", "x86"]
    else:
        order = [host, "x86_64", "arm64", "arm", "x86"]
    seen: list[str] = []
    for name in order:
        if name and name not in seen:
            seen.append(name)
    return seen


def lib_root(base: Path, candidates: list[str]) -> Path | None:
    """动态库实际所在目录：直接放在 lib/ 或 lib/<arch>/，按架构挑选。"""
    if not base.is_dir():
        return None
    if any(path.is_file() and path.name.lower().startswith(("fmod", "libfmod")) for path in base.iterdir()):
        return base

    arch_dirs = sorted(path for path in base.iterdir() if path.is_dir())
    if not arch_dirs:
        return None
    for name in candidates:
        for directory in arch_dirs:
            if directory.name == name:
                return directory
    return arch_dirs[0]


def collect_runtime(api_dir: Path, plat: str, cache: Path, arch: str | None = None) -> str:
    """把动态库复制到无空格路径，供 Rust build.rs 链接、rpath 与打包使用。

    `arch` 用于跨平台打包：Windows/Linux SDK 里每个架构一套库，必须选对，
    否则会把 arm64 的 dll 当成 x64 发出去。
    """
    runtime_dir = runtime_dir_for(cache, plat)
    runtime_dir.mkdir(parents=True, exist_ok=True)

    candidates = [name for name in [normalize_arch(arch, plat)] if name]
    candidates += [name for name in arch_candidates(plat) if name not in candidates]
    roots = [lib_root(api_dir / "core" / "lib", candidates), lib_root(api_dir / "studio" / "lib", candidates)]
    copied: set[str] = set()
    for root in [root for root in roots if root is not None]:
        print(f"[lib] {root.relative_to(api_dir)}")
        for library in sorted(root.iterdir()):
            if library.is_file():
                shutil.copy2(library, runtime_dir / library.name)
                copied.add(library.name)

    missing = [name for name in expected_libraries(plat, "both") if name not in copied]
    if missing:
        available = ", ".join(sorted(copied)) or "(空)"
        sys.exit(f"runtime 目录缺少 {missing}；SDK 里实际只有：{available}")
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
    parser.add_argument("--arch", help="指定 lib/<arch>/（跨平台打包用，默认按宿主机）")
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
        if args.extract_only:
            info = PLATFORMS[plat]
            archive = archives / info["file"].format(version=version)
            if not archive.exists():
                sys.exit(f"--extract-only 需要缓存里的安装包：{archive}")
        api_dir, runtime = fetch_platform(
            plat,
            cache,
            token,
            user,
            version,
            allow_download=not args.extract_only,
            arch=args.arch,
        )
        results[plat] = str(api_dir.resolve())
        runtimes[plat] = str(runtime.resolve())
        print(f"[ok] {plat}: {api_dir}")
        print(f"[ok] {plat} runtime: {runtime}")

    write_sdk_json(results, version, runtimes)
    print(f"[ok] SDK 定位文件：{cache / 'fmod_sdk.json'}")
    print("[next] 构建 vettura-audio：cargo build -p vettura-audio --features fmod")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
