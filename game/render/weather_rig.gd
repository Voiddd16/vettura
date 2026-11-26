extends WorldEnvironment
## 天气渲染状态机（P0）。
##
## 单一 `wetness ∈ [0,1]` 是全场唯一天气标量：
##   WeatherRig.wetness
##     -> RenderingServer.global_shader_parameter_set("wetness", w)
##     -> rain_drop / grass_blade / wet_road shader
##     -> Environment / DirectionalLight3D / glow / fog / ReflectionProbe
##     -> RainSystem（rain_amount / ground_wetness 由本节点派生写入）
##
## 8 秒渐变：target_wetness 用 move_toward(delta / 8.0) 逼近，不瞬切、不叠
## 全屏 ColorRect。dry / drizzle / storm 三个 preset 只定义目标 wetness 和
## 插值端点。
##
## 用法：
##   $WeatherRig.set_weather("storm")
##   $WeatherRig.debug_set_wetness(0.65)
##   $WeatherRig.sync_from_world($WorldBody)   # WeatherFX 自动接入
##
## 反射亮源：
## - 桌面 Forward+：SSR + ReflectionProbe；
## - 移动 Mobile：显式关闭 SSR，使用同一个 ReflectionProbe 的烘焙 cubemap。
## - 雨天把发光材质 / Light3D 能量提高，配合 glow 溢出；湿路面才能看到
##   灯杆/车灯的拉长倒影。
##
## 注意：SSAO/SSIL/SSR/体积雾是 Forward+ 专用；Mobile 显式走 probe 分支。

const BASE_ENV := "res://game_environment.tres"
const WET_ROAD_SHADER := preload("res://game/render/wet_road.gdshader")

## dry / storm 端点。中间用 wetness 线性插值。
const DRY := {
	"tonemap_exposure": 1.15,
	"tonemap_agx_white": 12.0,
	"tonemap_agx_contrast": 1.08,
	"ambient_color": Color(0.72, 0.77, 0.85),
	"ambient_energy": 2.6,
	"fog_density": 0.0008,
	"fog_aerial": 0.30,
	"fog_sky": 0.10,
	"glow_threshold": 1.5,
	"glow_intensity": 0.18,
	"glow_bloom": 0.015,
	"contrast": 1.04,
	"saturation": 1.0,
	"sun_energy": 2.8,
	"sun_color": Color(1.0, 0.94, 0.84),
	"shadow_opacity": 1.0,
	"shadow_blur": 1.0,
	"sun_angular": 0.5,
	"shadow_distance": 120.0,
	"sky_top": Color(0.13, 0.32, 0.66),
	"sky_horizon": Color(0.62, 0.76, 0.90),
	"cloud_coverage": 0.30,
	"cloud_opacity": 0.72,
	"sky_exposure": 1.0,
}

const STORM := {
	"tonemap_exposure": 1.30,
	"tonemap_agx_white": 8.0,
	"tonemap_agx_contrast": 1.0,
	"ambient_color": Color(0.52, 0.57, 0.64),
	"ambient_energy": 2.6,
	"fog_density": 0.006,
	"fog_aerial": 0.70,
	"fog_sky": 0.38,
	"glow_threshold": 1.25,
	"glow_intensity": 0.30,
	"glow_bloom": 0.055,
	"contrast": 0.96,
	"saturation": 0.82,
	"sun_energy": 0.40,
	"sun_color": Color(0.52, 0.62, 0.82),
	"shadow_opacity": 0.14,
	"shadow_blur": 3.8,
	"sun_angular": 7.0,
	"shadow_distance": 70.0,
	"sky_top": Color(0.42, 0.46, 0.52),
	"sky_horizon": Color(0.62, 0.65, 0.69),
	"cloud_coverage": 0.96,
	"cloud_opacity": 1.0,
	"sky_exposure": 0.95,
}

@export_group("Bindings")
## 不设时自动找当前场景第一盏 DirectionalLight3D；找不到则创建。
@export var sun: DirectionalLight3D
@export var camera: Camera3D
## 这些不参与 wet shader（避免把看台/墙体当路面）。
@export var wet_surface_keywords: PackedStringArray = PackedStringArray([
	"asphalt", "road", "tarmac", "rda", "surface", "line", "kerb", "apron",
	"pitlane", "concrete", "pavement", "startfinish", "sfline", "finishline",
	"startline", "tiles", "grille",
])

@export_group("Weather")
@export_range(0.0, 1.0, 0.001) var wetness := 0.0
@export_range(0.0, 1.0, 0.001) var target_wetness := 0.0
@export_range(1.0, 30.0, 0.1) var transition_seconds := 8.0
## 手动 set_weather/debug_set_wetness 后不再被 WorldBody 覆盖；调用
## `use_world_weather()` 恢复。
@export var manual_override := false

@export_group("Reflection")
@export var probe_size := Vector3(240.0, 70.0, 240.0)
@export var probe_refresh_interval := 4.0
@export var contact_shadow_enabled := true

var _forward_plus := false
var _sky_material: ShaderMaterial
var _sun_auto: DirectionalLight3D
var _probe: ReflectionProbe
var _probe_refreshing := false
var _probe_timer := 0.0
var _scan_timer := 0.0
var _runtime_timer := 0.0
var _runtime: Dictionary = {}
var _wet_materials: Dictionary = {}
var _emissive: Dictionary = {}
var _lights: Dictionary = {}
var _bulbs: Dictionary = {}
var _wetness_mask: Texture2D
var _neutral_normal: Texture2D
var _contact_shadow: MeshInstance3D
var _cloud_offset := Vector2.ZERO
var _cloud_accum := 0.0
const CLOUD_UPDATE_HZ := 10.0
var _wet_scan_done := false
var _applied_wetness := -1.0
var _last_manual := false
## 场景结构变化时才重新发现 WorldBody/Car/RainSystem；否则每 0.5 s 全树
## `find_children` 在几千节点的赛道上纯属浪费。
var _runtime_dirty := true
## 只有场景新增节点时才重扫 Light3D（新车灯/路灯）；不每帧/每秒遍历。
var _lights_dirty := true


func _ready() -> void:
	_forward_plus = RenderingServer.get_current_rendering_method() == "forward_plus"
	_prepare_environment()
	_prepare_resources()
	_find_bindings()
	_create_probe()
	call_deferred("_refresh_probe")
	# start at dry, then let the world weather drive if available.
	wetness = 0.0
	target_wetness = 0.0
	# 调试：VETTURA_WETNESS=0.85 直接冻结到指定湿度，方便 A/B 截图。
	var env_wet := OS.get_environment("VETTURA_WETNESS")
	if env_wet.is_valid_float():
		manual_override = true
		wetness = clampf(env_wet.to_float(), 0.0, 1.0)
		target_wetness = wetness
	_apply_wetness(wetness, true)
	RenderingServer.global_shader_parameter_set("wetness", wetness)
	set_process(true)
	print("[weather-rig] method=%s forward_plus=%s probe=%s" % [
		RenderingServer.get_current_rendering_method(), str(_forward_plus),
		str(_probe != null)])
	if get_tree() != null:
		# 不能用 current_scene.child_entered_tree：它只收到直接子节点，
		# 车辆挂在 WorldBody 下时永远触发不到。用全局 node_added/removed
		# 只置脏标记，真正的扫描仍由 0.5s 节流器做。
		get_tree().node_added.connect(_on_scene_child)
		get_tree().node_removed.connect(_on_scene_child)


func _prepare_environment() -> void:
	if environment == null:
		var base := load(BASE_ENV) as Environment
		if base != null:
			environment = base.duplicate(true) as Environment
		else:
			environment = Environment.new()
	environment = environment.duplicate(true) as Environment
	if environment.sky != null:
		environment.sky = environment.sky.duplicate(true) as Sky
		var mat := environment.sky.sky_material
		if mat is ShaderMaterial:
			_sky_material = (mat as ShaderMaterial).duplicate() as ShaderMaterial
			environment.sky.sky_material = _sky_material
			# 继承共享材质已经漂移到的云相位，切场景时云不跳回 0。
			var inherited: Variant = _sky_material.get_shader_parameter("cloud_offset")
			if inherited is Vector2:
				_cloud_offset = inherited as Vector2
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	# 初始/干燥用 sky；Mobile wet 分支在 _apply_wetness 里切到 probe。
	environment.reflected_light_source = Environment.REFLECTION_SOURCE_SKY
	environment.tonemap_mode = Environment.TONE_MAPPER_AGX
	environment.fog_enabled = true
	environment.fog_height = 0.0
	environment.fog_height_density = 0.0
	# 桌面屏幕空间效果；移动端显式关闭，走 ReflectionProbe。
	environment.ssao_enabled = _forward_plus
	environment.ssil_enabled = _forward_plus
	environment.ssr_enabled = _forward_plus
	environment.ssr_max_steps = 96
	environment.ssr_fade_in = 0.12
	environment.ssr_fade_out = 2.0
	environment.ssr_depth_tolerance = 0.2
	environment.volumetric_fog_enabled = _forward_plus
	environment.volumetric_fog_density = 0.0
	environment.volumetric_fog_albedo = Color(0.66, 0.72, 0.80)
	environment.volumetric_fog_anisotropy = 0.45
	environment.volumetric_fog_length = 96.0
	environment.volumetric_fog_ambient_inject = 0.15
	environment.volumetric_fog_sky_affect = 0.15
	environment.glow_enabled = true
	environment.glow_blend_mode = Environment.GLOW_BLEND_MODE_SCREEN
	environment.adjustment_enabled = true


func _prepare_resources() -> void:
	var noise := FastNoiseLite.new()
	noise.seed = 1337
	noise.noise_type = FastNoiseLite.TYPE_SIMPLEX_SMOOTH
	noise.frequency = 0.008
	noise.fractal_type = FastNoiseLite.FRACTAL_FBM
	noise.fractal_octaves = 4
	var mask := NoiseTexture2D.new()
	mask.width = 256
	mask.height = 256
	mask.seamless = true
	mask.generate_mipmaps = true
	mask.noise = noise
	_wetness_mask = mask

	var img := Image.create(1, 1, false, Image.FORMAT_RGBA8)
	img.set_pixel(0, 0, Color(0.5, 0.5, 1.0, 1.0))
	_neutral_normal = ImageTexture.create_from_image(img)


func _find_bindings() -> void:
	if sun == null:
		sun = _find_first_node(get_tree().current_scene, "DirectionalLight3D") as DirectionalLight3D
	if sun == null:
		_sun_auto = DirectionalLight3D.new()
		_sun_auto.name = "WeatherRigSun"
		_sun_auto.rotation_degrees = Vector3(-48.0, -125.0, 0.0)
		_sun_auto.shadow_enabled = true
		_sun_auto.shadow_bias = 0.0001
		_sun_auto.shadow_normal_bias = 0.08
		_sun_auto.directional_shadow_mode = DirectionalLight3D.SHADOW_PARALLEL_4_SPLITS
		var parent := get_tree().current_scene if get_tree().current_scene != null else self
		parent.add_child.call_deferred(_sun_auto)
		sun = _sun_auto
	if camera == null:
		camera = _find_camera()


func _find_camera() -> Camera3D:
	var vp := get_viewport()
	if vp != null:
		var cam := vp.get_camera_3d()
		if cam != null:
			return cam
	return _find_first_node(get_tree().current_scene, "Camera3D") as Camera3D


func _find_first_node(root: Node, type_name: String) -> Node:
	if root == null:
		return null
	if root.is_class(type_name):
		return root
	for child in root.get_children():
		var found := _find_first_node(child, type_name)
		if found != null:
			return found
	return null


func _create_probe() -> void:
	_probe = ReflectionProbe.new()
	_probe.name = "WeatherReflectionProbe"
	_probe.size = probe_size
	_probe.origin_offset = Vector3(0.0, 8.0, 0.0)
	_probe.box_projection = true
	_probe.interior = false
	_probe.enable_shadows = true
	_probe.intensity = 1.0
	_probe.max_distance = 500.0
	_probe.blend_distance = 2.0
	_probe.update_mode = ReflectionProbe.UPDATE_ONCE
	_probe.cull_mask = 0xFFFFFFFF
	add_child(_probe)
	_probe.global_position = _focus_position() + Vector3(0.0, 8.0, 0.0)


func _focus_position() -> Vector3:
	if camera != null and is_instance_valid(camera):
		return camera.global_position
	if _runtime.has("car") and is_instance_valid(_runtime["car"]):
		return (_runtime["car"] as Node3D).global_position
	return Vector3.ZERO


func _refresh_probe() -> void:
	if _probe == null or _probe_refreshing:
		return
	_probe_refreshing = true
	_probe.global_position = _focus_position() + Vector3(0.0, 8.0, 0.0)
	_probe.update_mode = ReflectionProbe.UPDATE_ALWAYS
	await get_tree().process_frame
	await get_tree().process_frame
	if is_instance_valid(_probe):
		_probe.update_mode = ReflectionProbe.UPDATE_ONCE
	_probe_refreshing = false


func _process(delta: float) -> void:
	_process_clouds(delta)
	wetness = move_toward(wetness, target_wetness, delta / maxf(transition_seconds, 0.01))
	RenderingServer.global_shader_parameter_set("wetness", wetness)
	if absf(wetness - _applied_wetness) > 0.0005:
		_apply_wetness(wetness, false)
	_runtime_timer -= delta
	if _runtime_timer <= 0.0:
		_runtime_timer = 0.5
		if _runtime_dirty or _runtime.is_empty():
			_runtime_dirty = false
			_discover_runtime()
		_drive_runtime()
	_scan_timer -= delta
	if _scan_timer <= 0.0:
		_scan_timer = 1.0
		_scan_wet_surfaces()
	_probe_timer -= delta
	if _probe_timer <= 0.0:
		_probe_timer = probe_refresh_interval
		if wetness > 0.15 or not _forward_plus:
			_refresh_probe()


## 云的缓慢漂移。10 Hz 写入：噪声纹理位移不到 1 像素，肉眼无差别，但天空
## uniform 不会每帧触发 radiance cubemap 重算。速率和 `render_quality.gd` /
## `weather_visuals.gd` 保持一致（约 5 分钟一个噪声周期）。
func _process_clouds(delta: float) -> void:
	if _sky_material == null:
		return
	_cloud_offset += Vector2(0.012, 0.004) * delta
	_cloud_offset = _cloud_offset.posmod(1024.0)
	_cloud_accum += delta
	if _cloud_accum < 1.0 / CLOUD_UPDATE_HZ:
		return
	_cloud_accum = 0.0
	_sky_material.set_shader_parameter("cloud_offset", _cloud_offset)


func _on_scene_child(_node: Node) -> void:
	# 场景树每加/删一个节点都会到这里；只置脏，不做任何遍历。
	_runtime_dirty = true
	_lights_dirty = true


func _discover_runtime() -> void:
	_runtime.clear()
	var scene := get_tree().current_scene
	if scene == null:
		return
	_find_bindings()
	for node in scene.find_children("*", "", true, false):
		if node is RainSystem:
			_runtime["rain"] = node
		elif node.get_class() == "VehicleBody":
			if not _runtime.has("car"):
				_runtime["car"] = node
		elif node is WorldBody:
			_runtime["world"] = node
		elif node.name == "GrassFX" or String(node.get_script().resource_path if node.get_script() else "").ends_with("grass_fx.gd"):
			_runtime["grass"] = node
	if _runtime.has("rain"):
		var rain: RainSystem = _runtime["rain"]
		rain.auto_weather = false
		rain.wet_surface_enabled = false
		rain.rain_amount = wetness
		rain.ground_wetness = wetness
		# RainSystem 不再拥有雾/环境；WeatherRig 是唯一 owner。
		rain._environment = null
		var mist := rain.get_node_or_null("RainMist")
		if mist != null:
			(mist as GPUParticles3D).emitting = false
			mist.visible = false
		for child in rain.find_children("*", "BackBufferCopy", true, false):
			var copy := child as BackBufferCopy
			if copy != null:
				copy.copy_mode = BackBufferCopy.COPY_MODE_VIEWPORT
		for child in rain.find_children("*", "ColorRect", true, false):
			var rect := child as ColorRect
			if rect != null and rect.material is ShaderMaterial:
				var refr := rect.material as ShaderMaterial
				if refr.shader != null and String(refr.shader.resource_path).ends_with("rain_refraction.gdshader"):
					refr.set_shader_parameter("quality", 1 if _forward_plus else 0)
					refr.set_shader_parameter("blur_strength", 1.0 if _forward_plus else 0.7)
	if _runtime.has("car"):
		_ensure_contact_shadow(_runtime["car"] as Node3D)
		_scan_car_emissive(_runtime["car"] as Node3D)
	if camera == null or not is_instance_valid(camera):
		camera = _find_camera()


func _drive_runtime() -> void:
	if _runtime.has("rain"):
		var rain: RainSystem = _runtime["rain"]
		if is_instance_valid(rain):
			rain.rain_amount = wetness
			rain.ground_wetness = wetness
	if _runtime.has("world") and not manual_override:
		var world = _runtime["world"]
		if is_instance_valid(world) and world.has_method("get_weather"):
			var w: Dictionary = world.get_weather()
			target_wetness = clampf(maxf(float(w.get("wetness", 0.0)), float(w.get("rain", 0.0))), 0.0, 1.0)
	if _contact_shadow != null and is_instance_valid(_contact_shadow) and _runtime.has("car"):
		var car := _runtime["car"] as Node3D
		if not is_instance_valid(car):
			return
		_contact_shadow.global_position = car.global_position + Vector3(0.0, 0.02, 0.0)
		_contact_shadow.global_rotation = Vector3(0.0, car.global_rotation.y, 0.0)
		var mat := _contact_shadow.material_override as ShaderMaterial
		if mat != null:
			mat.set_shader_parameter("strength", lerpf(0.10, 0.32, wetness))


func _ensure_contact_shadow(car: Node3D) -> void:
	if not contact_shadow_enabled or _contact_shadow != null:
		return
	var plane := PlaneMesh.new()
	plane.size = Vector2(2.6, 4.6)
	var mat := ShaderMaterial.new()
	mat.shader = load("res://game/render/contact_shadow.gdshader")
	mat.set_shader_parameter("strength", 0.10)
	plane.material = mat
	_contact_shadow = MeshInstance3D.new()
	_contact_shadow.name = "WeatherContactShadow"
	_contact_shadow.mesh = plane
	_contact_shadow.material_override = mat
	_contact_shadow.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_contact_shadow)
	_contact_shadow.global_position = car.global_position + Vector3(0.0, 0.02, 0.0)


func _f(key: String, w: float) -> float:
	return lerpf(float(DRY[key]), float(STORM[key]), w)


func _c(key: String, w: float) -> Color:
	return (DRY[key] as Color).lerp(STORM[key], w)


func _apply_wetness(w: float, force: bool) -> void:
	if environment == null:
		return
	_applied_wetness = w
	environment.tonemap_mode = Environment.TONE_MAPPER_AGX
	environment.tonemap_exposure = _f("tonemap_exposure", w)
	if environment.get("tonemap_agx_white") != null:
		environment.set("tonemap_agx_white", _f("tonemap_agx_white", w))
	if environment.get("tonemap_agx_contrast") != null:
		environment.set("tonemap_agx_contrast", _f("tonemap_agx_contrast", w))
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = _c("ambient_color", w)
	environment.ambient_light_energy = _f("ambient_energy", w)
	# 桌面 Forward+ 用 SSR + Sky；Mobile wet 时关闭 env sky reflection，改由
	# ReflectionProbe 提供亮源反射（否则湿路面会被亮天空刷成灰色，和桌面
	# 的 SSR 暗反射不一致）。干燥时两边都用 sky，保持晴天光比一致。
	if _forward_plus or w < 0.08:
		environment.reflected_light_source = Environment.REFLECTION_SOURCE_SKY
	else:
		environment.reflected_light_source = Environment.REFLECTION_SOURCE_DISABLED
	environment.fog_enabled = true
	environment.fog_density = _f("fog_density", w)
	environment.fog_aerial_perspective = _f("fog_aerial", w)
	environment.fog_sky_affect = _f("fog_sky", w)
	environment.fog_light_color = Color(0.62, 0.68, 0.76).lerp(Color(0.55, 0.60, 0.66), w)
	environment.fog_light_energy = lerpf(1.0, 0.85, w)
	# 关键：不要 height fog 的平面雾带；雾由距离/体积负责。
	environment.fog_height = 0.0
	environment.fog_height_density = 0.0
	if _forward_plus:
		environment.ssao_enabled = true
		environment.ssil_enabled = true
		environment.ssr_enabled = w > 0.05
		environment.volumetric_fog_enabled = w > 0.10
		environment.volumetric_fog_density = lerpf(0.0, 0.006, w)
		environment.volumetric_fog_albedo = Color(0.62, 0.68, 0.76).lerp(Color(0.35, 0.40, 0.48), w)
		environment.volumetric_fog_anisotropy = lerpf(0.35, 0.65, w)
		environment.volumetric_fog_length = lerpf(96.0, 56.0, w)
		environment.volumetric_fog_sky_affect = lerpf(0.10, 0.28, w)
	else:
		environment.ssao_enabled = false
		environment.ssil_enabled = false
		environment.ssr_enabled = false
		environment.volumetric_fog_enabled = false
	environment.glow_enabled = true
	environment.glow_hdr_threshold = _f("glow_threshold", w)
	environment.glow_intensity = _f("glow_intensity", w)
	environment.glow_bloom = _f("glow_bloom", w)
	environment.glow_hdr_scale = lerpf(2.0, 3.0, w)
	environment.glow_hdr_luminance_cap = lerpf(8.0, 14.0, w)
	environment.glow_blend_mode = Environment.GLOW_BLEND_MODE_SCREEN
	environment.adjustment_enabled = true
	environment.adjustment_contrast = _f("contrast", w)
	environment.adjustment_saturation = _f("saturation", w)
	if _probe != null:
		_probe.intensity = 1.0 if _forward_plus else lerpf(1.0, 0.18, smoothstep(0.10, 0.80, w))
	_apply_sky(w)
	_apply_sun(w)
	_apply_emissive(w)
	if _sky_material != null:
		_sky_material.set_shader_parameter("cloud_quality", 1.0 if _forward_plus else 0.0)


func _apply_sky(w: float) -> void:
	if _sky_material == null:
		return
	_sky_material.set_shader_parameter("sky_top_color",
		(DRY["sky_top"] as Color).lerp(STORM["sky_top"], w))
	_sky_material.set_shader_parameter("sky_horizon_color",
		(DRY["sky_horizon"] as Color).lerp(STORM["sky_horizon"], w))
	_sky_material.set_shader_parameter("cloud_coverage",
		lerpf(float(DRY["cloud_coverage"]), float(STORM["cloud_coverage"]), w))
	_sky_material.set_shader_parameter("cloud_opacity",
		lerpf(float(DRY["cloud_opacity"]), float(STORM["cloud_opacity"]), w))
	_sky_material.set_shader_parameter("cloud_sharpness", lerpf(0.24, 0.12, w))
	_sky_material.set_shader_parameter("cloud_lit_color",
		Color(0.96, 0.97, 0.99).lerp(Color(0.58, 0.62, 0.68), w))
	_sky_material.set_shader_parameter("cloud_shadow_color",
		Color(0.42, 0.46, 0.53).lerp(Color(0.26, 0.29, 0.34), w))
	_sky_material.set_shader_parameter("sky_exposure",
		lerpf(float(DRY["sky_exposure"]), float(STORM["sky_exposure"]), w))
	# 阴雨时太阳盘/光晕必须压掉；否则低粗糙度湿路面会反射出一个巨大的
	# 白色太阳斑（真实暴雨里看不到太阳）。
	_sky_material.set_shader_parameter("sun_disc_intensity", lerpf(1.0, 0.04, w))
	_sky_material.set_shader_parameter("sun_glow_intensity", lerpf(0.80, 0.08, w))


func _apply_sun(w: float) -> void:
	if sun == null or not is_instance_valid(sun):
		_find_bindings()
		if sun == null:
			return
	sun.light_energy = lerpf(float(DRY["sun_energy"]), float(STORM["sun_energy"]), w)
	sun.light_color = (DRY["sun_color"] as Color).lerp(STORM["sun_color"], w)
	sun.shadow_enabled = true
	sun.shadow_opacity = lerpf(float(DRY["shadow_opacity"]), float(STORM["shadow_opacity"]), w)
	# Godot 4.7 的 shadow_blur 是 0..4；阴雨给更宽的半影。
	sun.shadow_blur = lerpf(float(DRY["shadow_blur"]), float(STORM["shadow_blur"]), w)
	sun.light_angular_distance = lerpf(float(DRY["sun_angular"]), float(STORM["sun_angular"]), w)
	sun.directional_shadow_max_distance = lerpf(float(DRY["shadow_distance"]), float(STORM["shadow_distance"]), w)
	sun.light_volumetric_fog_energy = lerpf(1.6, 0.35, w)
	# 暴雨里太阳的镜面高光会在湿路面上炸成一大块白斑；只保留漫反射。
	sun.light_specular = lerpf(1.0, 0.05, w)


func _apply_emissive(w: float) -> void:
	for id in _emissive:
		var entry: Dictionary = _emissive[id]
		var mat: StandardMaterial3D = entry["material"]
		if mat != null and is_instance_valid(mat):
			mat.emission_energy_multiplier = float(entry["base"]) * lerpf(1.0, 1.8, w)
	for id in _lights:
		var entry: Dictionary = _lights[id]
		var light: Light3D = entry["light"]
		if light != null and is_instance_valid(light):
			light.light_energy = float(entry["base"]) * lerpf(1.0, 1.5, w)
			light.light_specular = lerpf(1.0, 0.20, w)
	for id in _bulbs:
		var entry: Dictionary = _bulbs[id]
		var bulb: MeshInstance3D = entry["mesh"]
		var mat: StandardMaterial3D = entry["material"]
		if bulb != null and is_instance_valid(bulb):
			bulb.visible = w > 0.08
			mat.emission_energy_multiplier = lerpf(1.6, 6.0, w)


func _scan_wet_surfaces() -> void:
	if _wet_scan_done:
		if _lights_dirty:
			_lights_dirty = false
			_scan_lights()
		return
	var scene := get_tree().current_scene
	if scene == null:
		return
	var converted := 0
	var scanned := 0
	var sample: Array[String] = []
	for node in scene.find_children("*", "MeshInstance3D", true, false):
		var mesh := node as MeshInstance3D
		if mesh == null or mesh.mesh == null or _under_vehicle(mesh):
			continue
		for surface in mesh.mesh.get_surface_count():
			var src := mesh.get_surface_override_material(surface)
			if src == null:
				src = mesh.mesh.surface_get_material(surface)
			if not (src is StandardMaterial3D):
				continue
			scanned += 1
			var mat_name := String(src.resource_name).to_lower()
			if sample.size() < 40:
				sample.append(mat_name)
			if not _is_wet_surface(mat_name):
				continue
			if src.transparency != BaseMaterial3D.TRANSPARENCY_DISABLED:
				continue
			var key := src.get_instance_id()
			var wet: ShaderMaterial
			if _wet_materials.has(key):
				wet = _wet_materials[key]
			else:
				wet = _make_wet_material(src, mat_name)
				_wet_materials[key] = wet
			mesh.set_surface_override_material(surface, wet)
			converted += 1
		_scan_emissive(mesh)
	_scan_lights()
	if OS.get_environment("VETTURA_WET_DEBUG") == "1":
		print("[weather-rig] wet scan scanned=%d converted=%d sample=%s" % [
			scanned, converted, str(sample)])
	# 无论是否有材质命中，第一轮完整扫描后就停止：材质名不匹配的赛道
	# （Nordschleife 等）否则会每秒遍历全场景所有 MeshInstance3D。
	_wet_scan_done = true
	if converted > 0:
		_refresh_probe()


func _scan_emissive(mesh: MeshInstance3D) -> void:
	for surface in mesh.mesh.get_surface_count():
		var mat := mesh.get_surface_override_material(surface)
		if mat == null:
			mat = mesh.mesh.surface_get_material(surface)
		if not (mat is StandardMaterial3D):
			continue
		var std := mat as StandardMaterial3D
		if not std.emission_enabled:
			continue
		var id := std.get_instance_id()
		if not _emissive.has(id):
			_emissive[id] = {"material": std, "base": std.emission_energy_multiplier}


func _scan_car_emissive(car: Node3D) -> void:
	for node in car.find_children("*", "MeshInstance3D", true, false):
		_scan_emissive(node as MeshInstance3D)


func _scan_lights() -> void:
	_lights_dirty = false
	var scene := get_tree().current_scene
	if scene == null:
		return
	for node in scene.find_children("*", "Light3D", true, false):
		var light := node as Light3D
		if light == null or light == sun:
			continue
		var id := light.get_instance_id()
		if not _lights.has(id):
			_lights[id] = {"light": light, "base": light.light_energy}
			_make_bulb(light)


func _make_bulb(light: Light3D) -> void:
	if light.get_node_or_null("WeatherBulb") != null:
		return
	var bulb := MeshInstance3D.new()
	var sphere := SphereMesh.new()
	sphere.radius = 0.085
	sphere.height = 0.17
	bulb.mesh = sphere
	bulb.name = "WeatherBulb"
	bulb.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.albedo_color = Color(0.0, 0.0, 0.0, 1.0)
	mat.emission_enabled = true
	mat.emission = light.light_color
	mat.emission_energy_multiplier = 2.0
	mat.disable_receive_shadows = true
	bulb.material_override = mat
	bulb.visible = wetness > 0.08
	light.add_child(bulb)
	_bulbs[light.get_instance_id()] = {"mesh": bulb, "material": mat}


func _make_wet_material(src: StandardMaterial3D, mat_name: String) -> ShaderMaterial:
	var wet := ShaderMaterial.new()
	wet.shader = WET_ROAD_SHADER
	wet.set_shader_parameter("albedo_texture", src.albedo_texture)
	wet.set_shader_parameter("normal_texture", src.normal_texture if src.normal_enabled else _neutral_normal)
	wet.set_shader_parameter("ao_texture", src.ao_texture if src.ao_enabled else null)
	wet.set_shader_parameter("wetness_mask", _wetness_mask)
	wet.set_shader_parameter("albedo_color", src.albedo_color)
	wet.set_shader_parameter("uv_scale", Vector2(src.uv1_scale.x, src.uv1_scale.y))
	wet.set_shader_parameter("normal_scale", src.normal_scale if src.normal_enabled else 1.0)
	wet.set_shader_parameter("base_roughness", src.roughness)
	wet.set_shader_parameter("base_specular", float(src.get("metallic_specular")))
	wet.set_shader_parameter("ao_strength", 1.0 if src.ao_enabled else 0.0)
	wet.set_shader_parameter("mask_scale", 0.055)
	wet.set_shader_parameter("is_line", 1.0 if _is_line(mat_name) else 0.0)
	return wet


func _is_wet_surface(name: String) -> bool:
	if name.is_empty():
		return false
	if name.contains("grass") or name.contains("tree") or name.contains("wall") or name.contains("crowd"):
		return false
	for key in wet_surface_keywords:
		if name.contains(String(key).to_lower()):
			return true
	return false


func _is_line(name: String) -> bool:
	return name.contains("line") or name.contains("startfinish") or name.contains("sfline")


func _under_vehicle(node: Node) -> bool:
	var current := node
	while current != null:
		if current.get_class() == "VehicleBody":
			return true
		current = current.get_parent()
	return false


func set_weather(kind: String) -> void:
	manual_override = true
	match kind.to_lower():
		"dry":
			target_wetness = 0.0
		"drizzle":
			target_wetness = 0.45
		"storm":
			target_wetness = 1.0
		_:
			push_warning("[weather-rig] unknown weather '%s'" % kind)
			return
	_probe_timer = 0.0
	_refresh_probe()
	print("[weather-rig] set_weather=%s target=%.2f" % [kind, target_wetness])


func debug_set_wetness(value: float) -> void:
	manual_override = true
	wetness = clampf(value, 0.0, 1.0)
	target_wetness = wetness
	_apply_wetness(wetness, true)
	RenderingServer.global_shader_parameter_set("wetness", wetness)
	_probe_timer = 0.0


## 恢复用 WorldBody 物理天气驱动；WeatherFX 每帧调用也不覆盖手动值。
func use_world_weather() -> void:
	manual_override = false


func sync_from_world(world: Node) -> void:
	if manual_override or world == null or not world.has_method("get_weather"):
		return
	var w: Dictionary = world.call("get_weather")
	target_wetness = clampf(maxf(float(w.get("wetness", 0.0)), float(w.get("rain", 0.0))), 0.0, 1.0)
	# 让 RainSystem 的密度和道路湿材质与统一值一致。
	if _runtime.has("rain"):
		var rain: RainSystem = _runtime["rain"]
		if is_instance_valid(rain):
			rain.rain_amount = wetness
			rain.ground_wetness = wetness
