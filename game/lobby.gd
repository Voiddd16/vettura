extends Node3D
## 游戏大厅：在可替换的“车库（track）模组”里展示当前车辆，慢速环绕。
##
## 数据流：
## - 车库目录来自 `GameSettings["garage"]["mod_path"]`，默认 `mods/ltk_csr_garage`；
## - `garage_mod.gd` 决定加载计划：单 KN5，或按 `models.ini` 拼多个视觉模型；
## - 车辆目录来自 `GameSettings["garage"]["car_path"]`，空则用默认 F1 2008；
## - 车模库面板（`garage.tscn`）与车库模组面板（`garage_mod_picker.tscn`）都只
##   负责选路径，选中后写回 `GameSettings`，本页再异步重建 3D。
##
## 加载使用 `Kn5SceneBuilder` 分帧推进，55 MB 的车库模型不会把首帧卡死；
## 车辆放在车库的 `AC_PIT_0` / `AC_START_0` 等 dummy 上，没有 dummy 时退回
## 场景包围盒中心。

const SafeArea := preload("res://game/safe_area.gd")
const GarageMod := preload("res://game/garage_mod.gd")

const DEFAULT_CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"

const GARAGE_LAYER := 2
const SETTINGS_LAYER := 3

## AC 车库/赛道里常见的起跑与维修区 dummy，按优先级找第一个。
const SPAWN_MARKERS := [
	"AC_PIT_0", "AC_START_0", "AC_HOTLAP_START_0",
	"AC_PIT_1", "AC_START_1", "AC_PIT_2", "AC_START_2",
]

@onready var _garage_root: Node3D = $GarageRoot
@onready var _car_pivot: Node3D = $CarPivot
@onready var _orbit_pivot: Node3D = $OrbitPivot
@onready var _camera: Camera3D = $OrbitPivot/Camera3D
@onready var _sun: DirectionalLight3D = $Sun
@onready var _key_light: OmniLight3D = $CarKeyLight
@onready var _fill_light: OmniLight3D = $GarageFill

@onready var _ui: Control = %Root
@onready var _safe_margin: MarginContainer = %Margin
@onready var _title: Label = %Title
@onready var _subtitle: Label = %Subtitle
@onready var _info: Label = %InfoLabel
@onready var _drive_button: Button = %DriveButton
@onready var _car_button: Button = %CarButton
@onready var _garage_button: Button = %GarageButton
@onready var _settings_button: Button = %SettingsButton
@onready var _fullscreen_button: Button = %FullscreenButton
@onready var _quit_button: Button = %QuitButton
@onready var _loading: Control = %Loading
@onready var _loading_bar: ProgressBar = %LoadingBar
@onready var _loading_label: Label = %LoadingLabel
@onready var _hint: Label = $UI/Root/Margin/Layout/Hint

var _garage_node: Node3D
var _car_node: Node3D
var _garage_name := ""
var _car_name := "（未选择车辆）"

var _focus := Vector3(0.0, 0.5, 0.0)
var _orbit_angle := 0.0
var _orbit_radius := 7.0
var _orbit_height := 1.65
var _orbit_paused_until := 0.0
## 设置项的缓存：`_process` 每帧读一次 `GameSettings.get_value()` 会做两次
## 字典查找（section → key），改由 `changed` 信号在设置变化时更新。
var _orbit_speed := 0.18
var _orbit_radius_target := 7.0
## 每次新建加载请求 +1；旧协程看到 token 变化就停手，避免两套模型叠在一起。
var _load_token := 0
var _overlay_layer: CanvasLayer
var _quit_dialog: ConfirmationDialog


func _ready() -> void:
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)
	_drive_button.pressed.connect(_on_drive_pressed)
	_car_button.pressed.connect(_on_car_pressed)
	_garage_button.pressed.connect(_on_garage_pressed)
	_settings_button.pressed.connect(_on_settings_pressed)
	_fullscreen_button.pressed.connect(_on_fullscreen_pressed)
	_quit_button.pressed.connect(_on_quit_pressed)
	GameSettings.changed.connect(_on_setting_changed)
	GameInput.push_nav_handler(_handle_nav)
	GameInput.device_changed.connect(_on_device_changed)
	_orbit_speed = float(GameSettings.get_value("garage", "orbit_speed", 0.18))
	_orbit_radius_target = float(GameSettings.get_value("garage", "orbit_radius", 7.0))
	_orbit_radius = _orbit_radius_target
	_apply_shadows()
	_update_device_hint()
	_build_quit_dialog()
	_update_info("正在准备车库…")
	await _boot()


func _exit_tree() -> void:
	GameInput.pop_nav_handler(_handle_nav)


func _handle_nav(kind: String) -> bool:
	if _quit_dialog != null and _quit_dialog.visible:
		_quit_dialog.hide()
		return true
	if _overlay_layer != null:
		_close_overlay()
		return true
	if kind == "menu":
		_on_settings_pressed()
		return true
	_request_quit()
	return true


func _build_quit_dialog() -> void:
	_quit_dialog = ConfirmationDialog.new()
	_quit_dialog.title = "退出 Vettura"
	_quit_dialog.dialog_text = "确定要退出游戏吗？"
	_quit_dialog.ok_button_text = "退出"
	_quit_dialog.cancel_button_text = "继续玩"
	_quit_dialog.confirmed.connect(func() -> void: get_tree().quit())
	add_child(_quit_dialog)


func _request_quit() -> void:
	if _quit_dialog == null:
		_build_quit_dialog()
	_quit_dialog.popup_centered()
	_quit_dialog.get_ok_button().grab_focus()


func _boot() -> void:
	# 先让首帧把大厅 UI 与加载遮罩画出来，再开始解析大 KN5。
	await get_tree().process_frame
	var garage := _preferred_garage()
	var garage_ok := await _load_garage(garage)
	if not garage_ok and garage != GarageMod.DEFAULT_PATH:
		push_warning("[lobby] 车库模组不可用，回退默认：%s" % GarageMod.DEFAULT_PATH)
		garage_ok = await _load_garage(GarageMod.DEFAULT_PATH)
	if garage_ok:
		await _load_car(_preferred_car())
	else:
		_loading_label.text = "没有可用车库。仍可打开设置或选择其它车库模组。"
		_update_info("没有可用车库，请点「车库模组」选择。")
		await get_tree().process_frame
	_finish_loading()


func _process(delta: float) -> void:
	# 自动环绕：滑杆实时改速度；拖动画面后暂停 2.5 秒，避免和用户抢镜头。
	var now := Time.get_ticks_msec() / 1000.0
	if now >= _orbit_paused_until:
		_orbit_angle += _orbit_speed * delta

	# 右摇杆 / 陀螺仪自由视角：左右转环绕，上下抬降镜头。
	var pads := Input.get_connected_joypads()
	if not pads.is_empty():
		var look := Vector2(
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_X),
			Input.get_joy_axis(pads[0], JOY_AXIS_RIGHT_Y))
		if look.length() > 0.18:
			_orbit_angle += look.x * delta * 1.8
			_orbit_height = clampf(_orbit_height - look.y * delta * 2.2, 0.6, 4.5)
			_orbit_paused_until = now + 2.5
	_orbit_angle += GameInput.gyro_look_delta.x
	_orbit_height = clampf(_orbit_height - GameInput.gyro_look_delta.y * 1.5, 0.6, 4.5)

	_orbit_radius = lerpf(_orbit_radius, _orbit_radius_target, 1.0 - exp(-8.0 * delta))

	_orbit_pivot.global_position = _focus
	_orbit_pivot.rotation = Vector3(0.0, _orbit_angle, 0.0)
	_camera.position = Vector3(0.0, _orbit_height, _orbit_radius)
	_camera.look_at(_focus, Vector3.UP)


func _unhandled_input(event: InputEvent) -> void:
	if _overlay_layer != null or _loading.visible:
		return
	var mouse_button := event as InputEventMouseButton
	if mouse_button != null:
		if mouse_button.pressed and mouse_button.button_index == MOUSE_BUTTON_WHEEL_UP:
			_orbit_radius = clampf(_orbit_radius - 0.5, 3.0, 10.0)
		elif mouse_button.pressed and mouse_button.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			_orbit_radius = clampf(_orbit_radius + 0.5, 3.0, 10.0)
		return
	var motion := event as InputEventMouseMotion
	if motion != null and (motion.button_mask & MOUSE_BUTTON_MASK_LEFT) != 0:
		_orbit_angle -= motion.relative.x * 0.006
		_orbit_paused_until = Time.get_ticks_msec() / 1000.0 + 2.5
		return
	var drag := event as InputEventScreenDrag
	if drag != null:
		_orbit_angle -= drag.relative.x * 0.006
		_orbit_paused_until = Time.get_ticks_msec() / 1000.0 + 2.5


func _apply_safe_area() -> void:
	SafeArea.apply(_safe_margin)


# ---------------------------------------------------------------- 加载

func _preferred_garage() -> String:
	var path := String(GameSettings.get_value("garage", "mod_path", GarageMod.DEFAULT_PATH))
	if not GarageMod.visual_plan(path).is_empty():
		return path
	if not GarageMod.visual_plan(GarageMod.DEFAULT_PATH).is_empty():
		return GarageMod.DEFAULT_PATH
	# 默认目录不在时，自动用 mods 库里第一个可展示的赛道/车库。
	var scenes := ModLibrary.visual_scenes()
	return scenes[0].path if not scenes.is_empty() else ""


func _preferred_car() -> String:
	var path := String(GameSettings.get_value("garage", "car_path", ""))
	if not path.is_empty() and DirAccess.dir_exists_absolute(path):
		return path
	if DirAccess.dir_exists_absolute(DEFAULT_CAR_DIR):
		return DEFAULT_CAR_DIR
	# 默认车不在时，自动用 mods 库里发现的第一辆车。
	var cars := ModLibrary.cars()
	return cars[0].path if not cars.is_empty() else DEFAULT_CAR_DIR


## 按 `garage_mod.gd` 的计划异步加载一个车库；返回是否至少有一个模型成功。
func _load_garage(path: String) -> bool:
	var token := ++_load_token
	_clear_car()
	_clear_garage()
	_garage_name = GarageMod.display_name(path)
	_update_info("正在加载车库：%s" % _garage_name)
	_show_loading("正在加载车库…", 0.0)

	var plan := GarageMod.visual_plan(path)
	if plan.is_empty():
		push_warning("[lobby] 无效车库模组：%s" % path)
		_loading_label.text = "车库模组无效：%s" % path
		_update_info("车库模组无效，请重新选择。")
		return false
	if not ClassDB.class_exists("Kn5SceneBuilder"):
		push_warning("[lobby] GDExtension 未注册（Kn5SceneBuilder 不存在）")
		_loading_label.text = "GDExtension 未注册，无法加载车库模型。"
		_update_info("GDExtension 未注册，无法加载车库。")
		return false

	var loaded := 0
	for index in plan.size():
		if token != _load_token:
			return false
		var entry: GarageMod.VisualEntry = plan[index]
		var wrapper := Node3D.new()
		wrapper.name = "GarageModel_%d" % index
		wrapper.transform = entry.transform
		_garage_root.add_child(wrapper)

		var builder := Kn5SceneBuilder.new()
		if not builder.start(entry.path):
			push_warning("[lobby] KN5 打不开：%s" % entry.path)
			wrapper.queue_free()
			continue
		while not builder.step(Kn5SceneBuilder.STEP_BUDGET, wrapper):
			if token != _load_token:
				builder.cancel()
				return false
			_set_progress((float(index) + builder.progress()) / float(plan.size()))
			await get_tree().process_frame
		if builder.root() == null:
			push_warning("[lobby] KN5 没有创建节点：%s" % entry.path)
			wrapper.queue_free()
		else:
			loaded += 1
		_set_progress(float(index + 1) / float(plan.size()))

	if loaded == 0:
		_loading_label.text = "车库模型没有加载成功：%s" % path
		_update_info("车库模型没有加载成功，请换一个车库模组。")
		return false
	_garage_node = _garage_root
	_place_car_anchor()
	_set_progress(1.0)
	_update_info()
	print("[lobby] garage loaded: %s models=%d plan=%d" % [_garage_name, loaded, plan.size()])
	return true


## 异步加载车辆；车辆模型挂在 `CarPivot` 下，位姿来自车库 dummy。
func _load_car(path: String) -> bool:
	var token := ++_load_token
	_clear_car()
	_show_loading("正在加载车辆…", 0.0)
	_update_info("正在加载车辆…")

	if not DirAccess.dir_exists_absolute(path):
		push_warning("[lobby] 车目录不存在：%s" % path)
		_loading_label.text = "车目录不存在：%s" % path
		_update_info("车辆加载失败：目录不存在。")
		return false
	if not ClassDB.class_exists("AcCar"):
		push_warning("[lobby] GDExtension 未注册（AcCar 不存在）")
		_loading_label.text = "GDExtension 未注册，无法加载车辆。"
		_update_info("GDExtension 未注册，无法加载车辆。")
		return false

	var car := AcCar.new()
	if not car.open(path):
		push_warning("[lobby] AcCar.open 失败：%s" % path)
		_loading_label.text = "AcCar.open 失败：%s" % path
		_update_info("车辆数据打不开，请重新选择车辆。")
		return false
	var model_path := car.model_path()
	if model_path.is_empty():
		push_warning("[lobby] 车模主 KN5 缺失：%s" % path)
		_loading_label.text = "车模主 KN5 缺失。"
		_update_info("车模缺少主 KN5，请重新选择车辆。")
		return false

	var builder := Kn5SceneBuilder.new()
	if not builder.start(model_path):
		_loading_label.text = "车模 KN5 打不开：%s" % model_path
		_update_info("车模 KN5 打不开，请重新选择车辆。")
		return false
	while not builder.step(Kn5SceneBuilder.STEP_BUDGET, _car_pivot):
		if token != _load_token:
			builder.cancel()
			return false
		_set_progress(builder.progress())
		await get_tree().process_frame
	_car_node = builder.root()
	if _car_node == null:
		_loading_label.text = "车模没有创建节点。"
		_update_info("车模没有创建节点，请重新选择车辆。")
		return false

	_car_name = car.display_name()
	if _car_name.is_empty():
		_car_name = path.get_file()
	_update_focus()
	_update_info()
	print("[lobby] car loaded: %s from %s" % [_car_name, path])
	return true


func _clear_garage() -> void:
	# 先摘再排队释放：queue_free 要到帧末才真正删除，如果新的车库模型在同一帧
	# 完成加载，find_child 可能先找到旧车库的 AC_* dummy。
	for child in _garage_root.get_children():
		_garage_root.remove_child(child)
		child.queue_free()
	_garage_node = null


func _clear_car() -> void:
	# 分帧加载被取消时 `_car_node` 还是 null，但已经建了一半的节点挂在 CarPivot
	# 下；统一清空子节点，避免换车后出现两套模型。
	for child in _car_pivot.get_children():
		_car_pivot.remove_child(child)
		child.queue_free()
	_car_node = null
	_car_name = "（未选择车辆）"


## 把 CarPivot 放到车库的起跑/维修区 dummy 上；没有 dummy 用包围盒中心。
func _place_car_anchor() -> void:
	var pose := _spawn_pose()
	# 车模原点通常就是地面接触点，抬高 2 cm 避免和地面 z-fighting。
	pose.origin.y += 0.02
	_car_pivot.global_transform = pose
	_update_focus()


func _spawn_pose() -> Transform3D:
	var marker_pose: Transform3D
	var marker_found := false
	for marker_name: String in SPAWN_MARKERS:
		var marker := _garage_root.find_child(marker_name, true, false) as Node3D
		if marker == null:
			continue
		# AC 的 AC_START_x 自身带 0.5 m 高，真正落地点是它的第一个子 dummy。
		for child in marker.get_children():
			var child_3d := child as Node3D
			if child_3d != null:
				marker_pose = child_3d.global_transform
				marker_found = true
				break
		if not marker_found:
			marker_pose = marker.global_transform
			marker_found = true
		break

	# 车库 mod 的 AC_PIT/AC_START 常常在**店外**（ltk_csr_garage 就是这样）。
	# 有独立室内地面（`2ROAD`，面积合理）而 marker 不在上面时，改用室内中心，
	# 车辆才是“摆在车库里”。
	var interior := _interior_bounds()
	if interior.has_volume() and (not marker_found or not _on_floor(interior, marker_pose.origin)):
		return _interior_pose(interior)
	if marker_found:
		return marker_pose

	var bounds := _combined_aabb(_garage_root)
	if bounds.has_volume():
		var center := bounds.get_center()
		center.y = bounds.position.y
		push_warning("[lobby] 车库没有 AC_* 起跑点，车辆放到包围盒中心")
		return Transform3D(Basis.IDENTITY, center)
	return Transform3D.IDENTITY


## 室内地面：AC 的 `2ROAD` 通常是维修区/车库地坪；面积过大的同名单可能是
## 赛道本身，不当作室内。
func _interior_bounds() -> AABB:
	var box := AABB()
	var has := false
	var stack: Array[Node] = [_garage_root]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		var mesh := node as MeshInstance3D
		if mesh != null and String(mesh.name).begins_with("2ROAD"):
			var candidate := mesh.global_transform * mesh.get_aabb()
			box = candidate if not has else box.merge(candidate)
			has = true
		stack.append_array(node.get_children())
	if not has:
		return AABB()
	var area := box.size.x * box.size.z
	if area <= 0.0 or area > 4000.0:
		return AABB()
	return box


func _on_floor(box: AABB, position: Vector3) -> bool:
	return position.x >= box.position.x and position.x <= box.end.x \
			and position.z >= box.position.z and position.z <= box.end.z


func _interior_pose(box: AABB) -> Transform3D:
	var center := box.get_center()
	center.y = box.end.y
	# 车头朝向门窗：窗/门 mesh 的水平方向就是镜头想看到的那面墙。找不到时
	# 沿房间长边放，至少不会横着卡在墙里。
	var direction := _door_direction(center)
	if direction == Vector3.ZERO:
		direction = Vector3(0.0, 0.0, -1.0) if box.size.z >= box.size.x \
				else Vector3(-1.0, 0.0, 0.0)
	# 车模本地前方是 -Z；world_forward = (-sin(yaw), 0, -cos(yaw))。
	var yaw := atan2(-direction.x, -direction.z)
	return Transform3D(Basis(Vector3.UP, yaw), center)


func _door_direction(center: Vector3) -> Vector3:
	var best := Vector3.ZERO
	var best_distance := 0.0
	var stack: Array[Node] = [_garage_root]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		var mesh := node as MeshInstance3D
		if mesh != null:
			var name := String(mesh.name).to_lower()
			if name.contains("window") or name.contains("door"):
				var point := (mesh.global_transform * mesh.get_aabb()).get_center()
				var flat := Vector3(point.x - center.x, 0.0, point.z - center.z)
				if flat.length() > best_distance:
					best = flat.normalized()
					best_distance = flat.length()
		stack.append_array(node.get_children())
	return best


func _combined_aabb(root: Node) -> AABB:
	var result := AABB()
	var has_bounds := false
	var stack: Array[Node] = [root]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		var mesh := node as MeshInstance3D
		if mesh != null and mesh.mesh != null:
			var box := mesh.global_transform * mesh.get_aabb()
			result = box if not has_bounds else result.merge(box)
			has_bounds = true
		stack.append_array(node.get_children())
	return result


func _update_focus() -> void:
	# 视点略低于车顶：车辆在画面里整体偏上，底部不会被大厅的底部信息栏遮住。
	_focus = _car_pivot.global_position + Vector3(0.0, 0.5, 0.0)
	_orbit_pivot.global_position = _focus
	# 主光跟随车辆：车库内没有烘焙 GI，车身边缘需要这两盏补光。
	_key_light.global_position = _focus + Vector3(3.5, 4.0, 3.2)
	_fill_light.global_position = _focus + Vector3(-3.0, 2.6, -2.0)


# ---------------------------------------------------------------- UI

func _show_loading(text: String, progress: float) -> void:
	_loading.visible = true
	_loading_label.text = text
	_set_progress(progress)


func _set_progress(value: float) -> void:
	_loading_bar.value = clampf(value, 0.0, 1.0) * 100.0


func _finish_loading() -> void:
	_loading.visible = false
	_update_info()
	_drive_button.grab_focus()


func _update_info(message: String = "") -> void:
	if _info == null:
		return
	var counts := ModLibrary.counts()
	var lines := PackedStringArray([
			"车辆：%s" % _car_name,
			"车库：%s" % (_garage_name if not _garage_name.is_empty() else "未加载"),
			"模组：%d 辆车 · %d 条赛道 · %d 个展示车库"
					% [int(counts.get("car", 0)), int(counts.get("track", 0)),
					int(counts.get("garage", 0))],
	])
	if not message.is_empty():
		lines.append(message)
	_info.text = "\n".join(lines)
	_title.text = "VETTURA"
	_subtitle.text = "GARAGE  /  游戏大厅"


func _apply_shadows() -> void:
	var enabled := bool(GameSettings.get_value("video", "shadows", true))
	_sun.shadow_enabled = enabled
	_key_light.shadow_enabled = false  # 近距补光开阴影只会增加噪点与开销。


func _on_setting_changed(section: String, key: String, value: Variant) -> void:
	if section == "video" and key == "shadows":
		_apply_shadows()
	elif section == "garage" and key == "orbit_speed":
		_orbit_speed = float(value)
	elif section == "garage" and key == "orbit_radius":
		_orbit_radius_target = float(value)


func _on_drive_pressed() -> void:
	get_tree().change_scene_to_file("res://game/main.tscn")


func _on_car_pressed() -> void:
	var garage_scene: PackedScene = load("res://game/garage.tscn")
	var panel := garage_scene.instantiate() as Control
	panel.car_selected.connect(_on_car_selected)
	panel.closed.connect(_close_overlay)
	_open_overlay(panel, GARAGE_LAYER)


func _on_garage_pressed() -> void:
	var picker_scene: PackedScene = load("res://game/garage_mod_picker.tscn")
	var panel := picker_scene.instantiate() as Control
	panel.mod_selected.connect(_on_garage_selected)
	panel.closed.connect(_close_overlay)
	_open_overlay(panel, GARAGE_LAYER)


func _on_settings_pressed() -> void:
	var popup_scene: PackedScene = load("res://game/settings_popup.tscn")
	var panel := popup_scene.instantiate() as Control
	panel.closed.connect(_close_overlay)
	_open_overlay(panel, SETTINGS_LAYER)


func _on_fullscreen_pressed() -> void:
	GameSettings.toggle_fullscreen()


func _open_overlay(panel: Control, layer_index: int) -> void:
	if _overlay_layer != null:
		return
	var layer := CanvasLayer.new()
	layer.layer = layer_index
	layer.add_child(panel)
	add_child(layer)
	_overlay_layer = layer
	_ui.visible = false


func _close_overlay() -> void:
	if _overlay_layer != null:
		_overlay_layer.queue_free()
		_overlay_layer = null
	_ui.visible = true
	_drive_button.grab_focus()


func _on_device_changed(_device: String) -> void:
	_update_device_hint()


func _update_device_hint() -> void:
	if _hint == null:
		return
	match GameInput.active_device():
		"gamepad":
			_hint.text = "手柄：左摇杆旋转 · 右摇杆/体感微调 · A 确认 · Start 设置 · B 返回"
		"touch":
			_hint.text = "拖动画面旋转镜头 · 滚轮/双指缩放 · 车库与车辆都可以随时替换"
		_:
			_hint.text = "拖动画面旋转镜头 · 滚轮缩放 · Enter 确认 · Esc 设置 · F11 全屏"


func _on_car_selected(car_path: String) -> void:
	GameSettings.set_value("garage", "car_path", car_path)
	GameSettings.save()
	_close_overlay()
	await _load_car(car_path)
	_finish_loading()


func _on_garage_selected(mod_path: String) -> void:
	GameSettings.set_value("garage", "mod_path", mod_path)
	GameSettings.save()
	_close_overlay()
	var ok := await _load_garage(mod_path)
	if not ok and mod_path != GarageMod.DEFAULT_PATH:
		ok = await _load_garage(GarageMod.DEFAULT_PATH)
		if ok:
			# 记下回退结果，下次启动不再反复尝试坏目录。
			GameSettings.set_value("garage", "mod_path", GarageMod.DEFAULT_PATH)
			GameSettings.save()
	if ok:
		await _load_car(_preferred_car())
	_finish_loading()


func _on_quit_pressed() -> void:
	_request_quit()
