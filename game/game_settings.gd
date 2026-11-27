extends Node
## 全局运行期设置：音频 / 画面 / 车库展示。
##
## 作为 `project.godot` 的 autoload 存在，窗口启动时读 `user://settings.cfg`，
## 场景只通过 `GameSettings.get_value()` / `set_value()` 读写；任何改动都会
## 立即应用（音量 / 全屏 / 缩放）并发出 `changed`，由当前场景更新自己的灯光、
## 相机等**场景局部**表现。
##
## 设计取舍：
## - 音频与显示设置属于引擎全局状态，放在 autoload 里只应用一次；
## - 阴影开关需要改具体灯光节点，autoload 不遍历场景树，由监听方处理；
## - 滑杆拖动会产生密集写入，`set_value()` 只改内存并发信号，`save()` 在弹窗
##   关闭时落盘，避免每帧写磁盘。

signal changed(section: String, key: String, value: Variant)

const CONFIG_PATH := "user://settings.cfg"
const DEFAULT_GARAGE_MOD := "res://mods/ltk_csr_garage"

## 窗口模式：0 = 窗口，1 = 全屏，2 = 无边框全屏。
const WINDOW_WINDOWED := 0
const WINDOW_FULLSCREEN := 1
const WINDOW_BORDERLESS := 2

## 所有设置项的默认值。新增字段只要加在这里，老配置文件自动补默认值。
const DEFAULTS := {
	"audio": {
		"master_volume": 0.8,
	},
	"video": {
		"window_mode": WINDOW_WINDOWED,
		"window_width": 1280,
		"window_height": 720,
		"fullscreen_adaptive": true,
		"vsync": true,
		"shadows": true,
		"render_scale": 1.0,
		"post_fx": true,
		## 上采样器：auto/native/fsr1/fsr2/fsr3_1/metalfx_spatial/metalfx_temporal。
		## 具体可用性由 autoload `DisplayPipeline` 探测；不可用时自动回退。
		"upscaler": "auto",
		## 可变着色率：auto/off/texture/xr。auto 在 XR 下用 VRS_XR，
		## 桌面 Forward+/Mobile 用中心清晰的纹理。
		"vrs": "auto",
		## 动态分辨率目标帧率：0=自动（最高 60），-1=跟随显示器刷新率，
		## 其他正数按 FPS 解释。
		"dynamic_target_fps": 0,
	},
	"input": {
		# 桌面没有陀螺仪时这个开关自动失效；移动端/带体感的手柄会生效。
		"gyro_enabled": true,
		"gyro_sensitivity": 1.0,
		"gyro_invert": false,
		"vibration": true,
		"touch_steering": true,
	},
	"garage": {
		"mod_path": DEFAULT_GARAGE_MOD,
		"car_path": "",
		"orbit_speed": 0.18,
		"orbit_radius": 7.0,
	},
	"ui": {
		## 空字符串 = 跟随系统；也可显式 "zh_CN" / "en"。
		"language": "",
	},
	"race": {
		## 大厅里选择的赛道与布局；空 = 使用 ModLibrary 自动挑选。
		"track_path": "",
		"track_layout": "",
		## 0=车后追随 1=驾驶位 2=前机盖。
		"camera": 0,
		"abs": true,
		"tc": true,
	},
}

var _values: Dictionary = {}
var _dirty := false
## 窗口模式切换期间忽略 `size_changed`：全屏/无边框会短暂报出屏幕尺寸，
## 不能把它当成用户想要的窗口尺寸存下来。
var _ignore_size_until := 0


func _ready() -> void:
	_values = _default_copy()
	_load()
	_apply_all()
	# 自动加载的 `_ready` 早于主窗口完全就绪；窗口模式延后一帧再设一次，
	# 避免被 project.godot 的默认窗口模式覆盖。
	call_deferred("_apply_window_mode", int(get_value("video", "window_mode", WINDOW_WINDOWED)))
	var root := get_tree().root
	if root != null and not root.size_changed.is_connected(_on_window_size_changed):
		root.size_changed.connect(_on_window_size_changed)


func _exit_tree() -> void:
	save()


## 读一个设置项。缺失时依次回落到默认值、调用方给的 `fallback`。
func get_value(section: String, key: String, fallback: Variant = null) -> Variant:
	if _values.has(section) and _values[section].has(key):
		return _values[section][key]
	if DEFAULTS.has(section) and DEFAULTS[section].has(key):
		return DEFAULTS[section][key]
	return fallback


## 写一个设置项：值不变时静默返回，变了就应用、发信号并标记待落盘。
func set_value(section: String, key: String, value: Variant) -> void:
	if not _values.has(section):
		_values[section] = {}
	if _values[section].has(key) and _values[section][key] == value:
		return
	_values[section][key] = value
	_apply_global(section, key, value)
	_dirty = true
	changed.emit(section, key, value)


## 恢复默认值（逐项 set，场景会收到完整通知）。
##
## `include_content = false` 时保留“当前选了哪台车/哪个车库”，只重置画质、
## 音频与镜头参数——设置弹窗里的「恢复默认」不应该把用户的车库选择也清掉。
func restore_defaults(include_content: bool = false) -> void:
	for section: String in DEFAULTS:
		for key: String in DEFAULTS[section]:
			if not include_content:
				if section == "garage" and key in ["mod_path", "car_path"]:
					continue
				if section == "race" and key in ["track_path", "track_layout"]:
					continue
			set_value(section, key, DEFAULTS[section][key])


## 把待写入的值落盘。失败只警告，不阻断游戏。
func save() -> void:
	if not _dirty:
		return
	var config := ConfigFile.new()
	for section: String in _values:
		for key: String in _values[section]:
			config.set_value(section, key, _values[section][key])
	var error := config.save(CONFIG_PATH)
	if error == OK:
		_dirty = false
	else:
		push_warning("[settings] 无法写入 %s（错误码 %d）" % [CONFIG_PATH, error])


func _load() -> void:
	var config := ConfigFile.new()
	if config.load(CONFIG_PATH) != OK:
		return
	for section: String in _values:
		for key: String in _values[section]:
			# `.get_value()` 的默认值保证老配置文件缺字段时仍然完整。
			var value: Variant = config.get_value(section, key, _values[section][key])
			_values[section][key] = value
	# 旧版本只有 `video/fullscreen` 布尔值；没有 window_mode 时迁移过来。
	if config.has_section_key("video", "fullscreen") \
			and not config.has_section_key("video", "window_mode"):
		_values["video"]["window_mode"] = WINDOW_FULLSCREEN \
			if bool(config.get_value("video", "fullscreen")) else WINDOW_WINDOWED
	# 修一下被全屏切换写坏/超大的窗口尺寸：窗口不应接近整块屏幕，
	# 否则 canvas_items 会按接近全屏的尺寸渲染，帧率会莫名暴跌。
	var screen := DisplayServer.screen_get_size()
	var width := int(_values["video"].get("window_width", 1280))
	var height := int(_values["video"].get("window_height", 720))
	if width < 640 or height < 360 or width > screen.x * 0.9 or height > screen.y * 0.9:
		_values["video"]["window_width"] = 1280
		_values["video"]["window_height"] = 720
		_dirty = true


func _apply_all() -> void:
	for section: String in _values:
		for key: String in _values[section]:
			_apply_global(section, key, _values[section][key])


func _apply_global(section: String, key: String, value: Variant) -> void:
	match section:
		"audio":
			if key == "master_volume":
				_apply_master_volume(float(value))
		"video":
			match key:
				"window_mode":
					_apply_window_mode(int(value))
				"fullscreen":
					_apply_window_mode(WINDOW_FULLSCREEN if bool(value) else WINDOW_WINDOWED)
				"vsync":
					_apply_vsync(bool(value))
				"render_scale":
					_apply_render_scale(float(value))


func set_window_mode(mode: int) -> void:
	set_value("video", "window_mode", clampi(mode, WINDOW_WINDOWED, WINDOW_BORDERLESS))


func window_mode() -> int:
	return clampi(int(get_value("video", "window_mode", WINDOW_WINDOWED)),
			WINDOW_WINDOWED, WINDOW_BORDERLESS)


func is_fullscreen() -> bool:
	return window_mode() != WINDOW_WINDOWED


## F11 / Alt+Enter / 手柄 Start 的全局切换：窗口 ↔ 全屏。
func toggle_fullscreen() -> void:
	if OS.has_feature("mobile"):
		return
	set_window_mode(WINDOW_WINDOWED if is_fullscreen() else WINDOW_FULLSCREEN)
	save()


func _apply_window_mode(mode: int) -> void:
	if DisplayServer.get_name() == "headless":
		return
	# 移动端由系统/项目设置决定，不能用桌面窗口 API。
	if OS.has_feature("mobile"):
		return
	_ignore_size_until = Time.get_ticks_msec() + 700
	match clampi(mode, WINDOW_WINDOWED, WINDOW_BORDERLESS):
		WINDOW_WINDOWED:
			DisplayServer.window_set_flag(DisplayServer.WINDOW_FLAG_BORDERLESS, false)
			DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
			var size := Vector2i(
				maxi(640, int(get_value("video", "window_width", 1280))),
				maxi(360, int(get_value("video", "window_height", 720))))
			var screen := DisplayServer.window_get_current_screen()
			var screen_size := DisplayServer.screen_get_size(screen)
			size.x = mini(size.x, maxi(640, screen_size.x - 40))
			size.y = mini(size.y, maxi(360, screen_size.y - 80))
			DisplayServer.window_set_size(size)
			var position := DisplayServer.screen_get_position(screen) \
				+ (screen_size - size) / 2
			DisplayServer.window_set_position(position)
		WINDOW_FULLSCREEN:
			DisplayServer.window_set_flag(DisplayServer.WINDOW_FLAG_BORDERLESS, false)
			# macOS 的原生全屏会沿用进入前的窗口尺寸；如果窗口还是 1280×720，
			# 全屏实际只放大一张 720p 画面（既糊又没有利用屏幕）。先把窗口
			# 尺寸铺到当前屏幕，再进全屏，确保渲染目标就是屏幕分辨率。
			var screen := DisplayServer.window_get_current_screen()
			DisplayServer.window_set_size(DisplayServer.screen_get_size(screen))
			DisplayServer.window_set_position(DisplayServer.screen_get_position(screen))
			DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)
		WINDOW_BORDERLESS:
			# macOS 的原生全屏本来就没有标题栏；再叠 borderless 会被系统解释成
			# “最大化 + 0x0 内容区”。直接走原生全屏，行为稳定。
			if OS.get_name() == "macOS":
				DisplayServer.window_set_flag(DisplayServer.WINDOW_FLAG_BORDERLESS, false)
				var screen := DisplayServer.window_get_current_screen()
				DisplayServer.window_set_size(DisplayServer.screen_get_size(screen))
				DisplayServer.window_set_position(DisplayServer.screen_get_position(screen))
				DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)
				return
			# 从全屏直接切无边框时，同一帧里“退出全屏 + 去边框”会被 macOS
			# 解释成最大化；先回窗口，下一帧再进无边框。
			var current := DisplayServer.window_get_mode()
			if current == DisplayServer.WINDOW_MODE_FULLSCREEN \
					or current == DisplayServer.WINDOW_MODE_EXCLUSIVE_FULLSCREEN:
				DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
			call_deferred("_enter_borderless")


func _enter_borderless() -> void:
	if DisplayServer.get_name() == "headless" or OS.has_feature("mobile"):
		return
	_ignore_size_until = Time.get_ticks_msec() + 700
	DisplayServer.window_set_flag(DisplayServer.WINDOW_FLAG_BORDERLESS, true)
	var screen := DisplayServer.window_get_current_screen()
	DisplayServer.window_set_position(DisplayServer.screen_get_position(screen))
	DisplayServer.window_set_size(DisplayServer.screen_get_size(screen))


func _on_window_size_changed() -> void:
	if OS.has_feature("mobile") or window_mode() != WINDOW_WINDOWED:
		return
	if Time.get_ticks_msec() < _ignore_size_until:
		return
	var size := DisplayServer.window_get_size()
	if size.x <= 0 or size.y <= 0:
		return
	# 接近整屏的“窗口”通常是全屏/无边框切换的中间态，不存成窗口尺寸。
	var screen := DisplayServer.screen_get_size()
	if size.x > screen.x * 0.9 or size.y > screen.y * 0.9:
		return
	# 直接写值，避免 `resize → set_value → _apply_window_mode → resize` 回环。
	_values["video"]["window_width"] = size.x
	_values["video"]["window_height"] = size.y
	_dirty = true


func _apply_master_volume(value: float) -> void:
	var bus := AudioServer.get_bus_index("Master")
	if bus < 0:
		return
	var volume := clampf(value, 0.0, 1.0)
	AudioServer.set_bus_mute(bus, volume <= 0.001)
	AudioServer.set_bus_volume_db(bus, linear_to_db(maxf(volume, 0.0001)))


func _apply_fullscreen(enabled: bool) -> void:
	_apply_window_mode(WINDOW_FULLSCREEN if enabled else WINDOW_WINDOWED)


func _apply_vsync(enabled: bool) -> void:
	if DisplayServer.get_name() == "headless":
		return
	DisplayServer.window_set_vsync_mode(
			DisplayServer.VSYNC_ENABLED if enabled else DisplayServer.VSYNC_DISABLED)


func _apply_render_scale(value: float) -> void:
	var root := get_tree().root
	if root == null:
		return
	# 只放大不缩小没有意义；下限 50% 保证低端设备仍能看清车辆细节。
	root.scaling_3d_scale = clampf(value, 0.5, 1.0)


func _default_copy() -> Dictionary:
	var copy := {}
	for section: String in DEFAULTS:
		copy[section] = DEFAULTS[section].duplicate()
	return copy
