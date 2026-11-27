extends Node3D
## 游戏大厅无头冒烟：默认车库 + 默认车能否完整加载，弹层能否打开/关闭。
##
## 用法：
##
## ```bash
## godot --headless --path . res://game/lobby_smoke.tscn
## ```
##
## 本地没有 `mods/` 真车/车库时输出 SKIP 并退出 0——大厅本身允许缺资产降级。

const Lobby := preload("res://game/lobby.tscn")
const GarageMod := preload("res://game/garage_mod.gd")
const DEFAULT_CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const LOAD_TIMEOUT_MS := 120000

var _ok := true


func _ready() -> void:
	# 断言中文 UI 文案；固定 locale，避免系统语言为英文时误报。
	TranslationServer.set_locale("zh_CN")
	if GarageMod.visual_plan(GarageMod.DEFAULT_PATH).is_empty() \
			or not DirAccess.dir_exists_absolute(DEFAULT_CAR_DIR):
		print("[lobby-smoke] SKIP: 本地 mods/ 资产不完整")
		get_tree().quit(0)
		return

	var lobby: Node3D = Lobby.instantiate()
	add_child(lobby)
	var started := Time.get_ticks_msec()
	while lobby._loading.visible and Time.get_ticks_msec() - started < LOAD_TIMEOUT_MS:
		await get_tree().process_frame
	await get_tree().process_frame

	var counts := ModLibrary.counts()
	print("[lobby-smoke] mods: car=%d track=%d garage=%d" % [
			int(counts.get("car", 0)), int(counts.get("track", 0)),
			int(counts.get("garage", 0))])
	if int(counts.get("car", 0)) < 1:
		_fail("ModLibrary 没有发现车辆 mod")
	if ModLibrary.visual_scenes().is_empty():
		_fail("ModLibrary 没有发现可用于大厅的赛道/车库 mod")

	if lobby._garage_root.get_child_count() == 0:
		_fail("车库没有加载出任何模型")
	if lobby._car_node == null:
		_fail("车辆没有加载")
	if String(lobby._car_name).is_empty() or lobby._info.text.find("车辆：") == -1:
		_fail("大厅信息没有显示车辆名：%s" % lobby._info.text)
	if String(lobby._track_path).is_empty():
		_fail("大厅没有自动选择可用赛道")
	if lobby._drive_button.disabled:
		_fail("有车有赛道时「开始驾驶」仍被禁用")

	var camera := lobby.get_node("OrbitPivot/Camera3D") as Camera3D
	var position := camera.global_position
	if not (is_finite(position.x) and is_finite(position.y) and is_finite(position.z)):
		_fail("相机位置不是有限数：%s" % position)
	if position.distance_to(lobby._focus) < 2.0:
		_fail("相机离车辆太近，可能没有更新环绕半径")

	# 换车 / 换车库都要清掉旧节点，不能出现两套模型叠在一起。直接调加载函数，
	# 不走 UI 回调，避免冒烟测试改写用户保存的选择。
	await lobby._load_car(DEFAULT_CAR_DIR)
	lobby._finish_loading()
	if lobby._car_pivot.get_child_count() != 1:
		_fail("换车后 CarPivot 子节点数 = %d" % lobby._car_pivot.get_child_count())

	await lobby._load_garage(GarageMod.DEFAULT_PATH)
	if lobby._garage_root.get_child_count() != 1:
		_fail("换车库后 GarageRoot 子节点数 = %d" % lobby._garage_root.get_child_count())
	await lobby._load_car(lobby._preferred_car())
	lobby._finish_loading()

	lobby._on_settings_pressed()
	await get_tree().process_frame
	if lobby._overlay_layer == null:
		_fail("设置弹窗没有打开")
	else:
		lobby._close_overlay()
	await get_tree().process_frame

	lobby._on_garage_pressed()
	await get_tree().process_frame
	if lobby._overlay_layer == null:
		_fail("车库模组选择器没有打开")
	else:
		var panel: Control = lobby._overlay_layer.get_child(0)
		var list := panel.get_node("%ModList") as ItemList
		if list.item_count == 0:
			_fail("车库模组选择器列表为空")
		lobby._close_overlay()
	await get_tree().process_frame

	lobby._on_track_pressed()
	await get_tree().process_frame
	if lobby._overlay_layer == null:
		_fail("赛道选择器没有打开")
	else:
		var panel: Control = lobby._overlay_layer.get_child(0)
		var list := panel.get_node("%TrackList") as ItemList
		if list.item_count == 0:
			_fail("赛道选择器列表为空")
		lobby._close_overlay()
	await get_tree().process_frame

	if _ok:
		print("[lobby-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[lobby-smoke] FAIL")
		get_tree().quit(1)


func _fail(message: String) -> void:
	push_error("[lobby-smoke] " + message)
	_ok = false
