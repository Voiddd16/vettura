extends Node3D
## 车库 UI 无头探针：真实实例化 garage.tscn（按钮/信号/节点路径全部解析），
## 再走一遍 refresh / 选中 / 懒统计，确保面板代码不只是"能解析"而是"能跑"。

func _ready() -> void:
	# 造一台占位车模，避免空库时按钮全禁用、走不到后续路径。
	var root := ProjectSettings.globalize_path("user://imported_cars/")
	var car := root.path_join("zz_ui_car")
	DirAccess.make_dir_recursive_absolute(car)
	var data := FileAccess.open(car.path_join("data.acd"), FileAccess.WRITE)
	if data:
		data.store_buffer(PackedByteArray([0, 1, 2, 3]))
		data.close()

	var garage: Control = load("res://test_visual/garage.tscn").instantiate()
	add_child(garage)
	await get_tree().process_frame
	garage.refresh()
	await get_tree().process_frame
	print("[garage-ui] refresh done")

	# 选中第一行（自动选中应已触发懒统计）。
	var garage_list: ItemList = garage.get_node("%CarList")
	print("[garage-ui] items=", garage_list.item_count, " selected=", garage_list.get_selected_items())
	if garage_list.item_count < 1:
		push_error("[garage-ui] 列表为空")
		get_tree().quit(1)
		return

	var info: Label = garage.get_node("%InfoLabel")
	print("[garage-ui] info=\n", info.text)
	if info.text.find("data.acd") == -1 or info.text.find("大小") == -1:
		push_error("[garage-ui] 信息面板没有懒统计字段")
		get_tree().quit(1)
		return

	# 载入预览按钮点击 → 信号应发出（没人接听也没关系，不能崩）。
	garage.get_node("%LoadButton").emit_signal("pressed")
	print("[garage-ui] load button ok")

	# 重命名/删除按钮存在即可点（不真删，探测完还有别的用例）。
	for button in ["%RenameButton", "%DeleteButton", "%OpenButton", "%CloseButton"]:
		if not garage.has_node(button):
			push_error("[garage-ui] 缺按钮 ", button)
			get_tree().quit(1)
			return
	print("[garage-ui] buttons ok")

	print("[garage-ui] PASS")
	get_tree().quit(0)
