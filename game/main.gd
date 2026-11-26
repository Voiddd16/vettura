extends Node3D
## 车辆预览 / 驾驶页：**默认车辆模型浮空预览**。
##
## 这一页只做视觉与交互，不接入赛道物理：
##
## - `AcCar` 打开默认 AC 车目录并实例化主模型（`vettura-native` 负责）；
## - 模型挂在 `CarPivot` 下、抬高，形成“浮空状态”，并缓慢自转方便观察；
## - 左半屏触摸 = 刹车，右半屏触摸 = 油门（真实多点触摸；桌面上点击/拖拽也生效）；
## - 桌面键盘：`W`/`↑` 油门、`S`/`↓` 刹车、`C` 循环切换视角；
## - 右上角按钮循环切换相机视角：车后 / 驾驶位 / 绕车 / 前机盖；
## - 油门/刹车会体现在车轮滚动与左上角 HUD 上，方便确认输入真的进来了。

const DEFAULT_CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"

## 车模库存放目录（`user://` 可写；res:// 在运行时是只读的）。由
## `game/garage.gd` 的车模库面板管理：导入 / 选择 / 重命名 / 删除 / 搜索。
const IMPORT_ROOT := "user://imported_cars/"

## 模型基准点不在地面上时，用这个高度把整车托起来。
const FLOAT_HEIGHT := 1.2
## 浮空展示时的自转速度（rad/s）。
const FLOATING_TURN_SPEED := 0.35
## HUD 文本刷新频率。车轮角速度/输入状态以 10 Hz 更新已经比人眼读数快，
## 不必每帧做字符串拼接并重设 Label（每帧都会触发布局与重绘）。
const HUD_UPDATE_INTERVAL_S := 0.1

## 车轮滚动/刹车衰减速度（rad/s、rad/s² 的等效输入都在 `_process` 里折算）。
const WHEEL_SPIN_SPEED := 14.0
const WHEEL_BRAKE_DECEL := 30.0
const WHEEL_COAST_DECEL := 6.0

## 相机局部锚点：`+X` 右、`+Y` 上、`-Z` 前（与 Godot/物理帧一致）。
const DRIVER_LOCAL := Vector3(0.0, 0.45, -0.10)
const HOOD_LOCAL := Vector3(0.0, 0.32, -0.80)
const BEHIND_LOCAL := Vector3(0.0, 1.60, 3.40)
const ORBIT_TARGET_LOCAL := Vector3(0.0, 0.35, 0.0)
const ORBIT_RADIUS := 5.0
const ORBIT_HEIGHT := 1.6
const ORBIT_SPEED := 0.6

const MODE_TEXTS := ["视角：车后", "视角：驾驶位", "视角：绕车", "视角：前机盖"]

## 车库 / 设置面板的 CanvasLayer 层级：HUD / TouchOverlay 各占一层（默认 1），
## 弹层要盖在它们上面。设置再盖住车库。
const GARAGE_LAYER := 2
const SETTINGS_LAYER := 3

const SafeArea := preload("res://game/safe_area.gd")

@onready var _car_pivot: Node3D = $CarPivot
@onready var _camera: Camera3D = $Camera3D
@onready var _mode_button: Button = $HUD/Panel/Columns/ModeButton
@onready var _info_label: Label = $HUD/Panel/Columns/Column/InfoLabel
@onready var _import_button: Button = $HUD/Panel/Columns/Column/ImportButton
@onready var _settings_button: Button = $HUD/Panel/Columns/Column/SettingsButton
@onready var _fullscreen_button: Button = $HUD/Panel/Columns/Column/FullscreenButton
@onready var _lobby_button: Button = $HUD/Panel/Columns/Column/LobbyButton
@onready var _hud_panel: MarginContainer = $HUD/Panel
@onready var _hud: CanvasLayer = $HUD
@onready var _touch_overlay: Control = $CanvasLayer/TouchOverlay
@onready var _sun: DirectionalLight3D = $Sun

## 打开中的车模库（连着它的 CanvasLayer），null = 没开。
var _garage_layer: CanvasLayer = null
## 打开中的设置弹窗（连着它的 CanvasLayer），null = 没开。
var _settings_layer: CanvasLayer = null

var _car_root: Node3D
var _wheels: Array[Node3D] = []
var _wheel_rest: Array[Transform3D] = []
var _car_name := "（未加载）"

## 模拟量输入：键盘/手柄/触摸/陀螺仪最终都折算成 0..1 的油门/刹车和
## -1..1 的转向，HUD 与车轮动画都读这三个值。
var _throttle := 0.0
var _brake := 0.0
var _steer := 0.0
var _steer_target := 0.0
var _touch_throttle := 0.0
var _touch_brake := 0.0
var _touch_steer := 0.0
var _wheel_spin := 0.0
var _spin_angle := 0.0

var _camera_mode := 0
## 绕车模式的角度，需要跨帧累积，所以放这里而不是 `update_camera()` 的局部量。
var _orbit_angle := 0.0
var _look_offset := Vector2.ZERO
var _hud_timer := 0.0
var _touch_layer: CanvasLayer
var _touch_slider: HSlider
var _device_hint := ""

const MAX_STEER_ANGLE := deg_to_rad(24.0)
const LOOK_YAW_LIMIT := 1.2
const LOOK_PITCH_LIMIT := 0.55

## 车模是否加载成功（`test/` 资源缺失时为 `false`）。
func has_car() -> bool:
	return _car_root != null

func _ready() -> void:
	_settings_button.pressed.connect(_on_settings_button_pressed)
	_lobby_button.pressed.connect(_on_lobby_button_pressed)
	GameSettings.changed.connect(_on_setting_changed)
	GameInput.push_nav_handler(_handle_nav)
	GameInput.device_changed.connect(_on_device_changed)
	GameInput.gyro_changed.connect(_on_gyro_changed)
	# 触屏踏板只在触屏设备显示；桌面上隐藏后鼠标左键拖动才能转到 3D 相机，
	# 不会被全屏触摸按钮吃掉。键鼠玩家用 W/S/A/D 或手柄驾驶。
	_touch_overlay.visible = GameInput.is_touch_device()
	# 驾驶页没有需要手柄焦点操作的 HUD：把焦点留给“油门/刹车”，否则手柄 A
	# 会去按按钮而不是给油。
	for button in [_mode_button, _import_button, _settings_button, _fullscreen_button, _lobby_button]:
		button.focus_mode = Control.FOCUS_NONE
	for child in _touch_overlay.get_children():
		_set_focus_none(child)
	_apply_shadows()
	_load_default_car()
	_update_device_hint()
	_build_touch_controls()
	_update_hud()
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)


func _exit_tree() -> void:
	GameInput.pop_nav_handler(_handle_nav)


func _set_focus_none(node: Node) -> void:
	if node is Control:
		(node as Control).focus_mode = Control.FOCUS_NONE
	for child in node.get_children():
		_set_focus_none(child)


## 手机旋转 / 刘海位置 / 分屏变化时都会触发；容器边距兜住安全区，
## 控件（HUD 按钮与信息）永远不会被屏幕边框或挖孔挡住。
func _apply_safe_area() -> void:
	SafeArea.apply(_hud_panel)


func _process(delta: float) -> void:
	_update_input_state(delta)
	_car_pivot.rotate_y(FLOATING_TURN_SPEED * delta)
	_update_wheels(delta)
	_update_camera(delta)
	_hud_timer -= delta
	if _hud_timer <= 0.0:
		_hud_timer = HUD_UPDATE_INTERVAL_S
		_update_hud()


## 统一输入：动作/模拟量来自键盘、手柄、触摸滑杆与陀螺仪。
func _update_input_state(delta: float) -> void:
	_throttle = maxf(Input.get_action_strength("game_throttle"), _touch_throttle)
	_brake = maxf(Input.get_action_strength("game_brake"), _touch_brake)

	var steer := Input.get_axis("game_steer_left", "game_steer_right")
	steer += _touch_steer + GameInput.tilt
	_steer_target = clampf(steer, -1.0, 1.0)
	_steer = lerpf(_steer, _steer_target, 1.0 - exp(-9.0 * delta))

	# 右摇杆 / 陀螺仪给相机自由视角；鼠标拖动在 `_unhandled_input` 里叠加。
	var pads := Input.get_connected_joypads()
	if not pads.is_empty():
		var look := Vector2(
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_X),
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_Y))
		if look.length() > 0.18:
			_look_offset += look * delta * 1.6
	_look_offset += GameInput.gyro_look_delta
	_look_offset.x = clampf(_look_offset.x, -LOOK_YAW_LIMIT, LOOK_YAW_LIMIT)
	_look_offset.y = clampf(_look_offset.y, -LOOK_PITCH_LIMIT, LOOK_PITCH_LIMIT)


func _on_device_changed(_device: String) -> void:
	_update_device_hint()
	_update_hud()


func _on_gyro_changed(_available: bool, _enabled: bool) -> void:
	_update_device_hint()
	_update_hud()


func _update_device_hint() -> void:
	_device_hint = GameInput.hint_text()
	if GameInput.is_touch_device():
		_device_hint += "\n触屏转向：" + ("滑杆 + 倾斜" if GameInput.gyro_enabled else "底部滑杆")

## 打开默认车目录并把主模型实例化进 `CarPivot`。
func _load_default_car() -> void:
	# 大厅里选过的车优先；没有或目录已被删掉时回落默认车，最后用 mods 库里
	# 自动发现的第一辆车。
	var preferred := String(GameSettings.get_value("garage", "car_path", ""))
	var path := DEFAULT_CAR_DIR
	if not preferred.is_empty() and DirAccess.dir_exists_absolute(preferred):
		path = preferred
	elif not DirAccess.dir_exists_absolute(DEFAULT_CAR_DIR):
		var cars := ModLibrary.cars()
		if not cars.is_empty():
			path = cars[0].path

	# `test/` 是本地大体积资源，不入库（见 README）。缺资产时降级成警告并跳过
	# 加载：预览页、其它冒烟场景和 `--install-android-build-template` 之类的
	# 工程级命令都不该因为一台没下载的车把控制台刷成一片 ERROR。
	if not DirAccess.dir_exists_absolute(path):
		push_warning("[visual] 默认车目录不存在，跳过模型加载：%s\n" % path
				+ "（把车放进 mods/ 或 user://mods/，或用左上角「导入车模」直接选一个）")
		_update_hud()
		return

	# GDExtension 还没注册（例如首次打开、缓存未生成）时也不刷错误。
	if not ClassDB.class_exists("AcCar"):
		push_warning("[visual] GDExtension 未注册（AcCar 不存在），跳过模型加载")
		_update_hud()
		return

	_load_car(path)

## 从任意路径加载车模（`res://` 之外的绝对路径也可以）。
func _load_car(path: String) -> bool:
	var car := AcCar.new()
	if not car.open(path):
		push_error("[visual] AcCar.open failed: %s" % path)
		return false

	var root := car.instantiate(_car_pivot)
	if root == null:
		push_error("[visual] AcCar.instantiate failed: %s" % path)
		return false

	# 换车：先把上一台清干净，否则多个车模会叠在同一个 pivot 下。
	if _car_root != null:
		_car_root.queue_free()
	_car_root = root
	_car_name = car.display_name()
	_car_pivot.position.y = FLOAT_HEIGHT
	_wheels.clear()
	_wheel_rest.clear()
	_wheel_spin = 0.0
	_spin_angle = 0.0
	_touch_steer = 0.0
	_look_offset = Vector2.ZERO

	# KN5 的 `WHEEL_*` dummy 在 `AcCar.instantiate()` 后仍按原名保留。
	var wheel_names := ["WHEEL_LF", "WHEEL_RF", "WHEEL_LR", "WHEEL_RR"]
	for wheel_name in wheel_names:
		var wheel := _car_root.find_child(wheel_name, true, false) as Node3D
		if wheel == null:
			push_warning("[visual] wheel dummy not found: %s" % wheel_name)
			continue
		_wheels.append(wheel)
		_wheel_rest.append(wheel.transform)

	print("[visual] car loaded: ", _car_name, " wheels=", _wheels.size(),
			" children=", _car_root.get_child_count())
	_update_hud()
	return true

## 「导入车模」按钮：打开车模库面板（导入 / 选择 / 重命名 / 删除 / 搜索）。
func _on_import_button_pressed() -> void:
	if _garage_layer != null:
		return
	var garage_scene: PackedScene = load("res://game/garage.tscn")
	var garage := garage_scene.instantiate() as Control
	garage.set_anchors_preset(Control.PRESET_FULL_RECT, true)
	garage.car_selected.connect(_on_garage_car_selected)
	garage.closed.connect(_close_garage)
	# 车库必须住在自己的 CanvasLayer 里：HUD / TouchOverlay 也是 CanvasLayer，
	# 直接 add_child 到场景根节点的 Control 只会落在默认画布层，会被 HUD 盖住
	# （真机上表现为 HUD 文字压在车库面板上，面板里的按钮还抢不到点击）。
	var layer := CanvasLayer.new()
	layer.layer = GARAGE_LAYER
	layer.add_child(garage)
	add_child(layer)
	_garage_layer = layer
	_hud.visible = false
	_touch_overlay.visible = false
	_touch_throttle = 0.0
	_touch_brake = 0.0
	_touch_steer = 0.0
	if _touch_layer != null:
		_touch_layer.visible = false
	_import_button.disabled = true

## 车模库里选了一台车：加载并关掉面板。选择写回设置，大厅也用它。
func _on_garage_car_selected(car_path: String) -> void:
	if not _load_car(car_path):
		_warn_import("加载失败：%s" % car_path)
		return
	GameSettings.set_value("garage", "car_path", car_path)
	GameSettings.save()
	_close_garage()

## 关掉车模库（选车 / 按「返回」都走这里），并恢复按钮与 HUD。
func _close_garage() -> void:
	if _garage_layer != null:
		_garage_layer.queue_free()
		_garage_layer = null
	else:
		for child in get_children():
			if child is Control and child.scene_file_path.ends_with("garage.tscn"):
				child.queue_free()
	_hud.visible = true
	_touch_overlay.visible = GameInput.is_touch_device()
	if _touch_layer != null:
		_touch_layer.visible = true
	_import_button.disabled = false

## 「设置」：打开可复用的设置弹窗，并盖住 HUD / 触摸层。
func _on_settings_button_pressed() -> void:
	if _settings_layer != null:
		return
	var popup: Control = load("res://game/settings_popup.tscn").instantiate()
	popup.closed.connect(_close_settings)
	var layer := CanvasLayer.new()
	layer.layer = SETTINGS_LAYER
	layer.add_child(popup)
	add_child(layer)
	_settings_layer = layer
	_hud.visible = false
	_touch_overlay.visible = false
	_touch_throttle = 0.0
	_touch_brake = 0.0
	_touch_steer = 0.0
	if _touch_layer != null:
		_touch_layer.visible = false


func _on_fullscreen_button_pressed() -> void:
	GameSettings.toggle_fullscreen()
	_update_hud()


func _close_settings() -> void:
	if _settings_layer != null:
		_settings_layer.queue_free()
		_settings_layer = null
	_hud.visible = true
	_touch_overlay.visible = GameInput.is_touch_device()
	if _touch_layer != null:
		_touch_layer.visible = (_touch_slider != null)
	_update_hud()


func _handle_nav(kind: String) -> bool:
	if _garage_layer != null:
		_close_garage()
		return true
	if _settings_layer != null:
		_close_settings()
		return true
	if kind == "menu":
		_on_settings_button_pressed()
		return true
	_on_lobby_button_pressed()
	return true


func _on_lobby_button_pressed() -> void:
	get_tree().change_scene_to_file("res://game/lobby.tscn")


func _on_setting_changed(section: String, key: String, _value: Variant) -> void:
	if section == "video" and key == "shadows":
		_apply_shadows()
	elif section == "input" and key == "touch_steering":
		_build_touch_controls()
		_update_device_hint()
		_update_hud()


func _apply_shadows() -> void:
	_sun.shadow_enabled = bool(GameSettings.get_value("video", "shadows", true))


func _warn_import(message: String) -> void:
	push_warning("[visual] " + message)

## 油门/刹车 → 车轮角速度 + 前轮转向。浮空预览不做悬挂/打滑，只用来确认
## 键鼠、手柄、触屏、陀螺仪输入都真的进到了车上。
func _update_wheels(delta: float) -> void:
	if _throttle > 0.02:
		_wheel_spin += WHEEL_SPIN_SPEED * _throttle * delta
	elif _brake > 0.02:
		_wheel_spin = move_toward(_wheel_spin, 0.0,
				WHEEL_BRAKE_DECEL * _brake * delta)
	else:
		_wheel_spin = move_toward(_wheel_spin, 0.0, WHEEL_COAST_DECEL * delta)
	_spin_angle += _wheel_spin * delta

	# 车头朝 `-Z`，向前滚动绕局部 `+X` 是负方向；前两轮（LF/RF）额外绕 Y 转向。
	for index in _wheels.size():
		var wheel := _wheels[index]
		if wheel == null or index >= _wheel_rest.size():
			continue
		var rest := _wheel_rest[index]
		var steer := _steer * MAX_STEER_ANGLE if index < 2 else 0.0
		var basis := rest.basis \
			* Basis(Vector3.UP, steer) \
			* Basis(Vector3.RIGHT, -_spin_angle)
		wheel.transform = Transform3D(basis, rest.origin)


func _update_camera(delta: float) -> void:
	if _car_root == null:
		return

	var anchor := _car_root.global_position
	var forward := -_car_root.global_transform.basis.z.normalized()

	match _camera_mode:
		0: # 车后：看向车前方，天然像“追尾视角”。
			_camera.global_position = _car_root.to_global(BEHIND_LOCAL)
			_camera.look_at(_car_root.to_global(Vector3(0.0, 0.30, -8.0)), Vector3.UP)
		1: # 驾驶位：直接从驾驶点往前看。
			var seat := _car_root.to_global(DRIVER_LOCAL)
			_camera.global_position = seat
			_camera.look_at(seat + forward * 20.0, Vector3.UP)
		2: # 绕车：按弧度缓慢环绕，任何角度都能看。
			_orbit_angle += ORBIT_SPEED * delta
			var offset := Vector3(cos(_orbit_angle) * ORBIT_RADIUS, ORBIT_HEIGHT, sin(_orbit_angle) * ORBIT_RADIUS)
			_camera.global_position = anchor + offset
			_camera.look_at(_car_root.to_global(ORBIT_TARGET_LOCAL), Vector3.UP)
		3: # 前机盖：贴着前部往前看。
			var hood := _car_root.to_global(HOOD_LOCAL)
			_camera.global_position = hood
			_camera.look_at(hood + forward * 20.0, Vector3.UP)

	# 鼠标/右摇杆/陀螺仪的自由视角叠加在基础相机上。
	if _look_offset.length_squared() > 1.0e-6:
		_camera.rotate_object_local(Vector3.UP, _look_offset.x)
		_camera.rotate_object_local(Vector3.RIGHT, _look_offset.y)


func _update_hud() -> void:
	_mode_button.text = MODE_TEXTS[_camera_mode]
	_info_label.text = (
		"车辆：%s\n"
		+ "油门：%d%%    刹车：%d%%    转向：%+d%%\n"
		+ "视角：%s（按钮 / C / 手柄 Y）\n"
		+ "车轮角速度：%.1f rad/s\n"
		+ "%s"
	) % [
		_car_name,
		int(roundf(_throttle * 100.0)),
		int(roundf(_brake * 100.0)),
		int(roundf(_steer * 100.0)),
		MODE_TEXTS[_camera_mode].trim_prefix("视角："),
		_wheel_spin,
		_device_hint,
	]


func _unhandled_input(event: InputEvent) -> void:
	# 弹层打开时输入归弹层，避免在设置界面里按 C 把后面的相机切走。
	if _garage_layer != null or _settings_layer != null:
		return
	if event.is_action_pressed("game_camera_next"):
		_on_camera_button_pressed()
		get_viewport().set_input_as_handled()
		return
	if event.is_action_pressed("game_garage"):
		_on_import_button_pressed()
		get_viewport().set_input_as_handled()
		return
	var motion := event as InputEventMouseMotion
	if motion != null and (motion.button_mask & MOUSE_BUTTON_MASK_LEFT) != 0:
		_look_offset += Vector2(-motion.relative.x, -motion.relative.y) * 0.0032
		_look_offset.x = clampf(_look_offset.x, -LOOK_YAW_LIMIT, LOOK_YAW_LIMIT)
		_look_offset.y = clampf(_look_offset.y, -LOOK_PITCH_LIMIT, LOOK_PITCH_LIMIT)
		get_viewport().set_input_as_handled()


## 右上角视角按钮：车后 → 驾驶位 → 绕车 → 前机盖。
func _on_camera_button_pressed() -> void:
	_camera_mode = (_camera_mode + 1) % MODE_TEXTS.size()
	_look_offset = Vector2.ZERO
	GameInput.vibrate(0.12, 0.2, 0.35)
	_update_hud()


func _on_throttle_pressed() -> void:
	_touch_throttle = 1.0


func _on_throttle_released() -> void:
	_touch_throttle = 0.0


func _on_brake_pressed() -> void:
	_touch_brake = 1.0


func _on_brake_released() -> void:
	_touch_brake = 0.0


## 触屏专用转向滑杆：叠在触摸踏板之上、HUD 之下，只在触屏且设置开启时创建。
func _build_touch_controls() -> void:
	if _touch_layer != null:
		_touch_layer.queue_free()
		_touch_layer = null
		_touch_slider = null
	if not GameInput.is_touch_device():
		return
	if not bool(GameSettings.get_value("input", "touch_steering", true)):
		return

	_touch_layer = CanvasLayer.new()
	_touch_layer.name = "TouchSteering"
	_touch_layer.layer = 1
	add_child(_touch_layer)

	var root := Control.new()
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_touch_layer.add_child(root)

	var panel := PanelContainer.new()
	panel.anchor_left = 0.18
	panel.anchor_right = 0.82
	panel.anchor_top = 1.0
	panel.anchor_bottom = 1.0
	panel.offset_left = 0.0
	panel.offset_right = 0.0
	panel.offset_top = -92.0
	panel.offset_bottom = -26.0
	panel.mouse_filter = Control.MOUSE_FILTER_STOP
	root.add_child(panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 16)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_right", 16)
	margin.add_theme_constant_override("margin_bottom", 8)
	panel.add_child(margin)
	var box := VBoxContainer.new()
	box.add_theme_constant_override("separation", 2)
	margin.add_child(box)
	var label := Label.new()
	label.text = "转向（触摸滑杆）"
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	label.add_theme_font_size_override("font_size", 14)
	box.add_child(label)
	_touch_slider = HSlider.new()
	_touch_slider.min_value = -1.0
	_touch_slider.max_value = 1.0
	_touch_slider.step = 0.01
	_touch_slider.value = 0.0
	_touch_slider.custom_minimum_size = Vector2(0, 32)
	_touch_slider.value_changed.connect(func(value: float) -> void:
		_touch_steer = value)
	_touch_slider.drag_ended.connect(func(changed: bool) -> void:
		if changed:
			_touch_slider.value = 0.0
			_touch_steer = 0.0)
	box.add_child(_touch_slider)

