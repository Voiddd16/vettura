extends Node3D
## GrassFX：在赛道表面（Grass/Gravel/Dirt/Sand）上生成实例化草。
##
## 不是 CSP 的顶点级草海，但已有真实几何：相机周围按表面类型采样高度，
## 用 MultiMesh 绘制朝向相机的草叶；移动后按块重建，避免全图常驻。

const BLADE_SIZE := Vector2(0.07, 0.48)
const BUILD_RADIUS := 46.0

var enabled := true
var density := 1.0

var _world: WorldBody
var _camera: Camera3D
var _multimesh_instance: MultiMeshInstance3D
var _multimesh: MultiMesh
var _material: StandardMaterial3D
var _last_build := Vector3(1e9, 0.0, 0.0)
var _timer := 0.0
var _quality := "medium"
var _rng := RandomNumberGenerator.new()
var _max_instances := 600


func setup(world: WorldBody, records: Array, quality: String) -> void:
	_world = world
	_quality = quality
	_rng.seed = 0x5EED
	_apply_records(records)
	_max_instances = _quality_count()
	_create_multimesh()


func _process(delta: float) -> void:
	if not enabled or _world == null:
		return
	if _camera == null or not is_instance_valid(_camera):
		_camera = get_viewport().get_camera_3d()
	if _camera == null:
		return
	_timer -= delta
	var moved := _camera.global_position.distance_to(_last_build)
	if _timer <= 0.0 and moved > 8.0:
		_timer = 0.8
		_rebuild()


func _apply_records(records: Array) -> void:
	for record in records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.grass":
			continue
		var values: Dictionary = record.get("values", {})
		if values.has("density"):
			density = clampf(float(values["density"]), 0.0, 4.0)


func _quality_count() -> int:
	match _quality:
		"off":
			return 0
		"low":
			return 220
		"medium":
			return 520
		"high":
			return 900
		_:
			return 1300


func _create_multimesh() -> void:
	var quad := QuadMesh.new()
	quad.size = BLADE_SIZE
	quad.center_offset = Vector3(0.0, BLADE_SIZE.y * 0.5, 0.0)

	_material = StandardMaterial3D.new()
	_material.albedo_color = Color(0.26, 0.52, 0.19, 1.0)
	_material.roughness = 0.95
	_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_material.billboard_mode = BaseMaterial3D.BILLBOARD_ENABLED
	_material.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL

	_multimesh = MultiMesh.new()
	_multimesh.transform_format = MultiMesh.TRANSFORM_3D
	_multimesh.mesh = quad
	_multimesh.instance_count = 0

	_multimesh_instance = MultiMeshInstance3D.new()
	_multimesh_instance.name = "GrassMultiMesh"
	_multimesh_instance.multimesh = _multimesh
	_multimesh_instance.material_override = _material
	_multimesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_multimesh_instance)


func _rebuild() -> void:
	if _world == null or _camera == null:
		return
	_last_build = _camera.global_position
	var transforms: Array[Transform3D] = []
	var attempts := _max_instances * 5
	var radius := minf(BUILD_RADIUS, 60.0)
	for _i in attempts:
		if transforms.size() >= _max_instances:
			break
		var angle := _rng.randf_range(0.0, TAU)
		var dist := sqrt(_rng.randf()) * radius
		var x := _last_build.x + cos(angle) * dist
		var z := _last_build.z + sin(angle) * dist
		var kind := String(_world.surface_kind_at(x, z))
		if kind.is_empty():
			continue
		if kind != "Grass" and kind != "Gravel" and kind != "Dirt" and kind != "Sand":
			continue
		var y := float(_world.surface_height_at(x, z))
		var scale := _rng.randf_range(0.55, 1.35) * clampf(density, 0.2, 2.0)
		var basis := Basis(Vector3.UP, _rng.randf_range(0.0, TAU))
		basis = basis.scaled(Vector3(scale, scale, scale))
		transforms.append(Transform3D(basis, Vector3(x, y, z)))

	_multimesh.instance_count = 0
	_multimesh.instance_count = transforms.size()
	for index in transforms.size():
		_multimesh.set_instance_transform(index, transforms[index])
