# 变更记录

本文件记录 Vettura 的版本级变更。版本号遵循 SemVer：`0.x` 阶段允许在次版本间
调整内部结构，公共 API（GDExtension 暴露给 GDScript 的类与方法）以“能用”为准。

## 未发布

### 修复

- **渲染：赛道贴图不全 / 局部发黑**（`gdext/src/kn5/texture.rs` +
  `kn5/src/dds.rs`）。根因不止一个，修法都拿 Daytona / F1 2008 的 274 + 50 张
  真实贴图逐张验过：
  - KN5 导出器的 DDS 头有的 mip 链被裁到 2x2、有的声明 5 级却塞了接近 12 级的
    尾随字节、有的 flags 没声明 PITCH/LINEARSIZE 值却非零、还有未压缩格式
    被裁链——这些 Godot 全部整张贴图拒收，模型退化成无贴图的纯色（看台/人群/
    护栏/棕榈树因此是黑的）。现在按顺序试：修头 → 原样加载 → 砍成“只有基础层”
    → 截 payload → 截到声明级数，并给失败贴图打名字+尺寸的 error；
  - `tex_audit` 实测同时证明：`main.tscn` 之外，页面场景以前没有 Environment
    ——太阳照得到的地方亮、照不到的地方死黑。新增 `game_environment.tres`
    （天空 + 恒定色环境光 1.2）并用 project.godot 的
    `environment/defaults/default_environment` 挂成工程默认；`ai_lap.gd` 的
    AI_LAP_CAM 模式不再自己建环境（运行时 `AMBIENT_LIGHT_SOURCE_SKY` 实测不产生
    任何环境光）。
- **渲染：闪烁**（`gdext/src/kn5/material.rs`）。`isAdditive == 1` 以前映射成
  `BlendMode::ADD`：AC 只在镜面通道叠加，整面改成 ADD 后 Daytona 33 个网格
  （沥青看台、护栏、碳纤维件）变成互相叠加的半透明层，画面出现闪烁/乱纹；现在
  不映射该值。另外 alpha 混合材质一律 `DepthDrawMode::DISABLED`，避免同网格多面
  排序抖动。
- **渲染：车模颜色 / 涂装 / 轮胎**。三个独立 bug：
  - 车漆的红色在 AC 里放在 `txDetail`（`f2008_top_2.dds`），`txDiffuse` 只是一张
    灰色底图；旧实现把 txDetail 接到 Godot 的 detail 层但没有 UV2，等于拿贴图
    左上角那个像素涂满全车（车漆直接变白）。现在按 AC 语义映射成
    detail(MULTIPLY) + `uv2_scale = detailUVMultiplier`，并给带 detail 材质的
    模型补一份 `TEX_UV2`（= UV1，按需生成，不给赛道白加显存）；
  - 车漆清漆层强度 1.0 → 0.35（1.0 会把环境光反射成一片白漆）；
  - `ksTyres`（胎纹/胎侧）的 `isAdditive == 2` 以前也当车漆，轮毂被清漆高光打成
    全白；现在按 shader 名排除轮胎。`AC_specular.dds`（纯白细节贴图）糊在白块
    上的问题随之消失。
  - `ksPerPixelReflection`（看台墙面、水面、护栏玻璃、车手玻璃）改映射成
    metal + 低 roughness：AC 里这类材质靠环境反射发亮，albedo 被刻意压暗。

### 新增

- **AI 难度档（AC 的 AI level，0..=110）**（`world/src/ai.rs` +
  `gdext/src/world/mod.rs`）。`AiDriverTuning.skill` 缩放的是司机的能力而不是
  车的性能：抓地使用率、规划减速、预瞄距离、方向盘可用行程、反馈权限、
  稳态侧偏补偿、贴线晃动、避让横移、出界恢复与油门/刹车前馈。`0` 是新手
  （晃动 3 m、只敢用 55% 锁角、晚刹），`100` 是职业（默认，即原先调好的水平），
  `110` 是 AC 社区那个抓地超出物理极限的「不可战胜」档。Godot 侧
  `WorldBody.ai_skill()` / `set_ai_skill(skill)` 运行时切换，
  `AiDriver::with_skill()` / `set_skill()` 方便链式构造。整圈实测（Daytona +
  真 F1，600 Hz 物理 / 60 Hz AI / ABS+TC）：roadcourse 0 → 226.7 s、40 → 199.7 s、
  80 → 172.6 s、100 → 169.5 s、110 → 165.4 s；oval 0 → 95.1 s、100 → 79.7 s、
  110 → 78.7 s。
- **AI 弯速/刹车改为逐点曲率 + 包络前馈**（`world/src/ai.rs`）。同一条规则跑
  两套完全不同的 Daytona 布局实测：roadcourse 169.5 s（偏线 2.3 m、10 次轻擦碰）、
  oval 79.7 s（偏线 2.4 m）。做法：
  - 规划器不再按固定 4 m 网格的切线差估曲率（会把 Daytona 半径 26 m 的回头弯
    平均成 40 m，等于把限速放宽两倍），改为逐点三点圆曲率 +
    `max_curvature_between()` 取区间极值，并且**带符号**；
  - 「现在必须维持的最小减速度」按弯速（而不是包络）算，车因此贴着刹车包络
    提前刹车，而不是等车速掉到目标以下才反应；
  - 弯速吃路面倾斜（`bank_assist_mps2`）：同一个 182 m 弯在 31° bank 上就是比
    平地上快，倾斜方向由带符号曲率决定；
  - 下压力份额 0.36（原 0.5），由 60–70 m/s 的真实横向包络标定——oval 的
    `fast_lane` 按 3.4 g 写成（79 m/s、最小半径 182 m，纳斯卡抓地），这台 F1
    物理上到不了，规划器于是自己降到 ~69 m/s 并贴线跑完，没有为 oval 改过任何
    数字（此前开过去 35 m 外抛、卡死）；
  - 规划目标乘 5% 余量：贴着极限进弯的车没有修正转向误差的余地。
  Roadcourse 整圈：**169.5 s、最大偏线 2.3 m、10 次轻微擦碰、无严重撞击**
  （此前 260+ s、外抛 18 m、撞墙）。
- `assets/tests/ai_lap.rs`：改为**任意赛道/任意车**（`AI_LAP_TRACK` /
  `AI_LAP_LAYOUT` / `AI_LAP_CAR` / `AI_LAP_SKILL`），不再写死 Daytona。
- **弯速吃路面倾斜**：bank 提供的横向加速度直接进限速模型（`bank_assist_mps2`
  + `road_right`），同一个 182 m 弯在 31° bank 上就是比平地上快；曲率改为
  带符号（`max_curvature_between()` 返回区间内最紧那一点的**带符号**曲率），
  规划器才知道倾斜是帮忙还是拖后腿。
- **全系统日志**（`gdext/src/logging/`，见 `docs/logging.md`）：`InitStage::Scene`
  安装全局 tracing subscriber；`user://log.cfg`（INI）驱动控制台（stdout）+
  滚动文件（`user://logs/vettura.<日期>.log`，daily × 7）两个 sink，环境变量
  `VETTURA_LOG`/`RUST_LOG` 优先。文件层走非阻塞 writer 线程，物理主循环不等 IO。
  业务 crate 只 `use tracing` 发事件；`kn5` 解析统计、`assets::track_load`
  span + 摘要、`audio` bank 失败 error 已插桩。
- **日志线索增强（Phase 2）**：RingSink 有界环形缓冲（默认 4096 条）+
  `VetturaLogger` GDScript 类（`reload` / `set_level` / `log_dir` / `recent` /
  `dump` / `dump_to_file`）；panic hook 把 panic 消息 + backtrace + ring 转储
  追加写 `user://logs/last-crash.log`（超 1MiB 重建）；`user://log.cfg` 级别类
  键热重载（mtime 0.5s 轮询，sink 结构变化只 warn）；WARN/ERROR 按 callsite
  5s 窗口聚合（per-sink gate，单聚合者发聚合行）。运行期换 filter 用自研
  `DynamicFilter`（0.3 无 `reloading` 模块）。
- `test_visual/log_smoke.tscn`：`VetturaLogger` API 冒烟；`LOG_SMOKE_PANIC=1`
  触发真实 panic 验证 last-crash.log 链路。
- `kn5` / `audio` 引入 `tracing` 依赖（workspace 共享版本）。

- **整圈视频录制链路**（`scripts/record_ai_lap.py` + `gdext/ai_lap.gd`）。
  `AI_LAP_CAPTURE=1` 时探针每个渲染帧存一张 JPEG（连续编号，按
  `AI_LAP_CAPTURE_EVERY` 抽帧），Godot 的 `--fixed-fps 60` 让物理按固定步长
  走，所以视频时长就是圈速、不随机器快慢变化；脚本跑完再用 ffmpeg 编码成
  H.264。窗口模式补了反向补光灯并给追逐相机设了 FOV——以前单盏日光下车身
  背光面、看台内壁全在阴影里。`--no-run` 可只编码已有帧，`--keep-frames`
  保留过渡帧。默认 720p30 / CRF 24，175 秒整圈约 48 MB。

- **每个轮子一个的动态摩擦圆**（`world/src/vehicle.rs`）。原来每轮每步都算
  联合滑移的椭圆，但三个地方不完备：椭圆只在纵向/横向两个方向上各乘各的
  （`0.6 + 0.4·√(1−d²)` 的形式重复算了预算）、地板是常数 0.6、整段逻辑散在
  `friction()` 里没法单测。现在抽成纯函数 `combined_slip_circle` + `slip_floor`，
  并让地板**动态**：
  - 轮胎自己的衰减特性 `falloff_level`（AC 的 `FALLOFF_LEVEL`，3 倍峰值滑移
    还剩多少）——曲线越平，打滑时剩下的越多；
  - 本轮所压的路面（冰/雪地板更低，避免 bank 起步瞬间失去全部侧向力）。
  - 磨损、轮胎温度/压力、赛道温度、湿滑、橡胶沉积仍然照旧通过
    `tyre_window` / `environmental` / `groove` 缩放摩擦圆的**大小**——六个
    动态量现在各自只负责一个维度。
  - 6 个单元测试钉住：单方向不被误砍、椭圆单调让路、地板随路面/曲线变、
    松弛缩放、荒谬输入不发散、整圈不回归（skill 100/110 与 oval 布局逐项
    回归过）。
- **发动机热模型改成真节温器**（`world/src/vehicle.rs`）。原实现两个硬伤：
  冷却液更新式写成 `T + heat·(1−frac) + ambient·frac`——向环境温度插值写成了
  **加** `ambient·frac`，每一步都在把温度往上推；散热速率又是固定系数而不是
  按负荷定标，平衡点随负荷漂。结果冷却液一路冲到 250 °C 钳位、触发热保护，
  这台 F1 整圈只有 55% 功率。改成「有负荷就向工作温度设定点、收油就向环境
  温度」的恒定速率松弛后，冷却液稳在 100 °C、功率 0.89。整圈
  155.0 → **154.7 s**，oval 70.2 → 68.6 s。

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

## 实时遥测（未定版，2026-11-13）

新增网络遥测链路：`user://telemetry.cfg`（`ConfigFile`/INI）配置 →
`WorldBody` 逐帧采样 → 一行 JSON 经 **UDP / TCP** 发出，约 0.5 秒热更新。

- **核心 `world/src/telemetry.rs`**（headless，不依赖 godot-rust）：配置解析、
  三方向帧组装（`serde_json`）、`TelemetrySink`（UDP 数据报 / TCP 流 + 断线
  重连 + 指数退避）。workspace 新增 `serde_json` 依赖。
- **三个方向 + 细粒度子开关**：`vehicle`（pose/controls/powertrain/wheels/tyres/
  brakes/fuel）、`track`（weather/groove/impacts + `max_impacts`）、
  `timing`（laps/sectors/last_lap），各自 `enabled` 总开关。
- **采集范围 `scope`**：`ego`（主车）/ `all`（全场）/ `indices`（发车顺序下标列表），
  采样率 `rate_hz` 节流（首帧立即发）。
- **接线 `gdext` `WorldBody`**：`ready()` 读配置（缺失 = 静默禁用）、
  `physics_process` 末尾（step + `LapTimer::update` 之后）采样发送；碰撞用只读
  游标消费累积缓冲，不 `drain`；暴露 `reload_telemetry_config()` /
  `set_telemetry_enabled()` / `is_telemetry_enabled()` 给 GDScript。
- 文档：`docs/telemetry.md`（配置全表、帧格式、行为约定）。

## 依赖重构：赛道加载搬入 assets（未定版，2026-11-13）

按"插件只管算、结果回传主程序"的 NumPy 模式收紧编译期依赖图：全仓唯一一处
插件越级依赖（`world → kn5/acdata`）被断开。

- **`TrackModule` 及全部赛道解析**（`models*.ini` / `surfaces.ini` /
  `sections.ini` / 碰撞 KN5 → 三角网）从 `world/src/track.rs` 搬到
  `assets/src/track.rs`；`assets` 定位为 AC 资产编排层（车目录 + 赛道目录），
  新增依赖 `vettura-world` / `vettura-core`。
- **拆分中的类型归属**：`ColliderTriangle` / `MeshSurfaceIndex` /
  `triangle_height` → `world/src/surface.rs`（surface 反向依赖它）；
  `TrackSection` → `world/src/timing.rs`（`LapTimer` 用它）——否则成环。
- **新错误类型 `TrackError` / `TrackResult`**（`assets`）替代原
  `WorldError::TrackIo/TrackFormat` 在赛道路径上的使用；`WorldError::Kn5`
  变体删除，`world` 不再认识 kn5。
- **测试搬迁**：`world/tests/track_module.rs` → `assets/tests/track_module.rs`
  （命令改 `cargo test -p vettura-assets --test track_module -- --ignored`）；
  `world` 对 `assets` 仅保留 dev 依赖（`real_car_gearbox`），cargo 允许的
  dev-cycle，编译时已验证。
- **收益后的依赖图**：叶子插件（kn5/acdata/physics/audio/core）零内部依赖；
  `world → physics/core`；编排层 `assets → 全部解析插件 + world`、
  `gdext → 全部`。插件之间零边。
- **顺手优化**：删除 `gdext` 每帧每车多余的 `VehicleSnapshot::clone()`
  （33 字段，改传引用）。
- 文档同步：`docs/architecture.md`（crate 图、职责、赛道数据流）、
  `world/README.md`、`assets/README.md`、`AGENTS.md`（仓库地图、测试命令、
  crate 边界规则）。

## 依赖重构：赛道数据拆出 vettura-track（未定版，2026-11-17）

在「赛道加载搬入 assets」之后继续收紧边界：`world` 里混着三层东西——
赛道/环境纯数据、Jolt 仿真内核、控制律/观察层。这次把**不需要 Jolt** 的
第一层拆成独立 crate：

- **新 crate `track/`（`vettura-track`）**：`SurfaceMap` / `SurfaceTable` /
  `MeshSurfaceIndex` / `ColliderTriangle`（`surface.rs`）、`GrooveMap`
  （`groove.rs`）、`Weather`（`weather.rs`）、`AiSpline` 与点/采样/投影类型
  （`spline.rs`，原 `world/src/ai.rs` 的解析半边）、`TrackSection`
  （`section.rs`，原 `world/src/timing.rs`）。
- **错误类型归位**：新增 `TrackError` / `TrackResult`（`Io` / `Format`），
  `WorldError::TrackIo/TrackFormat` 删除，改为
  `WorldError::Track(#[from] TrackError)`；`assets::TrackError` 新增
  `Data(#[from] vettura_track::TrackError)`。
- **`world` 保留**：`World`、`VehicleRuntime`、`debris`、`backend`、`frame`
  （依赖 Jolt 帧）、`AiDriver` 控制律、`LapTimer`、`telemetry`。
  `world/src/lib.rs` 重导出 `vettura_track` 的全部类型，`assets` / `gdext` /
  测试的 `vettura_world::SurfaceMap` 这类旧路径依旧可用。
- **`assets` 依赖调整**：赛道纯数据类型改从 `vettura_track` 取；`World` /
  `AiDriver` / `LapTimer` / `TrackObjectId` 仍来自 `vettura_world`。
- **拆分细节**：`AiSpline` 的格式版本公开为 `SPLINE_VERSION`（测试构造
  合成线不再复制魔数）；`SurfaceMap::table_arc` 由 `pub(crate)` 改 `pub`；
  `ai.rs` 的 21 个单测按归属拆到两个 crate（spline 7 例、driver 14 例），
  `world_physics` 的 63 例集成测试不变。
- **验证**：`cargo test --workspace` 全绿（`vettura-track` 25 例、
  `vettura-world` 55 例 + 63 例集成）。
- 文档同步：`track/README.md`（新）、`world/README.md`、`docs/architecture.md`、
  `README.md`、`AGENTS.md`。

## crate 整合：`vettura-core` 并入 `vettura-physics`（未定版，2026-11-17）

收紧编译期依赖图的下一步：一个整 crate 只有一个函数被真正调用的，不配单独

存在。删除 `vettura-core`（crate 目录已移除）。

- **搬入 `vettura-physics`**：`core/src/coords.rs` → `physics/src/coords.rs`
  （`ac_to_godot` / `godot_to_ac` / 六个轴常量），`core/src/units.rs` →
  `physics/src/units.rs`（`GRAVITY = 9.806_65`，原 `units` 里的
  km/h 换算与单位别名全仓零调用，随 crate 一起删除，不搬死代码）。
  `physics::coords` / `physics::units` 成为公开模块，`GRAVITY` 重导出。
- **glam 再导出删除**：`vettura_core::glam` 的唯一意义是避免各 crate 版本漂移，
  而 workspace 的 `glam` 本来就锁同一版本；各 crate 直接 `use glam::…`。
- **收益后的依赖图**：`kn5` / `acdata` / `audio` 零内部依赖 → `physics`
  → `track` → `world` → `assets` → `gdext`。叶子插件比之前少一个，
  且没有 crate 依赖一个"只提供两个函数"的中间层。
- `docs/architecture.md`、`physics/README.md`、`README.md`、`AGENTS.md`
  同步（crate 清单、"共享约定"小节、仓库地图）。
- **验证**：`cargo test --workspace` 全绿，无新增警告。

## `world/src/vehicle.rs` 按职责拆子模块（未定版，2026-11-17）

不新开 crate——`world` 的模块边界靠目录就能表达清楚，多一个 workspace 成员
只会让"每个 crate 一个顶层目录"的地图变糊。

- **拆分结果**（原单文件 3690 行）：
  - `vehicle/mod.rs`（~2590 行）：`VehicleRuntime` 生命周期、每步子系统的
    推进顺序、快照组装；
  - `vehicle/constraint.rs`：spec → Jolt `wheeled vehicle` 尺寸换算
    （悬架几何、刹车力矩分配、变速箱/差速器、整车惯量、合法性校验）；
  - `vehicle/friction.rs`：逐轮动态摩擦圆（纵横向共享抓地预算 + 随路面与
    轮胎衰减特性变的全滑移保底）；
  - `vehicle/grip.rs`：Jolt 轮胎回调的共享输入；
  - `vehicle/thermal.rs`：三层胎温与刹车盘面/核心温度的纯演化函数；
  - `vehicle/aids.rs`：ABS / TC 闭环。
- **拆分原则**：共享状态留在 `mod.rs`，纯函数与纯数据下沉；跨模块访问
  `pub(crate)` 显式重导出。`grip.rs` 的原子量只暴露
  `publish_water_film` / `publish_tyre_window` / `scale_tyre_window` /
  `publish_relaxation` / `publish_groove` 五方法，其它子系统不再直接写位模式。
- **行为等价性**：拆分前先把原始 3620 行文件存为基准，用符号集合对比核对
  ——105 个原符号一个没丢，新增只有 5 个 publisher 方法。
  `world_physics` 63 例集成测试 + `vettura-world` 55 例单测全绿。
- 踩到并修掉一个自己引入的行为回归：轮胎窗原本是"先绝对值写入、再按平点/
   camber 比例缩放"，第一版把两步都换成 scale 导致窗口每步累积衰减、
  9 例集成测试失败；已恢复为 publish + scale 两个语义。
- 文档同步：`world/README.md` 模块地图、`docs/architecture.md`（车辆子模块表）、
  `docs/physics.md` 路径引用。
