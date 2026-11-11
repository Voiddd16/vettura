# vettura-audio

AC 车模音频发现 + **FMOD Studio runtime 直连**。这个 crate 不解析 `.bank`：
bank 里的 FMOD 封装由 FMOD Engine 自己解码，Rust 侧只做"发现 + 递给 FMOD"。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `guids.rs` | 解析 `sfx/GUIDs.txt`：事件名 ↔ GUID 表（`SoundEvent`） |
| `manifest.rs` | `SoundManifest`：定位 `sfx/<car_id>.bank`，事件表落位 |
| `runtime.rs` | `AudioEngine`：`Studio::create` → `load_bank_memory` → `get_event` → `create_instance/start` → 每帧 `update`；3D 听者与车辆参数（rpm/load…）注入，候选参数名自适应不同 mod |
| `error.rs` | `AudioError`（含 `NotLinked`） |

## 版本与兼容规则（细节见 `docs/audio.md`）

- 锁定 FMOD Engine SDK **2.02.22**，`libfmod ~2.222`；
- 2.02.22 runtime 只保证加载 **2.02.x** 编出的 bank（2.03/2.04 不保证）；
- **只动态链接，SDK 二进制不入库**：本地由 `scripts/fetch_fmod.py` 拉到 `.cache/fmod/`，
  凭证在 `.fmod.env`（gitignore）。

## 无 SDK 也能编译

没找到本地 SDK 时，build.rs 输出 `fmod_sdk_missing`：`AudioEngine` API 完整保留，
`start()` 返回 `AudioError::NotLinked`。因此 workspace 在任何机器都能
编译/测试/跑视觉与物理链路，接真声音才需要 feature：

```bash
cd scripts && uv run fetch_fmod.py                    # 拉 SDK（一次性）
cargo build -p vettura-godot --features fmod          # 真机音频
```

APK 发行包默认**不含** FMOD 动态库（`scripts/pack_fmod.py` 负责打进导出目录），
这是预期行为，不是 bug。

## 边界（不放入）

- bank / FSB 解析、音频解码、混音策略（FMOD 负责）；
- Godot 节点封装（`gdext/fmod_system.rs` 的 `FmodSystem`）；
- 车目录结构（`assets/`）。

## 验证

```bash
cargo test -p vettura-audio
godot --headless --path . res://gdext/fmod_probe.tscn   # 需 --features fmod + mods/ 真车
```

## 相关文档

- [`docs/audio.md`](../docs/audio.md)：版本锁定、GUID 事件表、参数映射
- [`scripts/README.md`](../scripts/README.md)：SDK 拉取与打包
