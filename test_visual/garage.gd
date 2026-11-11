extends Control
## 车模库管理面板：导入 / 选择 / 重命名 / 删除 / 搜索。
##
## 数据模型很薄：`user://imported_cars/` 下一级目录就是一台车，列表渲染只需要
## 目录名；尺寸、`.kn5` 统计、物理参数这些**贵的东西一律懒加载**——只有真正选中
## 一台车时才去扫它的文件，因此十几台车的车库打开和单台一样快。
##
## 用法：
##
## ```gdscript
## var garage_scene: PackedScene = preload("res://test_visual/garage.tscn")
## var garage: Control = garage_scene.instantiate()
## garage.car_selected.connect(_on_garage_car_selected)
## add_child(garage)
## ```

const IMPORT_ROOT := "user://imported_cars/"

const SafeArea := preload("res://test_visual/safe_area.gd")

## 单台车的元信息。`size_bytes` / `kn5_count` 留空 = 还没算过（懒加载）。
class GarageCar:
	var name: String
	var path: String
	var size_bytes: int = -1
	var kn5_count: int = -1
	var data_present: bool = false

## 选择了一台车（"载入预览"）：把目录路径交出去。
signal car_selected(car_path: String)
## 面板自己关掉了（「返回」）：宿主用来恢复被它藏起来的 HUD 等 UI。
signal closed
## 需要刷新列表（导入/删除/重命名后）。
signal library_changed

@onready var _search: LineEdit = %SearchBox
@onready var _list: ItemList = %CarList
@onready var _info: Label = %InfoLabel
@onready var _import_button: Button = %ImportButton
@onready var _load_button: Button = %LoadButton
@onready var _rename_button: Button = %RenameButton
@onready var _delete_button: Button = %DeleteButton
@onready var _open_button: Button = %OpenButton
@onready var _close_button: Button = %CloseButton
@onready var _margin: MarginContainer = %Margin

## 全部车（不过滤），与 `_list` 的下标一一对应。
var _cars: Array[GarageCar] = []
## 搜索关键字（小写）。
var _filter := ""

func _ready() -> void:
	_search.text_changed.connect(_on_search_changed)
	_list.item_selected.connect(_on_car_selected)
	_import_button.pressed.connect(_on_import_pressed)
	_load_button.pressed.connect(_on_load_pressed)
	_rename_button.pressed.connect(_on_rename_pressed)
	_delete_button.pressed.connect(_on_delete_pressed)
	_open_button.pressed.connect(_on_open_pressed)
	_close_button.pressed.connect(_close)
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)
	refresh()

## 手机旋转 / 分屏变化时重算安全区边距：面板各按钮不被刘海、挖孔挡住。
func _apply_safe_area() -> void:
	SafeArea.apply(_margin)

## 重新扫描 `user://imported_cars/`（只读一级目录，不做深扫描）。
func refresh() -> void:
	_cars.clear()
	var root := ProjectSettings.globalize_path(IMPORT_ROOT)
	DirAccess.make_dir_recursive_absolute(root)

	var dir := DirAccess.open(root)
	if dir == null:
		_warn("打不开车模目录：%s" % root)
		return
	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		# 只列一级目录；隐藏目录（例如 macOS 的 `.TemporaryItems`）跳过。
		if dir.current_is_dir() and not entry.begins_with(".") and not entry.begins_with("__"):
			var car := GarageCar.new()
			car.name = entry
			car.path = root.path_join(entry)
			_cars.append(car)
		entry = dir.get_next()
	dir.list_dir_end()

	_cars.sort_custom(func(a: GarageCar, b: GarageCar) -> bool:
			return a.name.naturalcasecmp_to(b.name) < 0)
	_apply_filter()
	library_changed.emit()

## 搜索：只重排已有数据，绝不碰磁盘。
func _on_search_changed(text: String) -> void:
	_filter = text.strip_edges().to_lower()
	_apply_filter()

## 把过滤后的车灌进列表控件。
func _apply_filter() -> void:
	_list.clear()
	for car in _cars:
		if _filter.is_empty() or car.name.to_lower().contains(_filter):
			_list.add_item(car.name)
	_select_first()

func _select_first() -> void:
	if _list.item_count > 0:
		_list.select(0)
		_on_car_selected(0)
	else:
		_info.text = "还没有车模。点「导入」选一个目录或 zip。"
		_load_button.disabled = true
		_rename_button.disabled = true
		_delete_button.disabled = true
		_open_button.disabled = true

## 选中一台车：只有这一刻才去扫它的文件（懒加载的核心）。
func _on_car_selected(index: int) -> void:
	if index < 0 or index >= _list.item_count:
		return
	var car := _visible_car(index)
	if car == null:
		return
	_ensure_stats(car)

	var lines: PackedStringArray = PackedStringArray([
			"名称：%s" % car.name,
			"路径：%s" % car.path,
			"大小：%s" % _human_bytes(car.size_bytes),
			"网格：%d 个 .kn5" % car.kn5_count,
			"物理数据：%s" % ("data.acd" if car.data_present else "缺失（只能看不能开）"),
	])
	_info.text = "\n".join(lines)
	for button in [_load_button, _rename_button, _delete_button, _open_button]:
		button.disabled = false

## 取“当前可见的第 index 行”对应的车（搜索时列表是过滤视图）。
func _visible_car(index: int) -> GarageCar:
	var label := _list.get_item_text(index)
	for car in _cars:
		if car.name == label:
			return car
	return null

## 惰性统计：目录大小 / .kn5 数量 / data.acd 是否存在。
func _ensure_stats(car: GarageCar) -> void:
	if car.size_bytes >= 0:
		return
	car.size_bytes = 0
	car.kn5_count = 0
	car.data_present = FileAccess.file_exists(car.path.path_join("data.acd"))

	var dir := DirAccess.open(car.path)
	if dir == null:
		return
	var queue: Array = [car.path]
	# 防御：极端深的目录不要递归爆栈。
	while not queue.is_empty():
		var directory: String = queue.pop_front()
		var scanning := DirAccess.open(directory)
		if scanning == null:
			continue
		scanning.list_dir_begin()
		var entry := scanning.get_next()
		while entry != "":
			var full := directory.path_join(entry)
			if scanning.current_is_dir():
				queue.append(full)
			else:
				var file := FileAccess.open(full, FileAccess.READ)
				if file != null:
					car.size_bytes += file.get_length()
					file.close()
				if full.to_lower().ends_with(".kn5") and full.get_file() != "lod_gen.kn5":
					car.kn5_count += 1
			entry = scanning.get_next()
		scanning.list_dir_end()

## 「导入」：桌面用内置文件对话框（会调起系统原生面板）；Android 用系统
## 文件选择器（SAF）。
##
## 为什么 Android 不能直接用内置 `FileDialog`：
## - Android 11+ 分区存储下应用没有全盘扫描权限，`ACCESS_FILESYSTEM`
##   列不出 `/storage/emulated/0`，点开也是空列表；
## - 内置对话框是个内嵌 `Window`，在部分安卓设备/引擎版本上不渲染（表现为
##   点了按钮没反应）。
## 系统 `file_dialog_show` 由引擎封装：用户选完后，引擎把选中的文件/目录
## 拷进应用缓存目录并回传真实路径，不需要申请存储权限。
func _on_import_pressed() -> void:
	if OS.has_feature("android"):
		_import_via_system_dialog()
	else:
		_import_via_embedded_dialog()


func _import_via_embedded_dialog() -> void:
	var picker := FileDialog.new()
	picker.file_mode = FileDialog.FILE_MODE_OPEN_ANY
	picker.access = FileDialog.ACCESS_FILESYSTEM
	picker.current_dir = OS.get_system_dir(OS.SYSTEM_DIR_DESKTOP)
	picker.title = "选择车模目录或 zip"
	picker.dir_selected.connect(_import_from.bind(picker))
	picker.file_selected.connect(_import_from.bind(picker))
	add_child(picker)
	picker.popup_centered_ratio(0.7)


## Android 系统文件选择器：目录（整包车模）与文件（.zip）都允许选。
## 4.7 签名：`(title, current_directory, filename, show_hidden, mode,
## filters: PackedStringArray, callback, parent_window_id = 0)`。
func _import_via_system_dialog() -> void:
	var error := DisplayServer.file_dialog_show(
			"选择车模目录或 zip",
			ProjectSettings.globalize_path(IMPORT_ROOT),
			"",                                  # 初始文件名：不预选
			true,                                # show hidden
			DisplayServer.FILE_DIALOG_MODE_OPEN_ANY,
			PackedStringArray(),                 # filters：空 = 不筛类型
			_on_system_dialog_result)
	if error != OK:
		push_warning("[garage] 系统文件选择器不可用（错误码 %d），回落内置对话框" % error)
		_import_via_embedded_dialog()


## `file_dialog_show` 回调：`(status, 选中路径, 过滤器下标)`。
func _on_system_dialog_result(status: bool, paths: PackedStringArray,
		_filter_index: int) -> void:
	if not status or paths.is_empty():
		print("[garage] 导入已取消")
		return
	_import_from(paths[0], null)

## 把外部路径复制/解压成 `user://imported_cars/<名字>/`，然后刷新列表。
## `picker` 只在桌面分支有值（用于关闭对话框）；Android 回调进来时是 null。
func _import_from(source: String, picker: FileDialog = null) -> void:
	if picker != null:
		picker.queue_free()
	var name: String = source.get_file().get_basename()
	var target := ProjectSettings.globalize_path(IMPORT_ROOT.path_join(name))

	if source.to_lower().ends_with(".zip"):
		if not _unpack(source, target):
			_warn("解压失败：%s" % source)
			return
	elif FileAccess.file_exists(source.path_join("data.acd")) or \
			DirAccess.dir_exists_absolute(source.path_join("data")):
		_copy_directory(source, target)
	elif source.to_lower().ends_with(".kn5"):
		# 单个 .kn5：旁边只要再有一个 data 目录就能开；裸 .kn5 也允许导入看模型。
		_copy_file(source, target.path_join(name + ".kn5"))
	else:
		_warn("不是车模：%s" % source)
		return

	refresh()
	# 选中刚导入的那台。
	for index in _list.item_count:
		if _visible_car(index) != null and _visible_car(index).name == name:
			_list.select(index)
			_on_car_selected(index)
			break

func _unpack(archive: String, target: String) -> bool:
	var reader := ZIPReader.new()
	if reader.open(archive) != OK:
		return false
	DirAccess.make_dir_recursive_absolute(target)
	for entry in reader.get_entries():
		if entry.filename.ends_with("/"):
			DirAccess.make_dir_recursive_absolute(target.path_join(entry.filename))
			continue
		var where: String = target.path_join(entry.filename)
		DirAccess.make_dir_recursive_absolute(where.get_base_dir())
		var file := FileAccess.open(where, FileAccess.WRITE)
		if file != null:
			file.store_buffer(reader.read_file(entry.filename))
			file.close()
		else:
			push_warning("[garage] 写入失败：%s" % where)
	reader.close()
	return true

## 目录递归拷贝；同路径（源 == 目标）时跳过。
func _copy_directory(source: String, target: String) -> void:
	if source == target:
		return
	if DirAccess.dir_exists_absolute(target):
		_warn("同名车模已存在：%s" % name_of(target))
		return
	DirAccess.make_dir_recursive_absolute(target)
	var dir := DirAccess.open(source)
	if dir == null:
		return
	var queue: Array = [[source, target]]
	while not queue.is_empty():
		var pair: Array = queue.pop_front()
		var from: String = pair[0]
		var to: String = pair[1]
		var listing := DirAccess.open(from)
		if listing == null:
			continue
		listing.list_dir_begin()
		var entry := listing.get_next()
		while entry != "":
			var source_path := from.path_join(entry)
			var target_path := to.path_join(entry)
			if listing.current_is_dir():
				DirAccess.make_dir_recursive_absolute(target_path)
				queue.append([source_path, target_path])
			else:
				_copy_file(source_path, target_path)
			entry = listing.get_next()
		listing.list_dir_end()

func _copy_file(source: String, target: String) -> void:
	var reader := FileAccess.open(source, FileAccess.READ)
	if reader == null:
		return
	var writer := FileAccess.open(target, FileAccess.WRITE)
	if writer != null:
		writer.store_buffer(reader.get_buffer(reader.get_length()))
		writer.close()
	reader.close()

## 载入预览：把路径交出去，由调用方（主页面）真正实例化 KN5。
func _on_load_pressed() -> void:
	var car := _selected()
	if car:
		car_selected.emit(car.path)

## 重命名：直接改目录名（车模还没实例化，磁盘改名是最便宜的）。
func _on_rename_pressed() -> void:
	var car := _selected()
	if car == null:
		return
	var dialog := AcceptDialog.new()
	dialog.title = "重命名车模"
	var input := LineEdit.new()
	input.text = car.name
	input.custom_minimum_size = Vector2(360, 0)
	dialog.add_child(input)
	dialog.ok_button_text = "确定"
	add_child(dialog)
	dialog.popup_centered()
	dialog.confirmed.connect(func() -> void:
			var fresh: String = input.text.strip_edges()
			if fresh.is_empty() or fresh.contains("/") or fresh.contains("\\"):
				_warn("名字不能为空或含路径分隔符")
				return
			var destination: String = car.path.get_base_dir().path_join(fresh)
			if DirAccess.dir_exists_absolute(destination):
				_warn("已存在同名车模")
				return
			var err := DirAccess.open(car.path.get_base_dir()).rename(car.path, destination)
			if err != OK:
				_warn("重命名失败：%d" % err)
				return
			dialog.queue_free()
			refresh())

## 删除：目录整棵删掉，问都不问（用户主动点的删除按钮）。
func _on_delete_pressed() -> void:
	var car := _selected()
	if car == null:
		return
	var dialog := ConfirmationDialog.new()
	dialog.title = "删除车模"
	dialog.dialog_text = "确定删除「%s」？这会从磁盘上抹掉该目录。\n%s" % [car.name, car.path]
	dialog.ok_button_text = "删除"
	add_child(dialog)
	dialog.popup_centered()
	dialog.confirmed.connect(func() -> void:
			_remove_recursive(car.path)
			dialog.queue_free()
			refresh())

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

## 在系统文件管理器里打开（macOS Finder / Windows Explorer）。
func _on_open_pressed() -> void:
	var car := _selected()
	if car == null:
		return
	OS.shell_open(car.path)

func _close() -> void:
	# 先发信号再销毁：宿主（main.gd）要恢复 HUD / 触摸层。
	closed.emit()
	queue_free()

func _selected() -> GarageCar:
	if _list.item_count == 0:
		return null
	return _visible_car(_list.get_selected_items()[0]
			if not _list.get_selected_items().is_empty() else 0)

func name_of(path: String) -> String:
	return path.get_file()

func _warn(message: String) -> void:
	push_warning("[garage] " + message)
	_info.text = "⚠ " + message

func _human_bytes(bytes: int) -> String:
	if bytes < 0:
		return "（未计算）"
	var units := ["B", "KiB", "MiB", "GiB"]
	var value := float(bytes)
	var unit := 0
	while value > 1024.0 and unit < units.size() - 1:
		value /= 1024.0
		unit += 1
	return "%.1f %s" % [value, units[unit]]
