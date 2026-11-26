extends Node
## `ModLibrary` 无头冒烟：用合成文件验证递归发现与自动分类。
##
## 不依赖 `mods/` 真实资产；在 `user://mods/__mod_library_smoke__/` 下造
## 车辆包 / 赛道 / 单模型车库 / 多 KN5 杂项目录，断言分类结果后清理。
##
## ```bash
## godot --headless --path . res://game/mod_library_smoke.tscn
## ```

const ROOT := "user://mods/mod_library_smoke_tmp"

var _ok := true


func _ready() -> void:
	_remove_recursive(ProjectSettings.globalize_path(ROOT))
	_make_tree()
	ModLibrary.rescan()

	_check("car", ROOT.path_join("pack/assettocorsa/content/cars/smoke_car"))
	_check("track", ROOT.path_join("tracks/smoke_track"))
	_check("garage", ROOT.path_join("garages/smoke_garage"))
	if ModLibrary.find(ROOT.path_join("misc/alternate")) != null:
		_fail("多 KN5 且无元信息的目录不应被分类")

	_remove_recursive(ProjectSettings.globalize_path(ROOT))
	ModLibrary.rescan()
	if ModLibrary.find(ROOT.path_join("pack/assettocorsa/content/cars/smoke_car")) != null:
		_fail("清理后仍能在索引里找到测试 mod")

	if _ok:
		print("[mod-library-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[mod-library-smoke] FAIL")
		get_tree().quit(1)


func _make_tree() -> void:
	# 车辆：data.acd + .kn5，外层还有多层包目录。
	var car := ProjectSettings.globalize_path(
			ROOT.path_join("pack/assettocorsa/content/cars/smoke_car"))
	DirAccess.make_dir_recursive_absolute(car)
	_file(car.path_join("data.acd"))
	_file(car.path_join("smoke_car.kn5"))

	# 赛道：models.ini + 任意 KN5。
	var track := ProjectSettings.globalize_path(ROOT.path_join("tracks/smoke_track"))
	DirAccess.make_dir_recursive_absolute(track)
	_file(track.path_join("models.ini"), "[MODEL_0]\nFILE=track.kn5\n")
	_file(track.path_join("track.kn5"))

	# 单模型车库：目录名同名 KN5。
	var garage := ProjectSettings.globalize_path(ROOT.path_join("garages/smoke_garage"))
	DirAccess.make_dir_recursive_absolute(garage)
	_file(garage.path_join("smoke_garage.kn5"))

	# 杂项：多个 KN5 但没有车辆/赛道/车库元信息，不应硬分类。
	var misc := ProjectSettings.globalize_path(ROOT.path_join("misc/alternate"))
	DirAccess.make_dir_recursive_absolute(misc)
	_file(misc.path_join("one.kn5"))
	_file(misc.path_join("two.kn5"))


func _check(kind: String, path: String) -> void:
	var entry = ModLibrary.find(path)
	if entry == null:
		_fail("没有发现 %s：%s" % [kind, path])
	elif entry.kind != kind:
		_fail("%s 分类错误：期望 %s，实际 %s" % [path, kind, entry.kind])
	else:
		print("[mod-library-smoke] %s -> %s" % [kind, entry.display_name])


func _file(path: String, contents: String = "") -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		_fail("无法写入 %s" % path)
		return
	file.store_string(contents)
	file.close()


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
	push_error("[mod-library-smoke] " + message)
	_ok = false
