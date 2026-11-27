# 变更记录

本文件记录 Vettura 的版本级变更。版本号遵循 SemVer：`0.x` 阶段允许在次版本间
调整内部结构，公共 API（GDExtension 暴露给 GDScript 的类与方法）以“能用”为准。

## 未发布

### 显示管线与魔改引擎落点

- **统一显示管线**：新增 autoload `DisplayPipeline`
  （`game/render/display_pipeline.gd`），集中探测平台/后端/硬件能力并应用
  上采样、动态分辨率、VRS 与 XR。`RenderQuality` 不再直接写
  `scaling_3d_mode` / `scaling_3d_scale` / `vrs_*`。
- **上采样**：stock 4.7 支持 FSR1、FSR2、MetalFX Spatial、MetalFX Temporal；
  `auto` 在 macOS 用 MetalFX Spatial、其他 Forward+ 用 FSR1、移动端原生。
  显式选择不可用项会回退并记录一次 warning，不阻断画面。
- **VRS / XR**：`vrs_mode` 支持 `VRS_TEXTURE`（32×18 中心清晰密度图，
  `VRS_UPDATE_ONCE`）与 `VRS_XR`（XR 会话自动切换）；硬件/后端不支持时
  设置页标灰，Godot 侧忽略。
- **动态分辨率**：2 Hz 采样、5% 步进、降档 1s/升档 3s 滞回与冷却；目标
  帧率 `0=自动≤60`、`-1=跟随刷新率`；时域上采样最低比例固定 0.5。
- **魔改引擎落点**：新增 `engine/`（不是 crate）：Godot 4.7 补丁序列、
  `display_contract.json` 能力契约、`mods/*/manifest.json` 扩展模板。
  FSR3.1 通过补丁新增 `SCALING_3D_MODE_FSR3_1` 或 `VetturaEngineMod`
  桥接接入；没有补丁时自动回退。
- **设置页**：画面页新增「上采样」「可变着色率」「动态目标」三项与能力提示；
  设置键 `video/upscaler`、`video/vrs`、`video/dynamic_target_fps`。
- **冒烟**：新增 `game/display_smoke.tscn`，验证能力探测、回退、动态分辨率
  范围与 VRS 状态。

### 功能

- **输入系统重构**：新增 autoload `GameInput`（`game/input/game_input.gd`），
  运行时注册油门/刹车/转向/视角/车辆库/设置/返回/全屏动作；支持键鼠、手柄、
  触屏，以及手机陀螺仪/手柄体感（开关、灵敏度、反转、校准）。键位可在
  设置「输入」页重绑并持久化到 `user://controls.cfg`；手柄 UI 补 A/B 映射。
- **返回与焦点**：统一的返回栈（`push_nav_handler`）；Esc/手柄 Start 打开
  菜单，手柄 B/鼠标右键/Android 返回键关闭弹窗或返回上一页；大厅返回弹
  退出确认。驾驶页 HUD 不再抢手柄焦点（A 给油门），大厅加载完成后自动
  聚焦「开始驾驶」，车库/车库模组列表支持手柄确认。
- **显示模式**：`video/window_mode` 支持窗口 / 全屏 / 无边框（macOS 上无边框
  等价原生全屏），保存窗口尺寸；F11 / Alt+Enter / HUD 与大厅「全屏」按钮
  均可切换，启动时恢复上次模式。
- **驾驶页**：油门/刹车/转向改为模拟量，前轮可见转向；鼠标左键拖动、右摇杆、
  陀螺仪给相机自由视角；移动端底部新增触摸转向滑杆。

### 构建与平台

- 新增 macOS 导出预设（universal 模板，GDExtension 按本机架构）与
  `docs/build.md` 的 macOS 导出步骤；导出产物 `Vettura.app` 与同级 `mods/`
  组成可玩目录。
- `ModLibrary` 在 macOS 导出版额外扫描 `.app` 同级的 `mods/`：AC mod 的
  `extension/*.lua` 放进 `.app/Contents/MacOS` 会被 `codesign` 当成嵌套代码，
  导致签名失败，所以 mod 目录保持在包外。

### 全屏性能

- **全屏自适应分辨率**：桌面全屏默认把 3D 内部渲染控制在屏幕宽度约 90%
  （上限 2880），再按帧率在 35%~100% 之间调整；macOS 用 MetalFX Spatial、
  其他 Forward+ 用 FSR1 上采样，UI/后处理保持原生分辨率。全屏 MSAA 从
  4x 降到 2x，后处理切性能模式（保留暗角/锐化，去掉色散/桶形畸变）。
  M1 大厅全屏从约 14 fps 提升到 50~60 fps。
- **全屏锐化/模糊修复**：后处理锐化的 `viewport_size` 改用真实屏幕纹理尺寸，
  不再按 1280 设计分辨率算 texel；全屏/Retina 下锐化半径不再放大 2~3 倍。
- **全屏窗口尺寸修复**：进入全屏前先把窗口铺到当前屏幕，避免 macOS 沿用
  1280×720 窗口并把 3D 放大成模糊的 720p；窗口尺寸被写坏到接近整屏时启动
  自动重置为 1280×720，防止后续窗口/全屏继续以超高内部尺寸渲染。
- **全屏雨系统不再覆盖自适应比例**：`RainSystem` 在桌面全屏自动分辨率开启
  时不再把 `scaling_3d_scale` 拉回 1.0，也不再叠 FXAA。

### 加载与 AI 性能

- **赛道静态网格合并改用批量缓冲**（`gdext/src/kn5/merge.rs`）。旧实现逐顶点
  调用 `PackedVector3Array::push`/`get`，每个元素都要穿过一次 FFI；现在在
  Rust `Vec` 里累积，最后由 `PackedArray::from(Vec)` 整体移交给 Godot
  （纯内存移动），源网格也直接用 `as_slice()` 借用。实测 Nürburgring
  （2177 个网格 / 733 万顶点）合并 2315 ms → **约 750 ms**，Daytona
  （2008 个网格 / 299 万顶点）1165 ms → **约 250–330 ms**（同机、同资产），
  整体降到原来的 1/3 左右。逐顶点算力本来不是瓶颈
  （700 万次变换 + 法线归一化只占约 0.3 s），真正贵的是每元素 FFI 调用与
  `Packed*Array` 打包，所以收益来自批量内存搬运而不是手写 SIMD；日志新增
  合并耗时和顶点数，下次能直接看出回归。
- **AI 每帧投影从全量扫描改为提示窗口**（`track/src/spline.rs`、
  `world/src/ai.rs`）。`AiSpline::project_near` 用上一帧的段索引开一个
  ±64 段窗口，Nürburgring 的 `fast_lane.ai` 有 13 323 个点，单次投影
  **42 µs → 0.44 µs（96×）**；20 台 AI 车每帧从约 0.84 ms 降到约 9 µs。
  首次调用、传送/重置后提示失效、最优点落在窗口边界、或离提示路段超过
  30 m 时自动退回全量扫描；提示段本身离车超过 12 m（例如传送、或以后多车
  共用一个 driver）也直接全量扫描。结果与 `project` 一致（平局取更小索引）。
  验证：Nordschleife 20 s AI 探针最大速度 83.9 km/h、位移 253.2 m，与旧
  实现逐字节相同；`track_module` 真赛道测试全过；另加
  `track/tests/real_spline.rs`，在真实 13 323 点线上做 20 000 次逐帧查询，
  断言提示结果与全量扫描完全一致。
- **新增可复现基准**：`cargo bench -p vettura-world --bench step`
  （`World::step_exact` 吞吐，支持真实赛道/真车）与
  `cargo bench -p vettura-track --bench spline`（AI 线投影全量/窗口对比）。

### 性能（既有）

- **去掉每帧重复的 FX/UI 工作（全平台）**：天气同步 5 Hz、赛道灯光能量 4 Hz、
  刹车盘发光 20 Hz；雨系统的灯光/粒子/雾/屏幕叠层按 5~10 Hz 刷新，干燥天气
  跳过整条更新；大厅环绕缓存设置值，主页面 HUD 10 Hz 刷新；天空
  `cloud_offset` 改为 10 Hz 写入，避免每帧触发 radiance cubemap 重算。
- **GrassFX 重建去字符串/重复查询**：新增
  `WorldBody.surface_sample_at(x, z)`（一次返回表面类型编号 + 高度）和
  `SurfaceKind::index/name`；回退撒点改为 1.25 m 网格候选点。Daytona 的
  草重建从 43~142 ms 降到约 30 ms，实测平均帧时间 19.3 ms → 17.6 ms
  （M1 / 2560×1600 / 暴雨 + 全 FX）。
- **屏幕后处理不再生成 mip 链**：`post_fx.gdshader` / `rain_refraction.gdshader`
  只采样 LOD 0，`filter_linear_mipmap` 改为 `filter_linear`，省掉每帧全屏
  纹理 mip 生成，采样结果不变。

### 修复

- **起跑点雨天/夜晚出现滞留点光源**（`game/fx/lighting_fx.gd`）：LightingFX
  旧实现把赛道记录和车辆记录合在一起搜索，F1 车上的
  `[EMISSIVE_LIGHT_...] NAME=LED` 会匹配到起点附近几十个同名/同材质网格，
  给每个网格生成一盏默认 8 m 点光；车开走后灯留在起跑线。现在赛道记录只搜
  赛道、车辆记录只搜车辆；非 `LIGHT_SERIES` 只取一个代表网格；车灯直接挂到
  对应网格下跟随车辆移动，默认范围收到 2 m。

### 渲染

- **桌面切到 Forward+，移动端仍走 Mobile**（`project.godot`）。修正了
  `rendering/renderer/rendering_method` 与各向异性过滤键的错误前缀（旧值写成了
  `rendering/renderer/...`，实际落在 `rendering/rendering/...` 下，移动端覆盖
  从未生效）。新增
  `game/render/render_quality.gd`（autoload `RenderQuality`）按后端打开
  SSAO/SSIL/SSR/体积雾参数，避免把桌面专属特性写进共享 `.tres` 后在
  Android 上刷警告。
- 新增程序化天空 `game/render/sky.gdshader`：大气渐变 + 太阳盘/光晕 + 月亮 +
  云 + 夜间星空。云噪声改成预生成的 `NoiseTexture2D`（FastNoiseLite fBm），
  shader 从“每像素 20 层 value noise”降到桌面 4 次纹理采样、移动端 1 次；
  云层漂移改为每帧推进 `cloud_offset`（大厅/车库由 `RenderQuality` 推进），
  配合天空 cubemap 的 INCREMENTAL 更新，解决云彩 2 Hz 跳格和周期性卡顿。
  天气同步增加值死区（1% 湿度/雨量、0.05 m/s 风），不会因为物理层每帧的
  微小浮点变化反复重写天空/雾/体积雾参数。`game_environment.tres` 改用该
  天空，tonemap 切 ACES，glow/环境光/雾参数按 AC + CSP 天气 shader 的配方
  重调。
- 新增屏幕后处理 `game/render/post_fx.gdshader` + autoload `RenderFX`：
  暗角、径向色散、胶片颗粒、轻锐化、可选桶形畸变；层号低于 UI，不影响
  HUD。移动端默认关闭，`VETTURA_POST_FX=0` 可做 A/B。
- 材质层增强（`gdext/src/kn5/convert.rs`、`material.rs`）：
  `ksEmissive` / `*_emissive` 把 `txEmissive`（无则 `txDiffuse`）接进
  EMISSION，车灯/屏幕/招牌保留贴图细节；树叶/草/旗帜双面渲染；车窗在
  Forward+ 下开折射；车漆 clearcoat 重新调平，避免红色车漆被环境反射刷白。
- 湿路面（`game/fx/rain/rain_system.gd`）：桌面 Forward+ 下降低粗糙度、
  恢复部分介电高光并打开 SSR/体积雾，湿沥青能反射护栏/天空；金属度封顶，
  避免低角度太阳反射成过曝白斑。天气同步增加值比较，不再每帧重写环境。
- **GrassFX 参考 `acc-shaders`**：草卡片从硬 `discard` 改成
  `alpha_to_coverage`（`smoothA2C.hlsl` 的近似），叶片边缘随 MSAA 软化；
  加世界空间低频色斑（`txVariation` 的廉价版）；按 `WorldBody` 湿度同步
  `wetness`，湿草变暗、粗糙度降低并接住天空反射（`RAINFX_WET` /
  `RAINFX_REFLECTIVE` 的近似）。
- **RainFX 参考 `accRainDrops`**：雨丝增加深度软相交（`calculateSoft`
  的近似），靠近车/路面时淡出；加 Fresnel 环境反射色，边缘反射天空、
  中心偏背景折射色，不再整条纯白；保留软剖面、拖尾、逐滴差异和
  数量/粒径谱调整。
- **雨丝真实度**（`game/fx/rain/`）：雨滴不再用“满 alpha 的白棒”近似。
  横向改成二次方软剖面、纵向改成“亮头 + 渐隐拖尾”，每滴按实例 scale/位置
  散列改变长宽和亮度；`shutter_time` 从 1/60 提到 1/30，细/小雨滴占比提高，
  桌面总粒子 10000/16000（移动端约一半）；`exposure_gain` 从 8 降到 7，
  颜色从高饱和蓝白改为灰蓝。涟漪上限 6、透明度略增，让地面有雨滴反馈。
- **移动端专项**：桌面效果不硬塞给手机，但手机也有可见提升——
  - 移动端后处理改为 `post_fx_vignette.gdshader`：不读屏幕纹理，只做一层
    廉价径向暗角；
  - 天空云层移动端只采 1 次噪声纹理、128 辐射 cubemap，桌面 4 次采样/256；
  - 阴影降到 2 级 CSM / 64 m / 0.35° 软阴影，把预算让给雨/草/天空；
  - 草 medium 3200 / high 7200、半径 58 m，并按距离 alpha 淡出，近景密度
    基本不变、远景少 overdraw；
  - 湿路面在无 SSR 的移动端用 0.22~0.24 粗糙度 + 0.20 介电高光，靠天空
    cubemap 出湿亮高光，金属度仍封顶避免白斑。
- `gdext/desktop_probe.gd` 的赛道阶段改用工程默认环境（天空/环境光/后处理），
  探针截图与游戏观感一致。新增 [`docs/rendering.md`](docs/rendering.md)。

### 游戏大厅 / UI

- 新增 i18n：支持 **简体中文 / English**，默认跟随系统。autoload `I18n`
  （`game/i18n/i18n.gd`）以中文原文作为 key，向 `TranslationServer` 注册英文
  映射；静态 `.tscn` 靠 `auto_translate` 自动切换，动态文本走 `I18n.t()`。
  设置新增「界面 → 语言」，写入 `user://settings.cfg` 的 `[ui] language`；
  大厅/驾驶/展厅/车库/选择器/设置/输入绑定全部中英双语。新增
  `game/i18n_smoke.tscn` 验证运行时切换与映射完整性。
- 新增游戏大厅 `game/lobby.tscn`：用 AC 车库（track）模组做 3D 背景，
  把当前选择的车辆停在车库 `AC_PIT_0` / `AC_START_0` dummy 或室内地坪上，
  慢速环绕展示；支持拖动旋转与滚轮缩放。默认车库为
  `mods/ltk_csr_garage`，`project.godot` 主场景已切到大厅。
- 新增 `game/garage_mod.gd`：识别单 KN5 或 `models*.ini` 多模型车库，
  提供校验、展示名、预览图与 AC 放置帧 → Godot 帧转换；大厅按计划用
  `Kn5SceneBuilder` 分帧加载，55 MB 车库模型不会卡死首帧。
- 新增车库模组选择器 `game/garage_mod_picker.tscn`：列出 `ModLibrary`
  自动发现的赛道/车库，也可浏览任意目录；选择写入 `user://settings.cfg`，
  大厅立即重建。
- 新增游戏内设置弹窗 `game/settings_popup.tscn`：主音量、全屏、
  垂直同步、实时阴影、3D 分辨率、车库环绕速度/镜头距离、驾驶辅助与默认视角；
  大厅、驾驶页与 `main.tscn` 都可打开（`Esc` 亦可）。设置由 `GameSettings`
  autoload 统一应用并持久化。
- 新增赛道选择器 `game/track_picker.tscn`：按 `models_<layout>.ini` 展开布局，
  读取 `ui/<layout>/` 的预览/名称与 `<layout>/ai/` 的 AI 线/维修区；选择写入
  `GameSettings["race"]`。
- 新增真实驾驶页 `game/drive.tscn`：`WorldBody` 加载选中赛道与车辆，三种视角、
  圈速 HUD、ABS/TC、车辆重置、暂停设置、天气/FX、可选 FMOD 引擎声；
  键盘/手柄/触屏/体感输入全部复用 `GameInput`。大厅「开始驾驶」进入该页，
  「展厅预览」保留原 `main.tscn`。
- `ModLibrary` 赛道分类改为以 `models*.ini` 为准（WorldBody 的加载前提），
  并为每个赛道条目补充 `layouts` / `default_layout`；无 `models.ini` 的
  单模型目录归为展厅车库，不再出现在可驾驶赛道列表。
- 修复赛道视觉模型在未挂入场景树时做 AABB 合并导致的
  `!is_inside_tree()` 报错（`gdext/src/world/track_models.rs`）；
  `FmodSystem` 新增 `is_linked()`，无声构建可静默跳过 FMOD。
- 新增 `game/drive_smoke.tscn`：真实赛道 + 车、HUD、重置、暂停设置与
  1.5 秒油门移动验证。
- 新增 autoload `ModLibrary`（`game/mod_library.gd`）：把 `mods/` 作为统一
  mod 根目录，递归扫描 `res://mods`、`user://mods`（自动创建）与桌面导出版
  可执行文件旁的 `mods/`，自动分类为车辆 / 赛道 / 车库；车模库、车库模组
  选择器与大厅共用这份索引，导入的 mod 写入 `user://mods/imported_cars/`。
  默认车/默认车库缺失时会自动用库里发现的第一项。
- 车模库会自动列出 mods 目录下带 `data.acd`/`data/car.ini` 且有 `.kn5`
  的车，选择写回设置，大厅与 `main.tscn` 共用同一台车；`main.tscn` 增加
  「设置」「返回游戏大厅」按钮。新增 `lobby_smoke.tscn` 回归冒烟。

### 声效

- **修复引擎声几乎听不到 / 只有断续风噪**（`audio/src/runtime/engine.rs`、
  `audio/src/runtime.rs`、`gdext/src/fmod_system.rs`、`gdext/src/vehicle_body.rs`、
  `gdext/ai_lap.gd`）。直接根因：Core API `Channel::set_frequency` 的单位是 Hz，
  旧代码把 `rpm / 3000` 当倍率传进去，引擎采样被压到 0.35–3 Hz 次声波；
  同时 `Studio::update()` 后额外调了一次 `System::update()`，同一混音缓冲被
  推进两次，声音断续。改用 `Channel::set_pitch`（1.0=原速）并去掉重复 update，
  实测 WAV 频谱从 <20 Hz 占 99.8% 变成 200–1000 Hz 占 80%，`ai_lap` 实跑 8 s
  WAV rms 0.19、峰值仅 5 个采样触顶。
- fallback 从"第一个 FSB5 + 4 条固定采样"扩展成扫描所有 FSB5 块、按角色识别
  采样并随 rpm 交叉淡化：引擎 idle/on/off、风噪、胎噪、打滑、涡轮、换挡、
  回火、TC 都有独立音量。修掉 Porsche bank 的两个识别错误：`transmission_*`
  因含子串 `on_` 被误当成油门声、`acc*` 加速采样没被识别；现在 on 层是
  `acc1/acc_front/acc_rear_*` 等统一识别出的加速采样。
  `traction_control` 从逐帧一次性播放改成循环采样调音量，避免 60 Hz 下堆积
  上千个重叠 channel。没有转速标注的 bank 会均分到怠速..9000 rpm，不再统一
  估成一个参考转速导致低转整层静音。
- **修复声音发尖 / 变速箱声一直响**（`audio/src/runtime/engine.rs`）。三个根因：
  ① 引擎层把所有三角权重内的采样同时叠加，多份不同录音相加产生梳状滤波和
  尖锐感（Porsche 5 条 on 采样会一起响）；现在每层只交叉淡化相邻两条，单采样
  变调限制 0.75–1.20 倍，idle 在高转完全淡出。② 车外视角同时播放车头/车尾两套
  麦克风录音；现在优先车尾（`back`/`rear`），有车尾时排除 `front`。③ `upsh*` /
  `dwsh*` 是 30–90 秒的循环层，旧代码把它们当换挡一次性音效，每次换挡都启动
  一条几十秒的长采样，听起来像变速箱持续尖叫；现在换挡只播 `gearup*` /
  `geardn*` 短音效，`transmission_Nth` 仅在匹配挡位以 8% 音量循环播放。
  rpm 再做一阶低通，避免起步 0→红线跳变把 pitch 拉出尖啸。
- **声效算法统一化**（`audio/src/runtime/roles.rs` + `engine.rs`）。解析入口
  收敛为 `roles.rs` 里的单个 `classify()`：只依赖 AC bank 的通用命名、采样
  时长（`>=500 ms` 循环 / `<=1000 ms` 换挡一次性 / 回火可到 3 s）和
  `ext/int` 视角，不写任何车型分支；解析器带单元测试，可在没有 FMOD SDK 的
  机器上单独跑。交叉淡化、pitch 范围、音量映射对所有车一致，缺角色就静音。
  **音色完全来自 mod 自己的采样**：不合成换挡声、不做 EQ、不替换采样；
  一次性音效按视角选（车外 `EXT`、驾驶位优先 `INT`），不再在
  `gearup`/`gearupEXT` 之间轮换。F1 2008 没有换挡采样，因此没有换挡声，
  这是 mod 定义决定的。
- `VehicleParams` 新增 `gear` / `slip`，`VehicleBody` 暴露
  `max_wheel_slip_angle()` / `max_wheel_slip_ratio()`，`ai_lap.gd` 每帧喂入挡位
  和侧偏角；换挡声在 fallback 内部由挡位变化触发。`fmod_probe.gd` 同步改成
  8 参数调用。新增诊断例程 `audio/examples/fallback_probe.rs`（输出 WAV +
  打印选中的采样），用法与验收指标写进 `docs/audio.md`。
- 已知限制：F1 2008 bank 没有换挡采样，按“音色只取决于 mod”原则不做补音，
  因此该车没有换挡声；`game/main.tscn` 是静态模型预览页，没有接
  `FmodSystem`，只有 `ai_lap.tscn` / `fmod_probe.tscn` 有声。

### 渲染 / 天气

- **修复网格合并误删子节点（局部地面缺失/浮空的潜在根因）**（`gdext/src/kn5/merge.rs`）。
  合并时对原节点调用 `free()` 会连同整个子树删除；KN5 的节点树里 mesh 可能带
  子 mesh / dummy / 骨骼挂点，其中没被合并的（湿路面层、蒙皮、半透明）会跟着
  消失，表现为局部地面缺块、看起来悬空。现在删除前把子节点提升到父级并保持
  全局变换。验证：合并前后场景网格总数严格守恒（2943 - 2177 + 1472 = 2238）。
- **模型文件名单复数回退**（`gdext/src/world/track_models.rs`）：`models*.ini`
  写 `marshals.kn5`、目录里却是 `marshal.kn5` 时，自动尝试单复数互换再加载
  （Nürburgring 实测多救回 81 个工作人员网格），不再整块跳过。
- **`SurfaceMap::sample_height`**（`track/src/surface.rs`）：补上桥/贴地效果
  需要的便捷方法（转发到 `MeshSurfaceIndex::sample_height`）。
- **修复高速行驶时护栏渲染跟不上**（`gdext/src/kn5/merge.rs` 新增、`track_models.rs`）。
  实测 Daytona 全速时 draw call 828~1876、CPU 主线程 16~21 ms，已经贴着 60 fps
  的 16.7 ms 预算——高速时更多护栏进入视野就会掉帧/延迟。新增加载期**静态网格
  合并**：按「材质 + 200 m 空间块」把 2008 个 `MeshInstance3D` 合成 **773 个
  批次**，draw call 降到 858~915（高速时约为原来一半），同时保留空间剔除
  （prims 回到 119~132 万，不因合并而翻倍）。不合并湿路面层（天气要按节点
  切换）、蒙皮网格、半透明材质和隐藏节点；法线用
  `basis.inverse().transposed()` 烘焙。画面验证无误。
- **修复观众席看台浮空**（`gdext/src/kn5/mesh.rs`）。上一轮把 KN5 的
  `lod_in`/`lod_out` 直接映射成 `visibility_range` 是错的：AC 的这两个字段是
  “**当前这一级 LOD** 的可见距离”，超出时游戏会切换到下一级更简的 LOD 模型；
  我们只加载了主网格一份几何，按 `lod_out` 裁掉就变成整块网格凭空消失。
  Daytona 看台的分层网格 `gar_*_KSLAYER4` 的 `lod_out=400`，于是 400 m 外
  只剩上层结构、下半截消失，观众席看起来悬空。现在保留字段解析（模型信息 /
  未来真做 LOD 替换时用），但**不再参与渲染裁剪**。验证：追逐视角远处看台
  人群、分层结构、底部墙面全部完整。
- **赛道贴图细化：跨文件去重 + 法线全精度 + 可配置尺寸上限**
  （`gdext/src/kn5/{cache,texture}.rs`）。
  - **跨文件去重**：AC 赛道模组会把同一张贴图分别嵌进多个 KN5（Nürburgring
    的 `curb2`/`global_logos`/`walls_new` 等同时出现在 `fn_nurburgring.kn5` 与
    `gp.kn5`，`cmp` 确认字节完全相同）。按内容哈希共享同一份 `ImageTexture`：
    实测 Nürburgring 310 张贴图里 **60 张跨文件命中**，加载时间 4.7 s → **2.6 s**。
    去重表在切换赛道/引擎关闭时清空（`on_stage_deinit` 里清，否则退出时
    `Gd` 析构访问已销毁的引擎会 SIGABRT）。
  - **法线贴图全精度**：退化路径重压缩时，桌面上法线类贴图保留 RGBA8 不压
    BC3（BC3 的 RGB 端点插值会量化法线方向，高光/凹凸细节发脏）；漫反射仍压
    S3TC。移动端为显存仍压 ETC2。
  - **尺寸上限可配置**：`VETTURA_MAX_TEXTURE`（0/未设 = 不限，移动端默认
    1024）。Nürburgring 设 2048 时显存 **1596 MB → 1138 MB（-29%）**；
    修复了“`Image.resize` 对压缩图像无效”的坑（先解压再缩放再重压）。
  - 分析结论：Nürburgring 1125 MB 内嵌贴图**全部被材质引用**（0 张未使用），
    优化点只在重复与尺寸。
- **修复路面反光过度 / 像镜面不像沥青**（`gdext/src/kn5/convert.rs`）。路面材质名
  实测的 roughness 只有 0.08~0.33（AC 的 `ksSpecularEXP` 动辄 200+，换算后极
  光滑），在 Godot 里低角度看路面会把天空反射成一条刺眼白带。AC 沥青实际
  粗糙度 0.6~0.9，所以给路面类材质（名字含 `road`/`asph`/`tarmac`/`oval`/
  `line`/`kerb`/`apron`/`pitlane`/`safera` 或 `ksMultilayer_fresnel_nm` shader）
  设 roughness 下限 0.6。只影响路面/路肩，车漆/内饰的镜面感不变。
  验证：路面变为自然哑光沥青，Daytona / Nürburgring 两个赛道近景、低角度都正常。
- **贴图精度 + 各平台渲染效率优化**（`gdext/src/kn5/{texture,mesh}.rs`、
  `gdext/weather_visuals.gd`、`project.godot`）。
  - **显存**：修复链解压出的 RGBA8 贴图按平台重新压回 GPU 格式——桌面 S3TC
    (BC1/BC3)、移动 ETC2（Godot 运行时 `Image.compress`），失败则保留 RGBA8；
    原样加载成功的 DDS 仍保留原压缩格式，不走这条路径；
  - **移动端显存上限**：Android/iOS 上超过 1024 边的贴图等比降采样
    （LANCZOS），桌面保持原尺寸（精度优先）；
  - **LOD 距离**：KN5 的 `lod_in`/`lod_out` 以前解析后从未使用，现在映射到
    `visibility_range`（5 m 淡出余量），超距离网格不再提交绘制；
  - **按平台画质**：桌面 MSAA 4x + 8x 各向异性、移动端 MSAA 关 + 4x；
    `project.godot` 各向异性默认提到 8x（`anisotropic_filtering_level=4` 是
    16x 枚举，代码按平台覆盖）。终点线/标线在移动时的锯齿和摩尔纹明显减轻。
  实测（M1，Daytona roadcourse 全视觉模型）：加载 4.3 s、**58 FPS**、纹理显存
  445 MB、帧内 54 个渲染对象；Nürburgring Nordschleife：加载 4.7 s、**55.6 FPS**、
  516 个渲染对象、0 张贴图失败。
- **通用兜底：KN5 里的 PNG/JPEG 贴图 + Rust 侧 BC 块解码**（`kn5/src/dds.rs`、
  `gdext/src/kn5/texture.rs`），以 Nürburgring Nordschleife（`fn_nurburgring`）为
  试样、目标是覆盖更多赛道 mod。三处修复都是通用的：
  - **magic 检测**：新 mod 的 KN5 直接嵌 PNG/JPEG（如 Nordschleife 路面的
    `Road_base_work.png`），以前被当 DDS 白白跑一遍修复链再整体丢弃；现在按
    文件头分派给 Godot 的 `load_png/jpg_from_buffer`；
  - **BC 块解码兜底**（`decode_base_rgba`）：Godot 的 DDS 加载器对非 4 倍数
    宽度（`newPanels.dds` 2127x591）等头无论怎么修都拒绝——补一个 Rust 侧的
    BC1/BC2/BC3/BC4/BC5 → RGBA8 解码器（含 BC5 法线 z 重建、BC1 三色透明、
    未压缩通道掩码），所有修复路径失败后走它，附 5 个像素级单元测试；
  - **albedo 下限 0.35**：Nordschleife 路面 `asph_new` 的
    `ksDiffuse=0.12 / ksAmbient=0.23`（AC 靠 detail 层+envmap 提亮），旧下限
    0.05 让路面黑成一片；0.35 是“任何贴图都能读出来”的通用下限。
  验证：Nordschleife 加载后 0 张 "unusable"（修复前 7 张），路面/树墙/围挡/
  赛车正常；Nordschleife 橙色围挡为贴图本色（barrier1.dds 均值 0.43/0.33/0.33），
  非 bug。Daytona 回归（smoke/garage_smoke/整圈 AI）全部 PASS。
- **修复远处环境发黑 / 背光面像没被阳光照到**（`game_environment.tres`、
  `gdext/weather_visuals.gd`）。原因是环境光太弱：`AMBIENT_SOURCE_COLOR` 只用
  (0.55,0.6,0.68)×1.0，而看台、建筑、远处围栏朝赛道的面全部背光，只能靠环境光
  照亮 → 发黑。对照测试确认 `AMBIENT_SOURCE_SKY` 在 Godot 4.7 的资源文件方式下
  同样不产生环境光（人群直接全黑），因此改 `AMBIENT_SOURCE_COLOR` 为
  (0.72,0.77,0.85)×2.0；太阳能量从 1.4 恢复到 2.2（之前为压高光而降，高光已
  由 `metallic_specular` 修复，不必牺牲整体亮度）。整圈多点截图确认看台人群、
  建筑、围栏、路面都正常受光，且无白斑回归。
- **进一步抑制起跑区移动时的视觉抖动**（`project.godot`、`gdext/weather_visuals.gd`）。
  固定机位逐帧对比证明没有持续性的时间闪烁；移动时的差异主要来自黑白格/
  标线这类高频贴图的纹理 aliasing 和阴影边缘。启用 4x 各向异性过滤；太阳阴影
  从默认 2 级 CSM 改为 4 级、近景分割更细、开软阴影（`shadow_blur=1`），
  让起跑区大平面拿到更多阴影图分辨率、边缘不再沿像素网格跳。
- **修复路面反光过曝 / 大片纯白**（`gdext/src/kn5/{convert,material}.rs`）。起跑区
  右侧路面在低视角下反射日光变成一整片纯白（像素 1.0），盖过沥青纹理。根因：
  AC 的路面材质（`SAFERA` 等 `ksSpecular=1.0`、反射类 `ksPerPixelReflection`、
  多层 `ksMultilayer_fresnel_nm`）靠 envmap+fresnel 控制高光，我们没有 AC 的
  envmap，Godot 用默认天空+日光近似过强。逐材质/属性二分定位后修复：对这类
  材质关掉 Godot 4.7 的介电高光属性 `metallic_specular`（旧名 `specular` 的 setter
  在这版不写它），金属类（metallic>0.5）额外设粗糙度下限 0.3。草地
  (`ksSpecular=0`)、车漆 (0.3) 都不受影响。验证：白区像素 1.0→0.29，整块区域
  纯白占比从 ~50% 降到 0.02%，路面纹理恢复可见。
- **修复起点黑白终点线渲染错误**（`gdext/src/kn5/{convert,mesh}.rs`）。终点线
  （`startfinishline` → `sfline.dds`）有两个独立问题：① 网格与路面**完全共面**
  （AABB 一模一样），深度测试反复翻转 → 黑白格撕裂/缺角；② AC 的
  `ksDiffuse = 0.3` 在多层 shader 里是混合权重，被通用材质映射当 albedo 系数
  乘进贴图 → 白格变灰、黑格被偏蓝环境光染成深蓝。修复：终点线材质 albedo 固定
  1.0 保留贴图原色，网格沿局部 Y 微抬 4 mm 拉开深度差（实测 2 cm 太大会把黑格
  挡掉）。诊断确认贴图本身解码正确（256×2048 黑白格），对照实验证明抬升才是
  消除撕裂的关键、颜色调整单独无效。加了单元测试
  `start_finish_line_keeps_its_checker_colors`。
- **修复起跑区路面自遮挡条纹**（`gdext/src/world/{track_models,track_mesh}.rs`）。
  赛道视觉网格跨度数公里且近共面（Daytona `daytonamesh.kn5` 单文件 44 张路面），
  定向光阴影图的深度精度不够，路面投到自己身上形成细密条纹/闪烁；单纯抬
  bias/normal_bias 消不干净还会让车体阴影脱离地面。现在赛道网格只接收阴影、
  不投射，车辆照常投射，1920×1080 起跑区画面干净。
- **修复终点线前方路面移动时剧烈闪烁**（`gdext/src/kn5/mesh.rs`）。同一机位
  连抓帧发现：干燥时干/湿两套共面路面的 z-fighting 是主因——隐藏湿层后相邻帧
  差异从 126603 降到 38193（约 30%），不是此前误查的车身高光/阴影。最终方案：
  湿层**整体下移 3 mm**，不与干层共面（闪烁消除，实测差异 44892），同时保留
  几何——隐藏湿层虽然也能消除闪烁，但赛道边（路肩/缓冲区）往往只有湿层几何，
  隐藏后整圈消失、看起来悬空（用户实测反馈）。AABB 配对（命中率仅 5%）与
  "全部隐藏"两种方案均已废弃。
- **进一步抑制起跑区移动时的视觉抖动**
- `kn5-inspect --nodes` 输出补上 `visible` / `cast_shadows` 两个网格标志，便于
  排查这类渲染标志问题。
- **按 KN5 标记过滤不可渲染网格**（`gdext/src/kn5/builder.rs`）。`scene.rs` 文档一直
  要求 `active && is_visible && is_renderable` 才渲染，但 builder 只看了 `active`，
  把 `renderable=false` 的物理/辅助网格也画了出来：Nordschleife 的 `nords.kn5`
  护墙变成两侧橙色巨墙；Daytona 的 `daytonamesh` 物理路面叠在视觉路面上，也是
  之前起跑区条纹的来源之一。现在按标记过滤，两个赛道画面都正常。

### 验证 / 录像

- **整圈录制新增六种视角**（`gdext/ai_lap.gd`、`scripts/record_ai_lap.py`）：
  在原有 `chase`（车后）/ `cockpit`（驾驶位）之外加入 `hood` 前机盖、
  `bumper` 前保险杠低机位、`wheel` 轮胎特写、`drone` 无人机滞后跟随、
  `tcam` 赛道边定点机位追踪（车到点后切下一个机位，左右交替）、`orbit`
  环绕跟拍；别名 `bonnet` / `tyre` / `tv` / `fixed` 等自动折叠。机盖与保险杠
  机位按车模网格包围盒自动适配不同车型，可用 `AI_LAP_HOOD_*` /
  `AI_LAP_BUMPER_*` 覆盖；各镜头焦段/距离/高度均有 `AI_LAP_*_FOV` 等环境变量。
- 以 `--resolution 1920x1080` + `cargo build -p vettura-godot --features fmod`
  重跑 skill 110 单圈：Daytona roadcourse / Porsche 992 GT3 R 两段镜头（车后、
  驾驶位）均 PASS，圈速 168.05 s / valid。视频见 `build/ai_lap_110_hd/videos/`。
- **录制卡帧修复 + 环境兜底**（`gdext/ai_lap.gd`、`scripts/run_ai_lap_test.py`）：
  录制时每 2 帧同步做 1080p GPU 回读 + JPEG 编码 + 写盘，会卡住渲染循环，实时窗口
  看起来"停住"。现在 JPEG 编码/写盘放到后台线程（主线程只做回读），脚本两段默认
  **串行**、`--parallel` 仅作不推荐选项；QA 改用逐帧 `framemd5` 统计最长连续相同帧
  （成片实测 1–2 帧、无冻结；`freezedetect` 在赛道同色路面上会误报，已弃用）。
  另在 macOS 上检测 CLT 的 `usr/include/c++/v1` 是否缺头（本机只剩 3 个旧头），
  缺则用 SDK 的 libc++ 目录经 `CPLUS_INCLUDE_PATH` 兜底，避免 `cargo clean` 后
  Jolt 编不过。
- **整圈测试全流程脚本**（`scripts/run_ai_lap_test.py`）：一条命令完成
  `cargo clean` + workspace/fmod 编译 + test/fmt + 两段（车后/驾驶位）1080p 录像
  （JPEG 帧与 FMOD WAV 分开采集）+ atempo 对齐混流 + 两段合并 + 抽帧/音量 QA +
  `REPORT.md`。赛道、车辆、视角、难度、分辨率、相机（车后距离/高度/前视/FOV、
  驾驶位眼位/FOV）与天气均为参数，见 `--help` 与 `scripts/README.md`。
- **音视频分开采集后混流**（`audio/src/runtime/engine.rs`、`gdext/src/fmod_system.rs`、
  `gdext/ai_lap.gd`）：设 `VETTURA_AUDIO_WAV` 时 FMOD 用 WAVWRITER_NRT 把混音直接
  写成 WAV，GDScript 在每个物理帧调 `step_audio()`，音轨长度跟随仿真时间。AC 老
  bank 的事件引用 bank 外 bus，Studio 事件实际静音（`Bus not found`），因此录制
  模式下改从 bank 内嵌 FSB5 取真实采样（idle / onthrottle / wind / tyre_rolling），
  按 rpm/油门/车速经 Core API 驱动，常见于老 mod 的无 master bank 场景。正常游玩
  路径完全不变。实测发现 FSB5 的采样名带 mod 前缀（`056_ext_idle`、
  `056_ext_on_back_*`），旧代码用精确名匹配导致 fallback 初始化失败、WAV 全是
  静音；改为大小写无关的子串匹配（`ext_idle` / `ext_on` / `wind` / `tyre_rolling`）
  后录到真实引擎声（非静音样本占比 >99%）。

### 渲染 / 天气（历史修复）

- **修复赛道起点地面闪烁**（`gdext/src/kn5/{mesh,builder}.rs` + `game_environment.tres`）。
  两个根因叠加：① AC 赛道 KN5 把干/湿两套路面放在完全相同的坐标上（Daytona
  有 396 个 `_WET` 网格），以前两套全画 → 起跑线附近大面积 z-fighting；现在湿滑层
  打 `vettura_wet_layer` 元数据、默认隐藏，由天气系统按湿度只显示一套。② 太阳
  阴影自遮挡条纹（acne）——`shadow_normal_bias = 0.08` 修正。临时探针逐帧对比
  确认：修复后起点俯视机位的共面条斑完全消失。
- **补全天空 + 天气系统**（`gdext/weather_visuals.gd` / `.tscn`、`project.godot`、
  `gdext/src/world/mod.rs`）。根因是 `project.godot` 丢了
  `environment/defaults/default_environment`，且 `game_environment.tres` 把
  `ProceduralSkyMaterial` 直接当 `Sky` 用（Godot 3 写法），天空一直是 null →
  深灰。现在恢复默认环境并用 4.7 正确写法（`Sky` 包一层 `sky_material`），
  有蓝天渐变 + 太阳盘；新增天气视觉节点把 `vettura-track` 的物理天气映射到
  天色/日照/雾/雨粒子，`WorldBody` 新增 `get_weather()` / `set_weather()` 双向
  同步，`ai_lap.gd` 挂上该节点并支持 `AI_LAP_WEATHER=clear|overcast|rain|fog|night`。
  验证：晴/雨/雾/夜四组截图均正确切换，天空从深灰变蓝天。

### 赛道兼容

- **修复 `physics.kn5` 两位数字碰撞前缀被漏收**（`assets/src/track.rs`）。部分赛道
  （如 fn_nurburgring Nordschleife）把物理路面命名成 `01ROAD` / `02ASPH` / `01GRASS`
  这类两位数字材质组，旧逻辑只认 AC 的 `1` 前缀，导致 20.8 km 赛道只收到 610 个
  墙三角形，车辆一出生就下坠。现在 `physics.kn5` / `collision.kn5` 里的全部网格
  都参与碰撞，并用宽松表面键识别 `01ROAD` / `01GRASS` / `02CURB` 等。
  实测 Nordschleife 碰撞三角形 610 → **945,213**，车辆四轮接地正常行驶。
- **探针新增 `AI_LAP_SPAWN_GRID`**（`gdext/ai_lap.gd`，脚本对应 `--spawn-grid`，默认 1）：
  出生点正好在起跑线上时，首次越线要等整圈之后，第一圈不计时（Nordschleife 实测
  要 873 s 才开始计时）。默认改为在线后 8 m 的发车格出生，计时在起步后立即开始。

### AI / 物理性能

- **AI 配速倍率**（`world/src/ai.rs`、`gdext/src/world/mod.rs`、`gdext/ai_lap.gd`、
  `scripts/run_ai_lap_test.py`）：新增 `SkillProfile::pace` 与 `AiDriverTuning::pace`。
  110 档自带约 **1.65** 配速，同时放大 AI 线授权速度与规划横向包络（直道 ×pace、
  弯速约 ×√pace），100 档保持 1.0 不变；探针/脚本可用 `AI_LAP_PACE` / `--ai-pace`
  再乘外部倍率。实测：Nordschleife Porsche 992 GT3 R 869.83 → **596.83 s**（valid，
  最高 227 km/h）；Daytona roadcourse F1 144.4 → **137.74 s**（valid，311.6 km/h，
  最长腾空 0.97 s）。Nordschleife 再加 `--ai-pace 1.2`（有效约 1.98）可到
  **541.24 s**；但有效配速约 2.06 时 Daytona F1 会反复卡死（3 次救援）——所以
  110 档内置配速保持 1.65，更高倍率按车/赛道用 `AI_LAP_PACE` / `--ai-pace` 单独试。

- **辅助弹簧载荷并回轮胎摩擦**（`world/src/vehicle/{grip,mod}.rs`）。Jolt 给轮胎
  回调的 `suspension_impulse` 只含线性弹簧，而 AC 车有 53% 的角载荷由 heave/
  渐进弹簧以外力形式承担——轮胎一直只按 47% 载荷算抓地。现在每步把辅助弹簧
  载荷发布到轮胎回调，摩擦系数按 `总载荷/线性载荷` 放大（载荷敏感仍按总载荷
  算）。静态四轮载荷实测从 2769 N 恢复到车重 5945 N，4 个纯函数单测覆盖
  地板与钳位。
- **悬挂几何按 AC 止点重建**（`world/src/vehicle/constraint.rs` +
  `assets/src/physics_mapping.rs`）。`PACKER_RANGE` 和 `SPRING_RATE` 在真车上
  不自洽（总行程 0.04 m、静载压缩却要 0.037 m），静置就快顶到 bump stop，
  1 g 就顶死把车弹飞、两轮离地搁浅。改用 `BUMPSTOP_UP/DOWN` 作为真实止点，
  静置高度由力平衡决定，bump 可用行程恢复到 0.11 m。
- **AI 重标定与稳定性**（`world/src/ai.rs`）。刹车改成随速度变化
  （`μ·g·0.68 + 0.40·下压力/m` 与扭矩限制取小，再乘 0.85），不再用固定
  10 m/s²；侧偏补偿增益 0.0085 → 0.0015（载荷修复后轮胎只需很小侧偏角，
  旧值让 AI 在弯里反复摇摆）；110 档的「超人类加成」从放大抓地改为放大控制
  精度；新手晃动 3 → 2 m（3 m 会让车压路肩被弹飞后搁浅）。全程数据驱动，
  没有为单条赛道调参。
- **整圈实测**（Daytona 真 F1，600 Hz 物理 / 60 Hz AI / ABS+TC）：roadcourse
  0/50/100/110 = **196.0 / 165.9 / 145.9 / 144.4 s**（此前 207.6/176.0/154.7/149.4 s），
  100/110 档偏线仅 0.68/0.69 m、零腾空；oval 110 档 **61.8 s**（此前 68.6 s）。
- **车窗/挡风玻璃透光**（`gdext/src/kn5/convert.rs`）。AC 的车窗 shader
  （`ksWindscreen` / 名为 window/glass 的 `ksPerPixelReflection`）不写
  `ksOpacity`，Diffuse 贴图 alpha 也不是透明度，转换时被当成完全不透明，
  座舱里看出去是一整块黑板。现在给这类材质固定 0.30 alpha（同时让
  `alpha_blended` 生效、关闭金属度，避免半透明金属发黑）。判定只认 shader 名
  和材质名，不碰同样用 AlphaBlend 的赛道建筑（如 `mel_pit_building_b_02`）。
- **座舱相机刚性化 + 去掉终点标记牌**（`gdext/ai_lap.gd` +
  `gdext/src/world/track_models.rs`）。座舱相机原来是世界坐标每帧重算，车身
  俯仰时会相对座椅前后漂移、在 bank 上穿出车顶；现在作为车身节点的子对象
  固定局部变换，眼睛高度降到 1.00 m（座舱内、低于车顶），能看到方向盘按键、
  Porsche 仪表、A 柱。赛道视觉模型加载时跳过 `acstobjects.kn5`（AC 编辑器用
  的起终点/维修区标记，比赛视图里不显示）——它此前在终点附近渲染成一排
  巨大的黑底红禁止通行牌。
- **整圈录制支持驾驶位视角与换车**（`gdext/ai_lap.gd` + `scripts/record_ai_lap.py`）。
  录制脚本的 `--car` / `--track` / `--skill` 之前被 Godot 端硬编码吞掉；现在
  GDScript 真正读取这些环境变量（支持裸模组名 / `res://` 路径），新增
  `AI_LAP_VIEW=chase|cockpit` 与 `--view`，驾驶位相机按车的 `DRIVEREYES` 定位
  （可用 `AI_LAP_EYE_X/Y/Z` 覆盖）。Porsche 992 GT3 R + skill 110 实录两段：
  `build/ai_lap_porsche_992_gt3r_{chase,cockpit}.mp4`，均为 1280×720 30fps、
  165.2 s、圈速 165.14 s。
- **诊断探针**（`world/tests/{launch,longitudinal,steer}_probe.rs`）全部改为
  ignored 测试，测静载、自由落体、加减速包络、稳态圆；`ai_lap` 增加每轮
  横向 g 与授权线速列。

### 修复

- **单圈测试声效**（`gdext/ai_lap.gd` + `audio/src/runtime/engine.rs`）。整圈探针
  以前完全不碰 `FmodSystem`，窗口模式 / 录视频跑一圈一路静音；现在起一个
  `FmodSystem` 加载当前车的 `sfx/<car_id>.bank`，按视角循环播放 `engine_ext` /
  `engine_int`，每帧把 rpm / 油门 / 近似负载 / 车速喂给引擎事件，能听到随转速
  变化的引擎声。顺带修掉 `vettura-audio` 两处会让声效静默失效的坑：
  `EventDescription::get_parameter_description_*` 探测会让 `EventInstance` 变成
  invalid handle（改成固定候选名逐个尝试）；运行期不再设 3D 声源/听者，FMOD
  默认听者与事件都在原点，声音以固定音量持续播放（与 `fmod_probe` 一致）。
  `AI_LAP_AUDIO=0` 可关闭；没编 `--features fmod`、没 SDK 或 bank 兼容问题时
  只打 warning，不影响圈速 PASS/FAIL。
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
- `game/log_smoke.tscn`：`VetturaLogger` API 冒烟；`LOG_SMOKE_PANIC=1`
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
- **桌面探针 `gdext/desktop_probe.tscn`**：两阶段检查「加载车辆模组」——
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
- `game/smoke.tscn` 无车可测时只验证相机循环，输出 `PASS（部分）`
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
- `game/main.tscn` 的 `TouchOverlay` 没有铺满屏幕（塌缩到原点）：触摸
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
  `game/main.gd` 的 `DEFAULT_CAR_DIR`、`world/assets/audio` 三个真实资产测试的路径、
  gdext 文档注释、README、docs/godot.md、docs/kn5-format.md。
- **新增根级 `AGENTS.md`**：AI 助手仓库导航——目录是什么/不是什么、场景分工约定
  （`gdext/` 探针 vs `game/` 游戏页）、命令速查、提交约定、已知坑清单
  （MCP 导出摘除、`-lc++_shared`、FMOD 不进 APK、`--quit-after` 单位等）。
- **方案文档归档**：`主项目方案.md`（拟真赛车模拟器 v1.0 设计定稿，未入库）移入
  `docs/plans/`；其"自研 sim-* 物理核心"与 roadmap「非目标：不自研完整轮胎模型」
  是当前最大待决策分歧，先整理现状、后议物理路线。
- **docs 刷新**：`architecture.md` 总览图补上 `world/` / `audio/`（原图缺失）并增加
  场景分工表；`roadmap.md` 对齐 0.0.1 实际进度（加载缓存/分帧加载已勾、探针路径
  `godot/`→`gdext/`、相机与 HUD 在 game 已落地）。

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
