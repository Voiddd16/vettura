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
- **桌面探针 `test_visual/desktop_probe.tscn`**：两阶段检查「加载车辆模组」——
  阶段 1 在浮空预览页检查四个视角与油门/刹车；阶段 2 载入真实赛道，AI 沿
  `fast_lane.ai` 驾驶 16 s 再重刹 6 s，逐秒遥测 + 截图，以 `PASS`/`FAIL` 退出码验收。
- **`WorldBody.ai_controls(index)`**：把 `AiDriver`（`fast_lane.ai` 追踪控制器）
  桥接给 GDScript，返回 `[油门, 刹车, 转向]`；赛道没有 AI 线时降级为空数组。
- 预览页支持桌面键盘：`W`/`↑` 油门、`S`/`↓` 刹车、`C` 循环视角。
- **赛道真实视觉模型**（`gdext/src/world/track_models.rs`）：渲染侧按 `models.ini`
  逐个实例化非 collision 的 KN5（复用车模的按文件缓存与 `scene::instantiate`
  管线），放置变换与物理共用 `TrackModule` 的 Godot 帧矩阵；全部失败才回退
  原来的碰撞低模，缺失文件 warning 跳过不中断（Daytona 实测 6 模型 0.6 s
  加载，路面标线/看台/维修区/灯柱齐全）。
- `world` 新增真车回归测试 `real_car_gearbox`（`--ignored`，需 `mods/`）：
  全油门 12 s 必须正常升挡加速，静止满刹车不得倒车。

### 修复

- 预览页在本地缺少 `test/` 真车资产、或 GDExtension 尚未注册时不再刷一片
  ERROR，而是降级为警告并跳过模型加载（`--install-android-build-template` 等
  工程级命令不再被噪音淹没）。
- `test_visual/smoke.tscn` 无车可测时只验证相机循环，输出 `PASS（部分）`
  而不是 `FAIL`。
- 合成 DDS 头改用 `DDSD_LINEARSIZE`：Godot 会拒绝“未声明线性长度却给了非零
  pitch”的压缩贴图，此前合成贴图全部加载失败（探针的 `albedo=0` 由此而来）。
- KN5 网格两处坐标缺陷一起修（车模轮位出现悬浮碎片的根因）：节点变换做
  `S·M·S` 而顶点保持 AC 原样，复合后顶点上多乘了一个轴镜像，顶点离原点越远
  偏得越狠（刹车盘、内胎全部飞出轮位）。顶点分量（位置/法线/切线）现在统一
  镜像，与物理挂点严格一致；同时翻转三角形绕序（KN5 顺时针正面 vs Godot
  逆时针，此前从车外看到的是内壁）。
- TC 与变速箱互相卡死：Jolt 升挡门槛（驱动轮 slip > 0.1 禁止升挡）撞上 TC
  目标 0.12，全油门永远升不了挡。`TC_SLIP_TARGET` 降到 0.08，`TC_MIN_FACTOR`
  降到 0——引擎惯性极小（AC `INERTIA=0.012`），两成油门就够把引擎钉死在
  限制器上持续烧胎。
- **重刹时 gear=-1、转速顶限制器**：Jolt 自动变速箱把负油门解释为“换倒挡”
  请求（`forward × gear < 0` 立即换向），而 `[COAST_REF]` 收油制动此前以负
  forward 发送——重刹立刻切倒挡，rpm 被拖到 19000，降挡补油/限速器再与倒挡
  互斗。现在油门永远 ≥ 0，coast 改为额外刹车踏板比例（`coast(rpm) × 挡位
  齿比 × 尾速比 ÷ 刹车总容量`），加在 ABS 与刹车衰减之后；空挡不注入。
- **DDS 孤儿 pitch 值**：部分导出器写出的头 flags 未声明 `PITCH`/`LINEARSIZE`
  却在 `dwPitchOrLinearSize` 留了非零值，Godot 整张贴图拒绝（Daytona 274 张
  贴图 42 张中招，看台/人群全黑）。`vettura-kn5::fix_orphan_pitch_value` 把
  孤儿值清零使头自洽，`decode()` 入口先修头（实测 67 条 ERROR → 0，313 张
  赛道+车模贴图 0 丢失）。
- ABS/TC 增加 < 2 m/s 低速护栏：静止抱死与爬行时滑移率虚高，辅助判定没有意义。
- `test_visual/main.tscn` 的 `TouchOverlay` 没有铺满屏幕（塌缩到原点）：触摸
  区域实际不可用，按钮文字从左上角画出叠在 HUD 信息上。
- gdext 三个探针场景的资产路径 `res://test/…` 迁移到 `res://mods/…`；
  `desktop_probe` 截图前先把追逐相机摆到位，避免第一张拍到相机默认位置。

### 已知边界

- 冷路径首帧仍要解码全部贴图；把 DDS 解码搬进 Rust（BC1/BC3）以并行化/后台化，
  是下一个版本的首要候选。
- 显式倒挡输入属于后续工作（当前倒挡只能由 Jolt 在静止松油门时自动切入）。
- 少数贴图的 mip 链截断会让 Godot 打印 `Expected Image data size` 但仍返回
  可用图（赛道+车模模拟 decode 313/313 成功），属无害噪音，未处理。

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
adb devices -l                                   # MIUI 需先打开「通过 USB 安装」
python3 scripts/build.py apk
adb install -r -d build/android/Vettura.apk
adb logcat -c && adb shell am start -W -n com.vettura.game/com.godot.game.GodotAppLauncher
adb logcat -d | grep -E " E godot"               # 引擎错误（应为空/仅已知项）
adb exec-out screencap -p > /tmp/s.png
```

## 仓库整理（未定版，2026-11-12）

在不改任何 crate 包名 / Rust imports / 构建产物的前提下，把代码树理顺（人类友好 + AI 友好）：

- **`test/` → `mods/`**：目录里放的本来就是本地 AC 车模/赛道资产（gitignore + `.gdignore`），
  原名"test"与测试代码严重混淆。同步更新所有引用：`.gitignore`、
  `test_visual/main.gd` 的 `DEFAULT_CAR_DIR`、`world/assets/audio` 三个真实资产测试的路径、
  gdext 文档注释、README、docs/godot.md、docs/kn5-format.md。
- **新增根级 `AGENTS.md`**：AI 助手仓库导航——目录是什么/不是什么、场景分工约定
  （`gdext/` 探针 vs `test_visual/` 游戏页）、命令速查、提交约定、已知坑清单
  （MCP 导出摘除、`-lc++_shared`、FMOD 不进 APK、`--quit-after` 单位等）。
- **方案文档归档**：`主项目方案.md`（拟真赛车模拟器 v1.0 设计定稿，未入库）移入
  `docs/plans/`；其"自研 sim-* 物理核心"与 roadmap「非目标：不自研完整轮胎模型」
  是当前最大待决策分歧，先整理现状、后议物理路线。
- **docs 刷新**：`architecture.md` 总览图补上 `world/` / `audio/`（原图缺失）并增加
  场景分工表；`roadmap.md` 对齐 0.0.1 实际进度（加载缓存/分帧加载已勾、探针路径
  `godot/`→`gdext/`、相机与 HUD 在 test_visual 已落地）。

## 风格化渲染子项目 stylized/（未定版，2026-11-12）

新增画面风格增强子项目：**纯 `.gdshader` + GDScript，不依赖 Rust 构建**，
按用途分类六类 + 公共工具：

- `glass/`：赛车玻璃（透射+菲涅尔+边缘辉光）、护栏玻璃（网格线）；
- `hud/`：圆角面板发光描边、270° 圆环仪表（超阈值变警告色）、扫描线/故障叠层；
- `color/`：HSV 车漆换色（带清漆）、遮罩分区涂装换色；
- `shape/`：反壳轮廓描边（推荐 next_pass）、植被风摆（世界空间顶点位移）；
- `lighting/`：边缘光叠层、卡通分档光照、流光假反射车漆（无 cubemap）；
- `post/`：暗角/调色/速度线/热浪/色差五个全屏滤镜 + `FxLayer` 滤镜链挂载器
  （后推的滤镜采样前面的输出，链式生效）；
- `common/`：noise / HSV / 菲涅尔三个 `.gdshaderinc`（函数统一 `vfx_` 前缀）。

四种集成方式按成本排序：`next_pass` 叠加（不动 KN5 材质）→ `material_override`
整件替换 → `Control.material`（HUD）→ `FxLayer` 全屏滤镜。全部 shader 规避了
spatial 侧读屏幕纹理，移动端 `mobile` 渲染器安全。

`showcase.tscn` 展示页：六类效果各一个展台 + HUD + 滤镜链，纯合成几何不依赖
`mods/`。展示页默认滤镜链只挂调色 + 暗角（速度线按需 `FxLayer.push` 启用）。
验证：headless 冒烟 + 桌面窗口实跑 300 帧零 shader 错误
（修正一处：Godot 4 无 `CLEARCOAT_GLOSS` 内建）。
