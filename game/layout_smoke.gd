extends Node3D
## 不同窗口尺寸的布局冒烟：把 `main.tscn`（含 HUD）与 `garage.tscn` 实例化，
## 在一组典型分辨率（手机横/竖屏、窄桌面、超宽屏）下断言——
##
## 1. 所有按钮/信息控件整体都落在画布内（不被屏幕边框裁掉）；
## 2. 同屏控件两两不相交（按钮互不遮挡）；
##
## 用法：
##
## ```bash
## godot --headless --path . res://game/layout_smoke.tscn --quit-after 2000
## ```
##
## 期望最后一行 `[layout-smoke] PASS`，退出码 0。

const SIZES := [
	Vector2i(640, 360),   # 窄桌面小窗
	Vector2i(960, 540),
	Vector2i(800, 600),
	Vector2i(1280, 720),  # 基准分辨率
	Vector2i(1920, 1080), # 桌面全屏
	Vector2i(2400, 1080), # 手机横屏
	Vector2i(720, 1280),  # 手机竖屏
]

var _ok := true

func _ready() -> void:
	var main: Node3D = load("res://game/main.tscn").instantiate()
	add_child(main)
	var garage: Control = load("res://game/garage.tscn").instantiate()
	garage.set_anchors_preset(Control.PRESET_FULL_RECT, true)
	add_child(garage)
	await get_tree().process_frame

	for size in SIZES:
		DisplayServer.window_set_size(size)
		await get_tree().process_frame
		await get_tree().process_frame
		# 兜底调用：万一无头模式下 size_changed 不触发，也把安全区算进布局。
		main._apply_safe_area()
		garage._apply_safe_area()
		await get_tree().process_frame

		var canvas := get_viewport().get_visible_rect().size
		var label := "%dx%d -> 画布 %.0fx%.0f" % [size.x, size.y, canvas.x, canvas.y]
		var ok := _check_group(label + " HUD", [
			main.get_node("HUD/Panel/Columns/Column/InfoLabel"),
			main.get_node("HUD/Panel/Columns/Column/ImportButton"),
			main.get_node("HUD/Panel/Columns/Column/SettingsButton"),
			main.get_node("HUD/Panel/Columns/Column/LobbyButton"),
			main.get_node("HUD/Panel/Columns/ModeButton"),
		]) and _check_group(label + " Garage", [
			garage.get_node("%ImportButton"),
			garage.get_node("%SearchBox"),
			garage.get_node("%CarList"),
			garage.get_node("%InfoLabel"),
			garage.get_node("%LoadButton"),
			garage.get_node("%RenameButton"),
			garage.get_node("%DeleteButton"),
			garage.get_node("%OpenButton"),
			garage.get_node("%CloseButton"),
		])
		print("[layout-smoke] %s %s" % [label, "OK" if ok else "FAIL"])
		_ok = _ok and ok

	if _ok:
		print("[layout-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[layout-smoke] FAIL")
		get_tree().quit(1)

## 一组控件：全部在画布内 + 两两不相交。失败时打印具体矩形。
func _check_group(label: String, controls: Array) -> bool:
	var ok := true
	var rects: Array[Rect2] = []
	for control: Control in controls:
		var rect := control.get_global_rect()
		rects.append(rect)
		var canvas := control.get_viewport_rect().size
		if rect.position.x < -0.5 or rect.position.y < -0.5 \
				or rect.end.x > canvas.x + 0.5 or rect.end.y > canvas.y + 0.5:
			push_error("[layout-smoke] %s 越界：%s %s（画布 %s）"
					% [label, control.name, rect, canvas])
			ok = false
	for i in rects.size():
		for j in range(i + 1, rects.size()):
			if rects[i].intersects(rects[j]):
				push_error("[layout-smoke] %s 重叠：%s ↔ %s（%s ↔ %s）"
						% [label, controls[i].name, controls[j].name, rects[i], rects[j]])
				ok = false
	return ok
