extends Node3D
## FX 冒烟探针：Daytona + F1，强制雨天，驱动全部 FX 节点。
##
## 运行：
##   /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/fx_probe.tscn
## 桌面窗口下可截图观察雨丝/湿路面/草/灯光/水花。

const CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const TRACK_DIR := "res://mods/daytona_2017"
const TRACK_LAYOUT := "roadcourse"

var _world: WorldBody
var _car: VehicleBody
var _camera: Camera3D
var _fx: Node3D
var _elapsed := 0.0
var _failed := false
var _shot := false
const SHOT_PATH := "user://fx_probe.png"


func _ready() -> void:
	var track_dir := OS.get_environment("FX_TRACK")
	if track_dir.is_empty():
		track_dir = TRACK_DIR
	var car_dir := OS.get_environment("FX_CAR")
	if car_dir.is_empty():
		car_dir = CAR_DIR
	var layout := OS.get_environment("FX_LAYOUT")
	if layout.is_empty():
		layout = TRACK_LAYOUT
	elif layout == "auto":
		layout = ""

	_world = WorldBody.new()
	add_child(_world)
	if not _world.load_track(track_dir, layout):
		push_error("[fx-probe] load_track failed")
		get_tree().quit(1)
		return

	_car = _world.spawn_vehicle_from_path(car_dir)
	if _car == null:
		push_error("[fx-probe] spawn_vehicle_from_path failed")
		get_tree().quit(1)
		return

	_world.set_weather({
		"rain": 0.85,
		"wetness": 0.9,
		"fog": 0.25,
		"sun": 0.35,
		"wind": Vector3(2.0, 0.0, 1.0),
	})

	_camera = Camera3D.new()
	_camera.name = "FxProbeCamera"
	_camera.fov = 65.0
	add_child(_camera)
	_camera.current = true
	_frame_camera()

	_fx = load("res://game/fx/fx_runtime.gd").new()
	_fx.name = "FxRuntime"
	add_child(_fx)
	_fx.attach(_world, [_car], _camera)
	print("[fx-probe] attached rain=0.85 wetness=0.9")


func _physics_process(delta: float) -> void:
	if _car == null or _world == null:
		return
	_elapsed += delta
	var braking := _elapsed >= 3.0 and _elapsed < 4.5
	_car.set_controls(0.0 if braking else 0.55, 1.0 if braking else 0.0, 0.0, false, false, true, true)
	_frame_camera()
	if _elapsed >= 4.0 and not _shot:
		_shot = true
		if DisplayServer.get_name() != "headless":
			var image := get_viewport().get_texture().get_image()
			image.save_png(SHOT_PATH)
			print("[fx-probe] screenshot -> ", ProjectSettings.globalize_path(SHOT_PATH))
	if _elapsed >= 6.0 and not _failed:
		print("[fx-probe] PASS elapsed=%.1f speed=%.1f km/h" % [_elapsed, _car.speed_kmh()])
		get_tree().quit(0)


func _frame_camera() -> void:
	if _camera == null or _car == null:
		return
	var behind := _car.global_transform.basis.z * 7.0
	var up := _car.global_transform.basis.y * 2.2
	_camera.global_position = _car.global_position + behind + up
	_camera.look_at(_car.global_position + Vector3(0.0, 0.4, 0.0), Vector3.UP)
