extends Node
## 显示管线：上采样、动态分辨率、可变着色率（VRS）与 XR 的统一入口。
##
## Godot 4.7 自带 FSR 1/2、MetalFX Spatial/Temporal、VRS 与 XR 的 Viewport
## 属性；FSR 3.1 需要魔改引擎（见 `engine/README.md`）。本 autoload 把
## “平台/后端/硬件能力探测”和“设置值 → Viewport 属性”的映射集中到一处，
## `RenderQuality` 只负责环境/天空/后处理，不再直接改 `scaling_3d_*`。
##
## 设计取舍：
## - 能力探测只在启动/手动 `refresh()` 时做，不在每帧查 ClassDB；
## - FSR 3.1 优先认补丁新增的 `SCALING_3D_MODE_FSR3_1` 枚举，其次认
##   `engine/mods/*/manifest.json` 声明的桥接 singleton；
## - 不可用的上采样器不报错，按 `auto` 策略回退，并在日志里说明一次；
## - 时域上采样下限：FSR2/FSR3.1/MetalFX Temporal 统一 0.67（Apple MetalFX
##   示例的最小输入也是 720p 档），驱动给出更高下限时以驱动为准；
## - 移动端默认原生分辨率，画面缩放仍由 `RainSystem` 的画质档位决定。

signal scale_changed(scale: float, fps: float)
signal pipeline_changed(state: Dictionary)

const API_VERSION := 1

const UPSCALER_AUTO := "auto"
const UPSCALER_NATIVE := "native"
const UPSCALER_FSR1 := "fsr1"
const UPSCALER_FSR2 := "fsr2"
const UPSCALER_FSR31 := "fsr3_1"
const UPSCALER_METALFX_SPATIAL := "metalfx_spatial"
const UPSCALER_METALFX_TEMPORAL := "metalfx_temporal"

const UPSCALER_ORDER := [
	UPSCALER_AUTO,
	UPSCALER_NATIVE,
	UPSCALER_FSR1,
	UPSCALER_FSR2,
	UPSCALER_FSR31,
	UPSCALER_METALFX_SPATIAL,
	UPSCALER_METALFX_TEMPORAL,
]

const UPSCALER_LABELS := {
	"auto": "自动（推荐）",
	"native": "原生（双线性）",
	"fsr1": "AMD FSR 1.0",
	"fsr2": "AMD FSR 2.2",
	"fsr3_1": "AMD FSR 3.1（引擎扩展）",
	"metalfx_spatial": "MetalFX 空间",
	"metalfx_temporal": "MetalFX 时域",
}

const VRS_AUTO := "auto"
const VRS_OFF := "off"
const VRS_TEXTURE := "texture"
const VRS_XR := "xr"
const VRS_ORDER := [VRS_AUTO, VRS_OFF, VRS_TEXTURE, VRS_XR]
const VRS_LABELS := {
	"auto": "自动",
	"off": "关闭",
	"texture": "纹理（中心清晰，边缘降采样）",
	"xr": "XR 接口纹理",
}

const ENGINE_MOD_ROOTS := ["res://engine/mods", "user://engine/mods"]

## Apple MetalFX 示例推荐的纹理 LOD 偏置：
##   bias = log2(render_width / target_width) - 1
## 等价于 `log2(scaling_3d_scale) - 1`。Spatial 上采样不需要额外那一档，
## 用 `log2(scale)` 即可。运行时写 `Viewport.texture_mipmap_bias`，不把
## 一个固定值写死进 project.godot（窗口/全屏/动态分辨率都要变）。
const TEMPORAL_UPSCALERS := [UPSCALER_FSR2, UPSCALER_FSR31, UPSCALER_METALFX_TEMPORAL]
const SPATIAL_UPSCALERS := [UPSCALER_FSR1, UPSCALER_METALFX_SPATIAL]

## 动态分辨率采样频率：2 Hz 足够，避免帧率抖动导致反复改 scale。
const DYNAMIC_SAMPLE_SECONDS := 0.5
const SCALE_STEP := 0.05
const DOWN_AFTER_SECONDS := 1.0
const UP_AFTER_SECONDS := 3.0
const DOWN_COOLDOWN_SECONDS := 1.5
const UP_COOLDOWN_SECONDS := 3.0
const DEFAULT_MIN_SCALE := 0.35
const DEFAULT_MAX_SCALE := 1.0
## 时域上采样（MetalFX Temporal / FSR2 / FSR3.1）的输入下限。
## Apple MetalFX 示例把最小输入钳在 1280×720：1080p 时 = 0.67。0.5 的
## 低分辨率历史缓冲即使 bias 拉满也会明显发糊，所以这里不再下探。
const TEMPORAL_MIN_SCALE := 0.67

var _viewport: Viewport
var _caps: Dictionary = {}
var _engine_mods: Array = []

var _requested_upscaler := UPSCALER_AUTO
var _active_upscaler := UPSCALER_NATIVE
var _upscaler_mode := Viewport.SCALING_3D_MODE_BILINEAR
var _requested_vrs := VRS_AUTO
var _active_vrs := VRS_OFF

var _scale := 1.0
var _base_scale := 1.0
var _dynamic := false
var _mipmap_bias := 0.0
var _dynamic_min := DEFAULT_MIN_SCALE
var _dynamic_max := DEFAULT_MAX_SCALE
var _target_fps := 60.0
var _last_fps := 0.0
var _last_fullscreen := false
var _configured := false

var _timer := 0.0
var _low_time := 0.0
var _high_time := 0.0
var _cooldown := 0.0

var _vrs_texture: ImageTexture
var _has_vrs_property := false
var _engine_owns_mode := false
var _engine_bridge: Object = null
var _engine_bridge_methods: Dictionary = {}
var _warned: Dictionary = {}


func _ready() -> void:
	_viewport = get_viewport()
	_has_vrs_property = _object_has_property(_viewport, "vrs_mode")
	# XR 会话可能在启动后才接入：接口变化后重新探测，auto 才会切到 VRS_XR。
	if XRServer.has_signal("interface_added"):
		XRServer.interface_added.connect(_on_xr_interface_changed)
	if XRServer.has_signal("interface_removed"):
		XRServer.interface_removed.connect(_on_xr_interface_changed)
	set_process(true)
	refresh()


func _on_xr_interface_changed(_interface_name: String = "") -> void:
	# 接口加入的同一帧 primary_interface 可能还没更新，延后一帧再读。
	call_deferred("_refresh_xr_profile")


func _refresh_xr_profile() -> void:
	refresh()
	apply_profile()


## 重新探测能力、扫描引擎扩展。运行期热插拔 manifest 后可手动调用。
func refresh() -> void:
	_warned.clear()
	_scan_engine_mods()
	_caps = _probe_capabilities()
	pipeline_changed.emit(state())
	print("[display] platform=%s renderer=%s driver=%s upscalers=%s vrs=%s xr=%s mods=%d" % [
		String(_caps.get("platform", "")),
		String(_caps.get("renderer", "")),
		String(_caps.get("driver", "")),
		",".join(_available_upscaler_names()),
		str(_caps.get("vrs", {})),
		str(_caps.get("xr", false)),
		_engine_mods.size(),
	])


## 从 `GameSettings` 读取当前画质设置并应用。由 `RenderQuality` 统一调用。
func apply_profile() -> void:
	if _viewport == null:
		_viewport = get_viewport()
	var mobile := OS.has_feature("mobile")
	var fullscreen := GameSettings.is_fullscreen()
	configure({
		"fullscreen": fullscreen,
		"requested_upscaler": String(GameSettings.get_value("video", "upscaler", UPSCALER_AUTO)),
		"vrs": String(GameSettings.get_value("video", "vrs", VRS_AUTO)),
		"render_scale": clampf(float(GameSettings.get_value("video", "render_scale", 1.0)), 0.5, 1.0),
		"dynamic": bool(GameSettings.get_value("video", "fullscreen_adaptive", true)) \
			and fullscreen and not mobile,
		"target_fps": float(GameSettings.get_value("video", "dynamic_target_fps", 0)),
		# 移动端的 3D scale 由 RainSystem 的画质档位管理；这里只换模式/VRS。
		"apply_scale": not mobile,
	})


## 应用一份显式配置。`RenderQuality` 与冒烟测试都走这里。
func configure(options: Dictionary) -> void:
	var previous_fullscreen := _last_fullscreen
	var new_fullscreen := bool(options.get("fullscreen", previous_fullscreen))
	var new_dynamic := bool(options.get("dynamic", _dynamic))
	var dynamic_started := new_dynamic and not _dynamic
	var apply_scale := bool(options.get("apply_scale", true))

	_requested_upscaler = String(options.get("requested_upscaler", _requested_upscaler))
	_requested_vrs = String(options.get("vrs", _requested_vrs))
	_base_scale = clampf(float(options.get("render_scale", _base_scale)), 0.1, 1.0)
	_dynamic_min = clampf(float(options.get("min_scale", DEFAULT_MIN_SCALE)), 0.1, 1.0)
	_dynamic_max = clampf(float(options.get("max_scale", DEFAULT_MAX_SCALE)),
		_dynamic_min, DEFAULT_MAX_SCALE)
	_target_fps = _resolve_target_fps(float(options.get("target_fps", 0.0)))
	_dynamic = new_dynamic

	# 先决定上采样器，再按它决定动态分辨率下限。
	_apply_upscaler()
	_apply_vrs()

	if _dynamic and (dynamic_started or new_fullscreen != previous_fullscreen or not _configured):
		_scale = _initial_adaptive_scale()
		_low_time = 0.0
		_high_time = 0.0
		_cooldown = 1.0
		_timer = DYNAMIC_SAMPLE_SECONDS
	elif not _dynamic:
		_scale = _base_scale
		_low_time = 0.0
		_high_time = 0.0
		_timer = 0.0
	else:
		_scale = clampf(_scale, _effective_min_scale(), _dynamic_max)

	_last_fullscreen = new_fullscreen
	_configured = true
	if apply_scale:
		_apply_scale()
	scale_changed.emit(_scale, _last_fps)
	pipeline_changed.emit(state())


func _process(delta: float) -> void:
	if not _dynamic:
		return
	_cooldown = maxf(0.0, _cooldown - delta)
	_timer -= delta
	if _timer > 0.0:
		return
	_timer = DYNAMIC_SAMPLE_SECONDS
	var fps := Engine.get_frames_per_second()
	if fps <= 0.0:
		return
	_last_fps = fps
	scale_changed.emit(_scale, fps)
	_apply_dynamic_sample(fps)


func _apply_dynamic_sample(fps: float) -> void:
	var low_fps := _target_fps - maxf(2.0, _target_fps * 0.10)
	var high_fps := _target_fps - 1.0
	if fps < low_fps:
		_low_time += DYNAMIC_SAMPLE_SECONDS
		_high_time = 0.0
		if _low_time >= DOWN_AFTER_SECONDS and _cooldown <= 0.0:
			_set_scale(_scale - SCALE_STEP)
			_low_time = 0.0
			_cooldown = DOWN_COOLDOWN_SECONDS
	elif fps >= high_fps:
		_high_time += DYNAMIC_SAMPLE_SECONDS
		_low_time = 0.0
		if _high_time >= UP_AFTER_SECONDS and _cooldown <= 0.0:
			_set_scale(_scale + SCALE_STEP)
			_high_time = 0.0
			_cooldown = UP_COOLDOWN_SECONDS
	else:
		_low_time = 0.0
		_high_time = 0.0


## 仅测试/调试：用给定 FPS 驱动一次动态分辨率采样（默认忽略冷却）。
func debug_feed_fps(fps: float, ignore_cooldown: bool = true) -> float:
	if not _dynamic:
		return _scale
	if ignore_cooldown:
		_cooldown = 0.0
	_last_fps = fps
	_apply_dynamic_sample(fps)
	return _scale


func _set_scale(value: float) -> void:
	if _viewport == null:
		return
	var next := clampf(snappedf(value, SCALE_STEP), _effective_min_scale(), _dynamic_max)
	if absf(next - _scale) < 0.001:
		return
	_scale = next
	_apply_scale()
	scale_changed.emit(_scale, _last_fps)
	pipeline_changed.emit(state())


func _apply_scale() -> void:
	if _viewport == null:
		return
	var min_scale := _effective_min_scale()
	var max_scale := _dynamic_max if _dynamic else DEFAULT_MAX_SCALE
	_scale = clampf(_scale, min_scale, max_scale)
	_viewport.scaling_3d_scale = _scale
	_apply_mipmap_bias()


## 纹理 LOD 偏置：MetalFX/FSR 的输入是低分辨率，纹理仍按原分辨率 mip 采样会
## 发糊。Apple 的推荐公式（WWDC22 10103）：
##   temporal: log2(render_width / target_width) - 1
##   spatial : log2(render_width / target_width)
## 对 0.67 比例，temporal ≈ -1.58。这里按上采样器/动态比例实时更新。
func _apply_mipmap_bias() -> void:
	if _viewport == null:
		return
	var bias := 0.0
	var ratio := log(clampf(_scale, 0.01, 1.0)) / log(2.0)
	if _active_upscaler in TEMPORAL_UPSCALERS:
		bias = clampf(ratio - 1.0, -2.0, -0.5)
	elif _active_upscaler in SPATIAL_UPSCALERS:
		bias = clampf(ratio, -1.0, 0.0)
	_mipmap_bias = bias
	if _object_has_property(_viewport, "texture_mipmap_bias"):
		_viewport.set("texture_mipmap_bias", bias)
	else:
		_warn_once("mipmap_bias",
			"[display] Viewport 不支持 texture_mipmap_bias，MetalFX 细节会偏软")


func _initial_adaptive_scale() -> float:
	var window_width := maxf(float(DisplayServer.window_get_size().x), 1280.0)
	var texture_width := 0.0
	if _viewport != null and _viewport.get_texture() != null:
		texture_width = _viewport.get_texture().get_size().x
	var initial := 0.40
	if texture_width > 0.0 and texture_width >= window_width * 0.95:
		var base_width := maxf(texture_width, window_width)
		if base_width <= window_width * 1.05:
			initial = 1.0
		else:
			var target_width := minf(window_width * 0.90, 2880.0)
			initial = target_width / base_width
	return clampf(initial, _effective_min_scale(), _dynamic_max)


func _effective_min_scale() -> float:
	var base := _dynamic_min if _dynamic else 0.1
	return clampf(maxf(base, _min_scale_for(_active_upscaler)), 0.1, DEFAULT_MAX_SCALE)


func _min_scale_for(id: String) -> float:
	var up: Dictionary = _caps.get("upscalers", {})
	var configured := DEFAULT_MIN_SCALE
	if up.has(id):
		configured = clampf(float(up[id].get("min_scale", DEFAULT_MIN_SCALE)), 0.1, 1.0)
	if id in TEMPORAL_UPSCALERS:
		return maxf(configured, TEMPORAL_MIN_SCALE)
	return configured


func _resolve_target_fps(value: float) -> float:
	var refresh := DisplayServer.screen_get_refresh_rate()
	if refresh <= 1.0:
		refresh = 60.0
	refresh = clampf(refresh, 30.0, 240.0)
	if value < 0.0:
		return refresh
	if value == 0.0:
		# 默认不追 120/144 Hz 显示器，避免赛车场景为了高刷把分辨率压太低。
		return minf(refresh, 60.0)
	return clampf(value, 30.0, 240.0)


# ---------------------------------------------------------------- 上采样

func _apply_upscaler() -> void:
	var resolved := resolve_upscaler(_requested_upscaler)
	if _requested_upscaler != UPSCALER_AUTO and resolved != _requested_upscaler:
		var suffix := "；见 engine/README.md" if _requested_upscaler == UPSCALER_FSR31 else ""
		_warn_once("upscaler:" + _requested_upscaler,
			"[display] 上采样器 %s 当前不可用，回退到 %s%s" % [
				label_for(_requested_upscaler), label_for(resolved), suffix])
	if _active_upscaler == UPSCALER_FSR31 and resolved != UPSCALER_FSR31:
		_clear_engine_upscaler()
	_active_upscaler = resolved

	if resolved == UPSCALER_FSR31:
		if _apply_engine_upscaler():
			_upscaler_mode = _viewport_constant("SCALING_3D_MODE_FSR3_1")
			return
		_warn_once("upscaler:" + UPSCALER_FSR31,
			"[display] FSR 3.1 不可用，回退到自动策略；见 engine/README.md")
		_active_upscaler = _auto_upscaler([UPSCALER_FSR31])
		resolved = _active_upscaler

	var info: Dictionary = _caps.get("upscalers", {}).get(resolved, {})
	_upscaler_mode = int(info.get("mode", Viewport.SCALING_3D_MODE_BILINEAR))
	if _viewport != null:
		_viewport.scaling_3d_mode = _upscaler_mode


## 把“用户请求”解析成实际可用的上采样器。UI/冒烟测试可调用。
func resolve_upscaler(requested: String) -> String:
	if requested == UPSCALER_AUTO:
		return _auto_upscaler()
	if _upscaler_available(requested):
		return requested
	return _auto_upscaler()


func _auto_upscaler(exclude: Array = []) -> String:
	# 移动端默认保持原生；需要时用户在设置里显式选 FSR1/其余项。
	if OS.has_feature("mobile"):
		return UPSCALER_NATIVE
	if not exclude.has(UPSCALER_FSR31) and _upscaler_available(UPSCALER_FSR31):
		return UPSCALER_FSR31
	if _is_apple_metal():
		if _upscaler_available(UPSCALER_METALFX_SPATIAL):
			return UPSCALER_METALFX_SPATIAL
		if _upscaler_available(UPSCALER_METALFX_TEMPORAL):
			return UPSCALER_METALFX_TEMPORAL
	if _upscaler_available(UPSCALER_FSR1):
		return UPSCALER_FSR1
	return UPSCALER_NATIVE


func _upscaler_available(id: String) -> bool:
	if id == UPSCALER_AUTO:
		return true
	var up: Dictionary = _caps.get("upscalers", {})
	if not up.has(id):
		return false
	return bool(up[id].get("available", false))


## 补丁枚举或 manifest 桥接二选一；优先枚举，路径更短、跨平台更稳。
## 桥接路径下由 `display_apply` 接管整条 upscale 链，本函数不写
## `scaling_3d_mode`，避免和引擎侧的 FSR3.1 pass 双重放大。
func _apply_engine_upscaler() -> bool:
	var mode := _viewport_constant("SCALING_3D_MODE_FSR3_1")
	if mode >= 0:
		if _viewport != null:
			_viewport.scaling_3d_mode = mode
		_engine_owns_mode = false
		return true

	for mod: Dictionary in _engine_mods:
		if not bool(mod.get("bridge_available", false)):
			continue
		var provides: Array = mod.get("provides", [])
		if not (("upscaler." + UPSCALER_FSR31) in provides):
			continue
		var bridge: Object = mod.get("bridge_object", null)
		if !is_instance_valid(bridge):
			continue
		var methods: Dictionary = mod.get("bridge_methods", {})
		var available_method := String(methods.get("available", "display_available"))
		if bridge.has_method(available_method) and not bool(bridge.call(available_method, UPSCALER_FSR31)):
			continue
		var apply_method := String(methods.get("apply", "display_apply"))
		if not bridge.has_method(apply_method):
			continue
		var ok = bridge.call(apply_method, {
			"id": UPSCALER_FSR31,
			"scale": _scale,
			"target_fps": _target_fps,
			"viewport_rid": _viewport.get_viewport_rid() if _viewport != null else RID(),
		})
		if typeof(ok) == TYPE_BOOL and bool(ok):
			_engine_owns_mode = true
			_engine_bridge = bridge
			_engine_bridge_methods = methods
			return true
	return false


func _clear_engine_upscaler() -> void:
	if not _engine_owns_mode:
		return
	if is_instance_valid(_engine_bridge):
		var clear_method := String(_engine_bridge_methods.get("clear", "display_clear"))
		if _engine_bridge.has_method(clear_method):
			_engine_bridge.call(clear_method)
	_engine_owns_mode = false
	_engine_bridge = null
	_engine_bridge_methods = {}


# ---------------------------------------------------------------- VRS / XR

func _apply_vrs() -> void:
	if _viewport == null:
		return
	if not _has_vrs_property:
		_active_vrs = VRS_OFF
		return

	var requested := _requested_vrs
	if requested == VRS_AUTO:
		if bool(_caps.get("xr", false)):
			requested = VRS_XR
		elif bool(_caps.get("vrs", {}).get("texture", false)):
			requested = VRS_TEXTURE
		else:
			requested = VRS_OFF
	# 立体渲染必须用 XR 接口提供的 VRS 纹理（每眼一张），普通纹理不行。
	if requested == VRS_TEXTURE and bool(_caps.get("xr", false)):
		requested = VRS_XR
	if requested == VRS_XR and not bool(_caps.get("vrs", {}).get("xr", false)):
		requested = VRS_OFF
	if requested == VRS_TEXTURE and not bool(_caps.get("vrs", {}).get("texture", false)):
		requested = VRS_OFF

	match requested:
		VRS_TEXTURE:
			if _vrs_texture == null:
				_vrs_texture = _build_vrs_texture()
			_viewport.vrs_texture = _vrs_texture
			_viewport.vrs_update_mode = Viewport.VRS_UPDATE_ONCE
			_viewport.vrs_mode = Viewport.VRS_TEXTURE
		VRS_XR:
			_viewport.vrs_texture = null
			_viewport.vrs_update_mode = Viewport.VRS_UPDATE_ALWAYS
			_viewport.vrs_mode = Viewport.VRS_XR
		_:
			_viewport.vrs_mode = Viewport.VRS_DISABLED
			_viewport.vrs_texture = null
	_active_vrs = requested


## 生成一张 32×18 的中心清晰 VRS 密度图。
##
## 颜色按 Godot 文档的 VRS 密度映射：
## - 中心 1×1 = 黑；中间 2×2 = #555500；
## - 外围 4×4 = #aaaa00；上下边缘 2×4 = #55aa00。
## 纹理是小图，由引擎平铺到整个 viewport，显存/带宽可以忽略。
func _build_vrs_texture() -> ImageTexture:
	var width := 32
	var height := 18
	var image := Image.create_empty(width, height, false, Image.FORMAT_RGB8)
	var center := Color8(0, 0, 0)
	var middle := Color8(85, 85, 0)
	var outer := Color8(170, 170, 0)
	var band := Color8(85, 170, 0)
	for y in height:
		for x in width:
			var nx := (float(x) + 0.5) / float(width) * 2.0 - 1.0
			var ny := (float(y) + 0.5) / float(height) * 2.0 - 1.0
			var radius := sqrt(nx * nx + ny * ny)
			var color := outer
			if radius < 0.35:
				color = center
			elif radius < 0.70:
				color = middle
			elif absf(ny) > 0.75:
				color = band
			image.set_pixel(x, y, color)
	return ImageTexture.create_from_image(image)


func _warn_once(key: String, message: String) -> void:
	if _warned.has(key):
		return
	_warned[key] = true
	push_warning(message)


# ---------------------------------------------------------------- 能力探测

func _probe_capabilities() -> Dictionary:
	var renderer := RenderingServer.get_current_rendering_method()
	var driver := ""
	if RenderingServer.has_method("get_current_rendering_driver_name"):
		driver = RenderingServer.get_current_rendering_driver_name()
	var headless := DisplayServer.get_name() == "headless"
	var device: RenderingDevice = null
	if not headless:
		device = RenderingServer.get_rendering_device()
	var metal := driver == "metal"
	var apple_metal := OS.get_name() in ["macOS", "iOS"] and metal
	var metalfx_spatial := apple_metal \
		and _device_has_feature(device, "SUPPORTS_METALFX_SPATIAL")
	var metalfx_temporal := apple_metal \
		and _device_has_feature(device, "SUPPORTS_METALFX_TEMPORAL")
	var metalfx_temporal_min := TEMPORAL_MIN_SCALE
	if metalfx_temporal:
		# MetalFX 的最小 scale 由驱动给出（1e6 定点），不要拍脑袋写 0.5。
		var raw_min := _device_limit(device, "LIMIT_METALFX_TEMPORAL_SCALER_MIN_SCALE")
		if raw_min > 0:
			metalfx_temporal_min = maxf(
				TEMPORAL_MIN_SCALE,
				clampf(float(raw_min) / 1000000.0, 0.1, 1.0))
	var fsr31_mode := _viewport_constant("SCALING_3D_MODE_FSR3_1")
	var fsr31_bridge := _has_engine_mod_feature("upscaler." + UPSCALER_FSR31)
	var xr_active := XRServer.get_primary_interface() != null
	var vrs_texture := not headless and renderer in ["forward_plus", "mobile"]
	var vrs_xr := xr_active

	var upscalers := {
		UPSCALER_NATIVE: {
			"available": true,
			"mode": Viewport.SCALING_3D_MODE_BILINEAR,
			"min_scale": 0.1,
		},
		UPSCALER_FSR1: {
			"available": not headless and renderer != "gl_compatibility",
			"mode": Viewport.SCALING_3D_MODE_FSR,
			"min_scale": DEFAULT_MIN_SCALE,
		},
		UPSCALER_FSR2: {
			"available": not headless and renderer == "forward_plus",
			"mode": Viewport.SCALING_3D_MODE_FSR2,
			"min_scale": TEMPORAL_MIN_SCALE,
		},
		UPSCALER_FSR31: {
			"available": not headless and renderer != "gl_compatibility" \
				and (fsr31_mode >= 0 or fsr31_bridge),
			"mode": fsr31_mode,
			"min_scale": TEMPORAL_MIN_SCALE,
			"engine_mod": fsr31_bridge,
		},
		UPSCALER_METALFX_SPATIAL: {
			"available": metalfx_spatial,
			"mode": Viewport.SCALING_3D_MODE_METALFX_SPATIAL,
			"min_scale": DEFAULT_MIN_SCALE,
		},
		UPSCALER_METALFX_TEMPORAL: {
			"available": metalfx_temporal,
			"mode": Viewport.SCALING_3D_MODE_METALFX_TEMPORAL,
			"min_scale": metalfx_temporal_min,
		},
	}

	return {
		"platform": OS.get_name(),
		"renderer": renderer,
		"driver": driver,
		"headless": headless,
		"xr": xr_active,
		"metal": metal,
		"engine_version": str(Engine.get_version_info().get("string", "")),
		"upscalers": upscalers,
		"vrs": {
			"texture": vrs_texture,
			"xr": vrs_xr,
		},
		"engine_mods": _engine_mod_summary(),
	}


func _is_apple_metal() -> bool:
	if not (OS.get_name() in ["macOS", "iOS"]):
		return false
	if not RenderingServer.has_method("get_current_rendering_driver_name"):
		return false
	return RenderingServer.get_current_rendering_driver_name() == "metal"


func _device_has_feature(device: RenderingDevice, constant_name: String) -> bool:
	if device == null:
		return false
	var names := ClassDB.class_get_integer_constant_list("RenderingDevice")
	if not (constant_name in names):
		return false
	var feature := int(ClassDB.class_get_integer_constant("RenderingDevice", constant_name))
	return device.has_feature(feature)


func _device_limit(device: RenderingDevice, constant_name: String) -> int:
	if device == null:
		return -1
	var names := ClassDB.class_get_integer_constant_list("RenderingDevice")
	if not (constant_name in names):
		return -1
	var limit := int(ClassDB.class_get_integer_constant("RenderingDevice", constant_name))
	return device.limit_get(limit)


func _viewport_constant(constant_name: String) -> int:
	var names := ClassDB.class_get_integer_constant_list("Viewport")
	if not (constant_name in names):
		return -1
	return int(ClassDB.class_get_integer_constant("Viewport", constant_name))


func _object_has_property(object: Object, property: String) -> bool:
	if object == null:
		return false
	for entry in object.get_property_list():
		if String(entry.get("name", "")) == property:
			return true
	return false


func _available_upscaler_names() -> Array:
	var out: Array[String] = []
	var up: Dictionary = _caps.get("upscalers", {})
	for id in UPSCALER_ORDER:
		if id == UPSCALER_AUTO:
			continue
		if bool(up.get(id, {}).get("available", false)):
			out.append(String(id))
	return out


# ---------------------------------------------------------------- 引擎扩展 manifest

func _scan_engine_mods() -> void:
	_engine_mods.clear()
	for root_path in ENGINE_MOD_ROOTS:
		var root_dir := String(root_path)
		var dir := DirAccess.open(root_dir)
		if dir == null:
			continue
		dir.list_dir_begin()
		var entry := dir.get_next()
		while not entry.is_empty():
			if dir.current_is_dir() and not entry.begins_with("."):
				var manifest_path := root_dir.path_join(entry).path_join("manifest.json")
				var mod := _read_manifest(manifest_path)
				if not mod.is_empty():
					_engine_mods.append(mod)
			entry = dir.get_next()
		dir.list_dir_end()


func _read_manifest(path: String) -> Dictionary:
	if not FileAccess.file_exists(path):
		return {}
	var text := FileAccess.get_file_as_string(path)
	if text.is_empty():
		return {}
	var parsed = JSON.parse_string(text)
	if typeof(parsed) != TYPE_DICTIONARY:
		return {}
	var mod: Dictionary = parsed
	if int(mod.get("api", 0)) != API_VERSION:
		return {}
	if not bool(mod.get("enabled", false)):
		return {}
	if not (mod.get("provides", []) is Array):
		return {}

	var bridge_cfg: Dictionary = {}
	if mod.get("bridge", null) is Dictionary:
		bridge_cfg = mod["bridge"]
	var singleton := String(bridge_cfg.get("singleton", ""))
	var bridge_object: Object = null
	if not singleton.is_empty() and Engine.has_singleton(singleton):
		bridge_object = Engine.get_singleton(singleton)
	var methods: Dictionary = {}
	if bridge_cfg.get("methods", null) is Dictionary:
		methods = bridge_cfg["methods"]

	mod["id"] = String(mod.get("id", path.get_base_dir().get_file()))
	mod["manifest_path"] = path
	mod["bridge_object"] = bridge_object
	mod["bridge_available"] = bridge_object != null
	mod["bridge_methods"] = methods
	return mod


func _has_engine_mod_feature(feature: String) -> bool:
	for mod: Dictionary in _engine_mods:
		if not bool(mod.get("bridge_available", false)):
			continue
		var provides: Array = mod.get("provides", [])
		if feature in provides:
			return true
	return false


func _engine_mod_summary() -> Array:
	var out: Array = []
	for mod: Dictionary in _engine_mods:
		var provides: Array = []
		if mod.get("provides", null) is Array:
			provides = (mod["provides"] as Array).duplicate()
		out.append({
			"id": String(mod.get("id", "")),
			"name": String(mod.get("name", "")),
			"version": String(mod.get("version", "")),
			"provides": provides,
			"bridge_available": bool(mod.get("bridge_available", false)),
			"manifest_path": String(mod.get("manifest_path", "")),
		})
	return out


# ---------------------------------------------------------------- 对外只读状态

func capabilities() -> Dictionary:
	return _caps.duplicate(true)


func state() -> Dictionary:
	return {
		"requested_upscaler": _requested_upscaler,
		"active_upscaler": _active_upscaler,
		"upscaler_label": label_for(_active_upscaler),
		"upscaler_mode": _upscaler_mode,
		"scale": _scale,
		"base_scale": _base_scale,
		"mipmap_bias": _mipmap_bias,
		"dynamic": _dynamic,
		"dynamic_min": _effective_min_scale(),
		"dynamic_max": _dynamic_max,
		"target_fps": _target_fps,
		"vrs": _active_vrs,
		"xr": bool(_caps.get("xr", false)),
		"engine_mods": _engine_mods.size(),
	}


func report() -> Dictionary:
	return {
		"state": state(),
		"capabilities": capabilities(),
		"engine_mods": _engine_mod_summary(),
	}


func scale() -> float:
	return _scale


func is_dynamic() -> bool:
	return _dynamic


func active_upscaler() -> String:
	return _active_upscaler


func label_for(id: String) -> String:
	return String(UPSCALER_LABELS.get(id, id))


func upscaler_options() -> Array:
	var out: Array = []
	for id_value in UPSCALER_ORDER:
		var id := String(id_value)
		var available: bool = id == UPSCALER_AUTO or id == UPSCALER_NATIVE
		if not available:
			available = _upscaler_available(id)
		out.append({
			"id": id,
			"label": label_for(id),
			"available": available,
		})
	return out


func vrs_options() -> Array:
	var vrs: Dictionary = _caps.get("vrs", {})
	return [
		{"id": VRS_AUTO, "label": String(VRS_LABELS[VRS_AUTO]), "available": true},
		{"id": VRS_OFF, "label": String(VRS_LABELS[VRS_OFF]), "available": true},
		{"id": VRS_TEXTURE, "label": String(VRS_LABELS[VRS_TEXTURE]),
			"available": bool(vrs.get("texture", false))},
		{"id": VRS_XR, "label": String(VRS_LABELS[VRS_XR]),
			"available": bool(vrs.get("xr", false))},
	]


func set_upscaler(id: String) -> void:
	GameSettings.set_value("video", "upscaler", id)


func set_vrs(mode: String) -> void:
	GameSettings.set_value("video", "vrs", mode)


func set_dynamic(enabled: bool, target_fps: float = 0.0) -> void:
	GameSettings.set_value("video", "fullscreen_adaptive", enabled)
	GameSettings.set_value("video", "dynamic_target_fps", int(target_fps))
