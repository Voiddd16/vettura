extends Control
## 赛道选择器：列出 `ModLibrary` 自动发现的赛道，并展开每个赛道可用的布局。
##
## 一个赛道可能有多个 `models_<layout>.ini`（Daytona 的 roadcourse/oval、
## Nürburgring 的 nordschleife/24h），所以列表按“赛道 × 布局”展开；选择结果
## 写回 `GameSettings["race"]`，大厅的「开始驾驶」直接用它加载 WorldBody。

signal track_selected(track_path: String, layout: String)
signal closed

const SafeArea := preload("res://game/safe_area.gd")

@onready var _margin: MarginContainer = %Margin
@onready var _list: ItemList = %TrackList
@onready var _preview: TextureRect = %Preview
@onready var _info: Label = %InfoLabel
@onready var _select_button: Button = %SelectButton
@onready var _rescan_button: Button = %RescanButton
@onready var _close_button: Button = %CloseButton

## 每一行对应一个 (track_path, layout)。ItemList 的 metadata 直接存字典。
var _rows: Array[Dictionary] = []
var _preview_texture: Texture2D


func _ready() -> void:
	_apply_safe_area()
	get_viewport().size_changed.connect(_apply_safe_area)
	_list.item_selected.connect(_on_item_selected)
	_select_button.pressed.connect(_on_select_pressed)
	_rescan_button.pressed.connect(_rescan)
	_close_button.pressed.connect(_close)
	GameInput.push_nav_handler(_handle_nav)
	I18n.language_changed.connect(_on_language_changed)
	_rescan()


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
	ModLibrary.rescan()
	_rows.clear()
	_list.clear()
	for entry in ModLibrary.tracks():
		var layouts := _ordered_layouts(entry)
		for layout: String in layouts:
			var row := {"path": entry.path, "layout": layout}
			_rows.append(row)
			var label := ModLibrary.layout_display_name(entry.path, layout)
			if not layout.is_empty() and label.to_lower().find(layout.to_lower()) == -1:
				label += " · " + layout
			var index := _list.add_item(label)
			_list.set_item_metadata(index, row)
			_list.set_item_tooltip(index, entry.path)
	if _rows.is_empty():
		_info.text = I18n.t("没有找到赛道 mod。\n把赛道放进 mods/ 或 user://mods/ 后点「重新扫描」。")
		_select_button.disabled = true
		return

	var current_path := String(GameSettings.get_value("race", "track_path", ""))
	var current_layout := String(GameSettings.get_value("race", "track_layout", ""))
	if not _select_row(current_path, current_layout):
		_list.select(0)
		_on_item_selected(0)


func _ordered_layouts(entry) -> PackedStringArray:
	var layouts: PackedStringArray = entry.layouts.duplicate()
	if layouts.is_empty():
		layouts.append("")
	var default_layout := String(entry.default_layout)
	var ordered: PackedStringArray = PackedStringArray()
	if layouts.has(default_layout):
		ordered.append(default_layout)
	for layout: String in layouts:
		if layout != default_layout:
			ordered.append(layout)
	return ordered


func _select_row(path: String, layout: String) -> bool:
	for index in _rows.size():
		var row: Dictionary = _rows[index]
		if String(row.get("path", "")) == path \
				and String(row.get("layout", "")) == layout:
			_list.select(index)
			_on_item_selected(index)
			return true
	return false


func _on_language_changed(_locale: String) -> void:
	_rescan()


func _on_item_selected(index: int) -> void:
	if index < 0 or index >= _rows.size():
		return
	var row: Dictionary = _rows[index]
	var path := String(row.get("path", ""))
	var layout := String(row.get("layout", ""))
	var entry = ModLibrary.find(path)
	var lines := PackedStringArray()
	lines.append(I18n.t("名称：%s", [ModLibrary.layout_display_name(path, layout)]))
	lines.append(I18n.t("布局：%s", [ModLibrary.layout_label(layout)]))
	lines.append(I18n.t("路径：%s", [path]))
	lines.append(I18n.t("AI 线：%s", [I18n.t("有（可计时）")
			if _layout_has(path, layout, "ai/fast_lane.ai") else I18n.t("无")]))
	lines.append(I18n.t("维修区：%s", [I18n.t("有")
			if _layout_has(path, layout, "ai/pit_lane.ai") else I18n.t("无")]))
	if entry != null and not entry.subtitle.is_empty():
		lines.append(entry.subtitle)
	_info.text = "\n".join(lines)
	_select_button.disabled = false
	_show_preview(path, entry, layout)


func _layout_has(path: String, layout: String, relative: String) -> bool:
	if not layout.is_empty() and FileAccess.file_exists(path.path_join(layout).path_join(relative)):
		return true
	return FileAccess.file_exists(path.path_join(relative))


func _show_preview(path: String, entry, layout: String) -> void:
	_preview.texture = null
	_preview_texture = null
	var file := ModLibrary.layout_preview(path, layout)
	if file.is_empty() and entry != null:
		file = String(entry.preview_path)
	if file.is_empty():
		file = _fallback_preview(path)
	if file.is_empty():
		return
	var image := Image.new()
	if image.load(file) != OK:
		return
	if image.get_width() > 1200:
		image.resize(1200, int(1200.0 * image.get_height() / image.get_width()),
				Image.INTERPOLATE_LANCZOS)
	_preview_texture = ImageTexture.create_from_image(image)
	_preview.texture = _preview_texture


func _fallback_preview(path: String) -> String:
	for candidate in ["ui/preview.png", "preview.png", "ui/map.png"]:
		var file := path.path_join(candidate)
		if FileAccess.file_exists(file):
			return file
	return ""


func _on_select_pressed() -> void:
	if _list.item_count == 0:
		return
	var selected := _list.get_selected_items()
	var index := selected[0] if not selected.is_empty() else 0
	var row: Dictionary = _rows[index]
	track_selected.emit(String(row.get("path", "")), String(row.get("layout", "")))
	_close()


func _close() -> void:
	closed.emit()
	queue_free()
