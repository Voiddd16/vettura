extends Node
## 全局输入层：键鼠 / 手柄 / 触屏 / 陀螺仪 + 可重绑按键 + 返回栈。
##
## 设计目标：
## - 场景只读 `Input` 动作，不把键位硬编码在逻辑里；设置页可重绑并持久化；
## - 键鼠、手柄、触屏、体感共用同一套动作，手柄热插拔后自动识别；
## - “返回”统一走 `push_nav_handler()`：模态面板先消费，没有面板时场景兜底；
##   支持 Esc / 手柄 Start（菜单）、手柄 B / 鼠标右键 / Android 返回键（返回）。
##
## 动作在运行时注册（`project.godot` 不写死键位），绑定存在
## `user://controls.cfg`。`GameSettings` 负责音量/画面/体感开关等持久化。

signal nav_requested(kind: String)
signal device_changed(device: String)
signal gyro_changed(available: bool, enabled: bool)

const BINDINGS_PATH := "user://controls.cfg"
const BINDINGS_VERSION := 2

## 可显示、可重绑的动作。`category` 只用于分组显示。
const ACTIONS := {
	"game_throttle": {"label": "油门", "category": "驾驶"},
	"game_brake": {"label": "刹车", "category": "驾驶"},
	"game_steer_left": {"label": "左转", "category": "驾驶"},
	"game_steer_right": {"label": "右转", "category": "驾驶"},
	"game_camera_next": {"label": "切换视角", "category": "驾驶"},
	"game_garage": {"label": "车辆库", "category": "驾驶"},
	"game_menu": {"label": "打开设置 / 菜单", "category": "系统"},
	"game_back": {"label": "返回 / 取消", "category": "系统"},
	"game_fullscreen": {"label": "切换全屏", "category": "系统"},
}
const ACTION_ORDER := [
	"game_throttle", "game_brake", "game_steer_left", "game_steer_right",
	"game_camera_next", "game_garage", "game_menu", "game_back", "game_fullscreen",
]

const JOY_BUTTON_LABELS := {
	JOY_BUTTON_A: "A", JOY_BUTTON_B: "B", JOY_BUTTON_X: "X", JOY_BUTTON_Y: "Y",
	JOY_BUTTON_BACK: "Back", JOY_BUTTON_GUIDE: "Guide", JOY_BUTTON_START: "Start",
	JOY_BUTTON_LEFT_STICK: "L3", JOY_BUTTON_RIGHT_STICK: "R3",
	JOY_BUTTON_LEFT_SHOULDER: "LB", JOY_BUTTON_RIGHT_SHOULDER: "RB",
	JOY_BUTTON_DPAD_UP: "↑", JOY_BUTTON_DPAD_DOWN: "↓",
	JOY_BUTTON_DPAD_LEFT: "←", JOY_BUTTON_DPAD_RIGHT: "→",
}

const JOY_AXIS_LABELS := {
	JOY_AXIS_LEFT_X: "左摇杆↔", JOY_AXIS_LEFT_Y: "左摇杆↕",
	JOY_AXIS_RIGHT_X: "右摇杆↔", JOY_AXIS_RIGHT_Y: "右摇杆↕",
	JOY_AXIS_TRIGGER_LEFT: "左扳机", JOY_AXIS_TRIGGER_RIGHT: "右扳机",
}

const MOUSE_LABELS := {
	MOUSE_BUTTON_LEFT: "鼠标左键", MOUSE_BUTTON_RIGHT: "鼠标右键",
	MOUSE_BUTTON_MIDDLE: "鼠标中键", MOUSE_BUTTON_WHEEL_UP: "滚轮上",
	MOUSE_BUTTON_WHEEL_DOWN: "滚轮下",
}

var _bindings: Dictionary = {}
## 返回处理器：后进先出，签名 `func(kind: String) -> bool`；返回 true 表示已消费。
var _nav_handlers: Array[Callable] = []
## 重绑按键时临时屏蔽返回/全屏快捷键，避免“点一下鼠标左键就绑成返回”。
var _capture_input := false

var _device := "keyboard"
var _touch_device := false

## 陀螺仪 / 体感：`gyro_look_delta` 是“本帧应加到相机上的角度”，
## `tilt` 是重力方向换算出的绝对倾斜（-1..1），用于转向。
var gyro_available := false
var gyro_enabled := false
var gyro_look_delta := Vector2.ZERO
var tilt := 0.0

var _gyro_device := -1
var _gyro_bias := Vector3.ZERO
var _gyro_gravity_zero := Vector3.ZERO
var _gyro_last := Vector3.ZERO
var _gyro_sensitivity := 1.0
var _gyro_invert := false
var _gyro_calibrated := false


func _ready() -> void:
	_touch_device = OS.has_feature("mobile") or DisplayServer.is_touchscreen_available()
	_device = "touch" if _touch_device else "keyboard"
	_bindings = _defaults()
	_load_bindings()
	_apply_bindings()
	_ensure_ui_gamepad()
	Input.joy_connection_changed.connect(_on_joy_connection_changed)
	GameSettings.changed.connect(_on_settings_changed)
	_init_gyro()
	set_process(true)


func _process(delta: float) -> void:
	gyro_look_delta = Vector2.ZERO
	tilt = 0.0
	if not gyro_enabled or not gyro_available:
		return
	if not _gyro_calibrated:
		# 传感器刚打开时可能先返回 0；等重力向量有数据后再以自然握持姿态
		# 作为中位，避免一进游戏就被“倾斜”到满舵。
		if _current_gravity().length_squared() > 0.5:
			calibrate_gyro()
			_gyro_calibrated = true
		return

	var raw_gyro := _current_gyro()
	var raw_gravity := _current_gravity()
	# 低通，避免手机传感器噪声把镜头抖成帕金森。
	_gyro_last = _gyro_last.lerp(raw_gyro, clampf(delta * 18.0, 0.0, 1.0))
	var gyro := _gyro_last - _gyro_bias
	var sensitivity := _gyro_sensitivity
	var sense_sign := -1.0 if _gyro_invert else 1.0

	# 角速度积分成“本帧视角增量”：设备左右转 → 相机左右转。
	gyro_look_delta = Vector2(
		gyro.y * sensitivity * 0.55 * delta * sense_sign,
		gyro.x * sensitivity * 0.40 * delta * sense_sign,
	)

	# 重力方向 → 绝对倾斜；校准后设备自然握持角度为 0。
	var relative := raw_gravity - _gyro_gravity_zero
	var raw_tilt := relative.x * 2.2 * sensitivity * sense_sign
	if absf(raw_tilt) < 0.06:
		raw_tilt = 0.0
	elif raw_tilt > 0.06:
		raw_tilt = (raw_tilt - 0.06) / 0.94
	else:
		raw_tilt = (raw_tilt + 0.06) / 0.94
	tilt = clampf(raw_tilt, -1.0, 1.0)


# ---------------------------------------------------------------- 输入动作

func _input(event: InputEvent) -> void:
	_update_device(event)
	if _capture_input:
		return
	if event.is_action_pressed("game_fullscreen"):
		GameSettings.toggle_fullscreen()
		get_viewport().set_input_as_handled()
		return
	if event.is_action_pressed("game_back"):
		dispatch_nav("back")
		get_viewport().set_input_as_handled()
		return
	if event.is_action_pressed("game_menu"):
		dispatch_nav("menu")
		get_viewport().set_input_as_handled()
		return


func _notification(what: int) -> void:
	# Android 系统返回键 / 手势返回。
	if what == NOTIFICATION_WM_GO_BACK_REQUEST:
		dispatch_nav("back")


func dispatch_nav(kind: String) -> void:
	for index in range(_nav_handlers.size() - 1, -1, -1):
		var handler: Callable = _nav_handlers[index]
		if not handler.is_valid():
			_nav_handlers.remove_at(index)
			continue
		if bool(handler.call(kind)):
			return
	nav_requested.emit(kind)


func push_nav_handler(handler: Callable) -> void:
	_nav_handlers.append(handler)


func pop_nav_handler(handler: Callable) -> void:
	for index in range(_nav_handlers.size() - 1, -1, -1):
		if _nav_handlers[index] == handler:
			_nav_handlers.remove_at(index)
			return


func set_capture_input(enabled: bool) -> void:
	_capture_input = enabled


func is_capturing_input() -> bool:
	return _capture_input


func is_bindable_event(event: InputEvent) -> bool:
	return not _event_to_data(event).is_empty()


# ---------------------------------------------------------------- 设备状态

func active_device() -> String:
	return _device


func is_touch_device() -> bool:
	return _touch_device


func has_gamepad() -> bool:
	return not Input.get_connected_joypads().is_empty()


func gamepad_name() -> String:
	var pads := Input.get_connected_joypads()
	if pads.is_empty():
		return ""
	return Input.get_joy_name(pads[0])


func device_label() -> String:
	match _device:
		"gamepad":
			var name := gamepad_name()
			return "手柄：%s" % (name if not name.is_empty() else "已连接")
		"touch":
			return "触屏"
		"mouse":
			return "鼠标 / 键盘"
		_:
			return "键盘 / 鼠标"


func hint_text() -> String:
	match _device:
		"gamepad":
			return "手柄：RT/LT 油门/刹车 · 左摇杆 转向 · Y 视角 · LB 车辆库 · Start 设置 · B 返回 · F11 全屏"
		"touch":
			return "触屏：左半屏刹车 · 右半屏油门 · 底部滑杆/倾斜转向 · 右上角设置"
		_:
			return "键盘：W/S 油门/刹车 · A/D 转向 · C 视角 · G 车辆库 · Esc 设置 · F11 全屏"


func vibrate(duration_s: float = 0.18, weak: float = 0.35, strong: float = 0.55) -> void:
	if not bool(GameSettings.get_value("input", "vibration", true)):
		return
	for device in Input.get_connected_joypads():
		Input.start_joy_vibration(device, weak, strong, duration_s)
	if OS.has_feature("mobile"):
		Input.vibrate_handheld(int(duration_s * 1000.0), clampf(strong, 0.0, 1.0))


func _update_device(event: InputEvent) -> void:
	var next := _device
	if event is InputEventKey or event is InputEventMouseButton or event is InputEventMouseMotion:
		next = "keyboard"
	elif event is InputEventScreenTouch or event is InputEventScreenDrag:
		next = "touch"
	elif event is InputEventJoypadButton:
		next = "gamepad"
	elif event is InputEventJoypadMotion:
		if absf((event as InputEventJoypadMotion).axis_value) > 0.2:
			next = "gamepad"
	if next != _device:
		_device = next
		device_changed.emit(_device)


# ---------------------------------------------------------------- 绑定

func action_label(action: String) -> String:
	return String(ACTIONS.get(action, {}).get("label", action))


func binding_text(action: String) -> String:
	var labels: Array[String] = []
	for data in _bindings.get(action, []):
		labels.append(data_label(data))
	return " / ".join(labels) if not labels.is_empty() else "（未绑定）"


func data_label(data: Dictionary) -> String:
	match String(data.get("type", "")):
		"key":
			var text := ""
			if bool(data.get("ctrl", false)):
				text += "Ctrl+"
			if bool(data.get("alt", false)):
				text += "Alt+"
			if bool(data.get("shift", false)):
				text += "Shift+"
			if bool(data.get("meta", false)):
				text += "Cmd+"
			return text + OS.get_keycode_string(int(data.get("key", KEY_NONE)))
		"mouse":
			return String(MOUSE_LABELS.get(int(data.get("button", 0)), "鼠标"))
		"joy_button":
			return String(JOY_BUTTON_LABELS.get(int(data.get("button", 0)), "手柄键"))
		"joy_axis":
			var label := String(JOY_AXIS_LABELS.get(int(data.get("axis", 0)), "手柄轴"))
			return label + (" +" if float(data.get("value", 1.0)) > 0.0 else " -")
	return ""


## 把下一个按下的键/按钮绑定到动作。返回被顶掉的冲突动作名（无冲突为空）。
func rebind(action: String, event: InputEvent) -> String:
	var data := _event_to_data(event)
	if data.is_empty():
		return ""
	var conflict := ""
	for other in ACTIONS:
		if other == action:
			continue
		var kept: Array = []
		for existing in _bindings.get(other, []):
			if _same_binding(existing, data):
				conflict = other
				continue
			kept.append(existing)
		_bindings[other] = kept

	# 同一设备类别只保留一个：键鼠一组、手柄一组。这样用户重绑键盘时
	# 不会把默认手柄绑定也清掉，反之亦然。
	var target: Array = []
	for existing in _bindings.get(action, []):
		if _device_class(existing) != _device_class(data):
			target.append(existing)
	target.append(data)
	_bindings[action] = target
	_apply_bindings()
	_save_bindings()
	return conflict


func reset_action(action: String) -> void:
	var defaults := _defaults()
	_bindings[action] = defaults.get(action, [])
	_apply_bindings()
	_save_bindings()


func reset_all_bindings() -> void:
	_bindings = _defaults()
	_apply_bindings()
	_save_bindings()


func actions_in_order() -> Array[String]:
	var result: Array[String] = []
	for action in ACTION_ORDER:
		if ACTIONS.has(action):
			result.append(action)
	return result


func _defaults() -> Dictionary:
	return {
		"game_throttle": [
			_key_data(KEY_W), _key_data(KEY_UP),
			_joy_button_data(JOY_BUTTON_A), _joy_axis_data(JOY_AXIS_TRIGGER_RIGHT, 1.0),
			_joy_button_data(JOY_BUTTON_DPAD_UP),
		],
		"game_brake": [
			_key_data(KEY_S), _key_data(KEY_DOWN),
			_joy_axis_data(JOY_AXIS_TRIGGER_LEFT, 1.0),
			_joy_button_data(JOY_BUTTON_DPAD_DOWN),
		],
		"game_steer_left": [
			_key_data(KEY_A), _key_data(KEY_LEFT),
			_joy_axis_data(JOY_AXIS_LEFT_X, -1.0), _joy_button_data(JOY_BUTTON_DPAD_LEFT),
		],
		"game_steer_right": [
			_key_data(KEY_D), _key_data(KEY_RIGHT),
			_joy_axis_data(JOY_AXIS_LEFT_X, 1.0), _joy_button_data(JOY_BUTTON_DPAD_RIGHT),
		],
		"game_camera_next": [_key_data(KEY_C), _joy_button_data(JOY_BUTTON_Y)],
		"game_garage": [_key_data(KEY_G), _joy_button_data(JOY_BUTTON_LEFT_SHOULDER)],
		"game_menu": [_key_data(KEY_ESCAPE), _joy_button_data(JOY_BUTTON_START)],
		"game_back": [_joy_button_data(JOY_BUTTON_B), _mouse_data(MOUSE_BUTTON_RIGHT)],
		"game_fullscreen": [
			_key_data(KEY_F11), _key_data(KEY_ENTER, false, true),
			_key_data(KEY_F, false, false, true, true),
		],
	}


func _key_data(keycode: Key, shift: bool = false, alt: bool = false,
		ctrl: bool = false, meta: bool = false) -> Dictionary:
	return {
		"type": "key", "key": int(keycode),
		"shift": shift, "alt": alt, "ctrl": ctrl, "meta": meta,
	}


func _mouse_data(button: MouseButton) -> Dictionary:
	return {"type": "mouse", "button": int(button)}


func _joy_button_data(button: JoyButton) -> Dictionary:
	return {"type": "joy_button", "button": int(button)}


func _joy_axis_data(axis: JoyAxis, value: float) -> Dictionary:
	return {
		"type": "joy_axis", "axis": int(axis),
		"value": 1.0 if value > 0.0 else -1.0,
	}


func _joy_button(button: JoyButton) -> InputEventJoypadButton:
	var event := InputEventJoypadButton.new()
	event.button_index = button
	event.device = -1
	return event


func _event_to_data(event: InputEvent) -> Dictionary:
	if event is InputEventKey:
		var key := event as InputEventKey
		if not key.pressed or key.echo:
			return {}
		return _event_signature(event)
	if event is InputEventMouseButton:
		var mouse := event as InputEventMouseButton
		if not mouse.pressed:
			return {}
		return _event_signature(event)
	if event is InputEventJoypadButton:
		var button := event as InputEventJoypadButton
		if not button.pressed:
			return {}
		return _event_signature(event)
	if event is InputEventJoypadMotion:
		var axis := event as InputEventJoypadMotion
		if absf(axis.axis_value) < 0.5:
			return {}
		return _event_signature(event)
	return {}


## 不检查 pressed 的签名：用于比较 InputMap 里已有的默认事件。
func _event_signature(event: InputEvent) -> Dictionary:
	if event is InputEventKey:
		var key := event as InputEventKey
		return {
			"type": "key",
			"key": int(key.physical_keycode if key.physical_keycode != KEY_NONE else key.keycode),
			"shift": key.shift_pressed, "alt": key.alt_pressed,
			"ctrl": key.ctrl_pressed, "meta": key.meta_pressed,
		}
	if event is InputEventMouseButton:
		return {"type": "mouse", "button": int((event as InputEventMouseButton).button_index)}
	if event is InputEventJoypadButton:
		return {"type": "joy_button", "button": int((event as InputEventJoypadButton).button_index)}
	if event is InputEventJoypadMotion:
		var axis := event as InputEventJoypadMotion
		return {
			"type": "joy_axis", "axis": int(axis.axis),
			"value": 1.0 if axis.axis_value > 0.0 else -1.0,
		}
	return {}


func _data_to_event(data: Dictionary) -> InputEvent:
	match String(data.get("type", "")):
		"key":
			var key := InputEventKey.new()
			key.physical_keycode = int(data.get("key", KEY_NONE))
			key.shift_pressed = bool(data.get("shift", false))
			key.alt_pressed = bool(data.get("alt", false))
			key.ctrl_pressed = bool(data.get("ctrl", false))
			key.meta_pressed = bool(data.get("meta", false))
			return key
		"mouse":
			var mouse := InputEventMouseButton.new()
			mouse.button_index = int(data.get("button", MOUSE_BUTTON_LEFT))
			return mouse
		"joy_button":
			var button := InputEventJoypadButton.new()
			button.button_index = int(data.get("button", JOY_BUTTON_A))
			button.device = -1
			return button
		"joy_axis":
			var axis := InputEventJoypadMotion.new()
			axis.axis = int(data.get("axis", JOY_AXIS_LEFT_X))
			axis.axis_value = 1.0 if float(data.get("value", 1.0)) > 0.0 else -1.0
			axis.device = -1
			return axis
	return null


func _device_class(data: Dictionary) -> String:
	var type := String(data.get("type", ""))
	return "joy" if type.begins_with("joy") else "kb"


func _same_binding(a: Dictionary, b: Dictionary) -> bool:
	if String(a.get("type", "")) != String(b.get("type", "")):
		return false
	match String(a.get("type", "")):
		"key":
			return int(a.get("key", 0)) == int(b.get("key", 0)) \
				and bool(a.get("shift", false)) == bool(b.get("shift", false)) \
				and bool(a.get("alt", false)) == bool(b.get("alt", false)) \
				and bool(a.get("ctrl", false)) == bool(b.get("ctrl", false)) \
				and bool(a.get("meta", false)) == bool(b.get("meta", false))
		"mouse", "joy_button":
			return int(a.get("button", -1)) == int(b.get("button", -1))
		"joy_axis":
			return int(a.get("axis", -1)) == int(b.get("axis", -1)) \
				and signf(float(a.get("value", 1.0))) == signf(float(b.get("value", 1.0)))
	return false


func _apply_bindings() -> void:
	for action in ACTIONS:
		if not InputMap.has_action(action):
			InputMap.add_action(action)
		InputMap.action_erase_events(action)
		InputMap.action_set_deadzone(action, 0.18)
		for data in _bindings.get(action, []):
			var event := _data_to_event(data)
			if event != null:
				InputMap.action_add_event(action, event)


func _load_bindings() -> void:
	var config := ConfigFile.new()
	if config.load(BINDINGS_PATH) != OK:
		return
	if int(config.get_value("meta", "version", 0)) != BINDINGS_VERSION:
		return
	for action in ACTIONS:
		var raw = config.get_value("bindings", action, null)
		if raw is Array and not raw.is_empty():
			var cleaned: Array = []
			for item in raw:
				if item is Dictionary:
					cleaned.append(item)
			if not cleaned.is_empty():
				_bindings[action] = cleaned


func _save_bindings() -> void:
	var config := ConfigFile.new()
	config.set_value("meta", "version", BINDINGS_VERSION)
	for action in ACTIONS:
		config.set_value("bindings", action, _bindings.get(action, []))
	var error := config.save(BINDINGS_PATH)
	if error != OK:
		push_warning("[game-input] 无法写入 %s（错误码 %d）" % [BINDINGS_PATH, error])


func _ensure_ui_gamepad() -> void:
	# 默认的 ui_* 动作在多数平台已经带手柄映射；这里只补缺失的 A/B，
	# 保证菜单、列表、弹窗都能用手柄操作。
	_add_ui_event_once("ui_accept", _joy_button(JOY_BUTTON_A))
	_add_ui_event_once("ui_cancel", _joy_button(JOY_BUTTON_B))


func _add_ui_event_once(action: String, candidate: InputEvent) -> void:
	if not InputMap.has_action(action):
		return
	var data := _event_signature(candidate)
	for event in InputMap.action_get_events(action):
		if _same_binding(_event_signature(event), data):
			return
	InputMap.action_add_event(action, candidate)


# ---------------------------------------------------------------- 陀螺仪

func _init_gyro() -> void:
	gyro_available = OS.has_feature("mobile")
	_gyro_device = -1
	for device in Input.get_connected_joypads():
		_enable_joy_motion(device)
	_gyro_sensitivity = float(GameSettings.get_value("input", "gyro_sensitivity", 1.0))
	_gyro_invert = bool(GameSettings.get_value("input", "gyro_invert", false))
	gyro_enabled = bool(GameSettings.get_value("input", "gyro_enabled", true)) and gyro_available
	calibrate_gyro()
	_gyro_calibrated = false
	gyro_changed.emit(gyro_available, gyro_enabled)


func _enable_joy_motion(device: int) -> void:
	if Input.has_joy_motion_sensors(device):
		Input.set_joy_motion_sensors_enabled(device, true)
		_gyro_device = device
		gyro_available = true


func _on_joy_connection_changed(device: int, connected: bool) -> void:
	if connected:
		_enable_joy_motion(device)
	else:
		gyro_available = OS.has_feature("mobile")
		for pad in Input.get_connected_joypads():
			_enable_joy_motion(pad)
	if not gyro_available:
		gyro_enabled = false
	gyro_changed.emit(gyro_available, gyro_enabled)


func _on_settings_changed(section: String, key: String, value: Variant) -> void:
	if section != "input":
		return
	match key:
		"gyro_enabled":
			gyro_enabled = bool(value) and gyro_available
			_gyro_calibrated = false
			gyro_changed.emit(gyro_available, gyro_enabled)
		"gyro_sensitivity":
			_gyro_sensitivity = clampf(float(value), 0.2, 3.0)
		"gyro_invert":
			_gyro_invert = bool(value)


func _current_gyro() -> Vector3:
	if OS.has_feature("mobile"):
		return Input.get_gyroscope()
	if _gyro_device >= 0:
		return Input.get_joy_gyroscope(_gyro_device)
	return Vector3.ZERO


func _current_gravity() -> Vector3:
	if OS.has_feature("mobile"):
		return Input.get_gravity()
	if _gyro_device >= 0:
		return Input.get_joy_gravity(_gyro_device)
	return Vector3.ZERO


func calibrate_gyro() -> void:
	_gyro_bias = _current_gyro()
	_gyro_gravity_zero = _current_gravity()
	_gyro_last = _gyro_bias
	tilt = 0.0
	gyro_look_delta = Vector2.ZERO
	_gyro_calibrated = true


func set_gyro_enabled(enabled: bool) -> void:
	gyro_enabled = enabled and gyro_available
	GameSettings.set_value("input", "gyro_enabled", enabled)
	gyro_changed.emit(gyro_available, gyro_enabled)
	if gyro_enabled:
		calibrate_gyro()


func set_gyro_sensitivity(value: float) -> void:
	_gyro_sensitivity = clampf(value, 0.2, 3.0)
	GameSettings.set_value("input", "gyro_sensitivity", _gyro_sensitivity)


func set_gyro_invert(inverted: bool) -> void:
	_gyro_invert = inverted
	GameSettings.set_value("input", "gyro_invert", inverted)


func gyro_sensitivity() -> float:
	return _gyro_sensitivity


func gyro_inverted() -> bool:
	return _gyro_invert
