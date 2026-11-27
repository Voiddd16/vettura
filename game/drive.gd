extends Node3D
## 单人自由驾驶页：加载大厅选中的赛道 + 车辆，WorldBody/Jolt 负责物理。
##
## 与 `main.tscn` 的区别：这里不再浮空演示，而是真的把车放到 AC 赛道上，
## 车后/驾驶位/前机盖三种视角、键盘/手柄/触屏/体感输入、圈速 HUD、暂停设置、
## 天气/FX 全部接上。赛道/车辆由 `GameSettings["race"]` 与 `ModLibrary` 决定。

const DEFAULT_CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"

const CAMERA_TEXTS := ["视角：车后", "视角：驾驶位", "视角：前机盖"]
const CAMERA_CHASE := 0
const CAMERA_COCKPIT := 1
const CAMERA_HOOD := 2

const HUD_INTERVAL_S := 0.1
const LOOK_YAW_LIMIT := 1.15
const LOOK_PITCH_LIMIT := 0.55
const SETTINGS_LAYER := 3

const ENGINE_EXT := "engine_ext"
const ENGINE_INT := "engine_int"

const SafeArea := preload("res://game/safe_area.gd")

@onready var _camera: Camera3D = $Camera3D
@onready var _margin: MarginContainer = $HUD/Root/Margin
@onready var _touch_layer: CanvasLayer = $TouchLayer
@onready var _touch_root: Control = %TouchRoot
@onready var _brake_button: Button = %BrakeButton
@onready var _throttle_button: Button = %ThrottleButton

@onready var _car_label: Label = %CarLabel
@onready var _track_label: Label = %TrackLabel
@onready var _lap_label: Label = %LapLabel
@onready var _sector_label: Label = %SectorLabel
@onready var _speed_label: Label = %SpeedLabel
@onready var _gear_label: Label = %GearLabel
@onready var _rpm_bar: ProgressBar = %RpmBar
@onready var _rpm_label: Label = %RpmLabel
@onready var _assist_label: Label = %AssistLabel
@onready var _hint_label: Label = %HintLabel
@onready var _camera_button: Button = %CameraButton
@onready var _reset_button: Button = %ResetButton
@onready var _settings_button: Button = %SettingsButton
@onready var _lobby_button: Button = %LobbyButton

@onready var _loading: Control = %Loading
@onready var _loading_label: Label = %LoadingLabel
@onready var _loading_bar: ProgressBar = %LoadingBar
@onready var _loading_back_button: Button = %LoadingBackButton

var _world: WorldBody
var _car: VehicleBody
var _car_name := ""
var _track_name := ""
var _track_layout := ""

var _throttle := 0.0
var _brake := 0.0
var _steer := 0.0
var _steer_target := 0.0
var _touch_throttle := 0.0
var _touch_brake := 0.0
var _touch_steer := 0.0
var _look_offset := Vector2.ZERO

var _camera_mode := CAMERA_CHASE
var _camera_snap := true

var _hud_timer := 0.0
var _settings_layer: CanvasLayer
var _touch_slider: HSlider
var _started := false

var _audio: FmodSystem
var _audio_ready := false
var _engine_event := ""


func _ready() -> void:
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)
	GameInput.push_nav_handler(_handle_nav)
	GameInput.device_changed.connect(_on_device_changed)
	GameInput.gyro_changed.connect(_on_gyro_changed)
	GameSettings.changed.connect(_on_setting_changed)
	I18n.language_changed.connect(_on_language_changed)
	_camera_button.pressed.connect(_on_camera_pressed)
	_reset_button.pressed.connect(_on_reset_pressed)
	_settings_button.pressed.connect(_on_settings_pressed)
	_lobby_button.pressed.connect(_return_to_lobby)
	_loading_back_button.pressed.connect(_return_to_lobby)
	_brake_button.button_down.connect(func() -> void: _touch_brake = 1.0)
	_brake_button.button_up.connect(func() -> void: _touch_brake = 0.0)
	_throttle_button.button_down.connect(func() -> void: _touch_throttle = 1.0)
	_throttle_button.button_up.connect(func() -> void: _touch_throttle = 0.0)
	for button in [_camera_button, _reset_button, _settings_button, _lobby_button,
			_brake_button, _throttle_button]:
		button.focus_mode = Control.FOCUS_NONE
	_touch_layer.visible = GameInput.is_touch_device()
	_build_touch_steering()
	_camera_mode = clampi(int(GameSettings.get_value("race", "camera", CAMERA_CHASE)),
			CAMERA_CHASE, CAMERA_HOOD)
	_camera_button.text = I18n.t(CAMERA_TEXTS[_camera_mode])
	_rpm_bar.max_value = 9000.0
	_apply_shadows()
	_update_hint()
	_update_assist_label()
	_show_loading(I18n.t("正在准备赛道…"), 0.0)
	await _start_session()


func _exit_tree() -> void:
	GameInput.pop_nav_handler(_handle_nav)
	if get_tree() != null:
		get_tree().paused = false


func _apply_safe_area() -> void:
	SafeArea.apply(_margin)


# ---------------------------------------------------------------- 会话加载

func _start_session() -> void:
	await get_tree().process_frame
	if not ClassDB.class_exists("WorldBody") or not ClassDB.class_exists("AcCar"):
		_show_error(I18n.t("GDExtension 未注册，无法加载赛道。"))
		return

	var track := _preferred_track()
	if String(track.get("path", "")).is_empty():
		_show_error(I18n.t("没有可用赛道。请把赛道放进 mods/ 或 user://mods/。"))
		return
	var car_path := _preferred_car()
	if car_path.is_empty() or not DirAccess.dir_exists_absolute(car_path):
		_show_error(I18n.t("没有可用车辆。请先在大厅车辆库选择一辆车。"))
		return

	_show_loading(I18n.t("加载赛道：%s…", [track.get("name", "")]), 0.05)
	await get_tree().process_frame
	_world = WorldBody.new()
	_world.name = "WorldBody"
	add_child(_world)
	if not _world.load_track(String(track.get("path", "")), String(track.get("layout", ""))):
		_show_error(I18n.t("赛道加载失败：%s", [track.get("path", "")]))
		return
	_track_name = String(track.get("name", ""))
	_track_layout = String(track.get("layout", ""))
	_track_label.text = I18n.t("赛道：%s%s", [_track_name,
			"" if _track_layout.is_empty() else " · " + _track_layout])
	_set_progress(0.55)

	_show_loading(I18n.t("生成车辆…"), 0.65)
	await get_tree().process_frame
	var ac := AcCar.new()
	if not ac.open(car_path):
		_show_error(I18n.t("车辆数据打不开：%s", [car_path]))
		return
	var car := _world.spawn_vehicle_on_grid(ac, 1)
	if car == null:
		car = _world.spawn_vehicle(ac)
	if car == null:
		_show_error(I18n.t("车辆生成失败：%s", [car_path]))
		return
	_car = car
	_car_name = ac.display_name()
	if _car_name.is_empty():
		_car_name = car_path.get_file()
	_car_label.text = _car_name
	_world.warm_up_vehicle(0)
	_camera_mode = clampi(int(GameSettings.get_value("race", "camera", CAMERA_CHASE)),
			CAMERA_CHASE, CAMERA_HOOD)
	_camera_button.text = I18n.t(CAMERA_TEXTS[_camera_mode])
	_camera_snap = true
	_setup_audio(car_path)
	_started = true
	_set_progress(1.0)
	_loading.visible = false
	_update_hud()
	_update_hint()
	print("[drive] ready car=%s track=%s layout=%s vehicles=%d"
			% [_car_name, _track_name, _track_layout, _world.vehicle_count()])


func _preferred_car() -> String:
	var path := String(GameSettings.get_value("garage", "car_path", ""))
	if not path.is_empty() and DirAccess.dir_exists_absolute(path):
		return path
	if DirAccess.dir_exists_absolute(DEFAULT_CAR_DIR):
		return DEFAULT_CAR_DIR
	var cars := ModLibrary.cars()
	return cars[0].path if not cars.is_empty() else ""


func _preferred_track() -> Dictionary:
	var path := String(GameSettings.get_value("race", "track_path", ""))
	var layout := String(GameSettings.get_value("race", "track_layout", ""))
	var entry = ModLibrary.find(path)
	if entry == null or entry.kind != "track":
		entry = _default_track_entry()
		path = entry.path if entry != null else ""
		layout = String(entry.default_layout) if entry != null else ""
	elif entry.layouts.is_empty():
		layout = ""
	elif not entry.layouts.has(layout):
		layout = String(entry.default_layout)
	return {
		"path": path,
		"layout": layout,
		"name": String(entry.display_name) if entry != null else "",
	}


func _default_track_entry():
	var tracks := ModLibrary.tracks()
	if tracks.is_empty():
		return null
	for entry in tracks:
		if FileAccess.file_exists(entry.path.path_join("ai/fast_lane.ai")):
			return entry
	return tracks[0]


func _show_loading(text: String, progress: float) -> void:
	_loading.visible = true
	_loading_label.text = text
	_loading_bar.value = clampf(progress, 0.0, 1.0) * 100.0
	_loading_back_button.visible = false


func _set_progress(value: float) -> void:
	_loading_bar.value = clampf(value, 0.0, 1.0) * 100.0


func _show_error(message: String) -> void:
	push_error("[drive] " + message)
	_loading.visible = true
	_loading_label.text = message
	_loading_bar.value = 0.0
	_loading_back_button.visible = true
	_loading_back_button.grab_focus()


# ---------------------------------------------------------------- 每帧

func _process(delta: float) -> void:
	if not _started or _car == null:
		return
	_update_input(delta)
	_update_audio()
	_update_camera(delta)
	_hud_timer -= delta
	if _hud_timer <= 0.0:
		_hud_timer = HUD_INTERVAL_S
		_update_hud()


func _update_input(delta: float) -> void:
	_throttle = maxf(Input.get_action_strength("game_throttle"), _touch_throttle)
	_brake = maxf(Input.get_action_strength("game_brake"), _touch_brake)
	var steer := Input.get_axis("game_steer_left", "game_steer_right")
	steer += _touch_steer + GameInput.tilt
	_steer_target = clampf(steer, -1.0, 1.0)
	_steer = lerpf(_steer, _steer_target, 1.0 - exp(-10.0 * delta))

	var pads := Input.get_connected_joypads()
	if not pads.is_empty():
		var look := Vector2(
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_X),
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_Y))
		if look.length() > 0.18:
			_look_offset += look * delta * 1.8
	_look_offset += GameInput.gyro_look_delta
	_look_offset.x = clampf(_look_offset.x, -LOOK_YAW_LIMIT, LOOK_YAW_LIMIT)
	_look_offset.y = clampf(_look_offset.y, -LOOK_PITCH_LIMIT, LOOK_PITCH_LIMIT)

	var abs_on := bool(GameSettings.get_value("race", "abs", true))
	var tc_on := bool(GameSettings.get_value("race", "tc", true))
	_car.set_controls(_throttle, _brake, _steer, false, false, abs_on, tc_on)


## 引擎声是可选的：没编 `--features fmod`、车模没有 bank 时只警告，驾驶照常。
func _setup_audio(car_path: String) -> void:
	if not ClassDB.class_exists("FmodSystem"):
		return
	_audio = FmodSystem.new()
	add_child(_audio)
	if not _audio.is_linked():
		# 默认构建不带 FMOD SDK：静默跳过，不把 play 链路刷成错误。
		_audio.queue_free()
		_audio = null
		return
	if not _audio.start(64):
		push_warning("[drive] FMOD 未启动：%s" % _audio.last_error())
		return
	if not _audio.load_car_path(car_path):
		push_warning("[drive] 车模声效加载失败：%s" % _audio.last_error())
		return
	var events := _audio.event_names()
	var event := ENGINE_INT if _camera_mode == CAMERA_COCKPIT and events.has(ENGINE_INT) \
			else ENGINE_EXT
	if not events.has(event):
		event = ENGINE_INT if events.has(ENGINE_INT) else ENGINE_EXT
	if not events.has(event):
		push_warning("[drive] 车模没有 engine_ext / engine_int，无引擎声")
		return
	if not _audio.play(event):
		push_warning("[drive] 播放引擎声失败：%s" % _audio.last_error())
		return
	_engine_event = event
	_audio_ready = true
	print("[drive] audio on: %s" % _audio.fallback_summary())


func _update_audio() -> void:
	if not _audio_ready:
		return
	var rpm := _car.engine_rpm()
	var load := clampf(_throttle * (0.4 + 0.6 * (1.0 - clampf(rpm / 9000.0, 0.0, 1.0))),
			0.0, 1.0)
	_audio.set_vehicle_parameters(
		_engine_event, rpm, _throttle, load, _car.speed_kmh(),
		1.0 if _camera_mode == CAMERA_COCKPIT else 0.0,
		_car.gear(), _car.max_wheel_slip_angle())
	var forward := -_camera.global_transform.basis.z
	_audio.set_listener(_camera.global_position, Vector3.ZERO,
			forward, _camera.global_transform.basis.y)
	var car_forward := -_car.global_transform.basis.z
	_audio.set_event_transform(_engine_event, _car.global_position,
			car_forward * _car.forward_speed_mps(), car_forward,
			_car.global_transform.basis.y)
	if not OS.get_environment("VETTURA_AUDIO_WAV").is_empty():
		_audio.step_audio()


func _switch_engine_for_camera() -> void:
	if not _audio_ready:
		return
	var events := _audio.event_names()
	var desired := ENGINE_INT if _camera_mode == CAMERA_COCKPIT and events.has(ENGINE_INT) \
			else ENGINE_EXT
	if desired == _engine_event or not events.has(desired):
		return
	_audio.stop(_engine_event)
	if _audio.play(desired):
		_engine_event = desired


func _update_camera(delta: float) -> void:
	if _car == null:
		return
	var t := _car.global_transform
	var target_position := _camera.global_position
	var look_target := t * Vector3(0.0, 0.8, -12.0)

	match _camera_mode:
		CAMERA_COCKPIT:
			target_position = t * Vector3(0.0, 1.02, 0.05)
			look_target = t * Vector3(0.0, 0.95, -20.0)
			_camera_snap = true
		CAMERA_HOOD:
			target_position = t * Vector3(0.0, 0.72, -0.85)
			look_target = t * Vector3(0.0, 0.68, -22.0)
			_camera_snap = true
		_:
			target_position = t * Vector3(0.0, 1.85, 5.8)
			look_target = t * Vector3(0.0, 0.75, -10.0)

	if _camera_snap:
		_camera.global_position = target_position
		_camera_snap = false
	else:
		_camera.global_position = _camera.global_position.lerp(
				target_position, 1.0 - exp(-9.0 * delta))
	_camera.look_at(look_target, Vector3.UP)
	if _look_offset.length_squared() > 1.0e-6:
		_camera.rotate_object_local(Vector3.UP, -_look_offset.x)
		_camera.rotate_object_local(Vector3.RIGHT, -_look_offset.y)

	var fov := 70.0
	if _camera_mode == CAMERA_COCKPIT:
		fov = 72.0
	elif _camera_mode == CAMERA_HOOD:
		fov = 78.0
	_camera.fov = fov


func _update_hud() -> void:
	if _world == null or _car == null:
		return
	var speed := _car.speed_kmh()
	var gear := _car.gear()
	_speed_label.text = "%d" % int(roundf(speed))
	_gear_label.text = "R" if gear < 0 else ("N" if gear == 0 else str(gear))
	var rpm := _car.engine_rpm()
	_rpm_bar.value = clampf(rpm, 0.0, _rpm_bar.max_value)
	_rpm_label.text = "%d rpm" % int(roundf(rpm))
	_lap_label.text = I18n.t("第 %d 圈 · 本圈 %s", [
			int(_world.laps()) + 1, _format_time(_world.lap_time_s())])
	var sector := ""
	if _world.sector_count() > 0:
		sector = I18n.t("分段 %d/%d    ", [int(_world.current_sector()) + 1,
				int(_world.sector_count())])
	_sector_label.text = I18n.t("%s上一圈 %s    最佳 %s", [
			sector, _format_time(_world.last_lap_s()), _format_time(_world.best_lap_s())])
	_update_assist_label()


func _format_time(seconds: float) -> String:
	if seconds <= 0.0 or not is_finite(seconds):
		return "--:--.---"
	var minutes := int(seconds / 60.0)
	var rest := seconds - minutes * 60.0
	return "%02d:%06.3f" % [minutes, rest]


func _update_assist_label() -> void:
	var aids: Array[String] = []
	aids.append("ABS" if bool(GameSettings.get_value("race", "abs", true))
			else I18n.t("无 ABS"))
	aids.append("TC" if bool(GameSettings.get_value("race", "tc", true))
			else I18n.t("无 TC"))
	aids.append(I18n.t("自动挡"))
	_assist_label.text = " · ".join(PackedStringArray(aids))


func _update_hint() -> void:
	if _hint_label == null:
		return
	_hint_label.text = GameInput.hint_text()
	if GameInput.is_touch_device() and GameSettings.get_value("input", "touch_steering", true):
		_hint_label.text += "\n" + I18n.t("底部滑杆转向；车身倾斜也可转向。")


# ---------------------------------------------------------------- 输入 / 相机

func _unhandled_input(event: InputEvent) -> void:
	if _settings_layer != null:
		return
	if event.is_action_pressed("game_camera_next"):
		_on_camera_pressed()
		get_viewport().set_input_as_handled()
		return
	if event.is_action_pressed("game_reset_car"):
		_on_reset_pressed()
		get_viewport().set_input_as_handled()
		return
	var motion := event as InputEventMouseMotion
	if motion != null and (motion.button_mask & MOUSE_BUTTON_MASK_LEFT) != 0:
		_look_offset += Vector2(-motion.relative.x, -motion.relative.y) * 0.003
		_look_offset.x = clampf(_look_offset.x, -LOOK_YAW_LIMIT, LOOK_YAW_LIMIT)
		_look_offset.y = clampf(_look_offset.y, -LOOK_PITCH_LIMIT, LOOK_PITCH_LIMIT)
		get_viewport().set_input_as_handled()


func _on_camera_pressed() -> void:
	_camera_mode = (_camera_mode + 1) % CAMERA_TEXTS.size()
	_camera_button.text = I18n.t(CAMERA_TEXTS[_camera_mode])
	_look_offset = Vector2.ZERO
	_camera_snap = true
	GameSettings.set_value("race", "camera", _camera_mode)
	GameSettings.save()
	_switch_engine_for_camera()
	GameInput.vibrate(0.1, 0.15, 0.25)


func _on_reset_pressed() -> void:
	if _world == null or _car == null:
		return
	_world.reset_vehicle(_car)
	_throttle = 0.0
	_brake = 0.0
	_steer = 0.0
	_steer_target = 0.0
	_touch_throttle = 0.0
	_touch_brake = 0.0
	_touch_steer = 0.0
	_look_offset = Vector2.ZERO
	_camera_snap = true
	GameInput.vibrate(0.18, 0.3, 0.5)


# ---------------------------------------------------------------- 暂停 / 设置

func _handle_nav(kind: String) -> bool:
	if _settings_layer != null:
		_close_settings()
		return true
	if kind == "menu":
		_on_settings_pressed()
		return true
	_return_to_lobby()
	return true


func _on_settings_pressed() -> void:
	if _settings_layer != null:
		return
	_throttle = 0.0
	_brake = 0.0
	_touch_throttle = 0.0
	_touch_brake = 0.0
	_touch_steer = 0.0
	if _car != null:
		_car.set_controls(0.0, 0.0, 0.0, false, false,
				bool(GameSettings.get_value("race", "abs", true)),
				bool(GameSettings.get_value("race", "tc", true)))
	var popup: Control = load("res://game/settings_popup.tscn").instantiate()
	popup.process_mode = Node.PROCESS_MODE_ALWAYS
	popup.closed.connect(_close_settings)
	var layer := CanvasLayer.new()
	layer.layer = SETTINGS_LAYER
	layer.add_child(popup)
	add_child(layer)
	_settings_layer = layer
	_touch_layer.visible = false
	get_tree().paused = true


func _close_settings() -> void:
	if get_tree() != null:
		get_tree().paused = false
	if _settings_layer != null:
		_settings_layer.queue_free()
		_settings_layer = null
	_touch_layer.visible = GameInput.is_touch_device() and _started
	_camera_snap = true
	_update_hint()
	_update_hud()


func _return_to_lobby() -> void:
	if get_tree() != null:
		get_tree().paused = false
	get_tree().change_scene_to_file("res://game/lobby.tscn")


# ---------------------------------------------------------------- 设置 / 设备

func _on_setting_changed(section: String, key: String, _value: Variant) -> void:
	if section == "video" and key == "shadows":
		_apply_shadows()
	elif section == "race" and (key == "abs" or key == "tc"):
		_update_assist_label()
	elif section == "race" and key == "camera":
		_camera_mode = clampi(int(_value), CAMERA_CHASE, CAMERA_HOOD)
		_camera_button.text = I18n.t(CAMERA_TEXTS[_camera_mode])
		_camera_snap = true
		_switch_engine_for_camera()
	elif section == "input" and key == "touch_steering":
		_build_touch_steering()
		_update_hint()


func _apply_shadows() -> void:
	var enabled := bool(GameSettings.get_value("video", "shadows", true))
	for node in get_tree().root.find_children("*", "DirectionalLight3D", true, false):
		(node as DirectionalLight3D).shadow_enabled = enabled


func _on_device_changed(_device: String) -> void:
	_touch_layer.visible = GameInput.is_touch_device() and _settings_layer == null and _started
	_update_hint()


func _on_gyro_changed(_available: bool, _enabled: bool) -> void:
	_update_hint()


func _on_language_changed(_locale: String) -> void:
	_camera_button.text = I18n.t(CAMERA_TEXTS[_camera_mode])
	_update_hint()
	_update_hud()
	_build_touch_steering()


# ---------------------------------------------------------------- 触屏

func _build_touch_steering() -> void:
	for child in _touch_root.get_children():
		if child.name == "TouchSteering":
			_touch_root.remove_child(child)
			child.queue_free()
	_touch_slider = null
	if not GameInput.is_touch_device():
		return
	if not bool(GameSettings.get_value("input", "touch_steering", true)):
		return
	var panel := PanelContainer.new()
	panel.name = "TouchSteering"
	panel.anchor_left = 0.18
	panel.anchor_right = 0.82
	panel.anchor_top = 1.0
	panel.anchor_bottom = 1.0
	panel.offset_left = 0.0
	panel.offset_right = 0.0
	panel.offset_top = -96.0
	panel.offset_bottom = -24.0
	panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_touch_root.add_child(panel)
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
	label.text = I18n.t("转向")
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	label.add_theme_font_size_override("font_size", 14)
	box.add_child(label)
	var slider := HSlider.new()
	slider.min_value = -1.0
	slider.max_value = 1.0
	slider.step = 0.01
	slider.value = 0.0
	slider.custom_minimum_size = Vector2(0, 32)
	slider.value_changed.connect(func(value: float) -> void: _touch_steer = value)
	slider.drag_ended.connect(func(changed: bool) -> void:
		if changed:
			slider.value = 0.0
			_touch_steer = 0.0)
	box.add_child(slider)
	_touch_slider = slider
