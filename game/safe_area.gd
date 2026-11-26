extends RefCounted
## 把显示屏安全区（刘海 / 挖孔 / 系统栏 / macOS 菜单栏）换算成画布坐标边距。
##
## 工程开了 `canvas_items` + `expand` 拉伸，画布坐标 = 物理像素 / 统一缩放，
## 所以安全区内缩除以缩放系数即可。调用点通常是 `_ready` 与
## `viewport.size_changed`（旋转、窗口拖动、分屏都会变安全区）：
##
## ```gdscript
## const SafeArea := preload("res://game/safe_area.gd")
##
## func _apply_safe_area() -> void:
## 	SafeArea.apply(panel)                       # panel: MarginContainer
##
## func _ready() -> void:
## 	get_viewport().size_changed.connect(_apply_safe_area)
## ```

## 任何机器上都要留的基础边距，防止控件贴着物理屏幕边缘。
const MIN_MARGIN := 16.0


## 按 `target` 所在画布当前的缩放与安全区，重写它的四边 margin。
## Godot 4 的 MarginContainer 边距不是名为 `margin_left` 的属性，而是主题
## 常量（`.tscn` 里写作 `theme_override_constants/margin_left`），所以这里
## 统一走 `add_theme_constant_override()`。
static func apply(target: MarginContainer, min_margin: float = MIN_MARGIN) -> void:
	var window_size := Vector2(DisplayServer.window_get_size())
	if window_size.x <= 0.0 or window_size.y <= 0.0:
		return
	var canvas := target.get_viewport().get_visible_rect().size
	if canvas.x <= 0.0 or canvas.y <= 0.0:
		return

	var safe := Rect2(DisplayServer.get_display_safe_area())
	if safe.size == Vector2.ZERO:
		# 无头模式 / 普通桌面窗口拿不到有意义的安全区，只保留基础边距。
		_write_margins(target, int(min_margin), int(min_margin),
				int(min_margin), int(min_margin))
		return

	# canvas_items 拉伸下缩放是统一的；两个比值取大者，吸收亚像素取整误差。
	var factor := maxf(window_size.x / canvas.x, window_size.y / canvas.y)
	# 安全区在"整块显示屏"坐标系里，先换回"窗口内部"的剩余区域：
	# 窗口完全落在安全区内时差值为 ≤0，钳到基础边距。主题常量是 int。
	var window_pos := Vector2(DisplayServer.window_get_position())
	_write_margins(target,
			int(maxf(min_margin, maxf(0.0, safe.position.x - window_pos.x) / factor)),
			int(maxf(min_margin, maxf(0.0, safe.position.y - window_pos.y) / factor)),
			int(maxf(min_margin, maxf(0.0, window_pos.x + window_size.x - safe.end.x) / factor)),
			int(maxf(min_margin, maxf(0.0, window_pos.y + window_size.y - safe.end.y) / factor)))


static func _write_margins(target: MarginContainer, left: int, top: int,
		right: int, bottom: int) -> void:
	target.add_theme_constant_override("margin_left", left)
	target.add_theme_constant_override("margin_top", top)
	target.add_theme_constant_override("margin_right", right)
	target.add_theme_constant_override("margin_bottom", bottom)
