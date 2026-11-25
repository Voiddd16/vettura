@tool
extends Node3D
class_name RainSystem
## Godot 4.3+ Mobile 雨天系统（Android / iOS）。
##
## 选型：
## - 雨丝：GPUParticles3D + 自定义 spatial shader，GPU 模拟；不逐帧遍历粒子。
## - 地面涟漪：MultiMesh 小池（<=6）+ 程序化圆环 shader；不用 Decal，
##   规避 Mobile 每 Mesh 最多 8 个 Decal 的限制。
## - 水花：独立 GPUParticles3D，GPU 模拟；只在相机附近的地面平面发射。
## - 远景：WorldEnvironment depth fog + height fog；不使用 volumetric_fog。
## - 屏幕雨痕：CanvasLayer 程序化 shader，默认关闭，低画质强制关闭。
##
## 版本依赖：
## - Godot 4.3+（当前工程 4.7 验证）。
## - RenderingServer.get_current_rendering_method()（4.2+）。
## - GPUParticles3D / ParticleProcessMaterial.particle_flag_align_y（4.0+）。
## - MultiMesh.use_colors / set_instance_color（4.0+）。
## - AudioStreamGenerator 仅在显式开启 enable_audio 时使用。
##
## 挂载：作为 Camera3D 的子节点，或放在场景根后由脚本自动查找相机。
## 建议把本节点挂到相机下；它会 top_level=true 并每帧跟随相机位置，
## 发射体积不继承相机旋转，避免转头时雨跟着转/粘屏。

signal quality_changed(value: int)
signal parameters_changed

enum Quality { LOW, MEDIUM, HIGH }

const RAIN_DROP_SHADER := preload("res://game/fx/rain/rain_drop.gdshader")
const RAIN_SPLASH_SHADER := preload("res://game/fx/rain/rain_splash.gdshader")
const RAIN_RIPPLE_SHADER := preload("res://game/fx/rain/rain_ripple.gdshader")
const RAIN_SCREEN_SHADER := preload("res://game/fx/rain/rain_screen.gdshader")

const MAX_NEAR_BY_QUALITY := [700, 1800, 3600]
const MAX_FAR_BY_QUALITY := [0, 550, 1300]
const MAX_SPLASH_BY_QUALITY := [4, 12, 24]
const MAX_RIPPLES_BY_QUALITY := [2, 4, 6]
const DROP_WIDTH_BY_QUALITY := [0.008, 0.011, 0.014]
const DROP_LENGTH_BY_QUALITY := [0.24, 0.34, 0.46]
const SPLASH_AMOUNT_CAP := 240

@export_group("Rain")
@export_range(0.0, 1.0, 0.01) var rain_amount: float = 0.6:
	set(value):
		var clamped := clampf(value, 0.0, 1.0)
		if is_equal_approx(rain_amount, clamped):
			return
		rain_amount = clamped
		if auto_wetness_from_rain:
			ground_wetness = clampf(clamped * 0.95, 0.0, 1.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 40.0, 0.1) var wind_speed: float = 3.0:
	set(value):
		var clamped := maxf(value, 0.0)
		if is_equal_approx(wind_speed, clamped):
			return
		wind_speed = clamped
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 360.0, 0.5) var wind_direction_deg: float = 25.0:
	set(value):
		var wrapped := fposmod(value, 360.0)
		if is_equal_approx(wind_direction_deg, wrapped):
			return
		wind_direction_deg = wrapped
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(5.0, 25.0, 0.1) var fall_speed_mps: float = 13.0:
	set(value):
		fall_speed_mps = clampf(value, 5.0, 25.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 1.0, 0.01) var turbulence: float = 0.25:
	set(value):
		turbulence = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
		parameters_changed.emit()

@export_group("Drop Shape")
@export_range(0.003, 0.08, 0.001) var drop_width: float = 0.011:
	set(value):
		drop_width = clampf(value, 0.003, 0.08)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.05, 1.2, 0.01) var drop_length: float = 0.34:
	set(value):
		drop_length = clampf(value, 0.05, 1.2)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(1.0, 3.0, 0.05) var stretch_multiplier: float = 1.15:
	set(value):
		stretch_multiplier = clampf(value, 1.0, 3.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.5, 4.0, 0.05) var min_screen_width_px: float = 1.25:
	set(value):
		min_screen_width_px = clampf(value, 0.5, 4.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 2.0, 0.01) var drop_brightness: float = 1.0:
	set(value):
		drop_brightness = clampf(value, 0.0, 2.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export var drop_color: Color = Color(0.72, 0.82, 1.0, 1.0):
	set(value):
		drop_color = value
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 1.0, 0.01) var jitter_strength: float = 0.18:
	set(value):
		jitter_strength = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
		parameters_changed.emit()

@export_group("Ground & Atmosphere")
@export var auto_wetness_from_rain: bool = true:
	set(value):
		auto_wetness_from_rain = value
@export_range(0.0, 1.0, 0.01) var ground_wetness: float = 0.75:
	set(value):
		var clamped := clampf(value, 0.0, 1.0)
		if is_equal_approx(ground_wetness, clamped):
			return
		ground_wetness = clamped
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 1.0, 0.01) var splash_strength: float = 0.65:
	set(value):
		splash_strength = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.1, 3.0, 0.05) var ripple_size: float = 0.45:
	set(value):
		ripple_size = clampf(value, 0.1, 3.0)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0, 6, 1) var max_ripples: int = 4:
	set(value):
		max_ripples = clampi(value, 0, 6)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 0.08, 0.0005) var fog_density: float = 0.012:
	set(value):
		fog_density = clampf(value, 0.0, 0.08)
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(10.0, 160.0, 1.0) var rain_visibility_distance: float = 70.0:
	set(value):
		rain_visibility_distance = clampf(value, 10.0, 160.0)
		_parameters_dirty = true
		parameters_changed.emit()

@export_group("Quality")
@export var quality: Quality = Quality.MEDIUM:
	set(value):
		quality = value
		_parameters_dirty = true
		quality_changed.emit(value)
@export var mobile_low_end: bool = false:
	set(value):
		mobile_low_end = value
		_parameters_dirty = true
@export var auto_weather: bool = true:
	set(value):
		auto_weather = value
@export var wet_surface_enabled: bool = true:
	set(value):
		wet_surface_enabled = value
		_parameters_dirty = true
@export_range(32, 1024, 1) var max_wet_materials: int = 512:
	set(value):
		max_wet_materials = clampi(value, 32, 1024)
		_parameters_dirty = true

@export_group("Optional")
@export var screen_drops_enabled: bool = false:
	set(value):
		screen_drops_enabled = value
		_parameters_dirty = true
@export_range(-80.0, 0.0, 0.5) var rain_audio_db: float = -24.0
@export var enable_audio: bool = false:
	set(value):
		enable_audio = value
		_parameters_dirty = true
@export var fallback_ground_y: float = 0.0
@export var camera_path: NodePath

var _built := false
var _camera: Camera3D
var _world: WorldBody
var _cars: Array = []
var _records: Array = []
var _parameters_dirty := true
var _compat_fallback := false
var _rendering_method := ""

var _rain_near: GPUParticles3D
var _rain_far: GPUParticles3D
var _rain_near_shader: ShaderMaterial
var _rain_far_shader: ShaderMaterial
var _rain_near_process: ParticleProcessMaterial
var _rain_far_process: ParticleProcessMaterial
var _splash: GPUParticles3D
var _splash_process: ParticleProcessMaterial
var _ripple_instance: MultiMeshInstance3D
var _ripple_multimesh: MultiMesh
var _ripple_material: ShaderMaterial
var _ripples: Array[Dictionary] = []
var _ripple_accum := 0.0
var _effective_ripple_limit := 4
var _screen_layer: CanvasLayer
var _screen_rect: ColorRect
var _screen_material: ShaderMaterial
var _environment: Environment
var _world_env: WorldEnvironment
var _wet_entries: Array[Dictionary] = []
var _wet_filters: Array[String] = []
var _last_wetness := -1.0
var _viewport_size := Vector2(1920.0, 1080.0)
var _time := 0.0
var _audio_player: AudioStreamPlayer
var _audio_playback: AudioStreamGeneratorPlayback
var _audio_last := 0.0
var _logged := false


func _ready() -> void:
	_rendering_method = RenderingServer.get_current_rendering_method()
	if _rendering_method == "gl_compatibility":
		_compat_fallback = true
		push_warning("[RainSystem] gl_compatibility 后端：GPUParticles3D 会静默零粒子，"
			+ "已启用程序化屏幕雨痕降级；建议导出使用 mobile 渲染后端。")
	if Engine.is_editor_hint():
		_build()
		set_process(true)
		return
	_build()
	_find_runtime_context()
	_apply_records(_records)
	_collect_wet_materials()
	_apply_parameters()
	set_process(true)
	call_deferred("_apply_parameters")


func setup(world: WorldBody, cars: Array, records: Array, camera: Camera3D = null) -> void:
	_world = world
	_cars = cars
	_records = records
	if camera != null:
		_camera = camera
	_apply_records(_records)
	_collect_wet_materials()
	_apply_parameters()


func set_weather(rain: float, wetness: float, wind: Vector3) -> void:
	rain_amount = clampf(rain, 0.0, 1.0)
	ground_wetness = clampf(wetness, 0.0, 1.0)
	if wind.length_squared() > 1e-6:
		wind_speed = clampf(wind.length(), 0.0, 40.0)
		wind_direction_deg = fposmod(rad_to_deg(atan2(wind.x, wind.z)), 360.0)


func is_compat_fallback() -> bool:
	return _compat_fallback


func get_rendering_method() -> String:
	return _rendering_method


func _process(delta: float) -> void:
	_time += delta
	if not _built:
		_build()
	if _camera == null or not is_instance_valid(_camera):
		_camera = _find_camera()
	if _camera == null:
		return

	# 根节点独立于相机旋转；每帧只同步位置。GPUParticles3D.local_coords=false，
	# 已发射粒子留在世界，只有新粒子在相机附近生成，因此快速转头不会粘屏。
	top_level = true
	global_transform = Transform3D(Basis.IDENTITY, _camera.global_position + Vector3(0.0, 2.5, 0.0))

	if auto_weather and _world != null and is_instance_valid(_world):
		var weather: Dictionary = _world.get_weather()
		set_weather(
			float(weather.get("rain", rain_amount)),
			float(weather.get("wetness", ground_wetness)),
			weather.get("wind", Vector3.ZERO) as Vector3
		)

	if _parameters_dirty:
		_apply_parameters()
	_update_fall_direction()
	_update_particles()
	_update_ground_feedback(delta)
	_update_wetness()
	_update_atmosphere()
	_update_screen_overlay()
	if enable_audio:
		_update_audio()


func _build() -> void:
	if _built:
		return
	_built = true
	_viewport_size = Vector2(get_viewport().get_visible_rect().size) if get_viewport() != null else Vector2(1920, 1080)
	get_viewport().size_changed.connect(_on_viewport_size_changed)

	_rain_near_shader = _make_rain_material(0)
	_rain_far_shader = _make_rain_material(1)
	_rain_near = _make_rain_layer("RainNear", _rain_near_shader, 24.0, 0.7, 1.4)
	_rain_far = _make_rain_layer("RainFar", _rain_far_shader, 52.0, 0.5, 1.6)
	_rain_far.visible = false

	_splash = _make_splash()
	_make_ripples()
	_make_screen_overlay()
	_find_environment()
	_build_audio()
	_apply_parameters()


func _make_rain_material(layer: int) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = RAIN_DROP_SHADER
	material.set_shader_parameter("drop_color", drop_color)
	material.set_shader_parameter("brightness", drop_brightness * (1.0 if layer == 0 else 0.55))
	material.set_shader_parameter("base_width", drop_width * (1.0 if layer == 0 else 1.35))
	material.set_shader_parameter("drop_length", drop_length * stretch_multiplier * (1.0 if layer == 0 else 1.5))
	material.set_shader_parameter("min_screen_width_px", min_screen_width_px)
	material.set_shader_parameter("max_distance", rain_visibility_distance)
	material.set_shader_parameter("viewport_size", _viewport_size)
	material.set_shader_parameter("jitter_strength", jitter_strength)
	return material


func _make_rain_layer(name: String, material: ShaderMaterial, box_extent: float, scale_min: float, scale_max: float) -> GPUParticles3D:
	var process := ParticleProcessMaterial.new()
	process.direction = Vector3(0.0, -1.0, 0.0)
	process.spread = 1.5
	process.initial_velocity_min = 32.0
	process.initial_velocity_max = 56.0
	process.gravity = Vector3(0.0, -9.8, 0.0)
	process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	process.emission_box_extents = Vector3(box_extent, 6.0, box_extent)
	process.scale_min = scale_min
	process.scale_max = scale_max

	var quad := QuadMesh.new()
	quad.size = Vector2(1.0, 1.0)
	quad.material = material

	var particles := GPUParticles3D.new()
	particles.name = name
	particles.process_material = process
	particles.draw_pass_1 = quad
	particles.amount = 1
	particles.lifetime = 0.9 if name == "RainNear" else 1.4
	particles.local_coords = false
	particles.emitting = false
	particles.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	particles.visibility_aabb = AABB(Vector3(-box_extent, -12, -box_extent), Vector3(box_extent * 2.0, 28.0, box_extent * 2.0))
	add_child(particles)
	if name == "RainNear":
		_rain_near_process = process
	else:
		_rain_far_process = process
	return particles


func _make_splash() -> GPUParticles3D:
	_splash_process = ParticleProcessMaterial.new()
	_splash_process.direction = Vector3(0.0, 1.0, 0.0)
	_splash_process.spread = 32.0
	_splash_process.initial_velocity_min = 0.25
	_splash_process.initial_velocity_max = 1.0
	_splash_process.gravity = Vector3(0.0, -9.8, 0.0)
	_splash_process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	_splash_process.emission_box_extents = Vector3(16.0, 0.05, 16.0)
	_splash_process.scale_min = 0.25
	_splash_process.scale_max = 0.7

	# 水花用小球而不是 quad：近距离 quad 会变成明显的白方块；
	# 小球在移动端片元成本更低，也不会出现 UV 硬边。
	var drop := SphereMesh.new()
	drop.radius = 0.012
	drop.height = 0.024
	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = Color(drop_color.r, drop_color.g, drop_color.b, 0.12)
	drop.material = material

	var particles := GPUParticles3D.new()
	particles.name = "RainSplash"
	particles.process_material = _splash_process
	particles.draw_pass_1 = drop
	particles.amount = 1
	particles.lifetime = 0.22
	particles.local_coords = false
	particles.emitting = false
	particles.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	particles.visibility_aabb = AABB(Vector3(-22, -2, -22), Vector3(44, 8, 44))
	add_child(particles)
	return particles


func _make_ripples() -> void:
	var plane := PlaneMesh.new()
	plane.size = Vector2(1.0, 1.0)
	_ripple_material = ShaderMaterial.new()
	_ripple_material.shader = RAIN_RIPPLE_SHADER
	_ripple_material.set_shader_parameter("ripple_color", drop_color)

	_ripple_multimesh = MultiMesh.new()
	_ripple_multimesh.transform_format = MultiMesh.TRANSFORM_3D
	_ripple_multimesh.use_colors = true
	_ripple_multimesh.mesh = plane
	_ripple_multimesh.instance_count = 0

	_ripple_instance = MultiMeshInstance3D.new()
	_ripple_instance.name = "RainRipples"
	_ripple_instance.multimesh = _ripple_multimesh
	_ripple_instance.material_override = _ripple_material
	_ripple_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_ripple_instance)


func _make_screen_overlay() -> void:
	_screen_layer = CanvasLayer.new()
	_screen_layer.name = "RainScreen"
	_screen_layer.layer = 9
	add_child(_screen_layer)
	_screen_rect = ColorRect.new()
	_screen_rect.name = "RainScreenRect"
	_screen_rect.set_anchors_preset(Control.PRESET_FULL_RECT)
	_screen_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_screen_material = ShaderMaterial.new()
	_screen_material.shader = RAIN_SCREEN_SHADER
	_screen_material.set_shader_parameter("viewport_size", _viewport_size)
	_screen_rect.material = _screen_material
	_screen_rect.visible = false
	_screen_layer.add_child(_screen_rect)


func _find_environment() -> void:
	var root := get_tree().current_scene
	if root != null:
		for node in root.find_children("*", "WorldEnvironment", true, false):
			var world_env := node as WorldEnvironment
			if world_env != null and world_env.environment != null:
				_world_env = world_env
				_environment = world_env.environment
				return
	_world_env = WorldEnvironment.new()
	_world_env.name = "RainEnvironment"
	var env := Environment.new()
	env.background_mode = Environment.BG_SKY
	env.sky = Sky.new()
	env.sky.sky_material = ProceduralSkyMaterial.new()
	_world_env.environment = env
	add_child(_world_env)
	_environment = env


func _build_audio() -> void:
	if not enable_audio:
		return
	var generator := AudioStreamGenerator.new()
	generator.mix_rate = 22050.0
	generator.buffer_length = 0.4
	_audio_player = AudioStreamPlayer.new()
	_audio_player.name = "RainAudio"
	_audio_player.stream = generator
	_audio_player.volume_db = rain_audio_db
	add_child(_audio_player)
	_audio_player.play()
	_audio_playback = _audio_player.get_stream_playback() as AudioStreamGeneratorPlayback


func _find_runtime_context() -> void:
	_camera = _find_camera()
	_world = _find_world()
	if _world != null:
		_cars = _find_cars(_world)


func _find_camera() -> Camera3D:
	if camera_path != NodePath() and has_node(camera_path):
		return get_node(camera_path) as Camera3D
	if get_parent() is Camera3D:
		return get_parent() as Camera3D
	return get_viewport().get_camera_3d()


func _find_world() -> WorldBody:
	var root := get_tree().current_scene
	if root == null:
		return null
	for node in root.find_children("*", "WorldBody", true, false):
		if node is WorldBody:
			return node as WorldBody
	return null


func _find_cars(world: WorldBody) -> Array:
	var cars: Array = []
	for node in world.find_children("*", "VehicleBody", true, false):
		if node is VehicleBody:
			cars.append(node)
	return cars


func _apply_records(records: Array) -> void:
	_wet_filters.clear()
	for record in records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.rain":
			continue
		var values: Dictionary = record.get("values", {})
		for key in ["puddles_materials", "soaking_materials", "smooth_materials", "rough_materials", "lines_materials"]:
			if not values.has(key):
				continue
			for part in _parts(values[key]):
				var text := str(part).strip_edges()
				if not text.is_empty() and not _wet_filters.has(text):
					_wet_filters.append(text)


func _apply_parameters() -> void:
	if not _built:
		return
	_parameters_dirty = false
	_compat_fallback = _rendering_method == "gl_compatibility"

	var tier := clampi(int(quality), 0, 2)
	var near_max := int(MAX_NEAR_BY_QUALITY[tier])
	var far_max := int(MAX_FAR_BY_QUALITY[tier])
	var splash_max := int(MAX_SPLASH_BY_QUALITY[tier])
	if mobile_low_end:
		near_max = int(near_max * 0.55)
		far_max = int(far_max * 0.45)
		splash_max = int(splash_max * 0.6)

	_rain_near.amount = maxi(1, int(near_max * rain_amount))
	_rain_far.amount = maxi(1, int(far_max * rain_amount))
	_rain_far.visible = far_max > 0
	_splash.amount = maxi(1, mini(SPLASH_AMOUNT_CAP, int(splash_max * rain_amount)))
	_rain_near_shader.set_shader_parameter("drop_color", drop_color)
	_rain_far_shader.set_shader_parameter("drop_color", drop_color)
	_rain_near_shader.set_shader_parameter("brightness", drop_brightness)
	_rain_far_shader.set_shader_parameter("brightness", drop_brightness * 0.5)
	_rain_near_shader.set_shader_parameter("base_width", drop_width)
	_rain_far_shader.set_shader_parameter("base_width", drop_width * 1.4)
	_rain_near_shader.set_shader_parameter("drop_length", drop_length * stretch_multiplier)
	_rain_far_shader.set_shader_parameter("drop_length", drop_length * stretch_multiplier * 1.5)
	_rain_near_shader.set_shader_parameter("min_screen_width_px", min_screen_width_px)
	_rain_far_shader.set_shader_parameter("min_screen_width_px", min_screen_width_px)
	_rain_near_shader.set_shader_parameter("max_distance", rain_visibility_distance)
	_rain_far_shader.set_shader_parameter("max_distance", rain_visibility_distance * 1.35)
	_rain_near_shader.set_shader_parameter("jitter_strength", jitter_strength)
	_rain_far_shader.set_shader_parameter("jitter_strength", jitter_strength)
	_ripple_material.set_shader_parameter("ripple_color", drop_color)
	_splash.amount = maxi(1, _splash.amount)
	_splash.emitting = rain_amount > 0.02 and ground_wetness > 0.05 and not _compat_fallback

	var effective_ripples := mini(max_ripples, int(MAX_RIPPLES_BY_QUALITY[tier]))
	if mobile_low_end:
		effective_ripples = mini(effective_ripples, 2)
	_effective_ripple_limit = effective_ripples
	_apply_viewport_quality(tier)
	_update_screen_overlay()
	if not _logged:
		_logged = true
		print("[RainSystem] method=%s compat=%s near=%d far=%d splash=%d ripples=%d" % [
			_rendering_method, _compat_fallback, _rain_near.amount, _rain_far.amount,
			_splash.amount, max_ripples])


func _apply_viewport_quality(tier: int) -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	# Mobile 不用 TAA/FSR2；只用 MSAA/FXAA，低配直接用 FXAA。
	match tier:
		0:
			viewport.msaa_3d = Viewport.MSAA_DISABLED
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			viewport.scaling_3d_scale = 0.85 if mobile_low_end else 0.92
		1:
			viewport.msaa_3d = Viewport.MSAA_2X
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			viewport.scaling_3d_scale = 0.9 if mobile_low_end else 1.0
		_:
			viewport.msaa_3d = Viewport.MSAA_2X
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			viewport.scaling_3d_scale = 1.0 if not mobile_low_end else 0.92


func _update_fall_direction() -> void:
	var wind_rad := deg_to_rad(wind_direction_deg)
	var horizontal := Vector3(cos(wind_rad), 0.0, sin(wind_rad)) * wind_speed
	# 低频湍流只扰动水平分量，保持主方向一致；风为 0 时水平分量为 0。
	var gust := turbulence * sin(_time * 0.7 + _camera.global_position.x * 0.03) * 0.35
	var lateral := Vector3(-sin(wind_rad), 0.0, cos(wind_rad)) * gust
	var fall := Vector3(horizontal.x + lateral.x, -fall_speed_mps, horizontal.z + lateral.z).normalized()
	_rain_near_shader.set_shader_parameter("fall_direction", fall)
	_rain_far_shader.set_shader_parameter("fall_direction", fall)
	# 风向 → 倾角：atan(|horizontal| / fall_speed)。这里不直接设置角度，
	# 而是把合速度传给 shader，shader 用 cross(fall, to_camera) 构造面向相机的
	# 细长 quad；风为 0 时 fall=(0,-1,0)，雨丝垂直。


func _update_particles() -> void:
	var emitting := rain_amount > 0.02 and not _compat_fallback
	_rain_near.emitting = emitting
	_rain_far.emitting = emitting and _rain_far.visible
	if _rain_near_process != null:
		var direction := Vector3(0.0, -1.0, 0.0)
		var wind_rad := deg_to_rad(wind_direction_deg)
		direction = Vector3(cos(wind_rad) * wind_speed * 0.08, -fall_speed_mps, sin(wind_rad) * wind_speed * 0.08).normalized()
		_rain_near_process.direction = direction
		_rain_far_process.direction = direction
		_rain_near_process.spread = 1.0 + turbulence * 2.0
		_rain_far_process.spread = 1.0 + turbulence * 2.0


func _update_ground_feedback(delta: float) -> void:
	if _compat_fallback:
		_splash.emitting = false
	elif _splash != null:
		_splash.global_position = Vector3(_camera.global_position.x, _ground_height(_camera.global_position.x, _camera.global_position.z), _camera.global_position.z)
		_splash.emitting = rain_amount > 0.02 and ground_wetness > 0.05

	if _effective_ripple_limit <= 0 or rain_amount <= 0.02:
		_ripples.clear()
		_ripple_multimesh.instance_count = 0
		return

	_ripple_accum += delta * (1.5 + rain_amount * 9.0) * splash_strength
	while _ripple_accum >= 1.0 and _ripples.size() < _effective_ripple_limit:
		_ripple_accum -= 1.0
		_spawn_ripple()

	var alive: Array[Dictionary] = []
	for ripple in _ripples:
		ripple["age"] += delta
		if ripple["age"] >= ripple["life"]:
			continue
		alive.append(ripple)
	_ripples = alive

	_ripple_multimesh.instance_count = 0
	_ripple_multimesh.instance_count = _ripples.size()
	for index in _ripples.size():
		var ripple: Dictionary = _ripples[index]
		var t: float = ripple["age"] / ripple["life"]
		var size: float = ripple["scale"] * (1.0 + t * 1.5)
		var basis := Basis.IDENTITY.scaled(Vector3(size, 1.0, size))
		_ripple_multimesh.set_instance_transform(index, Transform3D(basis, ripple["position"] as Vector3))
		var color: Color = drop_color
		color.a = (1.0 - t) * 0.45 * rain_amount
		_ripple_multimesh.set_instance_color(index, color)


func _spawn_ripple() -> void:
	var angle := randf_range(0.0, TAU)
	var radius := sqrt(randf()) * 12.0
	var x := _camera.global_position.x + cos(angle) * radius
	var z := _camera.global_position.z + sin(angle) * radius
	var kind := _surface_kind_at(x, z)
	if kind != "" and kind != "Asphalt" and kind != "Concrete" and kind != "Curb":
		return
	_ripples.append({
		"position": Vector3(x, _ground_height(x, z) + 0.015, z),
		"age": 0.0,
		"life": randf_range(0.55, 1.0),
		"scale": ripple_size * randf_range(0.7, 1.25),
	})


func _update_wetness() -> void:
	if not wet_surface_enabled:
		return
	if absf(ground_wetness - _last_wetness) < 0.02:
		return
	_last_wetness = ground_wetness
	for entry in _wet_entries:
		var material: StandardMaterial3D = entry["material"]
		if material == null:
			continue
		var base_albedo: Color = entry["albedo"]
		var base_roughness: float = entry["roughness"]
		var wet := clampf(ground_wetness, 0.0, 1.0)
		material.albedo_color = base_albedo.darkened(0.28 * wet)
		# Mobile 无 SSR：用 roughness + 环境反射 + 雾近似湿感；不要压到 0，
		# 否则天空反射在 tile GPU 上容易变成过曝白斑。
		material.roughness = maxf(lerpf(base_roughness, 0.32, 0.6 * wet), 0.32)
		material.metallic = minf(material.metallic, 0.10)


func _update_atmosphere() -> void:
	if _environment == null:
		return
	var wet := clampf(ground_wetness, 0.0, 1.0)
	var rain := clampf(rain_amount, 0.0, 1.0)
	# 仅 depth fog / height fog；Mobile 禁用 volumetric_fog。
	_environment.fog_enabled = rain > 0.02 or wet > 0.05
	_environment.fog_density = lerpf(0.0004, fog_density, maxf(rain, wet * 0.7))
	_environment.fog_light_color = Color(0.52, 0.58, 0.68).lerp(Color(0.40, 0.45, 0.55), rain)
	if _environment.get("fog_height") != null:
		_environment.set("fog_height", 0.0)
		_environment.set("fog_height_density", lerpf(0.0, 0.08, rain))
	_environment.adjustment_enabled = true
	_environment.adjustment_contrast = lerpf(1.0, 0.88, rain)
	_environment.adjustment_saturation = lerpf(1.0, 0.82, rain)


func _update_screen_overlay() -> void:
	if _screen_rect == null:
		return
	var tier := clampi(int(quality), 0, 2)
	var allowed := screen_drops_enabled and tier > 0 and not mobile_low_end
	if _compat_fallback:
		# Compatibility 没有 GPU 粒子；屏幕雨痕是显式降级路径。
		allowed = rain_amount > 0.02
	_screen_rect.visible = allowed
	if allowed:
		_screen_material.set_shader_parameter("intensity", clampf(rain_amount * 0.55, 0.0, 0.55))
		_screen_material.set_shader_parameter("viewport_size", _viewport_size)
		_screen_material.set_shader_parameter("wind", Vector2(cos(deg_to_rad(wind_direction_deg)), sin(deg_to_rad(wind_direction_deg))) * wind_speed * 0.08)


func _update_audio() -> void:
	if _audio_playback == null or _audio_player == null:
		return
	_audio_player.volume_db = rain_audio_db
	var frames := _audio_playback.get_frames_available()
	if frames <= 0:
		return
	var amplitude := clampf(rain_amount, 0.0, 1.0) * 0.18
	for _i in frames:
		# 低通白噪声，避免每帧生成高频刺耳声；默认关闭，仅作可选氛围。
		var sample := randf_range(-1.0, 1.0)
		_audio_last = lerpf(_audio_last, sample, 0.08)
		_audio_playback.push_frame(Vector2(_audio_last * amplitude, _audio_last * amplitude))


func _collect_wet_materials() -> void:
	if not wet_surface_enabled:
		return
	_wet_entries.clear()
	var root: Node = _world if _world != null else get_tree().current_scene
	if root == null:
		return
	for node in root.find_children("*", "MeshInstance3D", true, false):
		if _wet_entries.size() >= max_wet_materials:
			break
		var mesh := node as MeshInstance3D
		if mesh == null or mesh.mesh == null or _under_vehicle(mesh):
			continue
		for surface in mesh.mesh.get_surface_count():
			var source := mesh.get_surface_override_material(surface)
			if source == null:
				source = mesh.mesh.surface_get_material(surface)
			if not (source is StandardMaterial3D):
				continue
			var material_name := String(source.resource_name).to_lower()
			if not _wet_filters.is_empty() and not _matches_filter(material_name):
				continue
			var duplicate := (source as StandardMaterial3D).duplicate() as StandardMaterial3D
			mesh.set_surface_override_material(surface, duplicate)
			_wet_entries.append({
				"material": duplicate,
				"albedo": duplicate.albedo_color,
				"roughness": duplicate.roughness,
			})
			if _wet_entries.size() >= max_wet_materials:
				break
	_last_wetness = -1.0


func _matches_filter(material_name: String) -> bool:
	for filter in _wet_filters:
		var needle := filter.to_lower().trim_prefix("*").trim_suffix("*")
		if not needle.is_empty() and material_name.contains(needle):
			return true
	return false


func _under_vehicle(node: Node) -> bool:
	var current := node
	while current != null:
		if current is VehicleBody:
			return true
		current = current.get_parent()
	return false


func _ground_height(x: float, z: float) -> float:
	if _world != null and is_instance_valid(_world):
		return float(_world.surface_height_at(x, z))
	return fallback_ground_y


func _surface_kind_at(x: float, z: float) -> String:
	if _world != null and is_instance_valid(_world):
		return String(_world.surface_kind_at(x, z))
	return ""


func _parts(value: Variant) -> Array:
	if value is Array:
		return value
	return str(value).split(",", false)


func _on_viewport_size_changed() -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	_viewport_size = Vector2(viewport.get_visible_rect().size)
	if _rain_near_shader != null:
		_rain_near_shader.set_shader_parameter("viewport_size", _viewport_size)
		_rain_far_shader.set_shader_parameter("viewport_size", _viewport_size)
	if _screen_material != null:
		_screen_material.set_shader_parameter("viewport_size", _viewport_size)
