extends Node3D
## ParticlesFX：把 ext_config 的 SMOKE_* / PARTICLES_* 记录变成 GPUParticles3D。
##
## v1 支持位置、方向、速度、大小、颜色、扩散；不执行 CSP 的粒子脚本。
## 车轮胎烟/尘由 OnCar 更新（后续接入车轮滑移），这里先保证场景粒子可见。

const MAX_EMITTERS := 64

var enabled := true
var _emitters: Array[GPUParticles3D] = []
var _quality := "medium"


func setup(_world: WorldBody, _cars: Array, track_records: Array, car_records: Array, quality: String) -> void:
	_quality = quality
	var records: Array = []
	records.append_array(track_records)
	records.append_array(car_records)
	for record in records:
		if _emitters.size() >= MAX_EMITTERS:
			break
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.particles":
			continue
		_apply_record(record)
	print("[particles] emitters=%d records=%d" % [_emitters.size(), records.size()])


func _apply_record(record: Dictionary) -> void:
	var values: Dictionary = record.get("values", {})
	if not values.has("position"):
		return
	var position := _vector(values["position"])
	var direction := _vector(values.get("direction", "0,1,0"))
	if direction.length_squared() < 0.001:
		direction = Vector3.UP
	var speed := _float(values.get("speed", 1.5), 1.5)
	var size := _float(values.get("size", 0.5), 0.5)
	var spread := _float(values.get("spread", 1.0), 1.0)
	var color := _color(values.get("color", "0.5,0.5,0.5,0.5"))

	var process := ParticleProcessMaterial.new()
	process.direction = direction.normalized()
	process.spread = clampf(spread * 20.0, 1.0, 90.0)
	process.initial_velocity_min = speed * 0.4
	process.initial_velocity_max = speed * 1.4
	process.gravity = Vector3(0.0, 0.4, 0.0)
	process.scale_min = maxf(0.1, size * 0.6)
	process.scale_max = maxf(0.2, size * 2.2)
	process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_SPHERE
	process.emission_sphere_radius = maxf(0.2, spread)

	var mesh := SphereMesh.new()
	mesh.radius = 0.5
	mesh.height = 1.0
	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = color
	mesh.material = material

	var emitter := GPUParticles3D.new()
	emitter.name = "FxParticle_%s" % record.get("name", "smoke")
	emitter.process_material = process
	emitter.draw_pass_1 = mesh
	emitter.amount = _amount()
	emitter.lifetime = 4.0
	emitter.preprocess = 1.0
	emitter.local_coords = false
	emitter.emitting = true
	emitter.visibility_aabb = AABB(Vector3(-8, -4, -8), Vector3(16, 16, 16))
	add_child(emitter)
	emitter.global_position = position
	_emitters.append(emitter)


func _amount() -> int:
	match _quality:
		"low":
			return 10
		"medium":
			return 24
		"high":
			return 40
		_:
			return 56


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


func _vector(value: Variant) -> Vector3:
	var parts := _parts(value)
	if parts.size() < 3:
		return Vector3.ZERO
	# ext_config 使用 AC 坐标；Godot 的 X/Z 轴镜像。
	return Vector3(-str(parts[0]).to_float(), str(parts[1]).to_float(), -str(parts[2]).to_float())


func _color(value: Variant) -> Color:
	var parts := _parts(value)
	var r := 0.5
	var g := 0.5
	var b := 0.5
	var a := 0.5
	if parts.size() >= 3:
		r = str(parts[0]).to_float()
		g = str(parts[1]).to_float()
		b = str(parts[2]).to_float()
	if parts.size() >= 4:
		a = str(parts[3]).to_float()
	var scale := 1.0
	var max_component := maxf(r, maxf(g, b))
	if max_component > 2.0:
		scale = 1.0 / max_component
	return Color(
		clampf(r * scale, 0.0, 1.0),
		clampf(g * scale, 0.0, 1.0),
		clampf(b * scale, 0.0, 1.0),
		clampf(a, 0.02, 1.0)
	)
