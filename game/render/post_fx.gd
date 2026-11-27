extends Node
## 全屏镜头后处理（autoload `RenderFX`）。
##
## 在 3D 场景之后、HUD 之前挂一层 `CanvasLayer(layer = -100)`：
## - 桌面：`post_fx.gdshader`，暗角 / 径向色散 / 胶片颗粒 / 轻锐化；
## - 移动端：`post_fx_vignette.gdshader`，只做一层不读屏幕纹理的廉价暗角。
## 参考 `mods/imitate/acc-weatherfx-base` 的 ppfilter（pp_final.fx）配方，
## 但移动端不付全屏 screen texture 读写的带宽。
##
## 为什么是 autoload：
## - 大厅、车库、赛道、各个冒烟场景都应该有同一层“相机感”，逐个场景挂节点
##   容易漏；
## - CanvasLayer 层号 -100 在 UI（默认 0）之下，所以不会给按钮/文字做暗角；
##   雨系统的 9/10 层仍叠在最上面，互不干扰。
##
## 需要更强/更弱时改 `PRESETS` 或运行期调 `set_parameters()`；也可以在
## `user://settings.cfg` 里把 `video/post_fx` 关掉。

const SHADER := preload("res://game/render/post_fx.gdshader")
const MOBILE_SHADER := preload("res://game/render/post_fx_vignette.gdshader")

## 预设：off 完全不创建；subtle 是桌面默认；cinematic 适合回放/截图。
const PRESETS := {
	"off": {
		"vignette_strength": 0.0,
		"chromatic_aberration": 0.0,
		"film_grain": 0.0,
		"sharpen": 0.0,
		"lens_distortion": 0.0,
	},
	"subtle": {
		"vignette_strength": 0.22,
		"chromatic_aberration": 0.0006,
		"film_grain": 0.012,
		"sharpen": 0.14,
		"lens_distortion": 0.0,
	},
	"cinematic": {
		"vignette_strength": 0.34,
		"chromatic_aberration": 0.0015,
		"film_grain": 0.022,
		"sharpen": 0.20,
		"lens_distortion": 0.02,
	},
}

var _layer: CanvasLayer
var _rect: ColorRect
var _material: ShaderMaterial
## `_viewport_size` 是 UI 画布（设计分辨率），`_screen_size` 是后处理真正读写的
## 屏幕纹理尺寸。`canvas_items` 拉伸下两者在 Retina/全屏能差 2~3 倍；锐化
## 的 texel 必须用后者，否则全屏会按 1280 的纹素做大半径模糊。
var _viewport_size := Vector2(1280.0, 720.0)
var _screen_size := Vector2(1280.0, 720.0)
var _enabled := true
var _preset := "subtle"
var _is_mobile := false
var _performance := false
## MetalFX Temporal / FSR2 这类时域上采样会让输出略软；Apple MetalFX 示例用
## 负 mipmap bias 解决，这里再补一档轻锐化。状态变化时才重设 uniform。
var _temporal_upscaler := false
var _upscaler_checked := false


func _ready() -> void:
	process_mode = Node.PROCESS_MODE_ALWAYS
	if DisplayServer.get_name() == "headless":
		return
	_is_mobile = OS.get_name() == "Android" or OS.get_name() == "iOS" \
		or RenderingServer.get_current_rendering_method() == "mobile"
	if not _wanted():
		return
	_build()
	_apply_preset(_preset)
	set_process(true)


func _process(_delta: float) -> void:
	if _is_mobile or _material == null:
		return
	_refresh_upscaler_sharpen()
	var viewport := get_viewport()
	if viewport == null:
		return
	var size := viewport.get_texture().get_size()
	if size.x <= 0.0 or size.y <= 0.0 or size == _screen_size:
		return
	_screen_size = size
	_push_viewport()


## 从 DisplayPipeline 读取当前上采样器；只在 temporal ↔ 其他切换时重设后处理。
func _refresh_upscaler_sharpen() -> void:
	var pipeline := get_node_or_null("/root/DisplayPipeline")
	var temporal := false
	if pipeline != null and pipeline.has_method("active_upscaler"):
		temporal = String(pipeline.call("active_upscaler")) in [
			"fsr2", "fsr3_1", "metalfx_temporal",
		]
	if _upscaler_checked and temporal == _temporal_upscaler:
		return
	_upscaler_checked = true
	_temporal_upscaler = temporal
	_apply_preset(_preset)


func _wanted() -> bool:
	# 调试开关：VETTURA_POST_FX=0 跳过整层后处理（截图对比/定位问题用）。
	if OS.get_environment("VETTURA_POST_FX") == "0":
		return false
	# 允许通过 user://settings.cfg 的 video/post_fx 关闭。
	var tree := get_tree()
	if tree != null and tree.root != null and tree.root.has_node("GameSettings"):
		var settings := tree.root.get_node("GameSettings")
		if settings.has_method("get_value"):
			return bool(settings.call("get_value", "video", "post_fx", true))
	return true


func _build() -> void:
	_layer = CanvasLayer.new()
	_layer.name = "PostFXLayer"
	# 3D 场景之后、普通 UI 之前。雨系统的 CanvasLayer 9/10 在这个之上。
	_layer.layer = -100
	add_child(_layer)

	_rect = ColorRect.new()
	_rect.name = "PostFXRect"
	_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_rect.color = Color(1.0, 1.0, 1.0, 1.0)
	_rect.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_material = ShaderMaterial.new()
	_material.shader = MOBILE_SHADER if _is_mobile else SHADER
	_rect.material = _material
	_layer.add_child(_rect)

	var viewport := get_viewport()
	if viewport != null:
		_viewport_size = viewport.get_visible_rect().size
		var tex := viewport.get_texture().get_size()
		_screen_size = tex if tex.x > 0.0 and tex.y > 0.0 else _viewport_size
		viewport.size_changed.connect(_on_viewport_size_changed)
	_push_viewport()


func _on_viewport_size_changed() -> void:
	var viewport := get_viewport()
	if viewport == null:
		return
	_viewport_size = viewport.get_visible_rect().size
	var tex := viewport.get_texture().get_size()
	if tex.x > 0.0 and tex.y > 0.0:
		_screen_size = tex
	_push_viewport()


func _push_viewport() -> void:
	if _material == null:
		return
	if _is_mobile:
		_material.set_shader_parameter(
			"aspect", _viewport_size.x / maxf(_viewport_size.y, 1.0))
	else:
		_material.set_shader_parameter("viewport_size", _screen_size)


## 全屏自动分辨率时使用的廉价后处理：保留暗角与锐化，去掉几乎看不见的
## 径向色散/桶形畸变，并把胶片颗粒限制在细粒度。少 2~3 次全屏纹理采样。
func set_performance_mode(enabled: bool) -> void:
	if _performance == enabled:
		return
	_performance = enabled
	_apply_preset(_preset)


func is_performance_mode() -> bool:
	return _performance


## 打开/关闭后处理（关闭时整层隐藏，零额外 draw）。
func set_enabled(enabled: bool) -> void:
	_enabled = enabled
	if _layer != null:
		_layer.visible = enabled


## 切换预设名（off / subtle / cinematic）。
func set_preset(preset: String) -> void:
	if not PRESETS.has(preset):
		push_warning("[post-fx] unknown preset '%s'" % preset)
		return
	_preset = preset
	_apply_preset(preset)


func _apply_preset(preset: String) -> void:
	if _material == null:
		return
	var values: Dictionary = PRESETS.get(preset, PRESETS["subtle"])
	if _is_mobile:
		# 移动端 shader 只有 strength/softness/aspect；把桌面 vignette 强度
		# 稍微收一点，配合更快的 FXAA/阴影降档，画面更稳。
		_material.set_shader_parameter(
			"strength", float(values["vignette_strength"]) * 0.8)
		_material.set_shader_parameter("softness", 0.72)
		_material.set_shader_parameter(
			"aspect", _viewport_size.x / maxf(_viewport_size.y, 1.0))
	else:
		for key in values.keys():
			_material.set_shader_parameter(key, values[key])
		if _performance:
			# 全屏自动分辨率：色散/桶形畸变在高 DPI 下几乎不可见，直接跳过
			# 对应的全屏采样分支；锐化略微提高，补偿 3D 上采样。
			_material.set_shader_parameter("chromatic_aberration", 0.0)
			_material.set_shader_parameter("lens_distortion", 0.0)
			_material.set_shader_parameter("film_grain",
				minf(float(values.get("film_grain", 0.012)), 0.006))
			_material.set_shader_parameter("sharpen",
				maxf(float(values.get("sharpen", 0.14)), 0.18))
		if _temporal_upscaler and preset != "off":
			# Temporal 上采样（MetalFX/FSR2）比 Spatial 更软；mipmap bias 拉回
			# 细节后，再补一点 unsharp，避免路面标线/远景发糊。上限 0.22，避免
			# 高对比护栏出现白边。
			var sharpen := float(values.get("sharpen", 0.14))
			if _performance:
				sharpen = maxf(sharpen, 0.18)
			_material.set_shader_parameter("sharpen", maxf(sharpen, 0.22))
	set_enabled(_enabled and preset != "off")


## 运行期微调单个参数；未知参数只警告，不打断渲染。
func set_parameters(values: Dictionary) -> void:
	if _material == null:
		return
	for key in values.keys():
		var name := String(key)
		if _is_mobile:
			if name == "vignette_strength":
				_material.set_shader_parameter("strength", values[key])
			elif name == "vignette_softness":
				_material.set_shader_parameter("softness", values[key])
			continue
		_material.set_shader_parameter(name, values[key])


func is_active() -> bool:
	return _layer != null and _layer.visible
