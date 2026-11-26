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
const RAIN_REFRACTION_SHADER := preload("res://game/fx/rain/rain_refraction.gdshader")
const RAIN_MIST_SHADER := preload("res://game/fx/rain/rain_mist.gdshader")

const MAX_NEAR_BY_QUALITY := [2500, 10000, 16000]
const MAX_FAR_BY_QUALITY := [0, 550, 1300]
const MAX_SPLASH_BY_QUALITY := [6, 20, 36]
const MAX_RIPPLES_BY_QUALITY := [3, 6, 8]
const DROP_WIDTH_BY_QUALITY := [0.008, 0.011, 0.014]
const DROP_LENGTH_BY_QUALITY := [0.24, 0.34, 0.46]
const SPLASH_AMOUNT_CAP := 240
## 灯光发现（`find_children`）的节流周期，秒。场景树很大时每帧遍历会吃掉
## 移动端预算；没有太阳/局部灯的场景（车库、夜晚）尤其会每帧全树扫描。
const LIGHT_SCAN_INTERVAL_S := 2.0
## 局部光源 uniform 名缓存，避免每帧用 `%` 拼字符串。
const LOCAL_LIGHT_UNIFORM_NAMES := [
	"local_light_pos_0", "local_light_pos_1", "local_light_pos_2",
	"local_light_color_0", "local_light_color_1", "local_light_color_2",
]
## 粒径谱的终端速度以 13 m/s 为基准；`fall_speed_mps` 用它做整体倍率，
## 这样 Inspector 里的「下落速度」仍然有效（预设也按它调）。
const FALL_SPEED_REFERENCE_MPS := 13.0
## `drop_length` / `stretch_multiplier` 的默认值：v3 的长度由曝光积分
## |v|·T 决定，这两个旧参数作为整体形状倍率保留，默认值必须等于 1.0。
const DROP_LENGTH_REFERENCE_M := 0.34
const STRETCH_REFERENCE := 1.15

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
@export var drop_color: Color = Color(0.66, 0.72, 0.80, 1.0):
	set(value):
		drop_color = value
		_parameters_dirty = true
		parameters_changed.emit()
@export_range(0.0, 1.0, 0.01) var jitter_strength: float = 0.45:
	set(value):
		jitter_strength = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
		parameters_changed.emit()

@export_group("Optics")
@export_range(0.002, 0.0667, 0.0001) var shutter_time: float = 1.0 / 30.0:
	set(value):
		shutter_time = clampf(value, 0.002, 0.0667)
		_parameters_dirty = true
@export_range(0.2, 40.0, 0.01) var exposure_gain: float = 7.0:
	set(value):
		exposure_gain = clampf(value, 0.2, 40.0)
		_parameters_dirty = true
@export_range(0.2, 4.0, 0.01) var drop_size_scale: float = 1.0:
	set(value):
		drop_size_scale = clampf(value, 0.2, 4.0)
		_parameters_dirty = true
@export_range(0.0, 1.0, 0.01) var streak_softness: float = 0.45:
	set(value):
		streak_softness = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
@export_range(0.0, 1.0, 0.01) var internal_pattern_strength: float = 0.40:
	set(value):
		internal_pattern_strength = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
@export_range(0.0, 12.0, 0.1) var oscillation_freq: float = 2.4:
	set(value):
		oscillation_freq = clampf(value, 0.0, 12.0)
		_parameters_dirty = true
@export_range(1.0, 3.0, 1.0) var highlight_count: float = 1.0:
	set(value):
		highlight_count = clampf(value, 1.0, 3.0)
		_parameters_dirty = true
@export_range(0.0, 1.0, 0.01) var background_response: float = 0.85:
	set(value):
		background_response = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
@export_range(0.0, 2.0, 0.01) var backlit_boost: float = 1.0:
	set(value):
		backlit_boost = clampf(value, 0.0, 2.0)
		_parameters_dirty = true
@export var light_direction_override: Vector3 = Vector3.ZERO
@export var dof_enabled: bool = false:
	set(value):
		dof_enabled = value
		_parameters_dirty = true
@export_range(0.0, 20.0, 0.1) var dof_near: float = 5.0
@export_range(0.0, 60.0, 0.1) var dof_mid: float = 25.0
@export_range(10.0, 160.0, 1.0) var dof_far: float = 65.0
@export_range(0.0, 1.0, 0.01) var defocus_amount: float = 0.0

@export_group("Gust & Shaft")
@export_range(0.0, 1.0, 0.01) var gust_strength: float = 0.25:
	set(value):
		gust_strength = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
@export_range(0.5, 12.0, 0.1) var gust_period: float = 3.0:
	set(value):
		gust_period = clampf(value, 0.5, 12.0)
		_parameters_dirty = true
@export_range(0.01, 0.5, 0.01) var shaft_scale: float = 0.08:
	set(value):
		shaft_scale = clampf(value, 0.01, 0.5)
		_parameters_dirty = true
@export_range(0.0, 1.0, 0.01) var shaft_contrast: float = 0.35:
	set(value):
		shaft_contrast = clampf(value, 0.0, 1.0)
		_parameters_dirty = true
@export var screen_refraction_enabled: bool = false
@export var backbuffer_copy_rect: Rect2 = Rect2()

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
@export_range(0, 6, 1) var max_ripples: int = 6:
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

## 四档粒径谱；每项独立粒子层、终端速度、宽度、softness 与数量占比。
## 终端速度经验值来自 Marshall–Palmer/Atlas 类关系
## v(D) ≈ 9.65 - 10.3·exp(-0.6·D_mm)（m/s，D 为直径 mm）；
## 这里取各档速度区间，避免在 shader 里做开方/指数。
const RAIN_CLASSES := [
	{"name": "Large", "d_min": 2.6, "d_max": 4.6, "v_min": 8.2, "v_max": 9.6,
		"fraction": 0.06, "width": 0.013, "softness": 0.12, "distance": 55.0},
	{"name": "Medium", "d_min": 1.2, "d_max": 2.6, "v_min": 6.2, "v_max": 8.2,
		"fraction": 0.22, "width": 0.0105, "softness": 0.26, "distance": 68.0},
	{"name": "Small", "d_min": 0.5, "d_max": 1.2, "v_min": 4.0, "v_max": 6.2,
		"fraction": 0.42, "width": 0.0075, "softness": 0.42, "distance": 78.0},
	{"name": "Fine", "d_min": 0.15, "d_max": 0.5, "v_min": 2.5, "v_max": 4.0,
		"fraction": 0.30, "width": 0.0055, "softness": 0.62, "distance": 88.0},
]

## 各粒径档的整体亮度：大滴最亮，小滴/细滴快速衰减。旧版每档只降 8%，
## 结果所有雨滴都接近同样的白线，密度一高就像撒了纸屑。
const CLASS_BRIGHTNESS := [1.0, 0.85, 0.70, 0.58]
var _rain_layers: Array[Dictionary] = []
var _splash: GPUParticles3D
var _splash_process: ParticleProcessMaterial
var _mist: GPUParticles3D
var _ripple_instance: MultiMeshInstance3D
var _ripple_multimesh: MultiMesh
var _ripple_material: ShaderMaterial
var _ripples: Array[Dictionary] = []
var _ripple_accum := 0.0
var _effective_ripple_limit := 4
var _screen_layer: CanvasLayer
var _screen_rect: ColorRect
var _screen_material: ShaderMaterial
var _refraction_layer: CanvasLayer
var _refraction_rect: ColorRect
var _refraction_material: ShaderMaterial
var _refraction_backbuffer: BackBufferCopy
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
var _frame_dt := 1.0 / 60.0
var _sun: DirectionalLight3D
var _local_lights: Array[Light3D] = []
## 下一次允许重扫场景灯光的时间（`_time`）。灯光只在到期时重新发现。
var _next_light_scan := 0.0
## 可选 DOF 的资源归属：申请一次、退出时还原相机原来的 attributes。
var _dof_attributes: CameraAttributesPractical
var _dof_previous_attributes: CameraAttributes
var _dof_applied := false

## 每帧都要写几十个 shader uniform / Environment 属性的函数按低频节流。
## 这些值以秒级变化（雨量、风向、灯光、雾），10/20 Hz 的更新肉眼无法分辨，
## 但能省掉雨系统在干燥天气下也要做的固定 CPU 工作。
const WEATHER_POLL_S := 0.2
const LIGHT_UPDATE_S := 0.1
const PARTICLE_UPDATE_S := 0.2
const OVERLAY_UPDATE_S := 0.1
const ATMOSPHERE_EPSILON := 0.002
var _weather_timer := 0.0
var _light_timer := 0.0
var _particle_timer := 0.0
var _overlay_timer := 0.0
var _atmo_rain := -1.0
var _atmo_wet := -1.0
## 干燥状态只做一次收尾（关粒子/雾/湿材质），之后每帧直接返回。
var _dry_settled := false


func _ready() -> void:
	# 让 WeatherVisuals 这类兜底雨实现知道已经有真正的雨系统，避免双层雨。
	add_to_group("vettura_rain")
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
	_frame_dt = delta
	if not _built:
		_build()
	if _camera == null or not is_instance_valid(_camera):
		_camera = _find_camera()
		# 相机换了：旧的 DOF 资源不再属于当前相机，重新申请。
		_dof_applied = false
		_dof_attributes = null
		_dof_previous_attributes = null
		if _camera != null and dof_enabled:
			_apply_dof()
	if _camera == null:
		return

	# 根节点独立于相机旋转；每帧只同步位置。GPUParticles3D.local_coords=false，
	# 已发射粒子留在世界，只有新粒子在相机附近生成，因此快速转头不会粘屏。
	top_level = true
	global_transform = Transform3D(Basis.IDENTITY, _camera.global_position + Vector3(0.0, 2.5, 0.0))

	if auto_weather and _world != null and is_instance_valid(_world):
		_weather_timer -= delta
		if _weather_timer <= 0.0:
			_weather_timer = WEATHER_POLL_S
			var weather: Dictionary = _world.get_weather()
			set_weather(
				float(weather.get("rain", rain_amount)),
				float(weather.get("wetness", ground_wetness)),
				weather.get("wind", Vector3.ZERO) as Vector3
			)

	if _parameters_dirty:
		_apply_parameters()

	# 干地（无雨、无湿度）不需要每帧重写 40 个雨 shader uniform、雾和湿材质。
	# 进入干地时完整跑一次收尾，把粒子/雾/湿材质恢复到干燥状态，之后直接
	# 返回；天气一变回湿/雨，下面的常规路径会自动恢复。
	var dry := rain_amount <= 0.001 and ground_wetness <= 0.001
	if dry:
		if not _dry_settled:
			_dry_settled = true
			_light_timer = 0.0
			_particle_timer = 0.0
			_overlay_timer = 0.0
			_update_lighting()
			_update_particles()
			_update_ground_feedback(delta)
			_update_wetness()
			_update_atmosphere()
			_update_screen_overlay()
			_update_refraction_overlay()
		return
	_dry_settled = false

	_light_timer -= delta
	if _light_timer <= 0.0:
		_light_timer = LIGHT_UPDATE_S
		_update_lighting()
	_update_fall_direction()
	_particle_timer -= delta
	if _particle_timer <= 0.0:
		_particle_timer = PARTICLE_UPDATE_S
		_update_particles()
	_update_ground_feedback(delta)
	_update_wetness()
	_update_atmosphere()
	_overlay_timer -= delta
	if _overlay_timer <= 0.0:
		_overlay_timer = OVERLAY_UPDATE_S
		_update_screen_overlay()
		_update_refraction_overlay()
	if enable_audio:
		_update_audio()


func _build() -> void:
	if _built:
		return
	_built = true
	_viewport_size = Vector2(get_viewport().get_visible_rect().size) if get_viewport() != null else Vector2(1920, 1080)
	get_viewport().size_changed.connect(_on_viewport_size_changed)

	_create_rain_layers()
	_splash = _make_splash()
	_mist = _make_mist()
	_make_ripples()
	_make_screen_overlay()
	_make_refraction_overlay()
	_find_environment()
	_build_audio()
	_apply_parameters()


func _create_rain_layers() -> void:
	_rain_layers.clear()
	var box_extents := [22.0, 28.0, 34.0, 40.0]
	for index in RAIN_CLASSES.size():
		var cfg: Dictionary = RAIN_CLASSES[index]
		var material := _make_rain_material(index, cfg)
		var particles := _make_rain_layer(cfg, material, float(box_extents[index]))
		_rain_layers.append({
			"config": cfg,
			"material": material,
			"particles": particles,
			"process": particles.process_material as ParticleProcessMaterial,
		})


func _make_rain_material(index: int, cfg: Dictionary) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = RAIN_DROP_SHADER
	material.set_shader_parameter("drop_color", drop_color)
	material.set_shader_parameter("brightness", drop_brightness * CLASS_BRIGHTNESS[index])
	material.set_shader_parameter("drop_diameter_mm", (float(cfg["d_min"]) + float(cfg["d_max"])) * 0.5 * drop_size_scale)
	material.set_shader_parameter("terminal_velocity", (float(cfg["v_min"]) + float(cfg["v_max"])) * 0.5 * _fall_speed_scale())
	material.set_shader_parameter("drop_width_scale", float(cfg["width"]) / maxf(drop_width, 0.001))
	material.set_shader_parameter("drop_length_scale", _drop_length_scale())
	material.set_shader_parameter("jitter_strength", jitter_strength)
	material.set_shader_parameter("min_screen_width_px", min_screen_width_px)
	material.set_shader_parameter("max_distance", float(cfg["distance"]) * (rain_visibility_distance / 70.0))
	material.set_shader_parameter("viewport_size", _viewport_size)
	material.set_shader_parameter("layer_softness", maxf(streak_softness, float(cfg["softness"])))
	material.set_shader_parameter("internal_pattern_strength", internal_pattern_strength)
	material.set_shader_parameter("oscillation_freq", oscillation_freq)
	material.set_shader_parameter("highlight_count", highlight_count)
	material.set_shader_parameter("shutter_time", shutter_time)
	material.set_shader_parameter("frame_time", _frame_dt)
	material.set_shader_parameter("exposure_gain", exposure_gain)
	material.set_shader_parameter("background_response", background_response)
	material.set_shader_parameter("backlit_boost", backlit_boost)
	material.set_shader_parameter("dof_near", dof_near)
	material.set_shader_parameter("dof_mid", dof_mid)
	material.set_shader_parameter("dof_far", dof_far)
	material.set_shader_parameter("defocus_amount", defocus_amount if dof_enabled else 0.0)
	material.set_shader_parameter("gust_strength", gust_strength)
	material.set_shader_parameter("gust_period", gust_period)
	material.set_shader_parameter("shaft_scale", shaft_scale)
	material.set_shader_parameter("shaft_contrast", shaft_contrast)
	return material


func _make_rain_layer(cfg: Dictionary, material: ShaderMaterial, box_extent: float) -> GPUParticles3D:
	var process := ParticleProcessMaterial.new()
	process.direction = Vector3(0.0, -1.0, 0.0)
	process.spread = 1.5
	process.initial_velocity_min = float(cfg["v_min"]) * _fall_speed_scale()
	process.initial_velocity_max = float(cfg["v_max"]) * _fall_speed_scale()
	process.gravity = Vector3.ZERO
	process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	process.emission_box_extents = Vector3(box_extent, 6.0, box_extent)
	process.scale_min = 0.65
	process.scale_max = 1.35

	var quad := QuadMesh.new()
	quad.size = Vector2(1.0, 1.0)
	quad.material = material

	var particles := GPUParticles3D.new()
	particles.name = "Rain" + String(cfg["name"])
	particles.process_material = process
	particles.draw_pass_1 = quad
	particles.amount = 1
	particles.lifetime = 1.2
	particles.local_coords = false
	particles.emitting = false
	particles.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	particles.visibility_aabb = AABB(Vector3(-box_extent, -12, -box_extent), Vector3(box_extent * 2.0, 28.0, box_extent * 2.0))
	add_child(particles)
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


func _make_mist() -> GPUParticles3D:
	var process := ParticleProcessMaterial.new()
	process.direction = Vector3(0.0, 0.15, 0.0)
	process.spread = 35.0
	process.initial_velocity_min = 0.05
	process.initial_velocity_max = 0.35
	process.gravity = Vector3(0.0, -0.15, 0.0)
	process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	process.emission_box_extents = Vector3(22.0, 0.4, 22.0)
	process.scale_min = 0.35
	process.scale_max = 0.9

	var quad := QuadMesh.new()
	quad.size = Vector2(4.2, 2.0)
	var material := ShaderMaterial.new()
	material.shader = RAIN_MIST_SHADER
	material.set_shader_parameter("mist_color", Color(0.45, 0.52, 0.62, 1.0))
	material.set_shader_parameter("intensity", 0.028)
	quad.material = material

	var particles := GPUParticles3D.new()
	particles.name = "RainMist"
	particles.process_material = process
	particles.draw_pass_1 = quad
	particles.amount = 1
	particles.lifetime = 2.4
	particles.local_coords = false
	particles.emitting = false
	particles.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	particles.visibility_aabb = AABB(Vector3(-28, -3, -28), Vector3(56, 10, 56))
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


func _make_refraction_overlay() -> void:
	_refraction_layer = CanvasLayer.new()
	_refraction_layer.name = "RainRefraction"
	_refraction_layer.layer = 10
	add_child(_refraction_layer)

	_refraction_backbuffer = BackBufferCopy.new()
	_refraction_backbuffer.name = "RainRefractionCopy"
	_refraction_backbuffer.copy_mode = BackBufferCopy.COPY_MODE_RECT
	_refraction_backbuffer.rect = Rect2(Vector2.ZERO, _viewport_size)
	_refraction_layer.add_child(_refraction_backbuffer)

	_refraction_rect = ColorRect.new()
	_refraction_rect.name = "RainRefractionDrops"
	_refraction_rect.set_anchors_preset(Control.PRESET_FULL_RECT)
	_refraction_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_refraction_material = ShaderMaterial.new()
	_refraction_material.shader = RAIN_REFRACTION_SHADER
	_refraction_material.set_shader_parameter("intensity", 0.0)
	_refraction_rect.material = _refraction_material
	_refraction_rect.visible = false
	_refraction_layer.add_child(_refraction_rect)


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
	# 没有现成环境时也复用工程默认环境（天空 shader / ACES / glow / 雾），
	# 不要退回裸 ProceduralSkyMaterial——那会让独立挂 RainSystem 的场景
	# 变成另一套画面。
	var base := load("res://game_environment.tres") as Environment
	var env: Environment
	if base != null:
		env = base.duplicate(true) as Environment
	else:
		env = Environment.new()
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
	var total_max := int(MAX_NEAR_BY_QUALITY[tier])
	var splash_max := int(MAX_SPLASH_BY_QUALITY[tier])
	# 桌面每档多给一些细/小雨滴（粒子只是小 quad，overdraw 很小）；
	# 移动端保持原来的预算，避免中端 GPU 被雨幕拖垮。
	if _rendering_method == "mobile":
		total_max = int(total_max * 0.5)
		splash_max = int(splash_max * 0.8)
	if mobile_low_end:
		total_max = int(total_max * 0.55)
		splash_max = int(splash_max * 0.6)

	# 粒径谱随雨量右移：雨越大，大滴占比越高，速度方差也越大。
	var fractions := _class_fractions()
	for index in _rain_layers.size():
		var layer: Dictionary = _rain_layers[index]
		var cfg: Dictionary = layer["config"]
		var particles: GPUParticles3D = layer["particles"]
		var material: ShaderMaterial = layer["material"]
		var amount := maxi(1, int(float(total_max) * fractions[index] * rain_amount))
		particles.amount = amount
		particles.visible = not _compat_fallback
		particles.emitting = rain_amount > 0.02 and not _compat_fallback
		material.set_shader_parameter("drop_color", drop_color)
		material.set_shader_parameter("brightness", drop_brightness * CLASS_BRIGHTNESS[index])
		material.set_shader_parameter("drop_diameter_mm", (float(cfg["d_min"]) + float(cfg["d_max"])) * 0.5 * drop_size_scale)
		material.set_shader_parameter("terminal_velocity", (float(cfg["v_min"]) + float(cfg["v_max"])) * 0.5 * _fall_speed_scale())
		material.set_shader_parameter("drop_width_scale", float(cfg["width"]) / maxf(drop_width, 0.001))
		material.set_shader_parameter("drop_length_scale", _drop_length_scale())
		material.set_shader_parameter("jitter_strength", jitter_strength)
		material.set_shader_parameter("min_screen_width_px", min_screen_width_px)
		material.set_shader_parameter("max_distance", float(cfg["distance"]) * (rain_visibility_distance / 70.0))
		material.set_shader_parameter("layer_softness", maxf(streak_softness, float(cfg["softness"])))
		material.set_shader_parameter("internal_pattern_strength", internal_pattern_strength)
		material.set_shader_parameter("oscillation_freq", oscillation_freq)
		material.set_shader_parameter("highlight_count", highlight_count)
		material.set_shader_parameter("shutter_time", shutter_time)
		material.set_shader_parameter("frame_time", _frame_dt)
		material.set_shader_parameter("exposure_gain", exposure_gain)
		material.set_shader_parameter("background_response", background_response)
		# 雨丝 fresnel 边缘反射色：取环境雾色/环境光，避免固定蓝白。
		var sky_tint := Color(0.62, 0.72, 0.85)
		var ground_tint := Color(0.10, 0.11, 0.13)
		if _environment != null:
			sky_tint = _environment.fog_light_color
			ground_tint = _environment.ambient_light_color.darkened(0.75)
		material.set_shader_parameter("sky_color", sky_tint)
		material.set_shader_parameter("ground_color", ground_tint)
		material.set_shader_parameter("backlit_boost", backlit_boost)
		material.set_shader_parameter("dof_near", dof_near)
		material.set_shader_parameter("dof_mid", dof_mid)
		material.set_shader_parameter("dof_far", dof_far)
		material.set_shader_parameter("defocus_amount", defocus_amount if dof_enabled else 0.0)
		material.set_shader_parameter("gust_strength", gust_strength)
		material.set_shader_parameter("gust_period", gust_period)
		material.set_shader_parameter("shaft_scale", shaft_scale)
		material.set_shader_parameter("shaft_contrast", shaft_contrast)

	_splash.amount = maxi(1, mini(SPLASH_AMOUNT_CAP, int(splash_max * rain_amount)))
	_splash.emitting = rain_amount > 0.02 and ground_wetness > 0.05 and not _compat_fallback
	_mist.amount = maxi(1, int(lerpf(4.0, 18.0, rain_amount) * (0.6 if mobile_low_end else 1.0)))
	_mist.emitting = rain_amount > 0.05 and not _compat_fallback
	_ripple_material.set_shader_parameter("ripple_color", drop_color)

	var effective_ripples := mini(max_ripples, int(MAX_RIPPLES_BY_QUALITY[tier]))
	if mobile_low_end:
		effective_ripples = mini(effective_ripples, 2)
	_effective_ripple_limit = effective_ripples
	_apply_viewport_quality(tier)
	_apply_dof()
	_update_screen_overlay()
	_update_refraction_overlay()
	if not _logged:
		_logged = true
		print("[RainSystem] method=%s compat=%s classes=%d total=%d splash=%d ripples=%d" % [
			_rendering_method, _compat_fallback, _rain_layers.size(), total_max,
			_splash.amount, effective_ripples])


func _fall_speed_scale() -> float:
	return clampf(fall_speed_mps / FALL_SPEED_REFERENCE_MPS, 0.1, 4.0)


func _drop_length_scale() -> float:
	return (drop_length / DROP_LENGTH_REFERENCE_M) * (stretch_multiplier / STRETCH_REFERENCE)


func _class_fractions() -> Array[float]:
	var shift := (rain_amount - 0.5) * 2.0
	var weights: Array[float] = [
		maxf(0.005, 0.05 * (1.0 + 0.60 * shift)),
		maxf(0.005, 0.20 * (1.0 + 0.20 * shift)),
		maxf(0.005, 0.42 * (1.0 - 0.10 * shift)),
		maxf(0.005, 0.33 * (1.0 - 0.20 * shift)),
	]
	var total := 0.0
	for w in weights:
		total += w
	for i in weights.size():
		weights[i] /= total
	return weights


func _apply_dof() -> void:
	if _camera == null or not is_instance_valid(_camera):
		return
	if not dof_enabled:
		# 关掉时还原相机原来的 attributes，否则 DOF 会一直挂在相机上。
		if _dof_applied:
			_camera.attributes = _dof_previous_attributes
			_dof_applied = false
			_dof_attributes = null
			_dof_previous_attributes = null
		return
	# 可选低品质 CameraAttributes DOF：Mobile 走光栅化 bokeh；默认关闭。
	# 这里只用于近景虚化，不是逐滴 DOF；低配建议保持关闭。
	#
	# 只在第一次申请资源，之后调参复用同一个 CameraAttributes，避免每次
	# `_apply_parameters`（雨量变化时会频繁触发）都 new 一个 Resource。
	if not _dof_applied:
		_dof_previous_attributes = _camera.attributes
		_dof_attributes = CameraAttributesPractical.new()
		_dof_applied = true
		_camera.attributes = _dof_attributes
	var attributes := _dof_attributes
	attributes.dof_blur_far_enabled = true
	attributes.dof_blur_far_distance = dof_far
	attributes.dof_blur_far_transition = maxf(dof_far - dof_mid, 0.1)
	attributes.dof_blur_near_enabled = true
	attributes.dof_blur_near_distance = dof_near
	attributes.dof_blur_near_transition = maxf(dof_mid - dof_near, 0.1)
	attributes.dof_blur_amount = clampf(defocus_amount * 0.08, 0.0, 0.2)


func _apply_viewport_quality(tier: int) -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	# Mobile 不用 TAA/FSR2；只用 MSAA/FXAA，低配直接用 FXAA。
	# 桌面全屏开启 RenderQuality 的自动分辨率时，不要在这里把
	# scaling_3d_scale 拉回 1.0（否则自适应白做）。
	var adaptive_fullscreen := GameSettings.is_fullscreen() \
		and bool(GameSettings.get_value("video", "fullscreen_adaptive", true)) \
		and not OS.has_feature("mobile")
	match tier:
		0:
			viewport.msaa_3d = Viewport.MSAA_DISABLED
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			if not adaptive_fullscreen:
				viewport.scaling_3d_scale = 0.85 if mobile_low_end else 0.92
		1:
			viewport.msaa_3d = Viewport.MSAA_2X
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			if not adaptive_fullscreen:
				viewport.scaling_3d_scale = 0.9 if mobile_low_end else 1.0
		_:
			viewport.msaa_3d = Viewport.MSAA_2X
			viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
			if not adaptive_fullscreen:
				viewport.scaling_3d_scale = 1.0 if not mobile_low_end else 0.92
	if adaptive_fullscreen:
		# MetalFX/FSR 上采样 + 2x MSAA 已经够；再叠 FXAA 只会把全屏磨糊。
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED


func _update_fall_direction() -> void:
	var wind_rad := deg_to_rad(wind_direction_deg)
	var horizontal := Vector3(cos(wind_rad), 0.0, sin(wind_rad)) * wind_speed
	# 阵风包络（低频振幅调制）× 空间相干噪声；相邻雨滴同相，主方向仍一致。
	var gust_envelope := 1.0 + gust_strength * sin(_time * 6.2831 / maxf(gust_period, 0.1))
	var coherent := turbulence * sin(_time * 0.7 + _camera.global_position.x * 0.03) * 0.22
	var lateral := Vector3(-sin(wind_rad), 0.0, cos(wind_rad)) * coherent * gust_envelope
	var fall := Vector3(horizontal.x + lateral.x, -fall_speed_mps, horizontal.z + lateral.z).normalized()
	for layer in _rain_layers:
		var material: ShaderMaterial = layer["material"]
		material.set_shader_parameter("fall_direction", fall)
		material.set_shader_parameter("frame_time", _frame_dt)
	# 风向 → 倾角：atan(|horizontal| / fall_speed)。风为 0 时 fall=(0,-1,0)。


func _update_particles() -> void:
	var emitting := rain_amount > 0.02 and not _compat_fallback
	var wind_rad := deg_to_rad(wind_direction_deg)
	var direction := Vector3(cos(wind_rad) * wind_speed * 0.08, -fall_speed_mps,
		sin(wind_rad) * wind_speed * 0.08).normalized()
	var speed_scale := _fall_speed_scale()
	for layer in _rain_layers:
		var particles: GPUParticles3D = layer["particles"]
		var process: ParticleProcessMaterial = layer["process"]
		var cfg: Dictionary = layer["config"]
		particles.emitting = emitting and particles.visible
		process.direction = direction
		process.spread = 1.0 + turbulence * 2.0
		# `fall_speed_mps` 是整体倍率：运行时改它要能立刻改变终端速度。
		process.initial_velocity_min = float(cfg["v_min"]) * speed_scale
		process.initial_velocity_max = float(cfg["v_max"]) * speed_scale


func _update_lighting() -> void:
	if _rain_layers.is_empty():
		return
	# 灯光发现是场景树遍历，不能每帧做。没有太阳/局部灯的场景（车库、夜晚）
	# 会让旧逻辑每帧全树 find_children；这里改成到期才重扫，同时让行驶中
	# 最近的 3 盏灯能随位置更新。
	if _time >= _next_light_scan:
		_next_light_scan = _time + LIGHT_SCAN_INTERVAL_S
		if not _light_is_usable(_sun):
			_sun = _find_first_directional_light()
		_local_lights = _find_local_lights()
	var sun_dir := light_direction_override
	if sun_dir.length_squared() < 1e-5:
		sun_dir = (_sun.global_transform.basis.z if _sun != null else Vector3(0.3, -0.6, 0.2)).normalized()
	var sky := 0.35
	if _environment != null:
		sky = clampf(_environment.ambient_light_energy * 0.28 + _environment.background_energy_multiplier * 0.25, 0.05, 1.5)
	if _sun != null:
		sky = clampf(sky + maxf(_sun.light_energy, 0.0) * 0.12, 0.65, 1.8)
	var ground := clampf(sky * 0.35, 0.02, 0.8)
	for layer in _rain_layers:
		var material: ShaderMaterial = layer["material"]
		material.set_shader_parameter("sun_direction", sun_dir)
		material.set_shader_parameter("sky_brightness", sky)
		material.set_shader_parameter("ground_brightness", ground)
		material.set_shader_parameter("frame_time", _frame_dt)
		for index in 3:
			var position := Vector3.ZERO
			var color := Color(0, 0, 0, 1)
			if index < _local_lights.size():
				var light := _local_lights[index]
				if is_instance_valid(light) and light.is_inside_tree():
					position = light.global_position
					color = light.light_color * clampf(light.light_energy, 0.0, 1.0)
			material.set_shader_parameter(LOCAL_LIGHT_UNIFORM_NAMES[index], position)
			material.set_shader_parameter(
				LOCAL_LIGHT_UNIFORM_NAMES[index + 3],
				Vector3(color.r, color.g, color.b))


## 灯光节点是否还能安全读取 world 变换：不在树里的节点调用 global_position
## 会每帧刷引擎错误。
func _light_is_usable(light: Light3D) -> bool:
	return light != null and is_instance_valid(light) and light.is_inside_tree() and light.visible


func _find_first_directional_light() -> DirectionalLight3D:
	var root := get_tree().current_scene
	if root == null:
		return null
	for node in root.find_children("*", "DirectionalLight3D", true, false):
		var light := node as DirectionalLight3D
		if light != null and light.visible and light.is_inside_tree():
			return light
	return null


func _find_local_lights() -> Array[Light3D]:
	var result: Array[Light3D] = []
	var root := get_tree().current_scene
	if root == null or _camera == null:
		return result
	for node in root.find_children("*", "Light3D", true, false):
		var light := node as Light3D
		if light == null or light is DirectionalLight3D:
			continue
		# 不在树里/已隐藏的灯不能参与排序（global_position 会报错），也不该
		# 占掉 3 个 shader slot。
		if not light.visible or not light.is_inside_tree():
			continue
		if light is OmniLight3D or light is SpotLight3D:
			result.append(light)
	result.sort_custom(func(a: Light3D, b: Light3D) -> bool:
		return a.global_position.distance_squared_to(_camera.global_position) < b.global_position.distance_squared_to(_camera.global_position))
	if result.size() > 3:
		result.resize(3)
	return result


func _update_ground_feedback(delta: float) -> void:
	if _compat_fallback:
		_splash.emitting = false
	elif _splash != null:
		var ground := Vector3(_camera.global_position.x,
			_ground_height(_camera.global_position.x, _camera.global_position.z),
			_camera.global_position.z)
		_splash.global_position = ground
		_splash.emitting = rain_amount > 0.02 and ground_wetness > 0.05
		if _mist != null:
			_mist.global_position = ground + Vector3(0.0, 0.7, 0.0)
			_mist.emitting = rain_amount > 0.05 and not _compat_fallback

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
		color.a = (1.0 - t) * 0.55 * rain_amount
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
	var forward_plus := _rendering_method == "forward_plus"
	for entry in _wet_entries:
		var material: StandardMaterial3D = entry["material"]
		if material == null:
			continue
		var base_albedo: Color = entry["albedo"]
		var base_roughness: float = entry["roughness"]
		var wet := clampf(ground_wetness, 0.0, 1.0)
		material.albedo_color = base_albedo.darkened(0.22 * wet)
		if forward_plus:
			# 桌面有 SSR + 天空反射：湿沥青可以做到较低的粗糙度，让远处的
			# 护栏/天空在路面上形成拉长的高光，而不是只有一层均匀的灰。
			material.roughness = maxf(lerpf(base_roughness, 0.30, 0.85 * wet), 0.24)
			# 湿沥青是电介质：保留/提高介电高光，但绝不能沿用某些反射类路面
			# 材质的 metallic=0.85——那会把低角度太阳反射成一整团过曝白斑。
			material.metallic = minf(material.metallic, 0.12)
			# 路面类材质的介电高光在加载时被压成 0（避免 AC envmap 缺失导致
			# 纯白反光）；湿路面必须把它找回一部分，否则 SSR 没有高光可反射。
			# 0.12 已足够让湿润表面接住天空/护栏的反射；再高会在低太阳角度
			# 把太阳反射成一整团过曝白斑（Forward+ 的 bloom 会把它放大）。
			var base_specular: Variant = entry.get("metallic_specular")
			if base_specular != null:
				material.set("metallic_specular",
					lerpf(float(base_specular), 0.10, wet))
		else:
			# Mobile 无 SSR：靠低粗糙度 + 天空/环境 cubemap 反射近似湿感。
			# 粗糙度 0.22~0.24、介电高光 0.20：能看到路面拉长的天光，但
			# 比桌面（0.24 粗糙度 + SSR）保守，避免低端 GPU 过曝白斑。
			material.roughness = maxf(lerpf(base_roughness, 0.24, 0.72 * wet), 0.22)
			material.metallic = minf(material.metallic, 0.08)
			var base_specular: Variant = entry.get("metallic_specular")
			if base_specular != null:
				material.set("metallic_specular",
					lerpf(float(base_specular), 0.20, wet))


func _update_atmosphere() -> void:
	if _environment == null:
		return
	# 值没变就不碰 Environment：每次赋值都会把环境标脏并重算一堆 uniform。
	# 雨量/湿度以秒级变化，2e-3 的死区足够跳过浮点抖动与无意义重写。
	if absf(rain_amount - _atmo_rain) < ATMOSPHERE_EPSILON \
			and absf(ground_wetness - _atmo_wet) < ATMOSPHERE_EPSILON:
		return
	_atmo_rain = rain_amount
	_atmo_wet = ground_wetness
	var wet := clampf(ground_wetness, 0.0, 1.0)
	var rain := clampf(rain_amount, 0.0, 1.0)
	var forward_plus := _rendering_method == "forward_plus"
	# 仅 depth fog / height fog；体积雾只在 Forward+ 桌面开。
	_environment.fog_enabled = rain > 0.02 or wet > 0.05
	_environment.fog_density = lerpf(0.0004, fog_density, maxf(rain, wet * 0.7))
	_environment.fog_light_color = Color(0.52, 0.58, 0.68).lerp(Color(0.40, 0.45, 0.55), rain)
	_environment.fog_aerial_perspective = lerpf(0.3, 0.6, rain)
	if _environment.get("fog_height") != null:
		_environment.set("fog_height", 0.0)
		_environment.set("fog_height_density", lerpf(0.0, 0.08, rain))
	if forward_plus:
		# 湿路面 + 雨幕需要屏幕空间反射和体积雾才能“像湿的”：SSR 提供拉长的
		# 护栏/天空高光，体积雾把雨丝的远景层次压出来。晴天/干地不付这笔钱。
		_environment.ssr_enabled = rain > 0.02 or wet > 0.05
		_environment.volumetric_fog_enabled = rain > 0.05 or wet > 0.2
		if _environment.volumetric_fog_enabled:
			_environment.volumetric_fog_density = lerpf(0.0015, 0.018, maxf(rain, wet * 0.6))
			_environment.volumetric_fog_albedo = Color(0.62, 0.68, 0.78)
			_environment.volumetric_fog_anisotropy = lerpf(0.35, 0.62, rain)
			_environment.volumetric_fog_length = lerpf(96.0, 56.0, rain)
			_environment.volumetric_fog_sky_affect = 0.25
	else:
		# Mobile / Compatibility 不支持体积雾：显式关掉，避免引擎告警。
		_environment.volumetric_fog_enabled = false
	_environment.adjustment_enabled = true
	_environment.adjustment_contrast = lerpf(1.0, 0.88, rain)
	_environment.adjustment_saturation = lerpf(1.0, 0.82, rain)


func _update_refraction_overlay() -> void:
	if _refraction_rect == null:
		return
	var tier := clampi(int(quality), 0, 2)
	var allowed := screen_refraction_enabled and tier >= 2 and not mobile_low_end and rain_amount > 0.05
	_refraction_rect.visible = allowed
	_refraction_backbuffer.visible = allowed
	if allowed:
		var rect := backbuffer_copy_rect
		if rect.size.x <= 0.0 or rect.size.y <= 0.0:
			rect = Rect2(Vector2.ZERO, _viewport_size)
		_refraction_backbuffer.rect = rect
		_refraction_material.set_shader_parameter("intensity", clampf(rain_amount * 0.55, 0.0, 0.55))
		_refraction_material.set_shader_parameter("gravity_speed", 0.18 + rain_amount * 0.35)


func _update_screen_overlay() -> void:
	if _screen_rect == null:
		return
	var tier := clampi(int(quality), 0, 2)
	var allowed := screen_drops_enabled and tier > 0 and not mobile_low_end
	if _compat_fallback:
		# Compatibility 没有 GPU 粒子；屏幕雨痕是显式降级路径，但仍然尊重
		# 用户关掉 screen_drops_enabled 的选择（README 的验证项依赖它）。
		allowed = screen_drops_enabled and rain_amount > 0.02
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
				# 桌面湿路面要恢复介电高光（加载时被压成 0 防止纯白反光），
				# 记下原值，干了再还原。
				"metallic_specular": duplicate.get("metallic_specular"),
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
	for layer in _rain_layers:
		var material: ShaderMaterial = layer["material"]
		material.set_shader_parameter("viewport_size", _viewport_size)
	if _screen_material != null:
		_screen_material.set_shader_parameter("viewport_size", _viewport_size)
