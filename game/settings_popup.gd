extends Control
## 游戏内设置弹窗：音频 / 画面 / 车库展示 / 驾驶辅助 / 输入。
##
## 一个自包含的模态层：宿主实例化后把它放进自己的 `CanvasLayer` 即可。它会
## 盖住整屏并吞掉输入，关闭时发 `closed` 让宿主恢复 HUD。
##
## ```gdscript
## var popup := load("res://game/settings_popup.tscn").instantiate()
## popup.closed.connect(_on_settings_closed)
## add_child(popup)
## ```
##
## 设置值统一走 `GameSettings` / `GameInput` autoload；本脚本只负责控件与
## 值之间的双向同步。输入页的按键绑定列表是运行时生成的，新增动作只要加到
## `GameInput.ACTIONS` 就会自动出现。

signal closed

const SafeArea := preload("res://game/safe_area.gd")

@onready var _margin: MarginContainer = %Margin
@onready var _panel: PanelContainer = %Panel

@onready var _master: HSlider = %MasterSlider
@onready var _master_value: Label = %MasterValue
@onready var _fullscreen: CheckBox = %FullscreenCheck
@onready var _borderless: CheckBox = %BorderlessCheck
@onready var _adaptive_check: CheckBox = %AdaptiveCheck
@onready var _adaptive_label: Label = %AdaptiveLabel
@onready var _vsync: CheckBox = %VsyncCheck
@onready var _shadows: CheckBox = %ShadowsCheck
@onready var _scale: HSlider = %ScaleSlider
@onready var _scale_value: Label = %ScaleValue
@onready var _upscaler_option: OptionButton = %UpscalerOption
@onready var _vrs_option: OptionButton = %VrsOption
@onready var _target_fps_option: OptionButton = %TargetFpsOption
@onready var _display_hint: Label = %DisplayHint
@onready var _orbit_speed: HSlider = %OrbitSpeedSlider
@onready var _orbit_speed_value: Label = %OrbitSpeedValue
@onready var _orbit_radius: HSlider = %OrbitRadiusSlider
@onready var _orbit_radius_value: Label = %OrbitRadiusValue
@onready var _garage_hint: Label = %GarageHint
@onready var _abs_check: CheckBox = %AbsCheck
@onready var _tc_check: CheckBox = %TcCheck
@onready var _camera_option: OptionButton = %CameraOption
@onready var _language_option: OptionButton = %LanguageOption
@onready var _tabs: TabContainer = $Margin/Center/Panel/PanelMargin/Body/Tabs
@onready var _input_box: VBoxContainer = %InputBox
@onready var _reset_button: Button = %ResetButton
@onready var _apply_button: Button = %ApplyButton
@onready var _close_button: Button = %CloseButton

## 载入值时禁止滑杆/勾选框把“初始化”误当成用户修改。
var _syncing := false

## 重绑状态。
var _rebind_action := ""
var _rebind_started_ms := 0
var _rebind_buttons: Dictionary = {}
var _rebind_status: Label
var _device_status: Label
var _gyro_check: CheckBox
var _gyro_hint: Label


func _ready() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_apply_safe_area()
	get_viewport().size_changed.connect(_on_viewport_size_changed)

	_master.value_changed.connect(_on_master_changed)
	_scale.value_changed.connect(_on_scale_changed)
	_upscaler_option.item_selected.connect(_on_upscaler_selected)
	_vrs_option.item_selected.connect(_on_vrs_selected)
	_target_fps_option.item_selected.connect(_on_target_fps_selected)
	_orbit_speed.value_changed.connect(_on_orbit_speed_changed)
	_orbit_radius.value_changed.connect(_on_orbit_radius_changed)
	_fullscreen.toggled.connect(_on_fullscreen_toggled)
	_borderless.toggled.connect(_on_borderless_toggled)
	_adaptive_check.toggled.connect(_on_adaptive_toggled)
	_vsync.toggled.connect(_on_vsync_toggled)
	_shadows.toggled.connect(_on_shadows_toggled)
	_abs_check.toggled.connect(_on_abs_toggled)
	_tc_check.toggled.connect(_on_tc_toggled)
	_camera_option.item_selected.connect(_on_camera_selected)
	_language_option.item_selected.connect(_on_language_selected)
	_reset_button.pressed.connect(_on_reset_pressed)
	_apply_button.pressed.connect(_close)
	_close_button.pressed.connect(_close)

	_load_values()
	_build_input_tab()
	_fit_panel()
	GameInput.push_nav_handler(_handle_nav)
	GameInput.device_changed.connect(_on_device_changed)
	GameSettings.changed.connect(_on_external_setting_changed)
	RenderQuality.adaptive_changed.connect(_on_adaptive_changed)
	I18n.language_changed.connect(_on_language_changed)
	_apply_tab_titles()
	# 弹窗出现时把键盘焦点收到“完成”键，手柄/键盘用户可直接操作。
	_apply_button.grab_focus()


func _exit_tree() -> void:
	GameInput.pop_nav_handler(_handle_nav)
	GameInput.set_capture_input(false)


func _handle_nav(_kind: String) -> bool:
	_close()
	return true


func _input(event: InputEvent) -> void:
	if _rebind_action.is_empty():
		return
	if event is InputEventKey:
		var key := event as InputEventKey
		if key.echo:
			return
		if key.pressed and key.physical_keycode == KEY_ESCAPE:
			_cancel_rebind()
			get_viewport().set_input_as_handled()
			return
	# 点击“重绑”按钮的那一下鼠标左键不要被当成新绑定。
	if Time.get_ticks_msec() - _rebind_started_ms < 250 \
			and event is InputEventMouseButton \
			and (event as InputEventMouseButton).button_index == MOUSE_BUTTON_LEFT:
		return
	if not GameInput.is_bindable_event(event):
		return
	var conflict := GameInput.rebind(_rebind_action, event)
	var action := _rebind_action
	_finish_rebind()
	if conflict.is_empty():
		_rebind_status.text = I18n.t("已绑定：%s → %s", [
			GameInput.action_label(action), GameInput.binding_text(action)])
	else:
		_rebind_status.text = I18n.t("已绑定：%s → %s（与「%s」冲突，已从对方移除）", [
			GameInput.action_label(action), GameInput.binding_text(action),
			GameInput.action_label(conflict)])
	get_viewport().set_input_as_handled()


func _unhandled_input(event: InputEvent) -> void:
	# GameInput 正常会先消费 Esc/B；这里保留兜底，避免焦点在 LineEdit 等
	# 控件里时 Esc 被吞掉。
	var key := event as InputEventKey
	if key != null and key.pressed and not key.echo \
			and key.physical_keycode == KEY_ESCAPE:
		get_viewport().set_input_as_handled()
		_close()


func _apply_safe_area() -> void:
	SafeArea.apply(_margin)


func _on_viewport_size_changed() -> void:
	_apply_safe_area()
	_fit_panel()


## 小屏 / 竖屏时把面板收进画布，避免标题和“完成”按钮被裁掉。
func _fit_panel() -> void:
	var canvas := get_viewport_rect().size
	var available := canvas - Vector2(32.0, 32.0)
	_panel.custom_minimum_size = Vector2(
		clampf(available.x, 420.0, 720.0),
		clampf(available.y, 320.0, 560.0))


func _load_values() -> void:
	_syncing = true
	_master.value = roundf(float(GameSettings.get_value("audio", "master_volume", 0.8)) * 100.0)
	var mode := int(GameSettings.get_value("video", "window_mode",
			GameSettings.WINDOW_WINDOWED))
	_fullscreen.button_pressed = mode == GameSettings.WINDOW_FULLSCREEN
	_borderless.button_pressed = mode == GameSettings.WINDOW_BORDERLESS
	_fullscreen.disabled = OS.has_feature("mobile")
	_borderless.disabled = OS.has_feature("mobile")
	_adaptive_check.button_pressed = bool(GameSettings.get_value("video", "fullscreen_adaptive", true))
	_adaptive_check.disabled = OS.has_feature("mobile")
	_vsync.button_pressed = bool(GameSettings.get_value("video", "vsync", true))
	_shadows.button_pressed = bool(GameSettings.get_value("video", "shadows", true))
	_scale.value = roundf(float(GameSettings.get_value("video", "render_scale", 1.0)) * 100.0)
	_fill_display_options()
	_orbit_speed.value = roundf(float(GameSettings.get_value("garage", "orbit_speed", 0.18)) * 100.0)
	_orbit_radius.value = roundf(float(GameSettings.get_value("garage", "orbit_radius", 7.0)))
	_abs_check.button_pressed = bool(GameSettings.get_value("race", "abs", true))
	_tc_check.button_pressed = bool(GameSettings.get_value("race", "tc", true))
	_fill_camera_option()
	_fill_language_option()
	_syncing = false
	_refresh_labels()


func _fill_camera_option() -> void:
	_camera_option.clear()
	_camera_option.add_item(I18n.t("车后追随"))
	_camera_option.add_item(I18n.t("驾驶位"))
	_camera_option.add_item(I18n.t("前机盖"))
	_camera_option.select(clampi(int(GameSettings.get_value("race", "camera", 0)), 0, 2))


func _fill_language_option() -> void:
	_language_option.clear()
	var current := I18n.language_setting()
	var options := I18n.language_options()
	var selected := 0
	for index in options.size():
		var option: Dictionary = options[index]
		var id := String(option.get("id", ""))
		_language_option.add_item(String(option.get("label", id)))
		_language_option.set_item_metadata(index, id)
		if id == current:
			selected = index
	_language_option.select(selected)


func _apply_tab_titles() -> void:
	if _tabs == null:
		return
	var titles := ["音频", "画面", "界面", "车库", "驾驶", "输入"]
	for index in mini(titles.size(), _tabs.get_tab_count()):
		_tabs.set_tab_title(index, I18n.t(String(titles[index])))


func _on_language_changed(_locale: String) -> void:
	_syncing = true
	_apply_tab_titles()
	_fill_language_option()
	_fill_camera_option()
	_fill_display_options()
	_syncing = false
	_refresh_labels()
	_build_input_tab()


func _fill_display_options() -> void:
	_fill_upscaler_option()
	_fill_vrs_option()
	_fill_target_fps_option()


func _fill_upscaler_option() -> void:
	var requested := String(GameSettings.get_value("video", "upscaler", "auto"))
	var options := DisplayPipeline.upscaler_options()
	if options.is_empty():
		return
	_upscaler_option.clear()
	var selected := 0
	for index in options.size():
		var option: Dictionary = options[index]
		var id := String(option.get("id", "auto"))
		_upscaler_option.add_item(I18n.t(String(option.get("label", id))))
		_upscaler_option.set_item_metadata(index, id)
		if not bool(option.get("available", false)):
			_upscaler_option.set_item_disabled(index, true)
			_upscaler_option.set_item_tooltip(index,
				I18n.t("当前引擎/平台不可用；选中时运行时会自动回退。"))
		if id == requested:
			selected = index
	if not bool(options[selected].get("available", false)):
		selected = 0
	_upscaler_option.select(selected)


func _fill_vrs_option() -> void:
	var requested := String(GameSettings.get_value("video", "vrs", "auto"))
	var options := DisplayPipeline.vrs_options()
	if options.is_empty():
		return
	_vrs_option.clear()
	var selected := 0
	for index in options.size():
		var option: Dictionary = options[index]
		var id := String(option.get("id", "auto"))
		_vrs_option.add_item(I18n.t(String(option.get("label", id))))
		_vrs_option.set_item_metadata(index, id)
		if not bool(option.get("available", false)):
			_vrs_option.set_item_disabled(index, true)
			_vrs_option.set_item_tooltip(index, I18n.t("当前后端/硬件不支持。"))
		if id == requested:
			selected = index
	if not bool(options[selected].get("available", false)):
		selected = 0
	_vrs_option.select(selected)


func _fill_target_fps_option() -> void:
	var requested := int(GameSettings.get_value("video", "dynamic_target_fps", 0))
	var refresh := DisplayServer.screen_get_refresh_rate()
	if refresh <= 1.0:
		refresh = 60.0
	var options := [
		{"value": 0, "label": I18n.t("自动（最高 60 FPS）")},
		{"value": 60, "label": "60 FPS"},
		{"value": 90, "label": "90 FPS"},
		{"value": 120, "label": "120 FPS"},
		{"value": 144, "label": "144 FPS"},
		{"value": -1, "label": I18n.t("跟随显示器（%d Hz）", [int(roundf(refresh))])},
	]
	_target_fps_option.clear()
	var selected := 0
	for index in options.size():
		var option: Dictionary = options[index]
		var value := int(option.get("value", 0))
		_target_fps_option.add_item(String(option.get("label", str(value))))
		_target_fps_option.set_item_metadata(index, value)
		if value == requested:
			selected = index
	_target_fps_option.select(selected)


func _refresh_labels() -> void:
	_master_value.text = "%d%%" % int(_master.value)
	_scale_value.text = "%d%%" % int(_scale.value)
	_orbit_speed_value.text = "%.2f rad/s" % (_orbit_speed.value / 100.0)
	_orbit_radius_value.text = "%.1f m" % _orbit_radius.value
	var mod_path := String(GameSettings.get_value("garage", "mod_path", GameSettings.DEFAULT_GARAGE_MOD))
	_garage_hint.text = I18n.t("当前车库模组：\n%s", [mod_path])
	_refresh_adaptive_label()
	_refresh_display_hint()


func _refresh_adaptive_label() -> void:
	if _adaptive_label == null:
		return
	var state := DisplayPipeline.state()
	var scale := int(roundf(float(state.get("scale", 1.0)) * 100.0))
	var upscaler := I18n.t(String(state.get("upscaler_label", "原生")))
	var min_scale := int(roundf(float(state.get("dynamic_min", 0.35)) * 100.0))
	var max_scale := int(roundf(float(state.get("dynamic_max", 1.0)) * 100.0))
	var target := int(roundf(float(state.get("target_fps", 60.0))))
	if OS.has_feature("mobile"):
		_adaptive_label.text = I18n.t("移动端：3D 缩放由雨/画质档位管理；上采样 %s。", [upscaler])
	elif not GameSettings.is_fullscreen():
		_adaptive_label.text = I18n.t("窗口模式：3D 分辨率 %d%%，上采样 %s；全屏可启用动态分辨率。", [
			scale, upscaler])
	elif not bool(GameSettings.get_value("video", "fullscreen_adaptive", true)):
		_adaptive_label.text = I18n.t("全屏固定：3D 分辨率 %d%%，上采样 %s。", [scale, upscaler])
	else:
		_adaptive_label.text = I18n.t("全屏动态：当前 3D %d%%（%s），目标 %d FPS，范围 %d%%~%d%%。", [
			scale, upscaler, target, min_scale, max_scale])


func _refresh_display_hint() -> void:
	if _display_hint == null:
		return
	var caps := DisplayPipeline.capabilities()
	var state := DisplayPipeline.state()
	var mods: Array = caps.get("engine_mods", [])
	var parts := [
		I18n.t("后端 %s / %s", [String(caps.get("renderer", "")),
			String(caps.get("driver", ""))]),
	]
	if bool(caps.get("xr", false)):
		parts.append("XR")
	if mods.size() > 0:
		parts.append(I18n.t("引擎扩展 %d", [mods.size()]))
	if not bool(caps.get("vrs", {}).get("texture", false)):
		parts.append(I18n.t("VRS 纹理不可用"))
	var requested := String(state.get("requested_upscaler", "auto"))
	var active := String(state.get("active_upscaler", "native"))
	if requested != "auto" and requested != active:
		parts.append(I18n.t("请求 %s → 回退 %s", [
			I18n.t(DisplayPipeline.label_for(requested)),
			I18n.t(DisplayPipeline.label_for(active))]))
	_display_hint.text = I18n.t("%s。FSR 3.1 需要魔改引擎，见 engine/README.md。",
			[" · ".join(PackedStringArray(parts))])


func _on_adaptive_changed(_scale: float, _fps: float) -> void:
	_refresh_adaptive_label()


func _on_master_changed(value: float) -> void:
	if _syncing:
		return
	GameSettings.set_value("audio", "master_volume", value / 100.0)
	_refresh_labels()


func _on_scale_changed(value: float) -> void:
	if _syncing:
		return
	GameSettings.set_value("video", "render_scale", value / 100.0)
	_refresh_labels()


func _on_upscaler_selected(index: int) -> void:
	if _syncing:
		return
	var id := String(_upscaler_option.get_item_metadata(index))
	GameSettings.set_value("video", "upscaler", id)
	_refresh_adaptive_label()


func _on_vrs_selected(index: int) -> void:
	if _syncing:
		return
	var id := String(_vrs_option.get_item_metadata(index))
	GameSettings.set_value("video", "vrs", id)
	_refresh_adaptive_label()


func _on_target_fps_selected(index: int) -> void:
	if _syncing:
		return
	var value := int(_target_fps_option.get_item_metadata(index))
	GameSettings.set_value("video", "dynamic_target_fps", value)
	_refresh_adaptive_label()


func _on_orbit_speed_changed(value: float) -> void:
	if _syncing:
		return
	GameSettings.set_value("garage", "orbit_speed", value / 100.0)
	_refresh_labels()


func _on_orbit_radius_changed(value: float) -> void:
	if _syncing:
		return
	GameSettings.set_value("garage", "orbit_radius", value)
	_refresh_labels()


func _on_fullscreen_toggled(enabled: bool) -> void:
	if _syncing:
		return
	if enabled:
		GameSettings.set_window_mode(GameSettings.WINDOW_FULLSCREEN)
		_syncing = true
		_borderless.button_pressed = false
		_syncing = false
	elif not _borderless.button_pressed:
		GameSettings.set_window_mode(GameSettings.WINDOW_WINDOWED)


func _on_borderless_toggled(enabled: bool) -> void:
	if _syncing:
		return
	if enabled:
		GameSettings.set_window_mode(GameSettings.WINDOW_BORDERLESS)
		_syncing = true
		_fullscreen.button_pressed = false
		_syncing = false
	elif not _fullscreen.button_pressed:
		GameSettings.set_window_mode(GameSettings.WINDOW_WINDOWED)


func _on_adaptive_toggled(enabled: bool) -> void:
	if _syncing:
		return
	GameSettings.set_value("video", "fullscreen_adaptive", enabled)
	_refresh_adaptive_label()


func _on_vsync_toggled(enabled: bool) -> void:
	if _syncing:
		return
	GameSettings.set_value("video", "vsync", enabled)


func _on_shadows_toggled(enabled: bool) -> void:
	if _syncing:
		return
	GameSettings.set_value("video", "shadows", enabled)


func _on_abs_toggled(enabled: bool) -> void:
	if _syncing:
		return
	GameSettings.set_value("race", "abs", enabled)


func _on_tc_toggled(enabled: bool) -> void:
	if _syncing:
		return
	GameSettings.set_value("race", "tc", enabled)


func _on_camera_selected(index: int) -> void:
	if _syncing:
		return
	GameSettings.set_value("race", "camera", clampi(index, 0, 2))


func _on_language_selected(index: int) -> void:
	if _syncing:
		return
	I18n.set_language(String(_language_option.get_item_metadata(index)))


func _on_reset_pressed() -> void:
	GameSettings.restore_defaults()
	GameInput.reset_all_bindings()
	_load_values()
	_build_input_tab()


func _close() -> void:
	# 拖动滑杆时只改了内存，关闭时统一落盘。
	_cancel_rebind(false)
	GameSettings.save()
	closed.emit()
	queue_free()


# ---------------------------------------------------------------- 输入页

func _build_input_tab() -> void:
	for child in _input_box.get_children():
		child.free()
	_rebind_buttons.clear()
	_game_input_section()
	_gyro_section()
	_bindings_section()


func _game_input_section() -> void:
	_input_box.add_child(_section_title("输入设备"))
	_device_status = _text_label(GameInput.device_label(), 17, Color(0.78, 0.86, 0.96))
	_input_box.add_child(_device_status)

	var vibration := CheckBox.new()
	vibration.text = "手柄震动 / 触屏振动"
	vibration.button_pressed = bool(GameSettings.get_value("input", "vibration", true))
	vibration.add_theme_font_size_override("font_size", 19)
	vibration.toggled.connect(func(enabled: bool) -> void:
		GameSettings.set_value("input", "vibration", enabled))
	_input_box.add_child(vibration)

	if GameInput.is_touch_device():
		var touch := CheckBox.new()
		touch.text = "显示屏幕转向滑杆（触屏）"
		touch.button_pressed = bool(GameSettings.get_value("input", "touch_steering", true))
		touch.add_theme_font_size_override("font_size", 19)
		touch.toggled.connect(func(enabled: bool) -> void:
			GameSettings.set_value("input", "touch_steering", enabled))
		_input_box.add_child(touch)


func _gyro_section() -> void:
	_input_box.add_child(_section_title("陀螺仪 / 体感"))
	_gyro_check = CheckBox.new()
	_gyro_check.text = "启用倾斜转向与体感视角"
	_gyro_check.button_pressed = GameInput.gyro_enabled
	_gyro_check.disabled = not GameInput.gyro_available
	_gyro_check.add_theme_font_size_override("font_size", 19)
	_gyro_check.toggled.connect(_on_gyro_toggled)
	_input_box.add_child(_gyro_check)

	_gyro_hint = _text_label(_gyro_hint_text(), 16, Color(0.62, 0.70, 0.82))
	_input_box.add_child(_gyro_hint)

	var sensitivity_row := HBoxContainer.new()
	sensitivity_row.add_theme_constant_override("separation", 12)
	var label := _text_label("灵敏度", 18, Color(0.85, 0.90, 0.98))
	label.custom_minimum_size = Vector2(100, 0)
	sensitivity_row.add_child(label)
	var slider := HSlider.new()
	slider.min_value = 20.0
	slider.max_value = 300.0
	slider.step = 5.0
	slider.value = GameInput.gyro_sensitivity() * 100.0
	slider.custom_minimum_size = Vector2(0, 32)
	slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	var value_label := _text_label("%d%%" % int(slider.value), 18, Color(0.85, 0.90, 0.98))
	value_label.custom_minimum_size = Vector2(64, 0)
	value_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	slider.value_changed.connect(func(value: float) -> void:
		GameInput.set_gyro_sensitivity(value / 100.0)
		value_label.text = "%d%%" % int(value))
	sensitivity_row.add_child(slider)
	sensitivity_row.add_child(value_label)
	_input_box.add_child(sensitivity_row)

	var invert := CheckBox.new()
	invert.text = "反转倾斜方向"
	invert.button_pressed = GameInput.gyro_inverted()
	invert.add_theme_font_size_override("font_size", 19)
	invert.toggled.connect(func(enabled: bool) -> void:
		GameInput.set_gyro_invert(enabled))
	_input_box.add_child(invert)

	var calibrate := Button.new()
	calibrate.text = "校准陀螺仪（自然握持后点一下）"
	calibrate.custom_minimum_size = Vector2(0, 40)
	calibrate.add_theme_font_size_override("font_size", 18)
	calibrate.disabled = not GameInput.gyro_available
	calibrate.pressed.connect(func() -> void:
		GameInput.calibrate_gyro()
		_rebind_status.text = I18n.t("陀螺仪已校准：当前姿态作为中位。"))
	_input_box.add_child(calibrate)


func _bindings_section() -> void:
	_input_box.add_child(_section_title("按键绑定"))
	_input_box.add_child(_text_label(
		"点右侧按钮后按下新键/手柄按钮。键鼠与手柄各保留一个绑定。",
		16, Color(0.62, 0.70, 0.82)))
	_rebind_status = _text_label(I18n.t("尚未重绑。"), 16, Color(0.72, 0.80, 0.92))
	_input_box.add_child(_rebind_status)

	for action in GameInput.actions_in_order():
		var row := HBoxContainer.new()
		row.add_theme_constant_override("separation", 10)
		var label := _text_label(GameInput.action_label(action), 18, Color(0.86, 0.91, 0.98))
		label.custom_minimum_size = Vector2(170, 0)
		row.add_child(label)
		var bind_button := Button.new()
		bind_button.text = GameInput.binding_text(action)
		bind_button.custom_minimum_size = Vector2(0, 38)
		bind_button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		bind_button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		bind_button.add_theme_font_size_override("font_size", 17)
		bind_button.pressed.connect(func() -> void: _begin_rebind(action))
		row.add_child(bind_button)
		_rebind_buttons[action] = bind_button
		var reset := Button.new()
		reset.text = "重置"
		reset.custom_minimum_size = Vector2(64, 38)
		reset.add_theme_font_size_override("font_size", 16)
		reset.pressed.connect(func() -> void:
			GameInput.reset_action(action)
			_refresh_binding_buttons()
			_rebind_status.text = I18n.t("已重置：%s", [GameInput.action_label(action)]))
		row.add_child(reset)
		_input_box.add_child(row)

	var reset_all := Button.new()
	reset_all.text = "全部恢复默认键位"
	reset_all.custom_minimum_size = Vector2(0, 42)
	reset_all.add_theme_font_size_override("font_size", 18)
	reset_all.pressed.connect(func() -> void:
		GameInput.reset_all_bindings()
		_refresh_binding_buttons()
		_rebind_status.text = I18n.t("已恢复默认键位。"))
	_input_box.add_child(reset_all)


func _begin_rebind(action: String) -> void:
	_rebind_action = action
	_rebind_started_ms = Time.get_ticks_msec()
	GameInput.set_capture_input(true)
	_rebind_status.text = I18n.t("请按下新键/手柄按钮…（Esc 取消）")
	var button: Button = _rebind_buttons.get(action, null)
	if button != null:
		button.text = I18n.t("请按下新键…")


func _finish_rebind() -> void:
	_rebind_action = ""
	GameInput.set_capture_input(false)
	_refresh_binding_buttons()


func _cancel_rebind(update_status: bool = true) -> void:
	if _rebind_action.is_empty():
		return
	_rebind_action = ""
	GameInput.set_capture_input(false)
	_refresh_binding_buttons()
	if update_status and _rebind_status != null:
		_rebind_status.text = I18n.t("已取消重绑。")


func _refresh_binding_buttons() -> void:
	for action in _rebind_buttons:
		var button: Button = _rebind_buttons[action]
		if is_instance_valid(button):
			button.text = GameInput.binding_text(action)


func _on_gyro_toggled(enabled: bool) -> void:
	GameInput.set_gyro_enabled(enabled)
	if _gyro_hint != null:
		_gyro_hint.text = _gyro_hint_text()


func _gyro_hint_text() -> String:
	if not GameInput.gyro_available:
		return "当前设备没有可用的陀螺仪/体感传感器；桌面默认无。接入手柄的体感传感器会自动识别。"
	if GameInput.gyro_enabled:
		return "已启用。倾斜设备转向；体感视角在驾驶页生效。设备方向不顺手时可点下方校准。"
	return "传感器可用，但当前关闭。"


func _on_device_changed(_device: String) -> void:
	if _device_status != null and is_instance_valid(_device_status):
		_device_status.text = GameInput.device_label()
	if _gyro_check != null and is_instance_valid(_gyro_check):
		_gyro_check.disabled = not GameInput.gyro_available
	if _gyro_hint != null and is_instance_valid(_gyro_hint):
		_gyro_hint.text = _gyro_hint_text()


func _on_external_setting_changed(section: String, key: String, value: Variant) -> void:
	if section != "video":
		return
	if key == "window_mode":
		_syncing = true
		var mode := int(value)
		_fullscreen.button_pressed = mode == GameSettings.WINDOW_FULLSCREEN
		_borderless.button_pressed = mode == GameSettings.WINDOW_BORDERLESS
		_syncing = false
		_refresh_adaptive_label()
	elif key == "fullscreen_adaptive":
		_syncing = true
		_adaptive_check.button_pressed = bool(value)
		_syncing = false
		_refresh_adaptive_label()
	elif key == "render_scale":
		_refresh_adaptive_label()
	elif key == "upscaler":
		_syncing = true
		_fill_upscaler_option()
		_syncing = false
		_refresh_adaptive_label()
		_refresh_display_hint()
	elif key == "vrs":
		_syncing = true
		_fill_vrs_option()
		_syncing = false
		_refresh_display_hint()
	elif key == "dynamic_target_fps":
		_syncing = true
		_fill_target_fps_option()
		_syncing = false
		_refresh_adaptive_label()


func _section_title(text: String) -> Label:
	return _text_label(text, 20, Color(0.98, 0.72, 0.35))


func _text_label(text: String, size: int, color: Color) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", size)
	label.add_theme_color_override("font_color", color)
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	return label
