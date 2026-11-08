extends Node3D
## KN5 → Godot 桥接的无头冒烟测试。
##
## 用法（macOS）：
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --headless --path .. \
##     res://gdext/kn5_probe.tscn
## ```
##
## 打开 test/ 下的真实 F1 2008 mod，把车模实例化到场景里，然后打印
## 节点/网格/材质/挂点信息。全部信息用 `[probe]` 前缀，便于 grep。

var car_root := "res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/"

func _ready() -> void:
	var car_dir := ProjectSettings.globalize_path(car_root.path_join("cim_2008_ferrari"))
	var car := AcCar.new()
	print("[probe] car.open = ", car.open(car_dir))
	print("[probe] name = ", car.display_name())
	print("[probe] mass = ", car.total_mass(), " wheelbase = ", car.wheelbase())
	print("[probe] data files = ", car.data_file_count())
	print("[probe] model = ", car.model_path())

	var root := car.instantiate(self)
	if root == null:
		push_error("[probe] instantiate() failed")
		get_tree().quit(1)
		return
	report(root, "ferrari")

	# 第二个车（同样走 AcCar → 主模型解析 → 实例化），确认不是只有一台车能跑。
	var other := car_root.path_join("cim_2008_mclaren")
	var car2 := AcCar.new()
	var ok := car2.open(ProjectSettings.globalize_path(other))
	print("[probe] mclaren.open = ", ok, " ", car2.display_name())
	var root2 := car2.instantiate(self)
	if root2 == null:
		push_error("[probe] mclaren instantiate() failed")
		get_tree().quit(1)
		return
	report(root2, "mclaren")

	# Kn5File 也直接对单个 .kn5 生效（不走车目录）。
	var kn5 := Kn5File.new()
	var kn5_path := ProjectSettings.globalize_path(
		car_root.path_join("cim_2008_toyota/cim_2008_toyota.kn5"))
	print("[probe] kn5.load = ", kn5.load(kn5_path), " nodes = ", kn5.node_count(),
			" meshes = ", kn5.mesh_count(), " textures = ", kn5.texture_count())
	print("[probe] dummy_position(WHEEL_LF) = ", kn5.dummy_position("WHEEL_LF"))
	print("[probe] dummy_position(nope) = ", kn5.dummy_position("nope"))
	var material: Material = kn5.build_material(0)
	print("[probe] build_material(0) = ", material)
	var kn5_root := kn5.instantiate(self)
	if kn5_root == null:
		push_error("[probe] Kn5File.instantiate() failed")
		get_tree().quit(1)
		return
	report(kn5_root, "toyota")

	# 单独生成一张网格（不必实例化整棵树）——用 KN5 原始节点名。
	var body_mesh: ArrayMesh = kn5.build_array_mesh("RED_LIGHT")
	print("[probe] build_array_mesh(RED_LIGHT) surfaces = ",
			body_mesh.get_surface_count() if body_mesh else -1)
	print("[probe] build_array_mesh(not_there) = ", kn5.build_array_mesh("not_there"))

	print("[probe] OK")
	get_tree().quit(0)

func report(root: Node3D, label: String) -> void:
	print("[probe] ", label, ": root = ", root.name, " children = ", root.get_child_count())

	var meshes := 0
	var surfaces := 0
	var invisible := 0
	var materials := 0
	var albedo := 0
	var normal := 0
	var stack: Array = [root]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		if node is MeshInstance3D:
			meshes += 1
			if not node.visible:
				invisible += 1
			var mesh := (node as MeshInstance3D).mesh
			if mesh:
				surfaces += mesh.get_surface_count()
				for surface in mesh.get_surface_count():
					var mat := mesh.surface_get_material(surface)
					if mat:
						materials += 1
						if mat.albedo_texture != null:
							albedo += 1
						if mat.normal_texture != null:
							normal += 1
		for child in node.get_children():
			stack.push_back(child)
	print("[probe] ", label, ": mesh instances = ", meshes, " surfaces = ", surfaces,
			" invisible = ", invisible)
	print("[probe] ", label, ": materials = ", materials, " with albedo = ", albedo,
			" with normal = ", normal)

	for dummy in ["WHEEL_LF", "WHEEL_RF", "WHEEL_RR", "WHEEL_LR"]:
		var shown := root.find_child(dummy, true, false) as Node3D
		print("[probe] ", label, " ", dummy, " -> ", shown.position if shown else "<missing>")
