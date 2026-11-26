extends Node3D
## `FmodSystem` 的最小冒烟：启动 FMOD runtime、加载真实 AC 车模 bank、播放引擎事件。
##
## 用法（macOS，需先 `cargo build -p vettura-godot --features fmod`）：
##
##     /Applications/Godot.app/Contents/MacOS/Godot --headless --path .. \
##         res://gdext/fmod_probe.tscn
##
## 期望输出：
##     [fmod] start= true
##     [fmod] car= ... bank= <.bank 绝对路径>
##     [fmod] events= ... engine_ext
##     [fmod] play engine_ext= true
##     [fmod] PASS
##
## 没有 SDK 或 bank 不兼容时返回非 0，用来把“环境问题”和“代码问题”分开。

const CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"

var _audio: FmodSystem
var _ok := true

func _ready() -> void:
	_audio = FmodSystem.new()
	add_child(_audio)

	if not _audio.start(64):
		_fail("start failed: " + _audio.last_error())

	var car := AcCar.new()
	if not car.open(CAR_DIR):
		_fail("car.open failed")

	print("[fmod] soundbank=", car.soundbank_path())
	print("[fmod] has_sound=", car.has_sound())
	print("[fmod] events=", car.sound_event_names())

	if not _audio.load_car(car):
		_fail("load_car failed: " + _audio.last_error())
	print("[fmod] loaded car=", _audio.car_id(), " events=", _audio.event_names())

	if not _audio.play("engine_ext"):
		_fail("play(engine_ext) failed: " + _audio.last_error())
	print("[fmod] play(engine_ext)= true")

	print("[fmod] params(engine_ext)=", _audio.parameter_names("engine_ext"))
	if not _audio.set_vehicle_parameters("engine_ext", 6000.0, 0.5, 0.25, 30.0, 0.0, 3, 0.0):
		print("[fmod] warn: set_vehicle_parameters failed: ", _audio.last_error())
	else:
		print("[fmod] set_vehicle_parameters= true")

	await get_tree().process_frame
	print("[fmod] PASS")
	get_tree().quit(0 if _ok else 1)

func _fail(message: String) -> void:
	push_error("[fmod] " + message)
	_ok = false
