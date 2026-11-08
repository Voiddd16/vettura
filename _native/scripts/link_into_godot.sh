#!/usr/bin/env bash
## 把 `vettura-native` 链接进 Godot 工程，让 `.gdextension` 里的
## `res://vettura-native/target/debug/libvettura_native.dylib` 能解析。
##
## 用法：
##
##   ./scripts/link_into_godot.sh [godot_project_dir]
##
## 默认认为 Godot 工程在本仓库的上一级 `vettura/`。链接不存在时创建，已存在时
## 直接跳过；想强制重建先删掉旧链接。
set -euo pipefail

native_root="$(cd "$(dirname "$0")/.." && pwd)"
godot_root="${1:-${native_root%/vettura-native}/vettura}"

if [ ! -f "${godot_root}/project.godot" ]; then
    echo "error: no project.godot under ${godot_root}" >&2
    exit 1
fi

link="${godot_root}/vettura-native"
if [ -L "${link}" ] || [ -e "${link}" ]; then
    echo "already linked: ${link}"
    exit 0
fi

ln -s "${native_root}" "${link}"
echo "linked ${link} -> ${native_root}"
echo "next: cargo build -p vettura-godot"
echo "then: godot --headless --path ${godot_root} res://vettura-native/godot/baseline.tscn"
