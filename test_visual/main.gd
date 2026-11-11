extends Node3D
## 主项目测试性开发页面：**默认车辆模型浮空预览**。
##
## 这一页只验证视觉/交互链路，不接入赛道物理：
##
## - `AcCar` 打开默认 AC 车目录并实例化主模型（`vettura-native` 负责）；
## - 模型挂在 `CarPivot` 下、抬高，形成“浮空状态”，并缓慢自转方便观察；
## - 左半屏触摸 = 刹车，右半屏触摸 = 油门（真实多点触摸；桌面上点击/拖拽也生效）；
## - 右上角按钮循环切换相机视角：车后 / 驾驶位 / 绕车 / 前机盖；
## - 油门/刹车会体现在车轮滚动与左上角 HUD 上，方便确认输入真的进来了。

const DEFAULT_CAR_DIR := "res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"

## 车模库存放目录（`user://` 可写；res:// 在运行时是只读的）。由
## `test_visual/garage.gd` 的车模库面板管理：导入 / 选择 / 重命名 / 删除 / 搜索。
const IMPORT_ROOT := "user://imported_cars/"

## 模型基准点不在地面上时，用这个高度把整车托起来。
const FLOAT_HEIGHT := 1.2
## 浮空展示时的自转速度（rad/s）。
const FLOATING_TURN_SPEED := 0.35

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

## 车库面板的 CanvasLayer 层级：HUD / TouchOverlay 各占一层（默认 1），
## 车库要盖在它们上面，所以单独给 2。
const GARAGE_LAYER := 2

const SafeArea := preload("res://test_visual/safe_area.gd")

@onready var _car_pivot: Node3D = $CarPivot
@onready var _camera: Camera3D = $Camera3D
@onready var _mode_button: Button = $HUD/Panel/Columns/ModeButton
@onready var _info_label: Label = $HUD/Panel/Columns/Column/InfoLabel
@onready var _import_button: Button = $HUD/Panel/Columns/Column/ImportButton
@onready var _hud_panel: MarginContainer = $HUD/Panel
@onready var _hud: CanvasLayer = $HUD
@onready var _touch_overlay: Control = $CanvasLayer/TouchOverlay

## 打开中的车模库（连着它的 CanvasLayer），null = 没开。
var _garage_layer: CanvasLayer = null

var _car_root: Node3D
var _wheels: Array[Node3D] = []
var _car_name := "（未加载）"

var _throttle := false
var _brake := false
var _wheel_spin := 0.0

var _camera_mode := 0
## 绕车模式的角度，需要跨帧累积，所以放这里而不是 `update_camera()` 的局部量。
var _orbit_angle := 0.0

## 车模是否加载成功（`test/` 资源缺失时为 `false`）。
func has_car() -> bool:
	return _car_root != null

func _ready() -> void:
	_load_default_car()
	_update_hud()
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)

## 手机旋转 / 刘海位置 / 分屏变化时都会触发；容器边距兜住安全区，
## 控件（HUD 按钮与信息）永远不会被屏幕边框或挖孔挡住。
func _apply_safe_area() -> void:
	SafeArea.apply(_hud_panel)

func _process(delta: float) -> void:
	_car_pivot.rotate_y(FLOATING_TURN_SPEED * delta)
	_update_wheels(delta)
	_update_camera(delta)
	_update_hud()

## 打开默认车目录并把主模型实例化进 `CarPivot`。
func _load_default_car() -> void:
	# `test/` 是本地大体积资源，不入库（见 README）。缺资产时降级成警告并跳过
	# 加载：预览页、其它冒烟场景和 `--install-android-build-template` 之类的
	# 工程级命令都不该因为一台没下载的车把控制台刷成一片 ERROR。
	if not DirAccess.dir_exists_absolute(DEFAULT_CAR_DIR):
		push_warning("[visual] 默认车目录不存在，跳过模型加载：%s\n" % DEFAULT_CAR_DIR
				+ "（放置教程见 README「本地测试资源」，或用左上角「导入车模」直接选一个）")
		_update_hud()
		return

	# GDExtension 还没注册（例如首次打开、缓存未生成）时也不刷错误。
	if not ClassDB.class_exists("AcCar"):
		push_warning("[visual] GDExtension 未注册（AcCar 不存在），跳过模型加载")
		_update_hud()
		return

	_load_car(DEFAULT_CAR_DIR)

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
	_wheel_spin = 0.0

	# KN5 的 `WHEEL_*` dummy 在 `AcCar.instantiate()` 后仍按原名保留。
	var wheel_names := ["WHEEL_LF", "WHEEL_RF", "WHEEL_LR", "WHEEL_RR"]
	for wheel_name in wheel_names:
		var wheel := _car_root.find_child(wheel_name, true, false) as Node3D
		if wheel == null:
			push_warning("[visual] wheel dummy not found: %s" % wheel_name)
			continue
		_wheels.append(wheel)

	print("[visual] car loaded: ", _car_name, " wheels=", _wheels.size(),
			" children=", _car_root.get_child_count())
	_update_hud()
	return true

## 「导入车模」按钮：打开车模库面板（导入 / 选择 / 重命名 / 删除 / 搜索）。
func _on_import_button_pressed() -> void:
	if _garage_layer != null:
		return
	var garage_scene: PackedScene = load("res://test_visual/garage.tscn")
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
	_import_button.disabled = true

## 车模库里选了一台车：加载并关掉面板。
func _on_garage_car_selected(car_path: String) -> void:
	if not _load_car(car_path):
		_warn_import("加载失败：%s" % car_path)
		return
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
	_touch_overlay.visible = true
	_import_button.disabled = false

func _warn_import(message: String) -> void:
	push_warning("[visual] " + message)

## 油门/刹车 → 车轮角速度。浮空预览不做悬挂/打滑，只用来确认输入可达。
func _update_wheels(delta: float) -> void:
	if _throttle:
		_wheel_spin += WHEEL_SPIN_SPEED * delta
	elif _brake:
		_wheel_spin = move_toward(_wheel_spin, 0.0, WHEEL_BRAKE_DECEL * delta)
	else:
		_wheel_spin = move_toward(_wheel_spin, 0.0, WHEEL_COAST_DECEL * delta)

	# 车头朝 `-Z`，所以向前滚动绕局部 `+X` 是负方向。
	for wheel in _wheels:
		if wheel == null:
			continue
		wheel.rotate_x(-_wheel_spin * delta)

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

func _update_hud() -> void:
	_mode_button.text = MODE_TEXTS[_camera_mode]
	_info_label.text = (
		"车辆：%s\n"
		+ "油门（右半屏）：%s\n"
		+ "刹车（左半屏）：%s\n"
		+ "车轮角速度：%.1f rad/s"
	) % [
		_car_name,
		"ON" if _throttle else "off",
		"ON" if _brake else "off",
		_wheel_spin,
	]

## 右上角视角按钮：车后 → 驾驶位 → 绕车 → 前机盖。
func _on_camera_button_pressed() -> void:
	_camera_mode = (_camera_mode + 1) % MODE_TEXTS.size()
	_update_hud()

func _on_throttle_pressed() -> void:
	_throttle = true

func _on_throttle_released() -> void:
	_throttle = false

func _on_brake_pressed() -> void:
	_brake = true

func _on_brake_released() -> void:
	_brake = false
