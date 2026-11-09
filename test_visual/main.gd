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

@onready var _car_pivot: Node3D = $CarPivot
@onready var _camera: Camera3D = $Camera3D
@onready var _mode_button: Button = $HUD/ModeButton
@onready var _info_label: Label = $HUD/InfoLabel

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
				+ "（放置教程见 README「本地测试资源」）")
		_update_hud()
		return

	# GDExtension 还没注册（例如首次打开、缓存未生成）时也不刷错误。
	if not ClassDB.class_exists("AcCar"):
		push_warning("[visual] GDExtension 未注册（AcCar 不存在），跳过模型加载")
		_update_hud()
		return

	var car := AcCar.new()
	if not car.open(DEFAULT_CAR_DIR):
		push_error("[visual] AcCar.open failed: %s" % DEFAULT_CAR_DIR)
		return

	_car_name = car.display_name()
	var root := car.instantiate(_car_pivot)
	if root == null:
		push_error("[visual] AcCar.instantiate failed: %s" % DEFAULT_CAR_DIR)
		return

	_car_root = root
	_car_pivot.position.y = FLOAT_HEIGHT

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
