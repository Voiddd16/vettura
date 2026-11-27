extends Node
## 极简 i18n：中文原文作为 key，英文映射用代码内的字典注册进 TranslationServer。
##
## 为什么用中文原文当 key：
## - 现有 `.tscn` / 动态 Label 全是中文，Control 的 `auto_translate` 默认开启，
##   把翻译注册成 `中文 → English` 后，静态文本不需要改成 `UI_KEY` 就能生效；
## - 中文界面不需要额外维护一份 identity 翻译，代码里 `I18n.t("中文模板")`
##   也可以直接读；
## - 只支持 zh_CN / en，规模小，放在 GDScript 里比 CSV/PO 少一层导入依赖。
##
## 动态文本（带名字/数字、ItemList/OptionButton 条目、对话框）仍必须显式调用
## `I18n.t()`：Godot 的自动翻译只翻译完整 text 属性，不做格式化。

signal language_changed(locale: String)

const SUPPORTED := ["zh_CN", "en"]
const FOLLOW_SYSTEM := ""

## 中文原文 → English。zh_CN 会为每个 key 注册 identity，避免 fallback 到英文。
const EN := {
	# ------------------------------------------------------------ 通用
	"设置": "Settings",
	"音频": "Audio",
	"画面": "Video",
	"输入": "Input",
	"返回": "Back",
	"取消": "Cancel",
	"确定": "OK",
	"完成": "Done",
	"退出": "Quit",
	"删除": "Delete",
	"重命名": "Rename",
	"导入": "Import",
	"全屏": "Fullscreen",
	"大厅": "Lobby",
	"车辆": "Car",
	"赛道": "Track",
	"车库": "Garage",
	"车辆库": "Car Library",
	"车库模组": "Garage Mods",
	"开始驾驶": "Start Driving",
	"选择赛道": "Select Track",
	"展厅预览": "Showroom",
	"返回游戏大厅": "Back to Lobby",
	"返回大厅": "Back to Lobby",
	"重置": "Reset",
	"刹车": "Brake",
	"油门": "Throttle",
	"刹车\n（左半屏触摸）": "Brake\n(left half touch)",
	"油门\n（右半屏触摸）": "Throttle\n(right half touch)",
	"转向": "Steer",
	"下载": "Download",
	"打开目录": "Open Folder",
	"载入预览": "Load Preview",
	"重新扫描": "Rescan",
	"浏览目录…": "Browse…",
	"选择一个赛道": "Choose a track",
	"使用这个赛道": "Use This Track",
	"选择一个车库模组": "Choose a garage mod",
	"使用这个车库": "Use This Garage",
	"选择一台车模": "Select a car",
	"搜索车模名称…": "Search cars…",
	"无": "No",
	"有": "Yes",
	"未选择": "Not selected",
	"未加载": "Not loaded",
	"未知": "Unknown",
	"（未加载）": "(not loaded)",
	"（未选择车辆）": "(No car selected)",
	"（未绑定）": "(Unbound)",
	"默认布局": "Default Layout",
	"自动挡": "Auto",
	"无 ABS": "No ABS",
	"无 TC": "No TC",
	"ABS": "ABS",
	"TC": "TC",

	# ------------------------------------------------------------ 大厅
	"GARAGE  /  游戏大厅": "GARAGE  /  LOBBY",
	"车辆：（未加载）\n车库：（未加载）": "Car: (not loaded)\nGarage: (not loaded)",
	"正在准备车库…": "Preparing garage…",
	"正在准备赛道…": "Preparing track…",
	"正在加载车库…": "Loading garage…",
	"正在加载车辆…": "Loading car…",
	"正在加载车库：%s": "Loading garage: %s",
	"车库模组无效：%s": "Invalid garage mod: %s",
	"车库模组无效，请重新选择。": "Invalid garage mod. Please choose another.",
	"车库模型没有加载成功：%s": "Garage model failed to load: %s",
	"车库模型没有加载成功，请换一个车库模组。": "Garage model failed to load. Please choose another garage mod.",
	"GDExtension 未注册，无法加载赛道。": "GDExtension is not registered; cannot load the track.",
	"GDExtension 未注册，无法加载车库模型。": "GDExtension is not registered; cannot load garage models.",
	"GDExtension 未注册，无法加载车库。": "GDExtension is not registered; cannot load the garage.",
	"GDExtension 未注册，无法加载车辆。": "GDExtension is not registered; cannot load the car.",
	"AcCar.open 失败：%s": "AcCar.open failed: %s",
	"车目录不存在：%s": "Car directory not found: %s",
	"车辆加载失败：目录不存在。": "Car failed to load: directory not found.",
	"车辆数据打不开，请重新选择车辆。": "Cannot open car data. Please choose another car.",
	"车模主 KN5 缺失。": "Car main .kn5 is missing.",
	"车模缺少主 KN5，请重新选择车辆。": "Car has no main .kn5. Please choose another car.",
	"车模 KN5 打不开：%s": "Cannot open car .kn5: %s",
	"车模 KN5 打不开，请重新选择车辆。": "Cannot open car .kn5. Please choose another car.",
	"车模没有创建节点。": "Car model created no nodes.",
	"车模没有创建节点，请重新选择车辆。": "Car model created no nodes. Please choose another car.",
	"没有可用车库。仍可打开设置或选择其它车库模组。": "No usable garage. You can still open Settings or pick another garage mod.",
	"没有可用车库，请点「车库模组」选择。": "No usable garage. Click Garage Mods to choose one.",
	"请先选择一条赛道。": "Please choose a track first.",
	"请先选择一辆车。": "Please choose a car first.",
	"先选择车辆": "Choose a car first",
	"先选择赛道": "Choose a track first",
	"车辆：%s": "Car: %s",
	"赛道：%s": "Track: %s",
	"车库：%s": "Garage: %s",
	"模组：%d 辆车 · %d 条赛道 · %d 个展示车库":
			"Mods: %d cars · %d tracks · %d showroom(s)",
	"拖动画面旋转镜头 · 滚轮缩放 · 车库与车辆都可以随时替换":
			"Drag to orbit · Wheel to zoom · Change garage and car anytime",
	"拖动画面旋转镜头 · 滚轮/双指缩放 · 车库与车辆都可以随时替换":
			"Drag to orbit · Wheel/pinch to zoom · Change garage and car anytime",
	"拖动画面旋转镜头 · 滚轮缩放 · Enter 确认 · Esc 设置 · F11 全屏":
			"Drag to orbit · Wheel to zoom · Enter confirm · Esc settings · F11 fullscreen",
	"手柄：左摇杆旋转 · 右摇杆/体感微调 · A 确认 · Start 设置 · B 返回":
			"Gamepad: Left stick orbit · Right stick/motion fine-tune · A confirm · Start settings · B back",
	"退出 Vettura": "Quit Vettura",
	"确定要退出游戏吗？": "Quit the game?",
	"继续玩": "Keep Playing",

	# ------------------------------------------------------------ 驾驶页
	"本圈 00:00.000": "Lap 00:00.000",
	"上一圈 --    最佳 --": "Last --    Best --",
	"视角：车后": "View: Chase",
	"视角：驾驶位": "View: Cockpit",
	"视角：前机盖": "View: Hood",
	"视角：绕车": "View: Orbit",
	"视角：%s": "View: %s",
	"车后": "Chase",
	"绕车": "Orbit",
	"触屏转向：%s": "Touch steering: %s",
	"第 %d 圈 · 本圈 %s": "Lap %d · Current %s",
	"分段 %d/%d    ": "Sector %d/%d    ",
	"%s上一圈 %s    最佳 %s": "%sLast %s    Best %s",
	"ABS · TC · 自动挡": "ABS · TC · Auto",
	"W/S 油门刹车 · A/D 转向 · C 视角 · R 重置 · Esc 设置":
			"W/S throttle/brake · A/D steer · C view · R reset · Esc settings",
	"赛道：%s%s": "Track: %s%s",
	"加载赛道：%s…": "Loading track: %s…",
	"赛道加载失败：%s": "Track failed to load: %s",
	"生成车辆…": "Spawning car…",
	"车辆数据打不开：%s": "Cannot open car data: %s",
	"车辆生成失败：%s": "Failed to spawn car: %s",
	"没有可用赛道。请把赛道放进 mods/ 或 user://mods/。":
			"No usable track. Put a track into mods/ or user://mods/.",
	"没有可用车辆。请先在大厅车辆库选择一辆车。":
			"No usable car. Choose one in the lobby car library first.",
	"底部滑杆转向；车身倾斜也可转向。":
			"Bottom slider steers; tilting the device also steers.",
	"没有可用车库。": "No usable garage.",

	# ------------------------------------------------------------ 车模库
	"还没有车模。点「导入」选一个目录或 zip。":
			"No cars yet. Click Import and choose a folder or zip.",
	"名称：%s": "Name: %s",
	"目录：%s": "Folder: %s",
	"路径：%s": "Path: %s",
	"大小：%s": "Size: %s",
	"网格：%d 个 .kn5": "Meshes: %d .kn5",
	"物理数据：%s": "Physics data: %s",
	"缺失（只能看不能开）": "missing (view only)",
	"（未计算）": "(not calculated)",
	"选择车模目录或 zip": "Choose a car folder or zip",
	"解压失败：%s": "Extraction failed: %s",
	"不是车模：%s": "Not a car mod: %s",
	"同名车模已存在：%s": "A car with that name already exists: %s",
	"重命名车模": "Rename Car",
	"名字不能为空或含路径分隔符": "Name cannot be empty or contain path separators.",
	"已存在同名车模": "A car with that name already exists.",
	"重命名失败：%d": "Rename failed: %d",
	"删除车模": "Delete Car",
	"确定删除「%s」？这会从磁盘上抹掉该目录。\n%s":
			"Delete \"%s\"? This removes the folder from disk.\n%s",
	"⚠ ": "⚠ ",

	# ------------------------------------------------------------ 车库/赛道选择
	"类型：%s": "Type: %s",
	"视觉模型：%d 个 KN5": "Visual models: %d KN5",
	"单模型车库。": "Single-model garage.",
	"包含 models.ini，按放置表拼装。": "Contains models.ini; assembled from its placement table.",
	"不是有效的车库模组：%s\n需要直接包含 .kn5 或 models.ini。":
			"Invalid garage mod: %s\nIt must directly contain .kn5 or models.ini.",
	"没有找到可用于大厅的 mod。\n把档案放进 mods/ 或 user://mods/ 后点「重新扫描」。":
			"No mods usable by the lobby.\nPut files into mods/ or user://mods/ and click Rescan.",
	"选择车库模组目录": "Choose Garage Mod Folder",
	"布局：%s": "Layout: %s",
	"AI 线：%s": "AI line: %s",
	"有（可计时）": "yes (lap timing)",
	"维修区：%s": "Pit lane: %s",
	"没有找到赛道 mod。\n把赛道放进 mods/ 或 user://mods/ 后点「重新扫描」。":
			"No track mods found.\nPut tracks into mods/ or user://mods/ and click Rescan.",
	"递归扫描 mods/ 与 user://mods/，自动识别赛道和车库 mod。":
			"Recursively scans mods/ and user://mods/; auto-detects track and garage mods.",
	"递归扫描 mods/；同一个赛道的每个 models_*.ini 布局单独列出。":
			"Recursively scans mods/; each models_*.ini layout is listed separately.",

	# ------------------------------------------------------------ 输入
	"左转": "Steer Left",
	"右转": "Steer Right",
	"切换视角": "Change View",
	"重置车辆": "Reset Car",
	"打开设置 / 菜单": "Open Settings / Menu",
	"返回 / 取消": "Back / Cancel",
	"切换全屏": "Toggle Fullscreen",
	"驾驶": "Driving",
	"系统": "System",
	"手柄：%s": "Gamepad: %s",
	"已连接": "Connected",
	"触屏": "Touch",
	"鼠标 / 键盘": "Mouse / Keyboard",
	"键盘 / 鼠标": "Keyboard / Mouse",
	"手柄：RT/LT 油门/刹车 · 左摇杆 转向 · Y 视角 · RB 重置车辆 · LB 车辆库 · Start 设置 · B 返回 · F11 全屏":
			"Gamepad: RT/LT throttle/brake · Left stick steer · Y view · RB reset car · LB car library · Start settings · B back · F11 fullscreen",
	"触屏：左半屏刹车 · 右半屏油门 · 底部滑杆/倾斜转向 · 右上角设置":
			"Touch: left half brake · right half throttle · bottom slider/tilt steer · top-right settings",
	"键盘：W/S 油门/刹车 · A/D 转向 · C 视角 · R 重置车辆 · G 车辆库 · Esc 设置 · F11 全屏":
			"Keyboard: W/S throttle/brake · A/D steer · C view · R reset car · G car library · Esc settings · F11 fullscreen",
	"鼠标": "Mouse",
	"手柄键": "Gamepad Button",
	"手柄轴": "Gamepad Axis",
	"左摇杆↔": "Left Stick ↔",
	"左摇杆↕": "Left Stick ↕",
	"右摇杆↔": "Right Stick ↔",
	"右摇杆↕": "Right Stick ↕",
	"左扳机": "Left Trigger",
	"右扳机": "Right Trigger",
	"鼠标左键": "Left Mouse",
	"鼠标右键": "Right Mouse",
	"鼠标中键": "Middle Mouse",
	"滚轮上": "Wheel Up",
	"滚轮下": "Wheel Down",
	"输入设备": "Input Device",
	"手柄震动 / 触屏振动": "Gamepad Vibration / Touch Haptics",
	"显示屏幕转向滑杆（触屏）": "Show On-screen Steering Slider (Touch)",
	"陀螺仪 / 体感": "Gyroscope / Motion",
	"启用倾斜转向与体感视角": "Enable Tilt Steering and Motion View",
	"灵敏度": "Sensitivity",
	"反转倾斜方向": "Invert Tilt",
	"校准陀螺仪（自然握持后点一下）": "Calibrate Gyro (hold naturally, then tap)",
	"陀螺仪已校准：当前姿态作为中位。": "Gyro calibrated: current pose is now neutral.",
	"按键绑定": "Key Bindings",
	"点右侧按钮后按下新键/手柄按钮。键鼠与手柄各保留一个绑定。":
			"Click a button, then press a new key/gamepad button. Keyboard/mouse and gamepad keep one binding each.",
	"尚未重绑。": "Not rebound yet.",
	"已绑定：%s → %s": "Bound: %s → %s",
	"已绑定：%s → %s（与「%s」冲突，已从对方移除）":
			"Bound: %s → %s (conflicted with \"%s\"; removed from the other action)",
	"已取消重绑。": "Rebind cancelled.",
	"已重置：%s": "Reset: %s",
	"已恢复默认键位。": "Default bindings restored.",
	"全部恢复默认键位": "Reset All Default Bindings",
	"请按下新键/手柄按钮…（Esc 取消）": "Press a new key/gamepad button… (Esc to cancel)",
	"请按下新键…": "Press a new key…",
	"当前设备没有可用的陀螺仪/体感传感器；桌面默认无。接入手柄的体感传感器会自动识别。":
			"No gyro/motion sensor available on this device. Gamepad motion is detected automatically.",
	"已启用。倾斜设备转向；体感视角在驾驶页生效。设备方向不顺手时可点下方校准。":
			"Enabled. Tilt to steer; motion view works on the drive page. Calibrate below if needed.",
	"传感器可用，但当前关闭。": "Sensor available, currently off.",

	# ------------------------------------------------------------ 设置
	"主音量影响游戏内所有声音（引擎、环境、界面）。":
			"Master volume affects all in-game sound (engine, environment, UI).",
	"主音量": "Master Volume",
	"无边框全屏（隐藏标题栏）": "Borderless Fullscreen (hide title bar)",
	"全屏自动分辨率（推荐，MetalFX/FSR 上采样）":
			"Adaptive Fullscreen Resolution (recommended, MetalFX/FSR upscaling)",
	"全屏自动：按帧率在 35%~100% 之间调整 3D 渲染分辨率。":
			"Adaptive fullscreen: adjusts 3D render resolution between 35%–100% by FPS.",
	"F11 / Alt+Enter 可在游戏中随时切换全屏。macOS 上“无边框全屏”等同原生全屏；移动端由系统全屏策略接管。":
			"F11 / Alt+Enter toggles fullscreen anytime. On macOS, Borderless Fullscreen is native fullscreen; mobile uses the system fullscreen policy.",
	"垂直同步（减少画面撕裂）": "V-Sync (reduce tearing)",
	"实时阴影": "Real-time Shadows",
	"3D 分辨率": "3D Resolution",
	"低端设备可以把 3D 分辨率降到 70% 左右，UI 仍保持清晰。":
			"Lower-end devices can drop 3D resolution to around 70%; UI stays sharp.",
	"上采样": "Upscaling",
	"可变着色率": "Variable Rate Shading",
	"动态目标": "Dynamic Target",
	"上采样 / 动态分辨率 / VRS 由 DisplayPipeline 统一应用。":
			"Upscaling / dynamic resolution / VRS are applied by DisplayPipeline.",
	"环绕速度": "Orbit Speed",
	"镜头距离": "Camera Distance",
	"当前车库模组：": "Current garage mod:",
	"当前车库模组：\n%s": "Current garage mod:\n%s",
	"驾驶辅助与默认视角；进入赛道后仍可随时按 C / 视角按钮切换。":
			"Driving assists and default view. You can still change view with C / the view button on track.",
	"ABS 防抱死": "ABS",
	"TC 牵引力控制": "Traction Control",
	"默认视角": "Default View",
	"恢复默认": "Reset Defaults",
	"语言": "Language",
	"界面": "Interface",
	"跟随系统": "Follow System",
	"简体中文": "Simplified Chinese",
	"English": "English",
	"语言切换立即生效；设置会保存到 user://settings.cfg。":
			"Language changes apply immediately and are saved to user://settings.cfg.",
	"车后追随": "Chase",
	"驾驶位": "Cockpit",
	"前机盖": "Hood",
	"当前引擎/平台不可用；选中时运行时会自动回退。":
			"Unavailable on this engine/platform; falls back automatically at runtime.",
	"当前后端/硬件不支持。": "Not supported by the current backend/hardware.",
	"自动（最高 60 FPS）": "Auto (up to 60 FPS)",
	"跟随显示器（%d Hz）": "Follow Display (%d Hz)",
	"移动端：3D 缩放由雨/画质档位管理；上采样 %s。":
			"Mobile: 3D scaling is managed by rain/quality; upscaling %s.",
	"窗口模式：3D 分辨率 %d%%，上采样 %s；全屏可启用动态分辨率。":
			"Windowed: 3D resolution %d%%, upscaling %s; fullscreen can enable dynamic resolution.",
	"全屏固定：3D 分辨率 %d%%，上采样 %s。":
			"Fullscreen fixed: 3D resolution %d%%, upscaling %s.",
	"全屏动态：当前 3D %d%%（%s），目标 %d FPS，范围 %d%%~%d%%。":
			"Fullscreen dynamic: current 3D %d%% (%s), target %d FPS, range %d%%–%d%%.",
	"后端 %s / %s": "Backend %s / %s",
	"引擎扩展 %d": "Engine mods %d",
	"VRS 纹理不可用": "VRS texture unavailable",
	"请求 %s → 回退 %s": "Requested %s → fallback %s",
	"%s。FSR 3.1 需要魔改引擎，见 engine/README.md。":
			"%s. FSR 3.1 requires the patched engine; see engine/README.md.",

	# ------------------------------------------------------------ 上采样 lab
	"自动（推荐）": "Auto (Recommended)",
	"原生（双线性）": "Native (Bilinear)",
	"AMD FSR 3.1（引擎扩展）": "AMD FSR 3.1 (Engine Mod)",
	"MetalFX 空间": "MetalFX Spatial",
	"MetalFX 时域": "MetalFX Temporal",
	"自动": "Auto",
	"关闭": "Off",
	"纹理（中心清晰，边缘降采样）": "Texture (sharp center, downsampled edges)",
	"XR 接口纹理": "XR Interface Texture",
	"原生": "Native",

	# ------------------------------------------------------------ 主页面/触摸
	"车辆：加载中…": "Car: loading…",
	"导入车模（目录 / zip）": "Import Car (folder / zip)",
	"设置（Esc）": "Settings (Esc)",
	"全屏（F11）": "Fullscreen (F11)",
	"滑杆 + 倾斜": "slider + tilt",
	"底部滑杆": "bottom slider",
	"转向（触摸滑杆）": "Steering (touch slider)",
	"油门：%d%%    刹车：%d%%    转向：%+d%%":
			"Throttle: %d%%    Brake: %d%%    Steer: %+d%%",
	"视角：%s（按钮 / C / 手柄 Y）": "View: %s (button / C / gamepad Y)",
	"车轮角速度：%.1f rad/s": "Wheel angular speed: %.1f rad/s",
	"车辆：%s\n油门：%d%%    刹车：%d%%    转向：%+d%%\n视角：%s（按钮 / C / 手柄 Y）\n车轮角速度：%.1f rad/s\n%s":
			"Car: %s\nThrottle: %d%%    Brake: %d%%    Steer: %+d%%\nView: %s (button / C / gamepad Y)\nWheel angular speed: %.1f rad/s\n%s",
	"触屏转向：": "Touch steering: ",
	"（把车放进 mods/ 或 user://mods/，或用左上角「导入车模」直接选一个）":
			"(Put a car into mods/ or user://mods/, or use Import Car in the top-left.)",

	# ------------------------------------------------------------ 其他
	"车模库": "Car Library",
}

var _locale := "zh_CN"
var _zh: Translation
var _en: Translation


func _ready() -> void:
	_build_translations()
	# 已为每个 key 注册 zh_CN identity，不需要额外 fallback locale。
	_apply(String(GameSettings.get_value("ui", "language", FOLLOW_SYSTEM)), false)
	GameSettings.changed.connect(_on_setting_changed)


func _on_setting_changed(section: String, key: String, value: Variant) -> void:
	if section == "ui" and key == "language":
		_apply(String(value), true)


func _build_translations() -> void:
	_zh = Translation.new()
	_zh.locale = "zh_CN"
	_en = Translation.new()
	_en.locale = "en"
	for key: String in EN:
		_zh.add_message(key, key)
		_en.add_message(key, EN[key])
	TranslationServer.add_translation(_zh)
	TranslationServer.add_translation(_en)


func _apply(setting: String, announce: bool) -> void:
	_locale = resolve_locale(setting)
	TranslationServer.set_locale(_locale)
	if announce:
		language_changed.emit(_locale)


static func resolve_locale(setting: String) -> String:
	if SUPPORTED.has(setting):
		return setting
	# 空字符串 = 跟随系统；目前只区分中文环境，其它语言回退英文。
	var system := OS.get_locale_language().to_lower()
	return "zh_CN" if system.begins_with("zh") else "en"


func set_language(setting: String) -> void:
	GameSettings.set_value("ui", "language", setting)
	GameSettings.save()


func language() -> String:
	return _locale


func language_setting() -> String:
	return String(GameSettings.get_value("ui", "language", FOLLOW_SYSTEM))


func language_options() -> Array:
	return [
		{"id": FOLLOW_SYSTEM, "label": t("跟随系统")},
		{"id": "zh_CN", "label": t("简体中文")},
		{"id": "en", "label": "English"},
	]


## 翻译 + 可选格式化。`args` 为空时等价于 `tr(key)`。
func t(key: String, args: Array = []) -> String:
	var text := tr(key)
	if args.is_empty():
		return text
	return text % args
