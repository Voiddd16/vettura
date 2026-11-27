extends Node
## 渲染档位：按后端给默认环境打开/关闭屏幕空间特性。
##
## 工程在桌面用 Forward+、移动端用 Mobile（`project.godot` 的
## `rendering/renderer/rendering_method.mobile`）。SSAO / SSIL / SSR / 体积雾
## 只有 Forward+ 支持；如果把它们直接写进 `game_environment.tres`，Android
## 启动时引擎会对每个属性打印 “only available when using the Forward+
## renderer” 警告。所以静态资源里保持关闭，由这个 autoload 在桌面打开。
##
## 时序：autoload 的 `_ready()` 早于主场景；改的是 `game_environment.tres`
## 的共享实例，之后 `weather_rig.gd` / `weather_visuals.gd` / `rain_system.gd` 再
## duplicate 时会继承这些开关。移动端不打开，保持零额外 pass。

## 云层 uniform 的写入频率。天空材质的 uniform 一旦变化，Godot 就会把
## radiance cubemap 标脏并重新渲染；每帧写 `cloud_offset` 等于每帧重算 256²
## 的天空辐射图。云本身只有 0.012/s 的漂移，10 Hz 写入时单步位移约
## 0.0012（噪声纹理上不到 1 像素），肉眼完全看不出跳格，却把辐射图更新
## 从“每帧”降到“每秒 10 次”，所有平台都省下一块稳定的 GPU 时间。
const CLOUD_UPDATE_HZ := 10.0

## 全屏自动分辨率：3D 内部目标宽度约 1440 px，按实际窗口宽度换算初始比例，
## 再用帧率在 [0.35, 1.0] 之间缓慢调整。UI/后处理仍在原生分辨率，只有 3D
## 走上采样（FSR1/FSR2/MetalFX/引擎补丁的 FSR3.1），避免整屏都糊。
##
## 上采样器选择、动态分辨率与 VRS 的 Viewport 写入都委托给 autoload
## `DisplayPipeline`；本脚本只负责“什么时候应用”和“同时调整 MSAA/后处理”。
signal adaptive_changed(scale: float, fps: float)

var _sky_material: ShaderMaterial
var _default_env: Environment
var _cloud_offset := Vector2.ZERO
var _cloud_accum := 0.0

func _ready() -> void:
	var env := load("res://game_environment.tres") as Environment
	if env == null:
		push_warning("[render-quality] 缺少 game_environment.tres")
		return
	var forward_plus := RenderingServer.get_current_rendering_method() == "forward_plus"
	if forward_plus:
		_apply_forward_plus(env)
	else:
		# 移动端 / Compatibility：显式关闭，避免资源被别处打开后残留。
		env.ssao_enabled = false
		env.ssil_enabled = false
		env.ssr_enabled = false
		env.volumetric_fog_enabled = false
	_apply_sky_quality(env, forward_plus)
	_apply_viewport_quality(forward_plus)
	# 大厅/车库/主菜单没有 WeatherRig，云彩也要动：由这个 autoload
	# 推进默认环境的 cloud_offset。天气场景会 duplicate 一份自己的材质并
	# 自行推进，两边互不影响。
	if env.sky != null and env.sky.sky_material is ShaderMaterial:
		_sky_material = env.sky.sky_material as ShaderMaterial
		_default_env = env
	GameSettings.changed.connect(_on_settings_changed)
	DisplayPipeline.scale_changed.connect(_on_display_scale_changed)
	_apply_fullscreen_profile()
	call_deferred("_apply_fullscreen_profile")
	# 云漂移仍由本脚本推进；动态分辨率在 DisplayPipeline 的 `_process` 里。
	set_process(true)
	print("[render-quality] method=%s ssao=%s ssr=%s upscaler=%s vrs=%s" % [
		RenderingServer.get_current_rendering_method(),
		str(env.ssao_enabled), str(env.ssr_enabled),
		DisplayPipeline.active_upscaler(), String(DisplayPipeline.state().get("vrs", "off"))])


func _on_settings_changed(section: String, key: String, _value: Variant) -> void:
	if section == "video" and (key == "window_mode" or key == "fullscreen_adaptive" \
			or key == "render_scale" or key == "upscaler" or key == "vrs" \
			or key == "dynamic_target_fps"):
		call_deferred("_apply_fullscreen_profile")


func _on_display_scale_changed(scale: float, fps: float) -> void:
	adaptive_changed.emit(scale, fps)


## 默认环境的云层漂移（没有天气系统的场景也保持云在动）。
func _process(delta: float) -> void:
	_process_clouds(delta)


func _process_clouds(delta: float) -> void:
	if _sky_material == null:
		return
	# 天气/自定义 WorldEnvironment 接管后，默认材质不再参与渲染；不要每帧
	# 无意义地写它，避免触发一份没人看的 cubemap 更新。
	var world := get_viewport().world_3d if get_viewport() != null else null
	if world != null and world.environment != null and world.environment != _default_env:
		return
	# 无天气系统的场景（大厅/车库）也保持云缓慢漂移；速率必须和
	# `weather_rig.gd` / `weather_visuals.gd` 一致，否则切场景时云会突然加速。
	_cloud_offset += Vector2(0.012, 0.004) * delta
	_cloud_offset = _cloud_offset.posmod(1024.0)
	_cloud_accum += delta
	if _cloud_accum < 1.0 / CLOUD_UPDATE_HZ:
		return
	_cloud_accum = 0.0
	_sky_material.set_shader_parameter("cloud_offset", _cloud_offset)


func adaptive_scale() -> float:
	return DisplayPipeline.scale()


func is_adaptive() -> bool:
	return DisplayPipeline.is_dynamic()


## 窗口尺寸变化 / 全屏切换后重设 3D 放大策略。
##
## 分辨率/上采样/VRS 全部交给 DisplayPipeline；这里只负责 MSAA、屏幕空间
## AA、去色带和后处理性能档位——这些和“用哪种上采样器”是两件事。
func _apply_fullscreen_profile() -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	var mobile := OS.has_feature("mobile")
	var fullscreen := GameSettings.is_fullscreen()

	if mobile:
		# 移动端的 scaling_3d_scale 由 RainSystem 画质档位管理，DisplayPipeline
		# 只换上采样模式/VRS，不抢 scale。
		_call_render_fx(false)
		DisplayPipeline.apply_profile()
		adaptive_changed.emit(DisplayPipeline.scale(), 0.0)
		return

	if not fullscreen:
		viewport.msaa_3d = Viewport.MSAA_4X
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
		viewport.use_debanding = true
		_call_render_fx(false)
	else:
		# 全屏像素多，MSAA 2x 已经足够；4x 的带宽在 4K 上代价过高。
		viewport.msaa_3d = Viewport.MSAA_2X
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
		viewport.use_debanding = true
		_call_render_fx(true)

	DisplayPipeline.apply_profile()
	var state := DisplayPipeline.state()
	adaptive_changed.emit(float(state.get("scale", 1.0)), 0.0)
	print("[render-quality] fullscreen=%s upscaler=%s scale=%.2f vrs=%s msaa=%s" % [
		str(fullscreen),
		String(state.get("active_upscaler", "native")),
		float(state.get("scale", 1.0)),
		String(state.get("vrs", "off")),
		"2x" if fullscreen else "4x",
	])


func _call_render_fx(performance: bool) -> void:
	var fx := get_node_or_null("/root/RenderFX")
	if fx != null and fx.has_method("set_performance_mode"):
		fx.call("set_performance_mode", performance)

## 抗锯齿/去色带也按后端选：桌面 MSAA 4x + debanding；移动端 FXAA、不开 MSAA
## （带宽/发热优先）。RainSystem 会按自己的画质再覆盖一次，二者不冲突。
func _apply_viewport_quality(forward_plus: bool) -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	if forward_plus:
		viewport.msaa_3d = Viewport.MSAA_4X
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
		viewport.use_debanding = true
	else:
		viewport.msaa_3d = Viewport.MSAA_DISABLED
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_FXAA
		viewport.use_debanding = false

## 天空也分档：移动端用 3 octave 云 + 128 辐射 cubemap，桌面用完整 5 octave。
## 云 shader 每像素的 fBm 是手机上最容易被低估的开销。
func _apply_sky_quality(env: Environment, forward_plus: bool) -> void:
	if env.sky == null:
		return
	env.sky.radiance_size = Sky.RADIANCE_SIZE_256 if forward_plus else Sky.RADIANCE_SIZE_128
	var material := env.sky.sky_material
	if material is ShaderMaterial:
		(material as ShaderMaterial).set_shader_parameter(
			"cloud_quality", 1.0 if forward_plus else 0.0)

## Forward+ 桌面基线。数值与旧 `game_environment.tres` 保持一致：
## SSAO 压暗接缝、SSIL 补一点反弹光、SSR 给湿路面/玻璃可反射内容。
## 体积雾不在大厅默认开（车展悬浮场景会显得发灰），由天气系统按雨/雾开。
func _apply_forward_plus(env: Environment) -> void:
	env.ssao_enabled = true
	env.ssao_radius = 1.1
	env.ssao_intensity = 1.15
	env.ssao_power = 2.0
	env.ssao_detail = 0.8
	env.ssao_horizon = 0.06
	env.ssao_sharpness = 0.98
	env.ssao_light_affect = 0.1
	env.ssao_ao_channel_affect = 0.0

	env.ssil_enabled = true
	env.ssil_radius = 4.5
	env.ssil_intensity = 0.55
	env.ssil_sharpness = 0.98
	env.ssil_normal_rejection = 1.0

	env.ssr_enabled = true
	env.ssr_max_steps = 96
	env.ssr_fade_in = 0.12
	env.ssr_fade_out = 2.0
	env.ssr_depth_tolerance = 0.2

	# 先写好参数；`volumetric_fog_enabled` 留给天气系统决定。
	env.volumetric_fog_density = 0.004
	env.volumetric_fog_albedo = Color(0.72, 0.78, 0.88)
	env.volumetric_fog_emission = Color(0.0, 0.0, 0.0)
	env.volumetric_fog_emission_energy = 0.0
	env.volumetric_fog_gi_inject = 0.0
	env.volumetric_fog_anisotropy = 0.35
	env.volumetric_fog_length = 96.0
	env.volumetric_fog_detail_spread = 2.0
	env.volumetric_fog_ambient_inject = 0.15
	env.volumetric_fog_sky_affect = 0.1
	env.volumetric_fog_temporal_reprojection_enabled = true
	env.volumetric_fog_temporal_reprojection_amount = 0.9
