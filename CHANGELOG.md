# 变更记录

本文件记录 Vettura 的版本级变更。版本号遵循 SemVer：`0.x` 阶段允许在次版本间
调整内部结构，公共 API（GDExtension 暴露给 GDScript 的类与方法）以“能用”为准。

## 0.0.1 —— 2026-11-10

第一个可运行版本：Rust workspace + Godot 4.7 GDExtension 打通
“读 AC 数据 → 物理 → 显示”，`gdext/baseline.tscn` 能在真实赛道 + 真实车上跑通。

### 新增

- **按文件共享的加载缓存**（`gdext/src/kn5/cache.rs`）：同一路径的 `.kn5` 只解析
  一次，每张内嵌 DDS 只解码一次，每个 KN5 材质只创建一份 `StandardMaterial3D`；
  同一 `Kn5File` / `AcCar` 的重复实例化因此快约 4 倍。
- **分帧加载器 `Kn5SceneBuilder`**：`start()` → 多次 `step(预算, parent)` →
  `root()`，带 `progress()` 与 `remaining_tasks()`，把一次长加载切成多帧小块
  （`scripts/load_probe.py` 可量化对比）。
- **合成模型生成器** `kn5/src/synth.rs`（合法 DXT1 DDS + 截断 mip 链），bench、
  测试与 `kn5-inspect --synth` 共用一个实现，加载探针不再依赖本地十几 GB 的真车
  资产。
- **加载探针场景** `gdext/load_probe.tscn`：分别测量解析、冷实例化、缓存命中、
  分帧加载四段耗时。
- `NodeKind::Mesh/SkinnedMesh` 改为 `Arc<Mesh>` / `Arc<SkinnedMesh>`：几何可在
  解析结果、任务列表、多个实例之间零拷贝共享。

### 修复

- 预览页在本地缺少 `test/` 真车资产、或 GDExtension 尚未注册时不再刷一片
  ERROR，而是降级为警告并跳过模型加载（`--install-android-build-template` 等
  工程级命令不再被噪音淹没）。
- `test_visual/smoke.tscn` 无车可测时只验证相机循环，输出 `PASS（部分）`
  而不是 `FAIL`。
- 合成 DDS 头改用 `DDSD_LINEARSIZE`：Godot 会拒绝“未声明线性长度却给了非零
  pitch”的压缩贴图，此前合成贴图全部加载失败（探针的 `albedo=0` 由此而来）。

### 已知边界

- 冷路径首帧仍要解码全部贴图；把 DDS 解码搬进 Rust（BC1/BC3）以并行化/后台化，
  是下一个版本的首要候选。

## 真机验证记录（未定版，2026-11-11）

设备：Xiaomi 2211133C（HyperOS，Android 16 / API 36，arm64-v8a，1080x2400，
440dpi，Adreno 740，Vulkan Forward Mobile）。APK = `build/android/Vettura.apk`，
通过 `python3 scripts/build.py apk` 导出后用 `adb install -r -d` 安装。

### 已修（真机 logcat 里逐条对应）

- `dlopen failed: cannot locate symbol "__cxa_pure_virtual"` —— GDExtension 整个
  加载失败。原因是 Rust 侧没写 `libc++_shared.so` 的 DT_NEEDED；已由
  `.cargo/config.toml` 的 `-C link-arg=-lc++_shared` 修掉。
- `Unrecognized UID: "uid://bsg12huaf1u5i"` —— godot_mcp 写的 `MCPRuntimeProbe`
  autoload 指向被 `exclude_filter` 排除的 addons 脚本；已由导出期间临时摘除
  autoload + 插件项修掉。
- 车库面板被 HUD/TouchOverlay 盖住（CanvasLayer 层级问题）、按「返回」后导入
  按钮永久禁用——已修，真机复测面板正常显示、HUD 正确隐藏。

### 仍待处理

- `Feature register-docs is enabled in an exported (non-editor) build`：
  `god.setdefault` 的 `register-docs` 应改为仅桌面/开发构型启用（会减小包体）。
- 外部 Activity（系统文件选择器）打断后 Vulkan surface 丢失：
  `VK_ERROR_SURFACE_LOST_KHR` → 画面全黑；随后冷启动会
  `Couldn't create Android Surface (VK_ERROR_NATIVE_WINDOW_IN_USE_ANDROID)`，
  需要重启设备才恢复。真机旋转方向被锁死 + 息屏后 adb 无法唤醒，导致这条没能
  进一步定位。
- 大 zip 在主线程同步解压，无进度提示（既有待办）。
- MIUI「应用锁」会拦在系统文件选择器前面（设备配置问题，非代码缺陷）；
  导入回调本身正常（取消时能收到 `[garage] 导入已取消`）。

### 复现/验证命令

```sh
adb devices -l                                   # 需要先开机 Kali 的 USB 安装开关
python3 scripts/build.py apk
adb install -r -d build/android/Vettura.apk
adb logcat -c && adb shell am start -W -n com.vettura.game/com.godot.game.GodotAppLauncher
adb logcat -d | grep -E " E godot"               # 引擎错误（应为空/仅已知项）
adb exec-out screencap -p > /tmp/s.png
```
- Android 导出链路尚未验证（见 README「环境要求」）。
