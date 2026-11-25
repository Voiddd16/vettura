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
	_environment.glow_intensity = _float("strength", 0.28)
	_environment.glow_bloom = 0.035
	_environment.glow_hdr_threshold = 1.2

	if high and forward_plus:
		_environment.ssao_enabled = true
		_environment.ssao_radius = 1.2
		_environment.ssao_intensity = _float("ssao_strength", 1.5)
		_environment.ssil_enabled = true
		_environment.ssil_intensity = 0.4
	else:
		_environment.ssao_enabled = false
		_environment.ssil_enabled = false

	# SSR 在 mobile 渲染器上不可用；即使配置写了也保持关闭。
	_environment.ssr_enabled = false

	_environment.tonemap_mode = Environment.TONE_MAPPER_ACES
	_environment.adjustment_enabled = true
	_environment.adjustment_brightness = _float("brightness", 1.0)
	_environment.adjustment_contrast = _float("contrast", 1.0)
	_environment.adjustment_saturation = _float("saturation", 1.0)


func _float(key: String, fallback: float) -> float:
	if not _settings.has(key):
		return fallback
	var value: Variant = _settings[key]
	var text := String(value).strip_edges()
	return text.to_float() if text.is_valid_float() else fallback
