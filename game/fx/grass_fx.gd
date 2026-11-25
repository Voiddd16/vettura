extends Node3D
## GrassFX v2：赛道级草卡片 + 纹理图集 + 颜色变化 + 基础草材质压暗。
##
## 与 v1 的区别：
## - 不再只画纯色 QuadMesh，而是 2x2/网格图集的 alpha-tested 草卡片；
## - 使用 MultiMesh + billboard，近处密度显著提高；
## - 读取 [GRASS_FX] 的 GRASS_MATERIALS / TEXTURE / TEXTURE_GRID / COLOR_FACTOR；
## - 对基础草地材质做轻微压暗/降对比，避免平铺贴图抢戏。
##
## 仍不是 CSP 的逐像素 mask/纹理组；无贴图时使用程序化草叶图集。

const BUILD_RADIUS := 38.0
const REBUILD_MOVE := 10.0
const BLADE_WIDTH := 0.5
const BLADE_HEIGHT := 0.9

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
var _max_instances := 520
var _variants: Array[MultiMeshInstance3D] = []
var _variant_meshes: Array[MultiMesh] = []
var _material: StandardMaterial3D
var _atlas: Texture2D
var _grid_cols := 2
var _grid_rows := 2
var _track_root := ""
var _base_tinted := false
var _logged := false


func setup(world: WorldBody, records: Array, quality_name: String) -> void:
	_world = world
	quality = quality_name
	_rng.seed = 0x5EED
	_track_root = String(world.track_root())
	_apply_records(records)
	_max_instances = _quality_count()
	_atlas = _load_or_make_atlas()
	_create_multimeshes()
	_tint_base_grass()


func _process(delta: float) -> void:
	if not enabled or _world == null:
		return
	if _camera == null or not is_instance_valid(_camera):
		_camera = get_viewport().get_camera_3d()
	if _camera == null:
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
			density = clampf(float(values["density"]), 0.05, 4.0)
		if values.has("color_factor"):
			color_factor = clampf(_float(values["color_factor"], 1.0), 0.2, 2.0)
		if values.has("grass_materials"):
			for part in _parts(values["grass_materials"]):
				var text := str(part).strip_edges()
				if not text.is_empty():
					grass_materials.append(text)
		if values.has("texture"):
			var path := str(values["texture"]).strip_edges()
			if not path.is_empty():
				_atlas = _try_load_track_texture(path)
		if values.has("texture_grid"):
			var grid := _parts(values["texture_grid"])
			if grid.size() >= 2:
				_grid_cols = maxi(1, int(str(grid[0]).to_float()))
				_grid_rows = maxi(1, int(str(grid[1]).to_float()))


func _quality_count() -> int:
	match quality:
		"off":
			return 0
		"low":
			return 1200
		"medium":
			return 3200
		"high":
			return 6500
		_:
			return 10000


func _create_multimeshes() -> void:
	_material = StandardMaterial3D.new()
	_material.albedo_texture = _atlas
	_material.albedo_color = Color(1.0, 1.0, 1.0, 1.0).darkened(1.0 - clampf(color_factor, 0.2, 1.0) * 0.2)
	_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR
	_material.alpha_scissor_threshold = 0.34
	_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_material.billboard_mode = BaseMaterial3D.BILLBOARD_ENABLED
	_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_material.roughness = 0.95
	_material.vertex_color_use_as_albedo = true

	var cells := _grid_cols * _grid_rows
	# 图集单元格太多时只取前 8 个，避免 MultiMesh 数量爆炸。
	cells = mini(cells, 8)
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


func _rebuild() -> void:
	if _world == null or _camera == null or _variants.is_empty():
		return
	_last_build = _camera.global_position
	var buckets: Array[Array] = []
	for _i in _variants.size():
		buckets.append([])
	var attempts := _max_instances * 12
	var placements := 0
	for _i in attempts:
		if placements >= _max_instances:
			break
		var angle := _rng.randf_range(0.0, TAU)
		var dist := sqrt(_rng.randf()) * BUILD_RADIUS
		var x := _last_build.x + cos(angle) * dist
		var z := _last_build.z + sin(angle) * dist
		var kind := String(_world.surface_kind_at(x, z))
		if not _is_grass_kind(kind):
			continue
		var y := float(_world.surface_height_at(x, z))
		var variant := _rng.randi_range(0, _variants.size() - 1)
		var scale := _rng.randf_range(0.55, 1.5) * clampf(density, 0.2, 2.0)
		var basis := Basis(Vector3.UP, _rng.randf_range(0.0, TAU))
		basis = basis.scaled(Vector3(scale, scale, scale))
		var color := Color(
			_rng.randf_range(0.72, 1.0),
			_rng.randf_range(0.82, 1.0),
			_rng.randf_range(0.62, 0.92),
			1.0
		)
		var data := {"transform": Transform3D(basis, Vector3(x, y, z)), "color": color}
		buckets[variant].append(data)
		placements += 1

	var total := 0
	for index in _variants.size():
		var multimesh := _variant_meshes[index]
		var items: Array = buckets[index]
		multimesh.instance_count = 0
		multimesh.instance_count = items.size()
		total += items.size()
		for item_index in items.size():
			var item: Dictionary = items[item_index]
			multimesh.set_instance_transform(item_index, item["transform"])
			multimesh.set_instance_color(item_index, item["color"])
	if not _logged:
		_logged = true
		print("[grass] instances=%d variants=%d atlas=%s" % [total, _variants.size(), _atlas != null])


func _is_grass_kind(kind: String) -> bool:
	if kind.is_empty():
		return false
	return kind == "Grass" or kind == "Gravel" or kind == "Dirt" or kind == "Sand"


func _tint_base_grass() -> void:
	if _base_tinted or _world == null:
		return
	_base_tinted = true
	if grass_materials.is_empty():
		return
	for node in _world.find_children("*", "MeshInstance3D", true, false):
		var mesh := node as MeshInstance3D
		if mesh == null or mesh.mesh == null or _under_vehicle(mesh):
			continue
		for surface in mesh.mesh.get_surface_count():
			var source := mesh.get_surface_override_material(surface)
			if source == null:
				source = mesh.mesh.surface_get_material(surface)
			if not (source is StandardMaterial3D):
				continue
			var name := String(source.resource_name).to_lower()
			var matched := false
			for selector in grass_materials:
				if name.contains(selector.to_lower().trim_prefix("*").trim_suffix("*")):
					matched = true
					break
			if not matched:
				continue
			var dup := (source as StandardMaterial3D).duplicate() as StandardMaterial3D
			# 让底层贴图退到“远景底色”，近处交给草卡片。
			dup.albedo_color = dup.albedo_color.darkened(0.22)
			dup.roughness = minf(dup.roughness, 0.95)
			mesh.set_surface_override_material(surface, dup)


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
	# 2x2、每格 64x64：画几片弯曲草叶，alpha 裁切后作为 billboard 卡片。
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
				rng.randf_range(0.28, 0.45),
				rng.randf_range(0.55, 0.82),
				rng.randf_range(0.18, 0.34),
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
