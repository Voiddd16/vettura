extends Node3D
## 自由驾驶页无头冒烟：真实赛道 + 真实车 + WorldBody + HUD 能否起来。
##
## ```bash
## godot --headless --path . res://game/drive_smoke.tscn
## ```
##
## 本地没有 mods/ 时输出 SKIP；不要求完成一圈，只验证加载、车辆生成、HUD 与
## 重置/暂停设置链路。

const Drive := preload("res://game/drive.tscn")
const LOAD_TIMEOUT_MS := 180000

var _ok := true


func _ready() -> void:
	# 断言中文 HUD；固定 locale，避免系统语言为英文时误报。
	TranslationServer.set_locale("zh_CN")
	if ModLibrary.tracks().is_empty() or ModLibrary.cars().is_empty():
		print("[drive-smoke] SKIP: 没有可用车辆/赛道")
		get_tree().quit(0)
		return

	var drive: Node3D = Drive.instantiate()
	add_child(drive)
	var started := Time.get_ticks_msec()
	while drive._loading.visible and Time.get_ticks_msec() - started < LOAD_TIMEOUT_MS:
		await get_tree().process_frame
	await get_tree().process_frame

	if drive._world == null or not drive._world.is_track_loaded():
		_fail("WorldBody 没有加载赛道")
	if drive._car == null:
		_fail("没有生成车辆")
	if drive._speed_label.text.is_empty() or drive._rpm_label.text.is_empty():
		_fail("HUD 没有刷新")
	if drive._camera_button.text.find("视角") == -1:
		_fail("相机按钮文本异常：%s" % drive._camera_button.text)

	# 真的给 1.5 秒油门，确认 WorldBody → VehicleBody 物理链路在驾驶页里是通的。
	if drive._car != null:
		var start: Vector3 = drive._car.global_position
		Input.action_press("game_throttle")
		await get_tree().create_timer(1.5, true).timeout
		Input.action_release("game_throttle")
		await get_tree().process_frame
		if start.distance_to(drive._car.global_position) < 1.0:
			_fail("油门 1.5 秒车辆没有移动")
		else:
			print("[drive-smoke] moved %.2f m" % start.distance_to(drive._car.global_position))

	if drive._car != null:
		drive._on_reset_pressed()
		await get_tree().process_frame
		if not (is_finite(drive._car.global_position.x)
				and is_finite(drive._car.global_position.y)
				and is_finite(drive._car.global_position.z)):
			_fail("重置后车辆位置不是有限数")

	# 设置弹窗会暂停整棵树；用 process_always 计时器等一帧，验证暂停时 UI 仍能跑。
	drive._on_settings_pressed()
	if not get_tree().paused:
		_fail("设置弹窗没有暂停游戏")
	await get_tree().create_timer(0.2, true).timeout
	if drive._settings_layer == null:
		_fail("暂停后设置弹窗没有保持存在")
	drive._close_settings()
	await get_tree().create_timer(0.05, true).timeout
	if get_tree().paused:
		_fail("关闭设置后没有恢复游戏")

	if _ok:
		print("[drive-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[drive-smoke] FAIL")
		get_tree().quit(1)


func _fail(message: String) -> void:
	push_error("[drive-smoke] " + message)
	_ok = false
