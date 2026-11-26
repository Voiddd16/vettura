extends Node3D
## 天气视觉系统：把 `vettura-track` 的天气参数映射成 Godot 场景视觉。
##
## 物理侧（Rust）已经有完整的 `Weather`（湿度/雨/雾/日照/风），但以前只影响
## 抓地力，画面上什么都没有。这个节点补齐表现层：
##   - 天空：`game/render/sky.gdshader`（大气渐变 + 太阳/月亮 + 云 + 星）；
##   - 太阳：DirectionalLight3D，能量/色温/体积雾贡献随日照；赛道网格只接收
##     阴影不投射（见 `gdext/src/world/track_models.rs`），避免大面积路面的
##     自遮挡条纹；
##   - 环境：日光/阴天/夜晚三档环境光；SSAO/SSIL/SSR/体积雾只在 Forward+
##     桌面后端启用，Mobile 保持原样；
##   - 雾：深度雾 + 高度雾，雨/雾天叠加体积雾（Forward+）；
##   - 雨：没有 RainSystem 时用 GPUParticles3D 雨丝兜底（跟随相机、强度随
##     rain）；有 RainSystem（group `vettura_rain`）的场景由它负责；
##   - 干湿路面层：AC 赛道把干/湿两套路面放在完全相同的坐标上（Daytona 有
##     396 个 `_WET` 网格），以前两套一起画导致起跑线地面 z-fighting 闪烁。
##     z-fighting 已在 `gdext/src/kn5/mesh.rs` 用湿层下移 3 mm 根治。
##
## 用法（在任意场景里实例化 weather_visuals.tscn，或 add_child 脚本节点）：
##   $WeatherVisuals.preset_clear()
##   $WeatherVisuals.set_conditions({"wetness": 0.8, "rain": 0.6, "sun": 0.15})
##   # 让天气跟随 World 的物理天气：
##   $WeatherVisuals.sync_from_world($WorldBody)

@export var wetness: float = 0.0:
	set(v):
		wetness = clampf(v, 0.0, 1.0)
		_request_apply()
@export var rain: float = 0.0:
	set(v):
		rain = clampf(v, 0.0, 1.0)
		_request_apply()
@export var fog: float = 0.0:
	set(v):
		fog = clampf(v, 0.0, 1.0)
		_request_apply()
@export var sun: float = 1.0:
	set(v):
		sun = clampf(v, 0.0, 1.0)
		_request_apply()
@export var wind: Vector3 = Vector3.ZERO

## 天空/雾色预设（晴 / 阴雨天 / 夜）。
const SKY_TOP_CLEAR := Color(0.13, 0.32, 0.66)
const SKY_HORIZON_CLEAR := Color(0.62, 0.76, 0.9)
const SKY_TOP_OVERCAST := Color(0.36, 0.40, 0.46)
const SKY_HORIZON_OVERCAST := Color(0.62, 0.65, 0.70)
const SKY_TOP_NIGHT := Color(0.015, 0.025, 0.07)
const SKY_HORIZON_NIGHT := Color(0.05, 0.07, 0.13)
const GROUND_CLEAR := Color(0.12, 0.13, 0.14)
const GROUND_OVERCAST := Color(0.10, 0.11, 0.12)
const FOG_CLEAR := Color(0.66, 0.72, 0.8)
const FOG_WET := Color(0.50, 0.54, 0.58)
const FOG_NIGHT := Color(0.07, 0.09, 0.14)
const AMBIENT_DAY := Color(0.72, 0.77, 0.85)
const AMBIENT_OVERCAST := Color(0.66, 0.70, 0.76)
const AMBIENT_NIGHT := Color(0.32, 0.38, 0.55)
const SUN_DAY := Color(1.0, 0.96, 0.88)
const SUN_NIGHT := Color(0.52, 0.64, 1.0)

var _env: Environment
var _sky_material: ShaderMaterial
var _procedural_sky: ProceduralSkyMaterial
var _sun_light: DirectionalLight3D
var _rain: GPUParticles3D
var _rain_mat: StandardMaterial3D
var _world_env: WorldEnvironment
var _applied := false
var _batching := false
var _dirty := false
var _forward_plus := false
var _cloud_offset := Vector2.ZERO
var _cloud_accum := 0.0
var _last_sync: Dictionary = {}

## 云层 uniform 写入频率。天空 material 的 uniform 变化会触发 radiance
## cubemap 重算，每帧写等于每帧重算天空辐射图。云只有 0.012/s 的漂移，
## 10 Hz 的步进在噪声纹理上不到 1 像素，视觉无差别，但省掉持续的全天空重渲染。
const CLOUD_UPDATE_HZ := 10.0

func _ready() -> void:
	_apply_platform_render_settings()
	_setup_environment()
	_setup_sun()
	_setup_rain()
	_applied = true
	_apply()

## 按运行平台调整渲染质量/开销。桌面 GPU 有富余，开 MSAA 4x 让高对比边缘
## （黑白格、护栏）在移动时不锯齿；移动端保持 MSAA 关闭（带宽/发热优先），
## 各向异性也降到 4x。项目默认值是枚举 4（16x），这里只是按平台覆盖。
func _apply_platform_render_settings() -> void:
	var os := OS.get_name()
	_forward_plus = RenderingServer.get_current_rendering_method() == "forward_plus"
	# 以“平台或渲染后端是 Mobile”为准：桌面强制 `--rendering-method mobile`
	# 调试时也要走移动端的 MSAA/各向异性设置，否则测到的不是真机画面。
	var mobile := os == "Android" or os == "iOS" or not _forward_plus
	var viewport := get_viewport()
	if viewport:
		viewport.msaa_3d = Viewport.MSAA_DISABLED if mobile else Viewport.MSAA_4X
		# 天空/雾的大面积渐变在 8-bit 缓冲上容易出现色带；debanding 只对
		# 桌面启用（移动端多一次抖动 pass）。
		viewport.use_debanding = not mobile
	# 各向异性过滤是纹理导入期设置；运行时改项目设置对已加载纹理无效，
	# 但“默认过滤”会在新纹理创建时读取，这里改对后续创建的贴图生效。
	# 枚举：0 关 / 1=2x / 2=4x / 3=8x / 4=16x。
	ProjectSettings.set_setting(
		"rendering/textures/default_filters/anisotropic_filtering_level",
		2 if mobile else 3,
	)
	print("[weather] platform=%s method=%s msaa=%s aniso=%s" % [
		os, RenderingServer.get_current_rendering_method(),
		"off" if mobile else "4x", "4x" if mobile else "8x"])

func _setup_environment() -> void:
	# 复制工程默认环境再改：不要把天气状态写回共享资源（影响编辑器与其它场景）。
	var base: Environment = load("res://game_environment.tres") as Environment
	_env = base.duplicate(true) as Environment
	if _env.sky != null:
		_env.sky = _env.sky.duplicate() as Sky
		var material: Material = _env.sky.sky_material
		if material is ShaderMaterial:
			_sky_material = (material as ShaderMaterial).duplicate() as ShaderMaterial
			_env.sky.sky_material = _sky_material
			# 继承大厅/上一场景已经漂移到的云相位，切场景时云不会跳回 0。
			var inherited: Variant = _sky_material.get_shader_parameter("cloud_offset")
			if inherited is Vector2:
				_cloud_offset = inherited as Vector2
		elif material is ProceduralSkyMaterial:
			_procedural_sky = (material as ProceduralSkyMaterial).duplicate() as ProceduralSkyMaterial
			_env.sky.sky_material = _procedural_sky
	else:
		# 极端情况：默认环境没有天空。建一个同款 sky shader，保证天色链路可用。
		_env.background_mode = Environment.BG_SKY
		_env.sky = Sky.new()
		_sky_material = ShaderMaterial.new()
		_sky_material.shader = load("res://game/render/sky.gdshader")
		_env.sky.sky_material = _sky_material
	_world_env = WorldEnvironment.new()
	_world_env.name = "WeatherEnvironment"
	_world_env.environment = _env
	add_child(_world_env)

func _setup_sun() -> void:
	# 复用场景里已有的第一盏 DirectionalLight3D（ai_lap.gd 等），没有才新建。
	_sun_light = _find_first_sun(get_tree().root)
	if _sun_light == null:
		_sun_light = DirectionalLight3D.new()
		_sun_light.name = "WeatherSun"
		_sun_light.rotation_degrees = Vector3(-48.0, -125.0, 0.0)
		add_child(_sun_light)
	# 起跑区路面 z-fighting 的一大半是阴影自遮挡条纹：normal bias 把采样点
	# 沿法线抬起，消除共面路面/标线上的 acne，而不会让车底阴影明显脱离。
	_sun_light.shadow_enabled = true
	_sun_light.shadow_bias = 0.00008
	_sun_light.shadow_normal_bias = 0.08
	_sun_light.shadow_opacity = 0.92
	_sun_light.light_color = SUN_DAY
	# 太阳有角直径，阴影边缘才不是刀切；0.5° 是真实太阳。
	_sun_light.light_angular_distance = 0.5
	var mobile := OS.get_name() == "Android" or OS.get_name() == "iOS" or not _forward_plus
	if mobile:
		# 移动端：2 级 CSM + 64m 阴影距离。4 级/120m 在手机上每帧多画两张
		# 阴影图，却很难在 6 寸屏上看出来；省下来的预算留给 MSAA/天空/雨。
		_sun_light.directional_shadow_mode = DirectionalLight3D.SHADOW_PARALLEL_2_SPLITS
		_sun_light.directional_shadow_max_distance = 64.0
		_sun_light.directional_shadow_split_1 = 0.16
		_sun_light.shadow_blur = 0.7
		# 软阴影降一档：角直径 0.35°，阴影边缘仍不硬切。
		_sun_light.light_angular_distance = 0.35
		_sun_light.shadow_bias = 0.00012
		_sun_light.shadow_normal_bias = 0.10
	else:
		# 桌面：4 级 CSM + 软阴影 + 更远的近景分割，让起跑区（近处大平面）
		# 拿到更多阴影图分辨率；2 级 + 100m 在相机移动时边缘会跳。
		_sun_light.directional_shadow_mode = DirectionalLight3D.SHADOW_PARALLEL_4_SPLITS
		_sun_light.directional_shadow_max_distance = 120.0
		_sun_light.directional_shadow_split_1 = 0.08
		_sun_light.directional_shadow_split_2 = 0.2
		_sun_light.shadow_blur = 1.0
	_sun_light.light_volumetric_fog_energy = 1.6

func _find_first_sun(node: Node) -> DirectionalLight3D:
	for child in node.get_children():
		if child is DirectionalLight3D:
			return child as DirectionalLight3D
		var found := _find_first_sun(child)
		if found != null:
			return found
	return null

func _setup_rain() -> void:
	var process := ParticleProcessMaterial.new()
	process.direction = Vector3(0.12, -1.0, 0.05)
	process.spread = 3.0
	process.initial_velocity_min = 28.0
	process.initial_velocity_max = 42.0
	process.gravity = Vector3(0.0, -4.0, 0.0)
	process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_BOX
	process.emission_box_extents = Vector3(24.0, 1.0, 24.0)

	var drop := BoxMesh.new()
	drop.size = Vector3(0.012, 0.55, 0.012)
	_rain_mat = StandardMaterial3D.new()
	_rain_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_rain_mat.albedo_color = Color(0.72, 0.82, 0.95, 0.42)
	_rain_mat.roughness = 1.0
	drop.material = _rain_mat

	_rain = GPUParticles3D.new()
	_rain.name = "Rain"
	_rain.process_material = process
	_rain.draw_pass_1 = drop
	_rain.amount = 2600
	_rain.lifetime = 0.9
	_rain.local_coords = false
	_rain.emitting = false
	_rain.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# 粒子在世界空间发射，必须给足包围盒，否则相机转动时整层雨会被视锥裁掉。
	_rain.visibility_aabb = AABB(Vector3(-28.0, -10.0, -28.0), Vector3(56.0, 40.0, 56.0))
	add_child(_rain)

func _process(delta: float) -> void:
	_drift_clouds(delta)
	if _rain == null or not _rain.emitting:
		return
	# 双保险：RainSystem 可能比本节点晚创建（FxRuntime 先建 WeatherFX 再建
	# RainFX）。一旦发现它，就关掉这层兜底雨，避免双层雨。
	if get_tree().get_first_node_in_group("vettura_rain") != null:
		_rain.emitting = false
		return
	# 雨始终下在相机周围（否则只在世界原点下一小片）。
	var cam := get_viewport().get_camera_3d()
	if cam != null:
		var forward := -cam.global_transform.basis.z
		_rain.global_position = cam.global_position + forward * 8.0 + Vector3(0.0, 6.0, 0.0)

## 云层漂移：按 `CLOUD_UPDATE_HZ` 推进 uniform，云仍然连续移动，
## 但不会每帧重建天空辐射 cubemap。
##
## 天空 material 的 uniform 变化会触发辐射 cubemap 更新；cubemap 的
## `process_mode` 已设为 INCREMENTAL（`game_environment.tres`），更新分摊到
## 多帧；再配合 10 Hz 的 uniform 写入，辐射图有时间收敛，不会一直重算。
## 云噪声本身来自预生成纹理，不再是每像素 20 层 value noise。
func _drift_clouds(delta: float) -> void:
	if _sky_material == null:
		return
	var wind2 := Vector2(wind.x, wind.z)
	# 云应该是「相对静止、缓慢漂移」：默认 0.012/s，风再加 0.003 倍。
	# 按 cloud_noise_scale=0.16 算，一次纹理重复约 5 分钟；以前的 0.35/s
	# 会让云在几分钟内跑完几十个周期，看起来像跟着车飞。
	var drift := Vector2(0.012, 0.004) + wind2 * 0.003
	_cloud_offset += drift * delta
	# 防止长跑后浮点精度变差；云噪声是周期性的，回绕不可见。
	_cloud_offset = _cloud_offset.posmod(1024.0)
	_cloud_accum += delta
	if _cloud_accum < 1.0 / CLOUD_UPDATE_HZ:
		return
	_cloud_accum = 0.0
	_sky_material.set_shader_parameter("cloud_offset", _cloud_offset)

func _request_apply() -> void:
	if not _applied or _batching:
		_dirty = true
		return
	_apply()

func _apply() -> void:
	if not _applied:
		return
	var overcast: float = clampf(
		(1.0 - sun) * 0.75 + fog * 0.4 + rain * 0.35 + wetness * 0.15,
		0.0, 1.0)
	# 夜晚：太阳能量低于 0.3 后逐步过渡；阴雨天不会因为 sun 低就变成黑夜，
	# 所以用“太阳低 + 没有雨雾”的联合条件。
	var night: float = clampf(1.0 - sun / 0.28, 0.0, 1.0)
	night *= clampf(1.0 - fog * 0.6 - rain * 0.4, 0.0, 1.0)
	var day: float = 1.0 - night

	var top := SKY_TOP_CLEAR.lerp(SKY_TOP_OVERCAST, overcast)
	var horizon := SKY_HORIZON_CLEAR.lerp(SKY_HORIZON_OVERCAST, overcast)
	top = top.lerp(SKY_TOP_NIGHT, night)
	horizon = horizon.lerp(SKY_HORIZON_NIGHT, night)

	_apply_sky(top, horizon, overcast, night, day)
	_apply_lighting(overcast, night, day)
	_apply_atmosphere(overcast, night)
	_apply_environment_quality(night)
	_apply_rain()
	_apply_wet_layers()
	_dirty = false


## 雨丝兜底：场景里有 RainSystem（group `vettura_rain`）时由它负责；没有
## 的场景（ai_lap / desktop_probe）才用这里的轻量 GPUParticles，保证
## `preset_rain()` / `AI_LAP_WEATHER=rain` 真的能看到雨，而不是死代码。
func _apply_rain() -> void:
	if _rain == null:
		return
	var external := get_tree().get_first_node_in_group("vettura_rain") != null
	_rain.emitting = rain > 0.02 and not external

func _apply_sky(top: Color, horizon: Color, overcast: float, night: float, day: float) -> void:
	if _sky_material != null:
		_sky_material.set_shader_parameter("sky_top_color", top)
		_sky_material.set_shader_parameter("sky_horizon_color", horizon)
		_sky_material.set_shader_parameter("ground_color",
			GROUND_CLEAR.lerp(GROUND_OVERCAST, overcast).lerp(SKY_TOP_NIGHT, night))
		# 云量：晴天也有薄云；阴雨天接近满云。夜间的云用月光调亮。
		var coverage := clampf(0.22 + overcast * 0.72 + rain * 0.12, 0.0, 1.0)
		_sky_material.set_shader_parameter("cloud_coverage", coverage)
		_sky_material.set_shader_parameter("cloud_opacity", lerpf(0.72, 1.0, overcast))
		_sky_material.set_shader_parameter("cloud_sharpness", lerpf(0.30, 0.14, overcast))
		_sky_material.set_shader_parameter("cloud_lit_color",
			Color(0.95, 0.96, 0.99).lerp(Color(0.62, 0.66, 0.74), overcast)
				.lerp(Color(0.28, 0.32, 0.42), night))
		_sky_material.set_shader_parameter("cloud_shadow_color",
			Color(0.38, 0.42, 0.50).lerp(Color(0.28, 0.31, 0.37), overcast)
				.lerp(Color(0.08, 0.10, 0.16), night))
		_sky_material.set_shader_parameter("sun_color", SUN_DAY.lerp(SUN_NIGHT, night))
		_sky_material.set_shader_parameter("night", night)
		_sky_material.set_shader_parameter("star_intensity",
			clampf(night * (1.0 - overcast * 0.85), 0.0, 1.0) * 1.1)
		_sky_material.set_shader_parameter("moon_intensity", night * (1.0 - overcast * 0.5))
		_sky_material.set_shader_parameter("horizon_haze",
			clampf(0.30 + fog * 0.45 + rain * 0.15, 0.0, 0.9))
		_sky_material.set_shader_parameter("sky_exposure", lerpf(1.0, 0.75, overcast))
		_sky_material.set_shader_parameter("cloud_offset", _cloud_offset)
	elif _procedural_sky != null:
		# 兼容旧环境资源：没有 sky shader 时退回 ProceduralSkyMaterial。
		_procedural_sky.sky_top_color = top
		_procedural_sky.sky_horizon_color = horizon
		_procedural_sky.energy_multiplier = lerpf(1.0, 0.45, overcast) * lerpf(1.0, 0.18, night)

func _apply_lighting(overcast: float, night: float, day: float) -> void:
	# 太阳恢复 2.2：高光过曝已由 metallic_specular 修复，不需要再压暗整体光照。
	var energy := lerpf(2.2, 0.55, overcast) * sun
	# 夜间给一点月光，纯 0 会让车漆/护栏完全失去轮廓。
	energy = maxf(energy, 0.14 * night)
	_sun_light.light_energy = energy
	_sun_light.light_color = SUN_DAY.lerp(SUN_NIGHT, night)
	_sun_light.light_volumetric_fog_energy = lerpf(1.6, 0.3, overcast) * maxf(day, 0.15)
	# 环境光是背光面（看台、建筑、远处围栏）的唯一光源。Godot 4.7 的
	# `AMBIENT_SOURCE_SKY` 实测不产生环境光，只能用 COLOR + 较高能量（1.85）；
	# 阴雨天略降，夜晚压到月光水平。
	var ambient := AMBIENT_DAY.lerp(AMBIENT_OVERCAST, overcast).lerp(AMBIENT_NIGHT, night)
	_env.ambient_light_color = ambient
	_env.ambient_light_energy = lerpf(2.0, 2.4, overcast) * lerpf(1.0, 0.45, night)

func _apply_atmosphere(overcast: float, night: float) -> void:
	_env.fog_enabled = fog > 0.01 or rain > 0.05
	_env.fog_density = 0.0012 + fog * 0.008 + rain * 0.0035
	_env.fog_light_color = FOG_CLEAR.lerp(FOG_WET, overcast).lerp(FOG_NIGHT, night)
	_env.fog_light_energy = lerpf(1.0, 0.55, night)
	_env.fog_aerial_perspective = lerpf(0.25, 0.65, overcast)
	_env.fog_sky_affect = lerpf(0.15, 0.45, overcast)
	# 地面雾贴着赛道，远处 (100m+) 衰减，避免整条赛道被一层平板雾盖住。
	_env.fog_height = 0.0
	_env.fog_height_density = lerpf(0.0, 0.05, clampf(fog + rain * 0.6, 0.0, 1.0))

func _apply_environment_quality(night: float) -> void:
	if _forward_plus:
		# 桌面 Forward+：屏幕空间遮蔽/反射/体积雾。降雨或积水时 SSR 才真正
		# 有可反射内容，晴天关掉省一次全屏 pass。
		_env.ssao_enabled = true
		_env.ssil_enabled = true
		_env.ssr_enabled = wetness > 0.05 or rain > 0.02
		_env.volumetric_fog_enabled = fog > 0.02 or rain > 0.05
		if _env.volumetric_fog_enabled:
			_env.volumetric_fog_density = 0.0015 + fog * 0.020 + rain * 0.012
			_env.volumetric_fog_albedo = Color(0.72, 0.78, 0.88).lerp(
				Color(0.28, 0.32, 0.42), night)
			_env.volumetric_fog_length = lerpf(96.0, 48.0, fog)
			_env.volumetric_fog_anisotropy = lerpf(0.35, 0.6, rain)
			_env.volumetric_fog_sky_affect = lerpf(0.1, 0.35, fog)
	elif _env.volumetric_fog_enabled:
		# Mobile / Compatibility 不支持体积雾：显式关掉，避免引擎告警。
		_env.volumetric_fog_enabled = false

## 干湿路面层：湿层始终可见，只控制透明度/细节。
##
## z-fighting 已经在 `gdext/src/kn5/mesh.rs` 里用“湿层整体下移 3 mm”根治；
## 隐藏湿层会让赛道边（路肩/缓冲区，往往只有湿层几何）整圈消失，所以这里
## 不再切换可见性。湿天额外稍微加强湿层的不透明度营造湿滑观感。
func _apply_wet_layers() -> void:
	pass

## 一次性设置整组天气参数（GDScript 字典或具名参数均可）。
func set_conditions(params: Dictionary) -> void:
	_batching = true
	if params.has("wetness"): wetness = float(params["wetness"])
	if params.has("rain"): rain = float(params["rain"])
	if params.has("fog"): fog = float(params["fog"])
	if params.has("sun"): sun = float(params["sun"])
	if params.has("wind"): wind = params["wind"] as Vector3
	_batching = false
	_apply()

func preset_clear() -> void:
	set_conditions({"wetness": 0.0, "rain": 0.0, "fog": 0.0, "sun": 1.0})

func preset_overcast() -> void:
	set_conditions({"wetness": 0.15, "rain": 0.0, "fog": 0.25, "sun": 0.35})

func preset_rain() -> void:
	set_conditions({"wetness": 0.9, "rain": 0.8, "fog": 0.4, "sun": 0.12})

func preset_fog() -> void:
	set_conditions({"wetness": 0.35, "rain": 0.0, "fog": 0.9, "sun": 0.3})

func preset_night() -> void:
	set_conditions({"wetness": 0.0, "rain": 0.0, "fog": 0.1, "sun": 0.0})

## 从 WorldBody 的物理天气同步（批量、低频调用即可，例如每 0.5 秒）。
##
## `weather_fx.gd` 目前每帧调用。这里先做一次值比较：天气没变就不重写环境；
## 有细微变化（<1% 湿度/雨量、<0.05 m/s 风）时也先不写，等累计变化超过阈值
## 再一次性应用，避免每帧重写天空/雾参数导致的抖动。云层漂移不经过这里，
## 在 `_process` 里每帧独立推进。
const SYNC_EPSILON := 0.01
const WIND_EPSILON := 0.05

func sync_from_world(world: Node) -> void:
	if world == null or not world.has_method("get_weather"):
		return
	var w: Dictionary = world.call("get_weather")
	var values := {
		"wetness": w.get("wetness", 0.0),
		"rain": w.get("rain", 0.0),
		"fog": w.get("fog", 0.0),
		"sun": w.get("sun", 1.0),
		"wind": w.get("wind", Vector3.ZERO),
	}
	if _last_sync == values:
		return
	if not _last_sync.is_empty() and _sync_close(_last_sync, values):
		return
	_last_sync = values
	set_conditions(values)

func _sync_close(a: Dictionary, b: Dictionary) -> bool:
	for key in ["wetness", "rain", "fog", "sun"]:
		if absf(float(a.get(key, 0.0)) - float(b.get(key, 0.0))) > SYNC_EPSILON:
			return false
	var wind_a: Vector3 = a.get("wind", Vector3.ZERO)
	var wind_b: Vector3 = b.get("wind", Vector3.ZERO)
	return wind_a.distance_to(wind_b) <= WIND_EPSILON
