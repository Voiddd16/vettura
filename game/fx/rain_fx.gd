extends Node3D
## RainFX：湿路面材质、相机局部雨丝、挡风玻璃雨滴、轮胎水花、低洼积水。
##
## 数据来源：
## - `wx.get_weather()`（物理天气）驱动 rain / wetness / wind；
## - track 的 `[RAIN_FX]` record（材质选择器）只作为开关/保留参数；
## - 当前 v1 用“全局湿滑材质 + 贴地水洼 + 相机局部雨丝”实现近似效果。
##
## 这不是 CSP `.fx` 的逐像素复刻，但已有真实可观察的雨天反馈：
## 路面变暗/变光滑、积水反光、雨丝跟随相机、车轮水花、屏幕雨滴。

const RAIN_DROPS_SHADER := preload("res://game/fx/rain_drops.gdshader")
const PUDDLE_SHADER := preload("res://game/fx/puddle.gdshader")
const PUDDLE_COUNT := 18
const PUDDLE_RADIUS := 42.0
const PUDDLE_SIZE := 5.5

var rain := 0.0
var wetness := 0.0
var wind := Vector3.ZERO
var enabled := true
var windshield := true
var spray := true

var _world: WorldBody
var _cars: Array = []
var _camera: Camera3D

var _rain: GPUParticles3D
var _rain_process: ParticleProcessMaterial
var _rain_material: StandardMaterial3D
var _rain_amount := 0
var _rain_fine: GPUParticles3D
var _rain_fine_process: ParticleProcessMaterial
var _rain_fine_material: StandardMaterial3D
var _rain_fine_amount := 0

var _wet_entries: Array = []
var _material_filters: Array[String] = []
var _last_wet := -1.0
const MAX_WET_MATERIALS := 1400

var _puddles: Array[MeshInstance3D] = []
var _puddle_material: ShaderMaterial
var _puddle_timer := 0.0

var _windshield_layer: CanvasLayer
var _windshield_rect: ColorRect
var _windshield_material: ShaderMaterial

var _spray_emitters: Array = []


func setup(world: WorldBody, cars: Array, records: Array, camera: Camera3D) -> void:
	_world = world
	_cars = cars
	_camera = camera
	_apply_records(records)
	_collect_wet_materials()
	_create_rain()
	_create_puddles()
	_create_windshield()
	_create_spray()
	print("[rain] wet_materials=%d puddles=%d spray=%d" % [_wet_entries.size(), _puddles.size(), _spray_emitters.size()])


func set_weather(rain_amount: float, wet: float, wind_vector: Vector3) -> void:
	rain = clampf(rain_amount, 0.0, 1.0)
	wetness = clampf(wet, 0.0, 1.0)
	wind = wind_vector
	if absf(wetness - _last_wet) > 0.03:
		_apply_wetness(wetness)
		_last_wet = wetness
	if _rain_process != null:
		var direction := Vector3(wind.x * 0.02, -1.0, wind.z * 0.02)
		if direction.length_squared() < 0.01:
			direction = Vector3(0.1, -1.0, 0.05)
		var normalized := direction.normalized()
		_rain_process.direction = normalized
		if _rain_fine_process != null:
			_rain_fine_process.direction = normalized


func _process(delta: float) -> void:
	if not enabled:
		return
	if _world != null:
		var weather: Dictionary = _world.get_weather()
		set_weather(
			float(weather.get("rain", rain)),
			float(weather.get("wetness", wetness)),
			weather.get("wind", wind) as Vector3
		)

	if _camera == null or not is_instance_valid(_camera):
		_camera = get_viewport().get_camera_3d()

	_update_rain()
	_update_puddles(delta)
	_update_windshield()
	_update_spray()


func _apply_records(records: Array) -> void:
	for record in records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.rain":
			continue
		var values: Dictionary = record.get("values", {})
		if values.has("windshield"):
			windshield = bool(values["windshield"])
		if values.has("spray"):
			spray = bool(values["spray"])
		for key in ["puddles_materials", "soaking_materials", "smooth_materials", "rough_materials", "lines_materials"]:
			if values.has(key):
				for part in _parts(values[key]):
					var text := str(part).strip_edges()
					if not text.is_empty():
						_material_filters.append(text)


func _collect_wet_materials() -> void:
	_wet_entries.clear()
	if _world == null:
		return
	var meshes := _world.find_children("*", "MeshInstance3D", true, false)
	for node in meshes:
		var mesh_instance := node as MeshInstance3D
		if mesh_instance == null or _under_vehicle(mesh_instance):
			continue
		var mesh := mesh_instance.mesh
		if mesh == null:
			continue
		for surface in mesh.get_surface_count():
			var material := mesh_instance.get_surface_override_material(surface)
			if material == null:
				material = mesh.surface_get_material(surface)
			if not (material is StandardMaterial3D):
				continue
			if _wet_entries.size() >= MAX_WET_MATERIALS:
				return
			if not _material_filters.is_empty():
				var name := String(material.resource_name).to_lower()
				var matched := false
				for filter in _material_filters:
					if _selector_matches(filter, name):
						matched = true
						break
				if not matched:
					continue
			var dup := (material as StandardMaterial3D).duplicate() as StandardMaterial3D
			mesh_instance.set_surface_override_material(surface, dup)
			_wet_entries.append({
				"material": dup,
				"albedo": dup.albedo_color,
				"roughness": dup.roughness,
				"metallic": dup.metallic,
			})


func _under_vehicle(node: Node) -> bool:
	var current := node
	while current != null:
		if current is VehicleBody:
			return true
		current = current.get_parent()
	return false


func _apply_wetness(value: float) -> void:
	var wet := clampf(value, 0.0, 1.0)
	for entry in _wet_entries:
		var material: StandardMaterial3D = entry["material"]
		if material == null:
			continue
		var base: Color = entry["albedo"]
		material.albedo_color = base.darkened(0.38 * wet)
		material.roughness = lerpf(float(entry["roughness"]), 0.16, 0.75 * wet)
		material.metallic = lerpf(float(entry["metallic"]), 0.22, 0.7 * wet)
		if wet < 0.99:
			material.metallic = minf(material.metallic, 0.85)


func _parts(value: Variant) -> Array:
	if value is Array:
		return value
	return str(value).split(",", false)


func _selector_matches(selector: String, name: String) -> bool:
	var needle := selector.to_lower().trim_prefix("*").trim_suffix("*")
	return name.contains(needle)


func _create_rain() -> void:
	_rain_process = ParticleProcessMaterial.new()
	_rain_process.direction = Vector3(0.08, -1.0, 0.04)
	_rain_process.spread = 2.5
	_rain_process.initial_velocity_min = 34.0
	_rain_process.initial_velocity_max = 56.0
	_rain_process.gravity = Vector3(0.0, -9.8, 0.0)
	_rain_process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	_rain_process.emission_box_extents = Vector3(30.0, 2.0, 30.0)
	_rain_process.scale_min = 0.55
	_rain_process.scale_max = 1.35
	_rain_process.particle_flag_align_y = true
	# 生命周期首尾淡入淡出，避免“一根根白棍”突然出现/消失。
	var gradient := Gradient.new()
	gradient.offsets = PackedFloat32Array([0.0, 0.18, 0.82, 1.0])
	gradient.colors = PackedColorArray([
		Color(1, 1, 1, 0.0),
		Color(1, 1, 1, 0.55),
		Color(1, 1, 1, 0.55),
		Color(1, 1, 1, 0.0),
	])
	var ramp := GradientTexture1D.new()
	ramp.gradient = gradient
	_rain_process.color_ramp = ramp

	# 柔边雨丝贴图：中间亮、两侧渐隐，避免 BoxMesh 的硬边白条。
	var streak := QuadMesh.new()
	streak.size = Vector2(0.018, 0.62)
	_rain_material = StandardMaterial3D.new()
	_rain_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_rain_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_rain_material.albedo_texture = _make_streak_texture()
	_rain_material.albedo_color = Color(0.72, 0.82, 0.95, 0.38)
	_rain_material.billboard_mode = BaseMaterial3D.BILLBOARD_ENABLED
	_rain_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	streak.material = _rain_material

	_rain = GPUParticles3D.new()
	_rain.name = "RainFX"
	_rain.process_material = _rain_process
	_rain.draw_pass_1 = streak
	_rain.amount = 1
	_rain.lifetime = 1.0
	_rain.local_coords = false
	_rain.emitting = false
	_rain.visibility_aabb = AABB(Vector3(-34, -14, -34), Vector3(68, 34, 68))
	add_child(_rain)
	_create_fine_rain()


func _create_fine_rain() -> void:
	_rain_fine_process = ParticleProcessMaterial.new()
	_rain_fine_process.direction = Vector3(0.08, -1.0, 0.04)
	_rain_fine_process.spread = 4.0
	_rain_fine_process.initial_velocity_min = 42.0
	_rain_fine_process.initial_velocity_max = 70.0
	_rain_fine_process.gravity = Vector3(0.0, -9.8, 0.0)
	_rain_fine_process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	_rain_fine_process.emission_box_extents = Vector3(22.0, 2.0, 22.0)
	_rain_fine_process.scale_min = 0.35
	_rain_fine_process.scale_max = 0.9
	_rain_fine_process.particle_flag_align_y = true

	var gradient := Gradient.new()
	gradient.offsets = PackedFloat32Array([0.0, 0.12, 0.85, 1.0])
	gradient.colors = PackedColorArray([
		Color(1, 1, 1, 0.0),
		Color(1, 1, 1, 0.42),
		Color(1, 1, 1, 0.42),
		Color(1, 1, 1, 0.0),
	])
	var ramp := GradientTexture1D.new()
	ramp.gradient = gradient
	_rain_fine_process.color_ramp = ramp

	var streak := QuadMesh.new()
	streak.size = Vector2(0.008, 0.28)
	_rain_fine_material = StandardMaterial3D.new()
	_rain_fine_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_rain_fine_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_rain_fine_material.albedo_texture = _rain_material.albedo_texture
	_rain_fine_material.albedo_color = Color(0.70, 0.80, 0.96, 0.24)
	_rain_fine_material.billboard_mode = BaseMaterial3D.BILLBOARD_ENABLED
	_rain_fine_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	streak.material = _rain_fine_material

	_rain_fine = GPUParticles3D.new()
	_rain_fine.name = "RainFine"
	_rain_fine.process_material = _rain_fine_process
	_rain_fine.draw_pass_1 = streak
	_rain_fine.amount = 1
	_rain_fine.lifetime = 0.8
	_rain_fine.local_coords = false
	_rain_fine.emitting = false
	_rain_fine.visibility_aabb = AABB(Vector3(-26, -14, -26), Vector3(52, 34, 52))
	add_child(_rain_fine)


func _make_streak_texture() -> Texture2D:
	var width := 32
	var height := 128
	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	image.fill(Color(0, 0, 0, 0))
	for y in height:
		var t := float(y) / float(height - 1)
		var vertical := smoothstep(0.0, 0.16, t) * (1.0 - smoothstep(0.78, 1.0, t))
		for x in width:
			var d := absf(float(x) - (width - 1) * 0.5) / (width * 0.5)
			var a := pow(maxf(0.0, 1.0 - d), 2.2) * vertical
			if a > 0.01:
				image.set_pixel(x, y, Color(0.74, 0.84, 1.0, a))
	return ImageTexture.create_from_image(image)


func _update_rain() -> void:
	if _rain == null:
		return
	_rain.emitting = rain > 0.02
	_rain_fine.emitting = rain > 0.02
	var target := maxi(1, int(lerpf(700.0, 3800.0, rain)))
	if target != _rain_amount:
		_rain_amount = target
		_rain.amount = target
	var fine_target := maxi(1, int(lerpf(1200.0, 7200.0, rain)))
	if fine_target != _rain_fine_amount:
		_rain_fine_amount = fine_target
		_rain_fine.amount = fine_target
	if _camera != null and _rain.emitting:
		var forward := -_camera.global_transform.basis.z
		var center := _camera.global_position + forward * 8.0 + Vector3(0.0, 8.0, 0.0)
		_rain.global_position = center
		_rain_fine.global_position = _camera.global_position + forward * 6.0 + Vector3(0.0, 7.0, 0.0)


func _create_puddles() -> void:
	_puddle_material = ShaderMaterial.new()
	_puddle_material.shader = PUDDLE_SHADER

	var plane := PlaneMesh.new()
	plane.size = Vector2(PUDDLE_SIZE, PUDDLE_SIZE)
	for i in PUDDLE_COUNT:
		var mesh := MeshInstance3D.new()
		mesh.name = "RainPuddle%d" % i
		mesh.mesh = plane
		mesh.material_override = _puddle_material
		mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		mesh.visible = false
		add_child(mesh)
		_puddles.append(mesh)


func _update_puddles(delta: float) -> void:
	_puddle_timer -= delta
	if _puddle_timer <= 0.0:
		_puddle_timer = 2.0
		_reposition_puddles()

	var visible := wetness > 0.2 and rain > 0.02
	_puddle_material.set_shader_parameter("wetness", clampf(wetness, 0.0, 1.0))
	for puddle in _puddles:
		puddle.visible = visible


func _reposition_puddles() -> void:
	if _world == null or _camera == null:
		return
	var origin := _camera.global_position
	for puddle in _puddles:
		var x := origin.x + randf_range(-PUDDLE_RADIUS, PUDDLE_RADIUS)
		var z := origin.z + randf_range(-PUDDLE_RADIUS, PUDDLE_RADIUS)
		var kind := String(_world.surface_kind_at(x, z))
		if kind != "Asphalt" and kind != "Concrete":
			puddle.visible = false
			continue
		var y := float(_world.surface_height_at(x, z))
		puddle.global_position = Vector3(x, y + 0.012, z)
		puddle.rotation_degrees = Vector3(0.0, randf_range(0.0, 360.0), 0.0)


func _create_windshield() -> void:
	_windshield_layer = CanvasLayer.new()
	_windshield_layer.name = "RainFXWindshield"
	_windshield_layer.layer = 8
	add_child(_windshield_layer)

	_windshield_rect = ColorRect.new()
	_windshield_rect.name = "RainDrops"
	_windshield_rect.set_anchors_preset(Control.PRESET_FULL_RECT)
	_windshield_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_windshield_rect.color = Color(1, 1, 1, 1)
	_windshield_material = ShaderMaterial.new()
	_windshield_material.shader = RAIN_DROPS_SHADER
	_windshield_rect.material = _windshield_material
	_windshield_rect.visible = false
	_windshield_layer.add_child(_windshield_rect)


func _update_windshield() -> void:
	if _windshield_rect == null:
		return
	# 只有相机贴近车辆（驾驶位/机盖）时才显示屏幕雨滴；追尾/环绕镜头下
	# 全屏雨滴会像 UI 噪点，直接关掉。
	var cockpit := false
	if _camera != null:
		for car in _cars:
			if car is VehicleBody and is_instance_valid(car):
				if _camera.global_position.distance_to(car.global_position) < 3.0:
					cockpit = true
					break
	var visible := windshield and cockpit and rain > 0.12
	_windshield_rect.visible = visible
	if visible:
		_windshield_material.set_shader_parameter("intensity", clampf(rain * 0.7, 0.0, 0.55))


func _create_spray() -> void:
	for car in _cars:
		if not (car is VehicleBody):
			continue
		for wheel in 4:
			var emitter := GPUParticles3D.new()
			emitter.name = "RainSpray_%s_%d" % [car.name, wheel]
			emitter.amount = 1
			emitter.lifetime = 0.55
			emitter.local_coords = false
			emitter.emitting = false
			emitter.visibility_aabb = AABB(Vector3(-6, -4, -6), Vector3(12, 8, 12))

			var process := ParticleProcessMaterial.new()
			process.direction = Vector3(0.0, 0.6, -1.0)
			process.spread = 28.0
			process.initial_velocity_min = 2.0
			process.initial_velocity_max = 5.0
			process.gravity = Vector3(0.0, -6.0, 0.0)
			process.scale_min = 0.4
			process.scale_max = 1.2
			emitter.process_material = process

			var drop := SphereMesh.new()
			drop.radius = 0.016
			drop.height = 0.032
			var mat := StandardMaterial3D.new()
			mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
			mat.albedo_color = Color(0.78, 0.86, 0.95, 0.055)
			mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
			drop.material = mat
			emitter.draw_pass_1 = drop
			add_child(emitter)
			_spray_emitters.append({"emitter": emitter, "car": car, "wheel": wheel})


func _update_spray() -> void:
	if not spray:
		for entry in _spray_emitters:
			(entry["emitter"] as GPUParticles3D).emitting = false
		return
	for entry in _spray_emitters:
		var emitter: GPUParticles3D = entry["emitter"]
		var car: VehicleBody = entry["car"]
		var wheel: int = entry["wheel"]
		if not is_instance_valid(car) or not car.is_ready():
			emitter.emitting = false
			continue
		var active := wheel >= 2 and car.wheel_contact(wheel) and car.speed_kmh() > 28.0 and wetness > 0.3
		emitter.emitting = active
		emitter.global_position = car.wheel_world_position(wheel) + Vector3(0.0, 0.12, 0.0)
		emitter.amount = maxi(1, int(lerpf(6.0, 28.0, wetness))) if active else 1
