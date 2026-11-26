extends Node3D
## GrassFX v3：按赛道草材质三角形分块的实例化草。
##
## 核心变化：
## - 不再只靠 SurfaceMap 的“草地种类”撒点；直接扫描 track MeshInstance3D
##   中 GRASS_MATERIALS 匹配的三角形，按世界坐标分块缓存；
## - 相机周围按 chunk 取三角形，随机重心采样，草能覆盖真正有草贴图的区域；
## - 图集草卡片（CSP TEXTURE/TEXTURE_GRID 或程序化 2x2）+ alpha scissor；
## - 读取 GRASS_MATERIALS / TEXTURE / TEXTURE_GRID / COLOR_FACTOR / density；
## - 基础草地材质轻微压暗，避免平铺贴图抢戏。
##
## 仍未实现 CSP 的逐像素 mask/纹理组权重和风摆 shader；没有草材质三角形时
## 回退到 SurfaceMap 的 Grass/Gravel/Dirt/Sand 采样。

const BUILD_RADIUS := 78.0
const REBUILD_MOVE := 9.0
const CHUNK_SIZE := 24.0
const MAX_GRASS_TRIANGLES := 120_000
const BLADE_WIDTH := 0.5
const BLADE_HEIGHT := 0.9
const GRASS_SHADER := preload("res://game/fx/grass_blade.gdshader")

var enabled := true
var density := 1.0
var quality := "medium"
var grass_materials: Array[String] = []
var color_factor := 1.0

var _world: WorldBody
var _camera: Camera3D
var _last_build := Vector3(1e9, 0.0, 0.0)
var _timer := 0.0
var _rng := RandomNumberGenerator.new()
var _max_instances := 4200
var _variants: Array[MultiMeshInstance3D] = []
var _variant_meshes: Array[MultiMesh] = []
var _material: ShaderMaterial
var _atlas: Texture2D
var _grid_cols := 2
var _grid_rows := 2
var _track_root := ""
var _logged := false
var _variant_weights: Array[float] = []
var _variant_weight_total := 1.0
var _density_clamped := 1.0
var _group_count := 0
var _mask_main_threshold := 0.0
var _full_track := false
var _full_built := false
var _is_mobile := false
var _build_radius := BUILD_RADIUS
var _wetness := 0.0
var _weather_timer := 0.0

## Vector2i(chunk_x, chunk_z) -> Array[PackedVector3Array]（每个三角形 3 点）。
var _chunks: Dictionary = {}
var _triangle_count := 0
var _used_geometry := false


func setup(world: WorldBody, records: Array, quality_name: String) -> void:
	_world = world
	quality = quality_name
	_rng.seed = 0x5EED
	_is_mobile = OS.get_name() == "Android" or OS.get_name() == "iOS" \
		or RenderingServer.get_current_rendering_method() == "mobile"
	_build_radius = 58.0 if _is_mobile else BUILD_RADIUS
	_track_root = String(world.track_root())
	_apply_records(records)
	_max_instances = _quality_count()
	_full_track = quality == "ultra"
	_atlas = _load_or_make_atlas()
	_create_multimeshes()
	_collect_geometry()
	if _chunks.is_empty():
		push_warning("[grass-fx] 没有匹配到 GRASS_MATERIALS 的三角形，回退到 SurfaceMap 采样")
	else:
		print("[grass-fx] chunks=%d triangles=%d" % [_chunks.size(), _triangle_count])


func _process(delta: float) -> void:
	if not enabled or _world == null:
		return
	if _camera == null or not is_instance_valid(_camera):
		_camera = get_viewport().get_camera_3d()
	if _camera == null:
		return
	if _full_track and not _full_built:
		_full_built = true
		_build_full_track()
		return
	if _full_track:
		return
	_timer -= delta
	var moved := _camera.global_position.distance_to(_last_build)
	if _timer <= 0.0 and moved > REBUILD_MOVE:
		_timer = 0.9
		_rebuild()




func _apply_records(records: Array) -> void:
	for record in records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.grass":
			continue
		var values: Dictionary = record.get("values", {})
		if values.has("density"):
			density = clampf(_float(values["density"], 1.0), 0.05, 4.0)
		if values.has("color_factor"):
			color_factor = clampf(_float(values["color_factor"], 1.0), 0.2, 2.0)
		if values.has("grass_materials"):
			for part in _parts(values["grass_materials"]):
				var text := str(part).strip_edges()
				if not text.is_empty() and not grass_materials.has(text):
					grass_materials.append(text)
		if values.has("texture"):
			var path := str(values["texture"]).strip_edges()
			if not path.is_empty():
				var loaded := _try_load_track_texture(path)
				if loaded != null:
					_atlas = loaded
		if values.has("texture_grid"):
			var grid := _parts(values["texture_grid"])
			if grid.size() >= 2:
				_grid_cols = maxi(1, int(str(grid[0]).to_float()))
				_grid_rows = maxi(1, int(str(grid[1]).to_float()))
		if values.has("mask_main_threshold"):
			_mask_main_threshold = _float(values["mask_main_threshold"], _mask_main_threshold)
		if String(record.get("name", "")).begins_with("GRASS_FX_TEXTURE_GROUP"):
			_group_count += 1
		# CONFIGURATION 里的 texture_group_N_chance；按变体权重使用。
		for key in values.keys():
			var key_text := String(key)
			if key_text.begins_with("texture_group_") and key_text.ends_with("_chance"):
				var weight := _float(values[key], 1.0)
				_variant_weights.append(maxf(0.01, weight))


func _quality_count() -> int:
	# 移动端把数量压到大约桌面的 55%，但保留近景密度（build radius 也缩到
	# 58m），画面不会一眼看出“秃”，GPU/CPU 实例更新成本明显下降。
	if _is_mobile:
		match quality:
			"off":
				return 0
			"low":
				return 900
			"medium":
				return 3200
			"high":
				return 7200
			_:
				return 10000
	match quality:
		"off":
			return 0
		"low":
			return 1500
		"medium":
			return 6000
		"high":
			return 14000
		_:
			return 18000


func _create_multimeshes() -> void:
	_material = ShaderMaterial.new()
	_material.shader = GRASS_SHADER
	_material.set_shader_parameter("grass_atlas", _atlas)
	_material.set_shader_parameter("tint", Color(1, 1, 1, 1).darkened(1.0 - clampf(color_factor, 0.2, 1.0) * 0.2))
	# 距离淡出：草只铺到 build radius，边缘用 alpha 淡出，避免移动时出现
	# 一圈“草突然消失”的硬边；移动端淡出更早，减少远景 overdraw。
	_material.set_shader_parameter("fade_start", 34.0 if _is_mobile else 46.0)
	_material.set_shader_parameter("fade_end", 56.0 if _is_mobile else 74.0)

	var cells := mini(_grid_cols * _grid_rows, 8)
	if _group_count > cells:
		cells = mini(_group_count, 8)
	if _variant_weights.size() < cells:
		_variant_weights.resize(cells)
		_variant_weights.fill(1.0)
	_variant_weight_total = 0.0
	for weight in _variant_weights:
		_variant_weight_total += maxf(0.0, weight)
	_density_clamped = clampf(density, 0.2, 2.0)
	for index in cells:
		var cell := Vector2(index % _grid_cols, index / _grid_cols)
		var mesh := _make_blade_mesh(cell)
		var multimesh := MultiMesh.new()
		multimesh.transform_format = MultiMesh.TRANSFORM_3D
		multimesh.use_colors = true
		multimesh.mesh = mesh
		multimesh.instance_count = 0
		var instance := MultiMeshInstance3D.new()
		instance.name = "GrassVariant%d" % index
		instance.multimesh = multimesh
		instance.material_override = _material
		instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		add_child(instance)
		_variants.append(instance)
		_variant_meshes.append(multimesh)


func _make_blade_mesh(cell: Vector2) -> ArrayMesh:
	var u0 := cell.x / float(_grid_cols)
	var v0 := cell.y / float(_grid_rows)
	var u1 := (cell.x + 1.0) / float(_grid_cols)
	var v1 := (cell.y + 1.0) / float(_grid_rows)
	var half := BLADE_WIDTH * 0.5
	var vertices := PackedVector3Array([
		Vector3(-half, 0.0, 0.0),
		Vector3(half, 0.0, 0.0),
		Vector3(half, BLADE_HEIGHT, 0.0),
		Vector3(-half, BLADE_HEIGHT, 0.0),
	])
	var normals := PackedVector3Array([
		Vector3(0, 0, 1), Vector3(0, 0, 1), Vector3(0, 0, 1), Vector3(0, 0, 1),
	])
	var uvs := PackedVector2Array([
		Vector2(u0, v1), Vector2(u1, v1), Vector2(u1, v0), Vector2(u0, v0),
	])
	var indices := PackedInt32Array([0, 1, 2, 0, 2, 3])
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh


func _collect_geometry() -> void:
	if _world == null:
		return
	for node in _world.find_children("*", "MeshInstance3D", true, false):
		var mesh_instance := node as MeshInstance3D
		if mesh_instance == null or mesh_instance.mesh == null or _under_vehicle(mesh_instance):
			continue
		var mesh := mesh_instance.mesh
		for surface in mesh.get_surface_count():
			var material := mesh_instance.get_surface_override_material(surface)
			if material == null:
				material = mesh.surface_get_material(surface)
			var material_name := ""
			if material != null:
				material_name = String(material.resource_name)
			var node_name := String(mesh_instance.name)
			if not _is_grass_material(material_name, node_name):
				continue
			_add_surface_triangles(mesh_instance, mesh, surface, material)
			_tint_material(mesh_instance, surface, material)
			if _triangle_count >= MAX_GRASS_TRIANGLES:
				return


func _is_grass_material(material_name: String, node_name: String) -> bool:
	var haystack := (material_name + " " + node_name).to_lower()
	if grass_materials.is_empty():
		return haystack.contains("grass") or haystack.contains("top2") or haystack.contains("dirt") or haystack.contains("sand")
	for selector in grass_materials:
		var needle := selector.to_lower().trim_prefix("*").trim_suffix("*")
		if not needle.is_empty() and haystack.contains(needle):
			return true
	return false


func _add_surface_triangles(mesh_instance: MeshInstance3D, mesh: Mesh, surface: int, _material: Material) -> void:
	var arrays := mesh.surface_get_arrays(surface)
	if arrays.size() <= Mesh.ARRAY_VERTEX:
		return
	var vertices = arrays[Mesh.ARRAY_VERTEX]
	if not (vertices is PackedVector3Array) or vertices.is_empty():
		return
	var index_array = arrays[Mesh.ARRAY_INDEX] if arrays.size() > Mesh.ARRAY_INDEX else PackedInt32Array()
	var indices: PackedInt32Array
	if index_array is PackedInt32Array and not index_array.is_empty():
		indices = index_array
	else:
		indices = PackedInt32Array()
		for i in vertices.size():
			indices.append(i)

	var transform := mesh_instance.global_transform
	var limit := mini(indices.size(), MAX_GRASS_TRIANGLES - _triangle_count)
	var i := 0
	while i + 2 < limit:
		var a: Vector3 = transform * vertices[indices[i]]
		var b: Vector3 = transform * vertices[indices[i + 1]]
		var c: Vector3 = transform * vertices[indices[i + 2]]
		i += 3
		var normal: Vector3 = (b - a).cross(c - a)
		if normal.length_squared() < 1e-8:
			continue
		if absf(normal.normalized().y) < 0.25:
			continue
		var centroid: Vector3 = (a + b + c) / 3.0
		var key := Vector2i(
			int(floor(centroid.x / CHUNK_SIZE)),
			int(floor(centroid.z / CHUNK_SIZE))
		)
		if not _chunks.has(key):
			_chunks[key] = []
		_chunks[key].append(PackedVector3Array([a, b, c]))
		_triangle_count += 1
		if _triangle_count >= MAX_GRASS_TRIANGLES:
			return


func _tint_material(mesh_instance: MeshInstance3D, surface: int, material: Material) -> void:
	if not (material is StandardMaterial3D):
		return
	# 只在真正草材质上压暗，不碰车漆/路面。
	if not _is_grass_material(String(material.resource_name), String(mesh_instance.name)):
		return
	var dup := (material as StandardMaterial3D).duplicate() as StandardMaterial3D
	dup.albedo_color = dup.albedo_color.darkened(0.22)
	dup.roughness = minf(dup.roughness, 0.95)
	mesh_instance.set_surface_override_material(surface, dup)


func _rebuild() -> void:
	if _world == null or _camera == null or _variants.is_empty():
		return
	var started := Time.get_ticks_usec()
	_last_build = _camera.global_position
	var buckets: Array[Array] = []
	for _i in _variants.size():
		buckets.append([])

	var candidates := _candidate_triangles()
	if not candidates.is_empty():
		_used_geometry = true
		_place_from_triangles(candidates, buckets)
	else:
		_used_geometry = false
		_place_from_surface_map(buckets)

	var total := 0
	for items: Array in buckets:
		total += items.size()
	_apply_buckets(buckets)
	if not _logged:
		_logged = true
		print("[grass] instances=%d variants=%d atlas=%s geometry=%s" % [
			total, _variants.size(), _atlas != null, _used_geometry])
	if OS.get_environment("VETTURA_GRASS_TIMING") == "1":
		print("[grass] rebuild %.1f ms instances=%d geometry=%s" % [
			(Time.get_ticks_usec() - started) / 1000.0, total, _used_geometry])


func _candidate_triangles() -> Array:
	var result: Array = []
	if _chunks.is_empty():
		return result
	var cx := int(floor(_last_build.x / CHUNK_SIZE))
	var cz := int(floor(_last_build.z / CHUNK_SIZE))
	var reach := int(ceil(_build_radius / CHUNK_SIZE))
	for dx in range(-reach, reach + 1):
		for dz in range(-reach, reach + 1):
			var key := Vector2i(cx + dx, cz + dz)
			if _chunks.has(key):
				result.append_array(_chunks[key])
	return result


func _place_from_triangles(candidates: Array, buckets: Array[Array]) -> void:
	var attempts := _max_instances * 7
	var placements := 0
	for _i in attempts:
		if placements >= _max_instances:
			break
		var triangle: PackedVector3Array = candidates[_rng.randi_range(0, candidates.size() - 1)]
		var r1 := _rng.randf()
		var r2 := _rng.randf()
		if r1 + r2 > 1.0:
			r1 = 1.0 - r1
			r2 = 1.0 - r2
		var point := triangle[0] + (triangle[1] - triangle[0]) * r1 + (triangle[2] - triangle[0]) * r2
		if point.distance_to(_last_build) > _build_radius:
			continue
		_add_instance(point, buckets)
		placements += 1


## 回退撒点的网格步长（米）。旧实现用“随机点 + 拒绝采样”，每次重建要
## 向 Rust 查询十几万次表面（Daytona 实测 12 次尝试/株 × 14000 株）；
## 改成先按 1.25 m 网格筛出真正长草的位置，再从候选点里抽样，查询量降一个
## 数据级，草的宏观分布仍是“草地区域内均匀”，肉眼看不出差别。
const GRASS_SAMPLE_STEP := 1.25

func _place_from_surface_map(buckets: Array[Array]) -> void:
	var points := _grass_candidate_points()
	if points.is_empty():
		return
	# 格心高度代表整格：抖动只在本格内（±0.35 格 ≈ 0.44 m），普通赛道边坡的
	# 高差在厘米级，草叶 0.9 m 高，看不出台阶；这样每株不用再跨语言查一次
	# 高度（Daytona 实测能省 14 ms/次重建）。
	var half := GRASS_SAMPLE_STEP * 0.35
	for _i in _max_instances:
		var base: Vector3 = points[_rng.randi_range(0, points.size() - 1)]
		var point := base + Vector3(
			_rng.randf_range(-half, half), 0.0, _rng.randf_range(-half, half))
		_add_instance(point, buckets)


## 按网格筛出当前相机附近真正长草（Grass/Gravel/Sand/Dirt）的地面点。
func _grass_candidate_points() -> Array[Vector3]:
	var points: Array[Vector3] = []
	var step := GRASS_SAMPLE_STEP
	var reach := int(ceil(_build_radius / step))
	var center_x := int(floor(_last_build.x / step))
	var center_z := int(floor(_last_build.z / step))
	var radius_sq := _build_radius * _build_radius
	for dz in range(-reach, reach + 1):
		for dx in range(-reach, reach + 1):
			var x := (center_x + dx) * step
			var z := (center_z + dz) * step
			var ox := x - _last_build.x
			var oz := z - _last_build.z
			if ox * ox + oz * oz > radius_sq:
				continue
			var sample: Vector2 = _world.surface_sample_at(x, z)
			if _is_grass_kind_id(int(sample.x)):
				points.append(Vector3(x, sample.y, z))
	return points


func _add_instance(point: Vector3, buckets: Array[Array]) -> void:
	var variant := _choose_variant()
	var scale := _rng.randf_range(0.55, 1.6) * _density_clamped
	var basis := Basis(Vector3.UP, _rng.randf_range(0.0, TAU))
	basis = basis.scaled(Vector3(scale, scale, scale))
	var color := Color(
		_rng.randf_range(0.72, 1.0),
		_rng.randf_range(0.82, 1.0),
		_rng.randf_range(0.62, 0.92),
		1.0
	)
	buckets[variant].append({
		"transform": Transform3D(basis, point),
		"color": color,
	})


func _choose_variant() -> int:
	if _variants.is_empty():
		return 0
	if _variant_weights.is_empty() or _variant_weight_total <= 0.0:
		return _rng.randi_range(0, _variants.size() - 1)
	var roll := _rng.randf() * _variant_weight_total
	for index in mini(_variants.size(), _variant_weights.size()):
		roll -= _variant_weights[index]
		if roll <= 0.0:
			return index
	return _variants.size() - 1


func _build_full_track() -> void:
	if _world == null or _camera == null:
		return
	_last_build = _camera.global_position
	var buckets: Array[Array] = []
	for _i in _variants.size():
		buckets.append([])
	var candidates := _all_triangles()
	if candidates.is_empty():
		return
	var target := mini(_max_instances * 2, 30000)
	var attempts := target * 3
	var placements := 0
	for _i in attempts:
		if placements >= target:
			break
		var triangle: PackedVector3Array = candidates[_rng.randi_range(0, candidates.size() - 1)]
		var r1 := _rng.randf()
		var r2 := _rng.randf()
		if r1 + r2 > 1.0:
			r1 = 1.0 - r1
			r2 = 1.0 - r2
		var point := triangle[0] + (triangle[1] - triangle[0]) * r1 + (triangle[2] - triangle[0]) * r2
		_add_instance(point, buckets)
		placements += 1
	# 全赛道只做“底色覆盖”；再叠一层相机附近高密度草，避免整体太稀。
	var local_candidates := _candidate_triangles()
	if not local_candidates.is_empty():
		var local_target := _max_instances
		var local_attempts := local_target * 4
		var local_placed := 0
		for _i in local_attempts:
			if local_placed >= local_target:
				break
			var triangle: PackedVector3Array = local_candidates[_rng.randi_range(0, local_candidates.size() - 1)]
			var r1 := _rng.randf()
			var r2 := _rng.randf()
			if r1 + r2 > 1.0:
				r1 = 1.0 - r1
				r2 = 1.0 - r2
			var point := triangle[0] + (triangle[1] - triangle[0]) * r1 + (triangle[2] - triangle[0]) * r2
			if point.distance_to(_last_build) > _build_radius:
				continue
			_add_instance(point, buckets)
			local_placed += 1
		placements += local_placed
	_apply_buckets(buckets)
	if not _logged:
		_logged = true
		print("[grass] full_track instances=%d variants=%d atlas=%s" % [
			placements, _variants.size(), _atlas != null])


func _all_triangles() -> Array:
	var result: Array = []
	for value in _chunks.values():
		result.append_array(value)
	return result


func _apply_buckets(buckets: Array[Array]) -> void:
	for index in _variants.size():
		var multimesh := _variant_meshes[index]
		var items: Array = buckets[index]
		multimesh.instance_count = 0
		multimesh.instance_count = items.size()
		for item_index in items.size():
			var item: Dictionary = items[item_index]
			multimesh.set_instance_transform(item_index, item["transform"])
			multimesh.set_instance_color(item_index, item["color"])


func _is_grass_kind_id(kind_id: int) -> bool:
	# 对应 `vettura_track::SurfaceKind::ALL` 的下标：Grass / Gravel / Sand / Dirt。
	return kind_id == 3 or kind_id == 4 or kind_id == 5 or kind_id == 6


func _under_vehicle(node: Node) -> bool:
	var current := node
	while current != null:
		if current is VehicleBody:
			return true
		current = current.get_parent()
	return false


func _load_or_make_atlas() -> Texture2D:
	if _atlas != null:
		return _atlas
	return _make_procedural_atlas()


func _try_load_track_texture(path: String) -> Texture2D:
	var normalized := path.replace("\\", "/")
	var candidates: Array[String] = []
	var cursor := _track_root
	for _i in 4:
		candidates.append(cursor.path_join("extension/textures").path_join(normalized))
		candidates.append(cursor.path_join("extension").path_join(normalized))
		candidates.append(cursor.path_join(normalized))
		var parent := cursor.get_base_dir()
		if parent == cursor or parent.is_empty():
			break
		cursor = parent
	for candidate in candidates:
		if FileAccess.file_exists(candidate):
			var image := Image.load_from_file(candidate)
			if image != null and not image.is_empty():
				return ImageTexture.create_from_image(image)
	return null


func _make_procedural_atlas() -> Texture2D:
	_grid_cols = 2
	_grid_rows = 2
	var cell := 64
	var image := Image.create(cell * 2, cell * 2, false, Image.FORMAT_RGBA8)
	image.fill(Color(0, 0, 0, 0))
	var rng := RandomNumberGenerator.new()
	rng.seed = 0x91A55
	for variant in 4:
		var cx := (variant % 2) * cell
		var cy := (variant / 2) * cell
		for blade in 5:
			var base_x := cx + rng.randf_range(10.0, float(cell) - 10.0)
			var bend := rng.randf_range(-10.0, 10.0)
			var height := rng.randf_range(34.0, 60.0)
			var green := Color(
				rng.randf_range(0.20, 0.34),
				rng.randf_range(0.40, 0.62),
				rng.randf_range(0.13, 0.24),
				1.0
			)
			for y in int(height):
				var t := float(y) / height
				var x := base_x + bend * t * t
				var width := lerpf(2.2, 0.7, t)
				for w in range(-int(width), int(width) + 1):
					var px := int(round(x)) + w
					var py := cy + cell - 1 - y
					if px >= cx and px < cx + cell and py >= cy and py < cy + cell:
						image.set_pixel(px, py, green)
	return ImageTexture.create_from_image(image)


func _parts(value: Variant) -> Array:
	if value is Array:
		return value
	return str(value).split(",", false)


func _float(value: Variant, fallback: float) -> float:
	if value is Array:
		if value.is_empty():
			return fallback
		value = value[0]
	var text := str(value).strip_edges()
	return text.to_float() if text.is_valid_float() else fallback
