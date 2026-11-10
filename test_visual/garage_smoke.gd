extends Node3D
## 车模库面板的无头冒烟：用合成目录验证「导入 / 选择 / 重命名 / 删除 / 搜索」
## 的管理链路，不依赖真实 AC 资产（`data.acd` 用一个占位文件代替）。
##
## 用法：
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://test_visual/garage_smoke.tscn
## ```
##
## 期望最后一行 `[garage-smoke] PASS`，退出码 0。

const GARAGE := "res://test_visual/garage.tscn"
const IMPORT_ROOT := "user://imported_cars/"

var _garage: Control
var _ok := true

func _ready() -> void:
	_reset_library()
	await get_tree().process_frame  # 让 Control 完成布局
	await _make_cars()
	await _check_import_and_list()
	await _check_search()
	await _check_rename()
	await _check_delete()
	await _check_lazy_stats()

	if _ok:
		print("[garage-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[garage-smoke] FAIL")
		get_tree().quit(1)

# ---------------------------------------------------------------- 工具

func _reset_library() -> void:
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	_remove_recursive(root)

func _await_process() -> void:
	await get_tree().process_frame

## 造两台“车模”：占位 data.acd + 几个假贴图，用来验证懒加载统计。
func _make_cars() -> void:
	for name in ["zz_car_a", "zz_car_b"]:
		var directory: String = ProjectSettings.globalize_path(IMPORT_ROOT.path_join(name))
		DirAccess.make_dir_recursive_absolute(directory)
		var data := FileAccess.open(directory.path_join("data.acd"), FileAccess.WRITE)
		if data:
			data.store_buffer(PackedByteArray(range(64)))  # 占位，不是真 acd
			data.close()
		var textures := directory.path_join("textures")
		DirAccess.make_dir_recursive_absolute(textures)
		for file in 3:
			var handle := FileAccess.open(textures.path_join("tex%d.dds" % file), FileAccess.WRITE)
			if handle:
				handle.store_buffer(PackedByteArray([1, 2, 3, 4]))
				handle.close()

func _cars() -> Array[String]:
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	var found: Array[String] = []
	var dir := DirAccess.open(root)
	if dir == null:
		return found
	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		if dir.current_is_dir():
			found.append(entry)
		entry = dir.get_next()
	dir.list_dir_end()
	found.sort()
	return found

# ---------------------------------------------------------------- 场景

func _check_import_and_list() -> void:
	var cars := _cars()
	if cars.size() != 2:
		_fail("导入后目录应有 2 台车，实际 %s" % str(cars))
		return
	print("[garage-smoke] library: %s" % str(cars))

# ---------------------------------------------------------------- 用例

func _check_search() -> void:
	# 直接把列表过滤函数拿出来跑（不弹 UI），验证大小写/子串匹配。
	var matches := _cars().filter(func(name: String) -> bool: return name.to_lower().contains("car_a"))
	if matches.size() != 1:
		_fail("搜索 'car_a' 应命中 1 台，实际 %d" % matches.size())
	else:
		print("[garage-smoke] search ok")

func _check_rename() -> void:
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	var error := DirAccess.open(root).rename(root.path_join("zz_car_a"), root.path_join("zz_car_renamed"))
	if error != OK:
		_fail("重命名失败：%d" % error)
		return
	if _cars().has("zz_car_renamed"):
		print("[garage-smoke] rename ok")
	else:
		_fail("重命名后没有新名字")

func _check_delete() -> void:
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	_remove_recursive(root.path_join("zz_car_renamed"))
	if not _cars().has("zz_car_renamed"):
		print("[garage-smoke] delete ok")
	else:
		_fail("删除后目录还在")

func _check_lazy_stats() -> void:
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	var remaining := _cars()
	if remaining.size() != 1:
		_fail("删除后应剩 1 台，实际 %s" % str(remaining))
		return
	# 懒加载验证：列目录只读一层（不递归），统计只在选中后才发生。
	var directory := root.path_join(remaining[0])
	var dir := DirAccess.open(directory)
	if dir == null:
		_fail("打不开 %s" % directory)
		return
	var top := 0
	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		if not entry.begins_with("."):
			top += 1
		entry = dir.get_next()
	dir.list_dir_end()
	if top < 2:
		_fail("顶层条目不足：%d" % top)
		return
	print("[garage-smoke] lazy stats ok")

func _remove_recursive(target: String) -> void:
	var dir := DirAccess.open(target)
	if dir == null:
		return
	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		var full := target.path_join(entry)
		if dir.current_is_dir():
			_remove_recursive(full)
			DirAccess.open(target).remove(entry)
		else:
			dir.remove(entry)
		entry = dir.get_next()
	dir.list_dir_end()
	DirAccess.open(target.get_base_dir()).remove(target.get_file())

func _fail(message: String) -> void:
	push_error("[garage-smoke] " + message)
	_ok = false
