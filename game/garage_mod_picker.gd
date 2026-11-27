extends Control
## 车库模组选择器：列出 `ModLibrary` 自动发现的赛道/车库 mod，也可以浏览任意目录。
##
## 只负责“选一个有效目录”，不加载模型；大厅拿到 `mod_selected` 后自己重建
## 3D 场景。分类规则见 `mod_library.gd`，加载规则见 `garage_mod.gd`。

signal mod_selected(mod_path: String)
signal closed

const GarageMod := preload("res://game/garage_mod.gd")
const SafeArea := preload("res://game/safe_area.gd")

@onready var _margin: MarginContainer = %Margin
@onready var _list: ItemList = %ModList
@onready var _preview: TextureRect = %Preview
@onready var _info: Label = %InfoLabel
@onready var _select_button: Button = %SelectButton
@onready var _close_button: Button = %CloseButton
@onready var _rescan_button: Button = %RescanButton
@onready var _browse_button: Button = %BrowseButton

var _mods: Array[String] = []
var _preview_texture: Texture2D


func _ready() -> void:
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)
	_list.item_selected.connect(_on_item_selected)
	_list.item_activated.connect(func(_index: int) -> void: _on_select_pressed())
	_select_button.pressed.connect(_on_select_pressed)
	_close_button.pressed.connect(_close)
	_rescan_button.pressed.connect(_rescan)
	_browse_button.pressed.connect(_on_browse_pressed)
	GameInput.push_nav_handler(_handle_nav)
	I18n.language_changed.connect(_on_language_changed)

	_rescan()
	var current := String(GameSettings.get_value("garage", "mod_path", GarageMod.DEFAULT_PATH))
	_select_path(current)


func _exit_tree() -> void:
	GameInput.pop_nav_handler(_handle_nav)


func _handle_nav(_kind: String) -> bool:
	_close()
	return true


func _unhandled_input(event: InputEvent) -> void:
	var key := event as InputEventKey
	if key != null and key.pressed and not key.echo \
			and key.physical_keycode == KEY_ESCAPE:
		get_viewport().set_input_as_handled()
		_close()


func _apply_safe_area() -> void:
	SafeArea.apply(_margin)


func _rescan() -> void:
	# 「重新扫描」是显式动作：重建 ModLibrary 索引，能立刻看到刚放进 mods 的目录。
	ModLibrary.rescan()
	_mods.clear()
	_list.clear()
	for entry in ModLibrary.visual_scenes():
		_mods.append(entry.path)
		var index := _list.add_item("%s · %s" % [entry.kind_label(), entry.display_name])
		_list.set_item_metadata(index, entry.path)
		_list.set_item_tooltip(index, entry.path)
	if _mods.is_empty():
		_info.text = I18n.t("没有找到可用于大厅的 mod。\n把档案放进 mods/ 或 user://mods/ 后点「重新扫描」。")
		_select_button.disabled = true
		return
	var current := String(GameSettings.get_value("garage", "mod_path", GarageMod.DEFAULT_PATH))
	if not _select_path(current):
		_list.select(0)
		_on_item_selected(0)
	_list.grab_focus()


func _select_path(path: String) -> bool:
	for index in _list.item_count:
		if String(_list.get_item_metadata(index)) == path:
			_list.select(index)
			_on_item_selected(index)
			return true
	return false


func _on_item_selected(index: int) -> void:
	if index < 0 or index >= _list.item_count:
		return
	var path := String(_list.get_item_metadata(index))
	var entry := ModLibrary.find(path)
	if entry == null:
		entry = ModLibrary.classify_path(path)
	var plan := GarageMod.visual_plan(path)
	_select_button.disabled = plan.is_empty()
	var lines := PackedStringArray()
	if entry != null:
		lines.append(I18n.t("类型：%s", [entry.kind_label()]))
		lines.append(I18n.t("名称：%s", [entry.display_name]))
		if not entry.subtitle.is_empty():
			lines.append(entry.subtitle)
	else:
		lines.append(I18n.t("名称：%s", [GarageMod.display_name(path)]))
	lines.append(I18n.t("路径：%s", [path]))
	lines.append(I18n.t("视觉模型：%d 个 KN5", [plan.size()]))
	lines.append(I18n.t("包含 models.ini，按放置表拼装。")
			if FileAccess.file_exists(path.path_join("models.ini"))
			else I18n.t("单模型车库。"))
	_info.text = "\n".join(lines)
	_show_preview(path, entry)


func _show_preview(path: String, entry = null) -> void:
	_preview.texture = null
	_preview_texture = null
	var file: String = String(entry.preview_path) if entry != null else ""
	if file.is_empty():
		file = GarageMod.preview_path(path)
	if file.is_empty():
		return
	var image := Image.new()
	if image.load(file) != OK:
		return
	# 预览图只用于面板，不必保留 4K 原始尺寸。
	if image.get_width() > 1200:
		image.resize(1200, int(1200.0 * image.get_height() / image.get_width()),
				Image.INTERPOLATE_LANCZOS)
	_preview_texture = ImageTexture.create_from_image(image)
	_preview.texture = _preview_texture


func _on_select_pressed() -> void:
	if _list.item_count == 0:
		return
	var selected := _list.get_selected_items()
	var index := selected[0] if not selected.is_empty() else 0
	var path := String(_list.get_item_metadata(index))
	if GarageMod.visual_plan(path).is_empty():
		return
	mod_selected.emit(path)
	_close()


func _on_language_changed(_locale: String) -> void:
	_rescan()


func _on_browse_pressed() -> void:
	if OS.has_feature("android"):
		_browse_android()
	else:
		_browse_embedded()


func _browse_embedded() -> void:
	var picker := FileDialog.new()
	picker.file_mode = FileDialog.FILE_MODE_OPEN_DIR
	picker.access = FileDialog.ACCESS_FILESYSTEM
	picker.current_dir = _browse_start()
	picker.title = I18n.t("选择车库模组目录")
	picker.dir_selected.connect(_on_browsed.bind(picker))
	add_child(picker)
	picker.popup_centered_ratio(0.72)


func _browse_android() -> void:
	var start := _browse_start()
	var error := DisplayServer.file_dialog_show(
			I18n.t("选择车库模组目录"),
			start,
			"", true, DisplayServer.FILE_DIALOG_MODE_OPEN_DIR,
			PackedStringArray(), _on_system_dir_selected)
	if error != OK:
		push_warning("[garage-mod] 系统目录选择器不可用（%d），回落内置对话框" % error)
		_browse_embedded()


func _on_system_dir_selected(status: bool, paths: PackedStringArray,
		_filter_index: int) -> void:
	if status and not paths.is_empty():
		_on_browsed(paths[0], null)


func _browse_start() -> String:
	for root in ["user://mods", "res://mods"]:
		var path := ProjectSettings.globalize_path(root)
		if DirAccess.dir_exists_absolute(path):
			return path
	return OS.get_system_dir(OS.SYSTEM_DIR_DESKTOP)


func _on_browsed(path: String, picker: FileDialog = null) -> void:
	if picker != null:
		picker.queue_free()
	if GarageMod.visual_plan(path).is_empty():
		_info.text = I18n.t("不是有效的车库模组：%s\n需要直接包含 .kn5 或 models.ini。",
				[path])
		return
	var entry := ModLibrary.classify_path(path)
	if not _mods.has(path):
		_mods.append(path)
		var label := entry.display_name if entry != null else GarageMod.display_name(path)
		var kind := entry.kind_label() if entry != null else I18n.t("车库")
		var index := _list.add_item("%s · %s" % [kind, label])
		_list.set_item_metadata(index, path)
		_list.set_item_tooltip(index, path)
	_select_path(path)


func _close() -> void:
	closed.emit()
	queue_free()
