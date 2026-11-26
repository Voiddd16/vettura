extends Node3D
## ExtraFX：把 EXTRA_FX / EXTRA_FX_SSAO / BLOOM / DOF ... 映射到 Godot Environment。
##
## v1 只做移动端安全的子集：
## - glow / tonemap / adjustment；
## - 高画质才开 SSAO/SSR/SSIL；
## - 不运行 CSP shader，不替换整条后处理链。
##
## 它只修改本节点自己创建的 WorldEnvironment；如果场景里已有，则复用。

var enabled := true
var _environment: Environment
var _world_env: WorldEnvironment
var _quality := "medium"
var _settings: Dictionary = {}


func setup(_world: WorldBody, records: Array, quality: String) -> void:
	_quality = quality
	_apply_records(records)
	_ensure_environment()
	_apply_settings()
	print("[extra] environment=%s" % str(_environment != null))


func _apply_records(records: Array) -> void:
	for record in records:
		if not (record is Dictionary):
			continue
		var module := String(record.get("module", ""))
		if not module.begins_with("vettura.fx.extra"):
			continue
		var values: Dictionary = record.get("values", {})
		for key in values.keys():
			_settings[String(key).to_lower()] = values[key]


func _ensure_environment() -> void:
	var root := get_tree().current_scene
	if root != null:
		for node in root.find_children("*", "WorldEnvironment", true, false):
			var env_node := node as WorldEnvironment
			if env_node != null and env_node.environment != null:
				_world_env = env_node
				_environment = env_node.environment
				return
	_world_env = WorldEnvironment.new()
	_world_env.name = "ExtraFXEnvironment"
	# 没有现成环境时复用工程默认环境，避免独立挂 ExtraFX 的场景丢掉天空/tonemap。
	var base := load("res://game_environment.tres") as Environment
	if base != null:
		_environment = base.duplicate(true) as Environment
	else:
		_environment = Environment.new()
		_environment.background_mode = Environment.BG_SKY
		_environment.sky = Sky.new()
		_environment.sky.sky_material = ProceduralSkyMaterial.new()
	_world_env.environment = _environment
	add_child(_world_env)


func _apply_settings() -> void:
	if _environment == null:
		return
	var high := _quality == "high" or _quality == "ultra"
	var medium := _quality != "low" and _quality != "off"
	# Mobile 后端不支持 SSAO/SSIL/SSR；不要为了“高画质”在手机上打开这些，
	# 它们会被引擎忽略或直接告警。只有 Forward+ 才启用屏幕空间遮蔽。
	var forward_plus := RenderingServer.get_current_rendering_method() == "forward_plus"

	_environment.glow_enabled = medium
	_environment.glow_intensity = _float("strength", 0.18)
	_environment.glow_bloom = _float("bloom", 0.02)
	_environment.glow_hdr_threshold = _float("threshold", 1.5)
	_environment.glow_blend_mode = Environment.GLOW_BLEND_MODE_SCREEN

	# 桌面 Forward+ 默认开 SSAO/SSIL（基线环境已经配好半径/强度）；ext_config
	# 只覆盖记录里显式给出的强度，不把天气系统/基线环境的设置整片关掉。
	if forward_plus and medium:
		_environment.ssao_enabled = true
		_environment.ssil_enabled = true
		if _settings.has("ssao_strength"):
			_environment.ssao_intensity = _float("ssao_strength", 1.15)
		if _settings.has("ssil_strength"):
			_environment.ssil_intensity = _float("ssil_strength", 0.55)
	else:
		_environment.ssao_enabled = false
		_environment.ssil_enabled = false

	# SSR 是否开启由天气系统按湿度/降雨决定；ext_config 只有显式写了
	# `ssr` 时才覆盖，避免这里把湿路面的反射整片关掉。
	if _settings.has("ssr"):
		_environment.ssr_enabled = forward_plus and high and _float("ssr", 1.0) > 0.5

	_environment.tonemap_mode = Environment.TONE_MAPPER_ACES
	_environment.adjustment_enabled = true
	# 没有记录时保留基线环境调好的轻微 contrast/saturation；有记录才覆盖。
	_environment.adjustment_brightness = _float("brightness",
		_environment.adjustment_brightness)
	_environment.adjustment_contrast = _float("contrast",
		_environment.adjustment_contrast)
	_environment.adjustment_saturation = _float("saturation",
		_environment.adjustment_saturation)


func _float(key: String, fallback: float) -> float:
	if not _settings.has(key):
		return fallback
	var value: Variant = _settings[key]
	var text := String(value).strip_edges()
	return text.to_float() if text.is_valid_float() else fallback
