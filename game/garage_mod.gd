extends RefCounted
## 车库（track）模组的无 UI 工具层：校验、主模型计划与展示名。
## 目录发现/分类由 `mod_library.gd` 统一负责。
##
## 大厅只认两件事：
## 1. 目录里**直接**有 `.kn5`（例如 `mods/ltk_csr_garage/ltk_csr_garage.kn5`）；
## 2. 或者有 `models.ini` / `models_*.ini`，按 AC 的放置表加载全部视觉模型
##    （多模型赛道 mod 也能当车库用）。
##
## 不递归扫描整个目录：车模目录动辄上百 MB、几千个文件，大厅打开时必须瞬间
## 返回。主 `.kn5` 只看根目录；`.kn5` 放进子目录的 mod 需要作者把主模型放回根。

const DEFAULT_PATH := "res://mods/ltk_csr_garage"


## 一个可展示的模型：KN5 绝对路径 + 已转换成 Godot 帧的放置变换。
class VisualEntry:
	var path: String
	var transform: Transform3D


## 是否是存在的目录。
static func is_dir(path: String) -> bool:
	return not path.is_empty() and DirAccess.dir_exists_absolute(path)


## 主模型计划；返回空数组表示不是可用的车库模组。
##
## 车模目录（有 `data.acd`）会被排除，避免用户把车当成车库选进来。
static func visual_plan(path: String) -> Array[VisualEntry]:
	var plan: Array[VisualEntry] = []
	if not is_dir(path):
		return plan
	if FileAccess.file_exists(path.path_join("data.acd")) \
			or FileAccess.file_exists(path.path_join("data/car.ini")):
		return plan

	# 多模型赛道：按 models.ini 的放置表逐个数。AC 的 INI 很宽松（引号、
	# 注释、Windows 换行、键名大小写都见过），这里用最小解析器而不是
	# ConfigFile——后者遇到真实 mod 会直接报 parse error 并中断整个文件。
	var models := _find_models_ini(path)
	if not models.is_empty():
		for section: Dictionary in _parse_ini_sections(FileAccess.get_file_as_string(models)):
			var name := String(section.get("__name", "")).to_upper()
			if not name.begins_with("MODEL_"):
				continue
			var file := _unquote(String(section.get("FILE", "")))
			if file.is_empty():
				continue
			var lower := file.to_lower()
			if lower.contains("physics") or lower.contains("collision") \
					or lower.contains("collider") or lower.contains("spawns") \
					or lower.contains("sounds") or lower == "acstobjects.kn5":
				continue
			var full := path.path_join(file)
			if not FileAccess.file_exists(full):
				continue
			var entry := VisualEntry.new()
			entry.path = full
			entry.transform = _placement(
					_parse_vec3(_unquote(String(section.get("POSITION", "0,0,0")))),
					_parse_vec3(_unquote(String(section.get("ROTATION", "0,0,0")))))
			plan.append(entry)
		# models.ini 存在但没有任何可用模型时，继续尝试根目录的主 KN5。
		if not plan.is_empty():
			return plan

	# 单模型车库：根目录体积最大的 `.kn5`（跳过 lod/阴影这类辅助模型）。
	var main := main_kn5(path)
	if not main.is_empty():
		var entry := VisualEntry.new()
		entry.path = main
		entry.transform = Transform3D.IDENTITY
		plan.append(entry)
	return plan


## 主 `.kn5`：优先同名文件，否则根目录里体积最大的非辅助模型。
static func main_kn5(path: String) -> String:
	if not is_dir(path):
		return ""
	var preferred := path.path_join(path.get_file() + ".kn5")
	if FileAccess.file_exists(preferred):
		return preferred

	var best := ""
	var best_size := -1
	var dir := DirAccess.open(path)
	if dir == null:
		return ""
	var files := dir.get_files()
	for file: String in files:
		var lower := file.to_lower()
		if not lower.ends_with(".kn5"):
			continue
		if lower.contains("collision") or lower.contains("lod") \
				or lower.contains("shadow") or lower.contains("physics"):
			continue
		var full := path.path_join(file)
		var handle := FileAccess.open(full, FileAccess.READ)
		if handle == null:
			continue
		var size := handle.get_length()
		handle.close()
		if size > best_size:
			best = full
			best_size = size
	return best


## 展示名：优先 `ui/ui_track.json` 的 `name`，否则目录名。
static func display_name(path: String) -> String:
	for candidate in ["ui/ui_track.json", "ui_track.json"]:
		var file := path.path_join(candidate)
		if not FileAccess.file_exists(file):
			continue
		var parsed: Variant = JSON.parse_string(FileAccess.get_file_as_string(file))
		if parsed is Dictionary:
			var name := String(parsed.get("name", "")).strip_edges()
			if not name.is_empty():
				return name
	return path.get_file()


## 预览图路径；没有时返回空字符串。
static func preview_path(path: String) -> String:
	for candidate in ["ui/preview.png", "preview.png", "ui/outline.png"]:
		var file := path.path_join(candidate)
		if FileAccess.file_exists(file):
			return file
	return ""


## 极简 INI 解析：返回 [{ "__name": 节名, KEY: VALUE }, ...]。
##
## 只服务于 `models*.ini` 的读取，不追求通用性；空行、`;` / `#` 注释、
## `KEY=VALUE` 两侧空白与 Windows 换行都能处理。
static func _parse_ini_sections(text: String) -> Array[Dictionary]:
	var sections: Array[Dictionary] = []
	var current: Dictionary = {}
	for raw_line: String in text.split("\n"):
		var line := raw_line.strip_edges()
		if line.is_empty() or line.begins_with(";") or line.begins_with("#"):
			continue
		if line.begins_with("[") and line.ends_with("]"):
			if not current.is_empty():
				sections.append(current)
			current = {"__name": line.substr(1, line.length() - 2).strip_edges()}
			continue
		var equals := line.find("=")
		if equals <= 0 or current.is_empty():
			continue
		current[line.substr(0, equals).strip_edges().to_upper()] = \
				line.substr(equals + 1).strip_edges()
	if not current.is_empty():
		sections.append(current)
	return sections


static func _unquote(text: String) -> String:
	var value := text.strip_edges()
	if value.length() >= 2 and value.begins_with("\"") and value.ends_with("\""):
		value = value.substr(1, value.length() - 2)
	return value.strip_edges()


static func _find_models_ini(path: String) -> String:
	var default_file := path.path_join("models.ini")
	if FileAccess.file_exists(default_file):
		return default_file
	var dir := DirAccess.open(path)
	if dir == null:
		return ""
	var matches: Array[String] = []
	for file: String in dir.get_files():
		var lower := file.to_lower()
		if lower.begins_with("models") and lower.ends_with(".ini"):
			matches.append(path.path_join(file))
	matches.sort()
	return matches[0] if not matches.is_empty() else ""


static func _parse_vec3(text: String) -> Vector3:
	var parts := text.split(",", false)
	if parts.size() < 3:
		return Vector3.ZERO
	return Vector3(
			parts[0].strip_edges().to_float(),
			parts[1].strip_edges().to_float(),
			parts[2].strip_edges().to_float())


## AC 放置帧 → Godot 帧：`C · T · R · C`，C = scale(-1, 1, -1)。
##
## 与 Rust `vettura_assets::euler_deg_to_mat4` 同一套数学（那边是物理帧，
## 这里给 KN5 节点用同一帧，因为 KN5 顶点也已经转换到 Godot 帧）。
static func _placement(position: Vector3, rotation_deg: Vector3) -> Transform3D:
	var rotation := Basis.from_euler(
			Vector3(deg_to_rad(rotation_deg.x), deg_to_rad(rotation_deg.y),
					deg_to_rad(rotation_deg.z)),
			EULER_ORDER_XYZ)
	var ac := Transform3D(rotation, position)
	var conjugate := Transform3D(Basis.from_scale(Vector3(-1.0, 1.0, -1.0)), Vector3.ZERO)
	return conjugate * ac * conjugate
