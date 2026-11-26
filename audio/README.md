# vettura-audio

AC 车模音频发现 + **FMOD Studio runtime 直连**。Rust 侧不解码音频：bank 里的
FMOD 封装由 FMOD Engine 解码。老 AC bank 缺 master bus、Studio 事件静音时，
会扫描 bank 内嵌的 FSB5 容器，把真实采样交给 Core API 播放（见 `docs/audio.md`）。

## 模块地图

| 文件 | 职责 |
| --- | --- |
| `guids.rs` | 解析 `sfx/GUIDs.txt`：事件名 ↔ GUID 表（`SoundEvent`） |
| `manifest.rs` | `SoundManifest`：定位 `sfx/<car_id>.bank`，事件表落位 |
| `runtime.rs` | `AudioEngine` 门面：`Studio::create` → `load_bank_memory` → `get_event` → `create_instance/start` → 每帧 `update`；3D 听者与车辆参数（rpm/load…）注入，候选参数名自适应不同 mod |
| `runtime/roles.rs` | **统一采样解析器**：按通用 AC 命名 + 时长 + `ext/int` 视角给 FSB 采样分配角色；不含车型分支，带单元测试 |
| `runtime/engine.rs` | `FmodEngine`：真正调用 libfmod 的实现（仅 `fmod_sdk_found` 时编译），含参数名探测、3D 属性转换与 FSB5 fallback 混音 |
| `examples/fallback_probe.rs` | 诊断例程：加载真车 bank，按角色驱动 fallback 并写 WAV |
| `runtime/plugins.rs` | bank 插件（FMOD Distance Filter）的搜索顺序与注册（仅 `fmod_sdk_found` 时编译） |
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

- bank/FSB 的音频解码与最终混音策略（FMOD 负责）；Rust 只切出 FSB5 容器、
  按采样名挑角色并设音量/音调；
- Godot 节点封装（`gdext/fmod_system.rs` 的 `FmodSystem`）；
- 车目录结构（`assets/`）。

## 验证

```bash
cargo test -p vettura-audio
cargo run -q -p vettura-audio --features fmod --example fallback_probe -- \
  "mods/trr_porsche_992_gt3r" /tmp/fallback.wav        # 需 SDK + mods/ 真车
godot --headless --path . res://gdext/fmod_probe.tscn   # 需 --features fmod + mods/ 真车
```

## 相关文档

- [`docs/audio.md`](../docs/audio.md)：版本锁定、GUID 事件表、参数映射
- [`scripts/README.md`](../scripts/README.md)：SDK 拉取与打包
