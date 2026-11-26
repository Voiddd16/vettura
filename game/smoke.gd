extends Node3D
## `main.tscn` 的无头交互冒烟：把页面实例化出来，直接驱动它的回调，
## 验证“触摸输入 → HUD / 车轮 / 相机模式”这条链路没有运行时报错。
##
## 用法：
##
## ```bash
## godot --headless --path . res://game/smoke.tscn --quit-after 240
## ```
##
## 期望输出 `[visual-smoke] PASS`，退出码 `0`。

const MAIN_SCENE := preload("res://game/main.tscn")

var _main: Node3D
var _ok := true

func _ready() -> void:
	_main = MAIN_SCENE.instantiate()
	add_child(_main)

	# `test/` 真车资产不在库里：没有车可加载时只验证视角循环这一段。
	if not _main.has_car():
		push_warning("[visual-smoke] SKIP: 没有可用车模（test/ 资源缺失），只测相机循环")
		await _check_camera_modes()
		print("[visual-smoke] PASS（部分）")
		get_tree().quit(0)
		return

	# 给 `_process` 一次机会把相机/HUD 刷到最新状态。
	await get_tree().process_frame
	await get_tree().process_frame

	_check_wheels()
	await _check_touch()
	await _check_camera_modes()

	if _ok:
		print("[visual-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[visual-smoke] FAIL")
		get_tree().quit(1)

func _fail(message: String) -> void:
	push_error("[visual-smoke] " + message)
	_ok = false

func _check_wheels() -> void:
	if _main._wheels.size() != 4:
		_fail("expected 4 bound wheel dummies, got %s" % _main._wheels.size())

func _check_touch() -> void:
	# 直接走按钮信号，而不是绕过控件调用脚本回调——这样场景里的连线也被验证了。
	var throttle_button := _main.get_node("CanvasLayer/TouchOverlay/ThrottleArea/ThrottleButton") as Button
	var brake_button := _main.get_node("CanvasLayer/TouchOverlay/BrakeArea/BrakeButton") as Button
	throttle_button.emit_signal("button_down")
	brake_button.emit_signal("button_down")
	await get_tree().create_timer(0.15).timeout

	if _main._throttle <= 0.5:
		_fail("throttle button did not drive _throttle")
	if _main._brake <= 0.5:
		_fail("brake button did not drive _brake")

	# 左侧刹车/右侧油门是同一个 HUD 文本域里的两个状态位，直接看文本最省事。
	var info := _main.get_node("HUD/Panel/Columns/Column/InfoLabel") as Label
	if info.text.find("油门：100%") == -1 or info.text.find("刹车：100%") == -1:
		_fail("HUD did not reflect touch state: %s" % info.text)

	# 松手后状态必须能回落，否则真机上手指离开后车会永远算在加油。
	throttle_button.emit_signal("button_up")
	brake_button.emit_signal("button_up")
	await get_tree().process_frame
	await get_tree().process_frame
	if _main._throttle > 0.01 or _main._brake > 0.01:
		_fail("touch state did not clear on release")

func _check_camera_modes() -> void:
	var mode_button := _main.get_node("HUD/Panel/Columns/ModeButton") as Button
	var camera := _main.get_node("Camera3D") as Camera3D

	# 页面默认从“车后”开始，按钮再点一次应该循环到“驾驶位”。
	var expected_texts := ["视角：驾驶位", "视角：绕车", "视角：前机盖", "视角：车后"]
	for expected in expected_texts:
		mode_button.emit_signal("pressed")
		await get_tree().process_frame

		if mode_button.text != expected:
			_fail("camera mode text mismatch: expected %s, got %s" % [expected, mode_button.text])

		var position := camera.global_position
		if not (is_finite(position.x) and is_finite(position.y) and is_finite(position.z)):
			_fail("camera position is not finite in mode %s" % expected)

		print("[visual-smoke] mode=%s camera=%s" % [expected, position])
