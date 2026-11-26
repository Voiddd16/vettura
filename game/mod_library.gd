extends Node
## 游戏内 mod 库：递归扫描 `mods/`，自动识别车辆 / 赛道 / 车库。
##
## 统一的 mod 根目录（按优先级）：
## 1. `res://mods`：开发期工程内目录，导出时不会自动存在；
## 2. `user://mods`：游戏可写目录（Android / 沙盒环境的主目录），启动时自动创建；
## 3. 导出版可执行文件旁的 `mods/`：桌面版用户最容易找到的目录。
##
## 扫描规则是“命中即停”：一旦目录被识别成一个 mod，就不再深入，避免把
## car 的 skins/textures/sfx 当成新的 mod。未识别的包目录会继续向下递归，
## 所以 `mods/某车包/assettocorsa/content/cars/xxx` 这种结构也能找到。
##
## 分类：
## - `car`：有 `data.acd` / `data/car.ini`，且目录里有 `.kn5`；
## - `track`：有 `models*.ini`、`data/surfaces.ini`、`ai/fast_lane.ai` 或
##   `ui_track.json`；
## - `garage`：不是车/赛道，但目录里有 `.kn5`（适合大厅展示的车库 mod）。
##
## 本节点只做发现和元信息，不解析/实例化 KN5；真正加载由使用方按需调用
## `GarageMod.visual_plan()`、`AcCar`、`Kn5SceneBuilder`。

signal scanned

class ModEntry:
	var path: String
	var kind: String
	var display_name: String
	var folder_name: String
	var subtitle: String
	var preview_path: String
	var tags: PackedStringArray

	func kind_label() -> String:
		match kind:
			"car":
				return "车辆"
			"track":
				return "赛道"
			"garage":
				return "车库"
		return "未知"

	func is_car() -> bool:
		return kind == "car"

	func is_visual_scene() -> bool:
		return kind == "track" or kind == "garage"


const MOD_DIR := "mods"
const USER_MODS := "user://mods"
## 旧版车模导入目录：保留兼容，新的导入会写进 `user://mods/imported_cars/`。
const LEGACY_IMPORT_ROOT := "user://imported_cars"
const SCAN_MAX_DEPTH := 12

## 这些子目录永远不是 mod 根，递归时直接跳过，省掉大量文件系统往返。
const SKIP_DIRS := [
	".git", "__pycache__", "actools",
	"skins", "textures", "texture", "sfx", "sound", "sounds",
	"ui", "data", "extension", "animations", "driver", "maps",
]

var _entries: Array[ModEntry] = []
var _by_key: Dictionary = {}
var _roots: Array[String] = []
var _scanned := false
var _scanning := false


func _ready() -> void:
	rescan()


## 当前实际存在的 mod 根目录（诊断用）。
func roots() -> Array[String]:
	return _roots.duplicate()


## 扫描一次；重复调用会重建索引并发出 `scanned`。
func rescan() -> void:
	if _scanning:
		return
	_scanning = true
	# 每次重新发现根目录：运行中新建的 `user://mods`、旧导入目录或导出版
	# 旁边的 mods/ 都能在下一次 rescan 时进入索引。
	_refresh_roots()
	_entries.clear()
	_by_key.clear()
	for root: String in _roots:
		_scan_dir(root, 0)
	_entries.sort_custom(_sort_entries)
	_scanned = true
	_scanning = false
	scanned.emit()


func ensure_scanned() -> void:
	if not _scanned:
		rescan()


## 按类型列出：`car` / `track` / `garage`；空字符串表示全部。
func entries(kind: String = "") -> Array[ModEntry]:
	ensure_scanned()
	if kind.is_empty():
		return _entries.duplicate()
	var filtered: Array[ModEntry] = []
	for entry in _entries:
		if entry.kind == kind:
			filtered.append(entry)
	return filtered


func cars() -> Array[ModEntry]:
	return entries("car")


func tracks() -> Array[ModEntry]:
	return entries("track")


func garages() -> Array[ModEntry]:
	return entries("garage")


## 可作为大厅背景的 mod：车库 + 赛道（赛道同样可以当 3D 展厅）。
func visual_scenes() -> Array[ModEntry]:
	ensure_scanned()
	var filtered: Array[ModEntry] = []
	for entry in _entries:
		if entry.is_visual_scene():
			filtered.append(entry)
	return filtered


func counts() -> Dictionary:
	ensure_scanned()
	var result := {"car": 0, "track": 0, "garage": 0}
	for entry in _entries:
		result[entry.kind] = int(result.get(entry.kind, 0)) + 1
	return result


## 按路径取已扫描的条目；没命中也允许调用方用 `classify_path()` 临时识别。
func find(path: String) -> ModEntry:
	ensure_scanned()
	return _by_key.get(_key(path))


## 临时识别一个目录（不写入索引），用于“浏览目录”选中 mods 之外的位置。
func classify_path(path: String) -> ModEntry:
	if not DirAccess.dir_exists_absolute(path):
		return null
	var kind := _classify(path)
	if kind.is_empty():
		return null
	return _make_entry(path, kind)


func _refresh_roots() -> void:
	_roots.clear()
	var seen := {}
	if DirAccess.dir_exists_absolute("res://mods"):
		_add_root("res://mods", seen)

	# 可写 mod 目录永远存在，用户把 mod 丢进来即可被发现。
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(USER_MODS))
	_add_root(USER_MODS, seen)

	# 导出版桌面游戏：可执行文件旁边的 mods/。编辑器运行时指向 Godot.app，
	# 不参与扫描；Android 没有“旁边目录”的概念，也跳过。
	if not OS.has_feature("editor") and not OS.has_feature("mobile"):
		var exe_dir := OS.get_executable_path().get_base_dir()
		var beside_exe := exe_dir.path_join(MOD_DIR)
		if DirAccess.dir_exists_absolute(beside_exe):
			_add_root(beside_exe, seen)
		# macOS .app：可执行文件在 `Vettura.app/Contents/MacOS/Vettura`，把 mods
		# 放进包内会破坏代码签名（AC 的 extension/*.lua 会被 codesign 当成嵌套
		# 代码）。所以同时扫描 `.app` 同级的 mods/，玩家移动整个文件夹即可。
		if exe_dir.ends_with("/Contents/MacOS"):
			var bundle_parent := exe_dir.get_base_dir().get_base_dir().get_base_dir()
			var beside_bundle := bundle_parent.path_join(MOD_DIR)
			if DirAccess.dir_exists_absolute(beside_bundle):
				_add_root(beside_bundle, seen)

	if DirAccess.dir_exists_absolute(LEGACY_IMPORT_ROOT):
		_add_root(LEGACY_IMPORT_ROOT, seen)


func _add_root(path: String, seen: Dictionary) -> void:
	var key := _key(path)
	if seen.has(key):
		return
	seen[key] = true
	_roots.append(path)


func _scan_dir(path: String, depth: int) -> void:
	if depth > SCAN_MAX_DEPTH:
		return

	var kind := _classify(path)
	if not kind.is_empty():
		var entry := _make_entry(path, kind)
		var key := _key(path)
		if not _by_key.has(key):
			_by_key[key] = entry
			_entries.append(entry)
		# 命中 mod 根后不再向下，避免 skins/textures 被当成子 mod。
		return

	var dir := DirAccess.open(path)
	if dir == null:
		return
	for child: String in dir.get_directories():
		if _should_skip(child):
			continue
		_scan_dir(path.path_join(child), depth + 1)


func _classify(path: String) -> String:
	var kn5_files := _kn5_files(path)
	if _has_car_data(path) and not kn5_files.is_empty():
		return "car"

	var has_models := not _find_models_ini(path).is_empty()
	if kn5_files.is_empty() and not has_models:
		return ""

	var track_signature := has_models \
			or FileAccess.file_exists(path.path_join("data/surfaces.ini")) \
			or FileAccess.file_exists(path.path_join("ai/fast_lane.ai")) \
			or FileAccess.file_exists(path.path_join("ui/ui_track.json")) \
			or FileAccess.file_exists(path.path_join("ui_track.json")) \
			or FileAccess.file_exists(path.path_join("extension/ext_config.ini"))
	if track_signature:
		return "track"

	# 单模型车库 mod：同名 `<目录名>.kn5`，或目录里只有一个 KN5。
	# 多 KN5 且没有任何赛道/车库元信息的目录（例如 `alternate_kn5/` 这种
	# 备用车模合集）不硬分类，避免把一堆车模当展厅。
	var named := path.get_file() + ".kn5"
	if kn5_files.size() == 1 or kn5_files.has(named):
		return "garage"
	return ""


func _make_entry(path: String, kind: String) -> ModEntry:
	var entry := ModEntry.new()
	entry.path = path
	entry.kind = kind
	entry.folder_name = path.get_file()
	entry.display_name = entry.folder_name
	entry.subtitle = ""
	entry.preview_path = _preview_for(path, kind)

	var json := _read_json(path)
	if kind == "car":
		var name := String(json.get("name", "")).strip_edges()
		if not name.is_empty():
			entry.display_name = name
		entry.subtitle = _join_parts([
				String(json.get("brand", "")).strip_edges(),
				String(json.get("class", "")).strip_edges(),
		])
	else:
		var name := String(json.get("name", "")).strip_edges()
		if not name.is_empty():
			entry.display_name = name
		var pits := String(json.get("pitboxes", "")).strip_edges()
		entry.subtitle = _join_parts([
				String(json.get("country", "")).strip_edges(),
				"PIT %s" % pits if not pits.is_empty() else "",
		])

	var raw_tags: Variant = json.get("tags", [])
	if raw_tags is Array:
		for tag in raw_tags:
			entry.tags.append(String(tag))
	return entry


func _join_parts(parts: Array) -> String:
	var kept: PackedStringArray = PackedStringArray()
	for part: Variant in parts:
		var text := String(part).strip_edges()
		if not text.is_empty():
			kept.append(text)
	return " · ".join(kept)


func _read_json(path: String) -> Dictionary:
	for candidate in ["ui/ui_car.json", "ui/ui_track.json", "ui_track.json"]:
		var file := path.path_join(candidate)
		if not FileAccess.file_exists(file):
			continue
		var parsed: Variant = JSON.parse_string(FileAccess.get_file_as_string(file))
		if parsed is Dictionary:
			return parsed
	return {}


func _preview_for(path: String, kind: String) -> String:
	var candidates := ["ui/preview.png", "preview.png", "ui/badge.png", "ui/outline.png"]
	if kind == "car":
		candidates = ["ui/badge.png", "ui/preview.png", "preview.png"]
	for candidate: String in candidates:
		var file := path.path_join(candidate)
		if FileAccess.file_exists(file):
			return file
	return ""


func _has_car_data(path: String) -> bool:
	return FileAccess.file_exists(path.path_join("data.acd")) \
			or FileAccess.file_exists(path.path_join("data/car.ini"))


func _kn5_files(path: String) -> PackedStringArray:
	var result: PackedStringArray = PackedStringArray()
	var dir := DirAccess.open(path)
	if dir == null:
		return result
	for file: String in dir.get_files():
		if file.to_lower().ends_with(".kn5"):
			result.append(file)
	return result


func _find_models_ini(path: String) -> String:
	var dir := DirAccess.open(path)
	if dir == null:
		return ""
	var default_file := path.path_join("models.ini")
	if FileAccess.file_exists(default_file):
		return default_file
	var matches: Array[String] = []
	for file: String in dir.get_files():
		var lower := file.to_lower()
		if lower.begins_with("models") and lower.ends_with(".ini"):
			matches.append(path.path_join(file))
	matches.sort()
	return matches[0] if not matches.is_empty() else ""


func _should_skip(name: String) -> bool:
	var lower := name.to_lower()
	return name.begins_with(".") or name.begins_with("__") \
			or SKIP_DIRS.has(lower)


func _key(path: String) -> String:
	return ProjectSettings.globalize_path(path).trim_suffix("/")


func _sort_entries(a: ModEntry, b: ModEntry) -> bool:
	var rank := {"car": 0, "track": 1, "garage": 2}
	var ra := int(rank.get(a.kind, 9))
	var rb := int(rank.get(b.kind, 9))
	if ra != rb:
		return ra < rb
	var by_name := a.display_name.naturalcasecmp_to(b.display_name)
	if by_name != 0:
		return by_name < 0
	return a.path < b.path
