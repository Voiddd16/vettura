extends Node3D
## KN5 → Godot 加载路径的无头性能探针。
##
## 用量化数据回答“加载慢在哪”：合成模型（`kn5-inspect --synth` 生成）替代
## 十几 GB 的真车资产，因此任何机器、任何一次运行都可比。测量的四段：
##
## 1. `Kn5File.load()`——文件读取 + 解析（纯 Rust，不碰引擎）；
## 2. 第一次 `instantiate()`——贴图解压、ArrayMesh 上传、节点树创建；
## 3. 第二次 `instantiate()`——同一路径再实例化一次，测缓存命中率；
## 4. 分步 `Kn5SceneBuilder`——按帧预算切片，测“进度反馈”路径。
##
## 用法：
##
## ```bash
## uv run ../scripts/load_probe.py     # 生成合成模型 + 跑本探针
## # 或手动：
## cargo run -p kn5-inspect -- --synth target/probe/car.kn5
## /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/load_probe.tscn
## ```

## 合成模型路径（res:// 形式；实际是 target/probe 下的本地文件）。
const SYNTH_MODEL := "res://target/probe/car.kn5"

var _ok := true

func _ready() -> void:
	var path := ProjectSettings.globalize_path(SYNTH_MODEL)
	if not FileAccess.file_exists(path):
		push_error("[load-probe] synthetic model missing: %s\n" % path
				+ "Generate one with: cargo run -p kn5-inspect -- --synth %s" % path)
		get_tree().quit(2)
		return

	measure_load(path)
	measure_first_instantiate(path)
	measure_second_instantiate(path)
	measure_same_handle_instantiate(path)
	measure_stepwise(path)

	if _ok:
		print("[load-probe] PASS")
		get_tree().quit(0)
	else:
		push_error("[load-probe] FAIL")
		get_tree().quit(1)

func measure_load(path: String) -> void:
	var kn5 := Kn5File.new()
	var started := Time.get_ticks_usec()
	var ok := kn5.load(path)
	var elapsed_ms := (Time.get_ticks_usec() - started) / 1000.0
	print("[load-probe] load          %8.1f ms  ok=%s nodes=%d meshes=%d textures=%d" % [
			elapsed_ms, ok, kn5.node_count(), kn5.mesh_count(), kn5.texture_count()])

func measure_first_instantiate(path: String) -> void:
	var kn5 := Kn5File.new()
	if not kn5.load(path):
		_fail("load failed in first instantiate")
		return
	var started := Time.get_ticks_usec()
	var root := kn5.instantiate(self)
	var elapsed_ms := (Time.get_ticks_usec() - started) / 1000.0
	_report("instantiate #1", elapsed_ms, root)

func measure_second_instantiate(path: String) -> void:
	# 第一个 Kn5File 已销毁，缓存条目随之失效：这是“重新加载同一台车”的冷路径。
	var kn5 := Kn5File.new()
	if not kn5.load(path):
		_fail("load failed in second instantiate")
		return
	var started := Time.get_ticks_usec()
	var root := kn5.instantiate(self)
	var elapsed_ms := (Time.get_ticks_usec() - started) / 1000.0
	_report("instantiate #2", elapsed_ms, root)

func measure_same_handle_instantiate(path: String) -> void:
	# 同一个 Kn5File 第二次实例化：解析结果、贴图、材质全部命中缓存。
	var kn5 := Kn5File.new()
	if not kn5.load(path):
		_fail("load failed in same-handle instantiate")
		return
	var first := kn5.instantiate(self)
	var started := Time.get_ticks_usec()
	var second := kn5.instantiate(self)
	var elapsed_ms := (Time.get_ticks_usec() - started) / 1000.0
	_report("instantiate #3 (缓存)", elapsed_ms, second)
	print("[load-probe] instantiate #3    textures decoded = %d/%d" % [
			kn5.decoded_texture_count(), kn5.texture_count()])

func measure_stepwise(path: String) -> void:
	var builder := Kn5SceneBuilder.new()
	var started := Time.get_ticks_usec()
	if not builder.start(path):
		_fail("Kn5SceneBuilder.start failed")
		return
	var first_step := Time.get_ticks_usec()
	var steps := 0
	# 每次最多处理 32 个节点：把几百个节点的加载摊到多帧，模拟“进度条”场景。
	while not builder.step(32, self):
		steps += 1
	var elapsed_ms := (Time.get_ticks_usec() - first_step) / 1000.0
	var root := builder.root()
	_report("stepwise(%d steps)" % steps, elapsed_ms, root)
	print("[load-probe] stepwise start %8.1f ms  total=%.1f ms" % [
			(first_step - started) / 1000.0,
			(Time.get_ticks_usec() - started) / 1000.0])
	if builder.progress() < 1.0:
		_fail("builder progress %f < 1.0" % builder.progress())

func _report(label: String, elapsed_ms: float, root: Node3D) -> void:
	var span := root if root else self
	print("[load-probe] %-16s %8.1f ms  %s" % [label, elapsed_ms, _stats(span)])

func _stats(root: Node) -> String:
	var meshes := 0
	var albedo := 0
	var unique_materials := {}
	var stack: Array = [root]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		if node is MeshInstance3D:
			meshes += 1
			var mesh: Mesh = (node as MeshInstance3D).mesh
			if mesh:
				var mat: Material = mesh.surface_get_material(0)
				if mat:
					unique_materials[mat.get_instance_id()] = true
					if mat.albedo_texture != null:
						albedo += 1
		for child in node.get_children():
			stack.push_back(child)
	return "meshes=%d unique_materials=%d albedo=%d" % [
			meshes, unique_materials.size(), albedo]

func _fail(message: String) -> void:
	push_error("[load-probe] " + message)
	_ok = false
