extends Node3D
## 上采样 × 多视角截图探针：真车 + 真赛道，显式指定上采样器，
## 等 AI 把车开起来后按视角各截一张 PNG，并打印 `DisplayPipeline` 实际状态。
##
## 用途：MetalFX / FSR 这类上采样在真实赛道画面上到底有没有生效、锐度如何，
## headless 只能验状态机，必须窗口模式回读 GPU 帧。默认跑 Porsche 992 GT3 R
## + 纽北 Nordschleife，车后 / 驾驶位各一张。
##
## ```bash
## # 默认 1280×720 窗口
## /Applications/Godot.app/Contents/MacOS/Godot --path . \
##     res://gdext/upscaler_shots.tscn
##
## # 全屏原生 1:1（2880×1800）+ MetalFX Spatial，最清晰
## SHOT_FULLSCREEN=1 SHOT_1TO1=1 SHOT_SCALE=1.0 SHOT_UPSCALER=metalfx_spatial \
##     /Applications/Godot.app/Contents/MacOS/Godot --path . \
##     res://gdext/upscaler_shots.tscn
## ```
##
## 环境变量：
## - `SHOT_CAR` / `SHOT_TRACK` / `SHOT_LAYOUT`：车、赛道、布局（支持裸模组名）；
## - `SHOT_UPSCALER`：auto/native/fsr1/fsr2/fsr3_1/metalfx_spatial/metalfx_temporal，
##   默认 `metalfx_temporal`；不可用时 `DisplayPipeline` 自动回退并告警；
## - `SHOT_SCALE`：3D 渲染比例（默认 0.8，时域上采样下限 0.5）；
## - `SHOT_VIEWS`：逗号分隔，默认 `chase,cockpit`（见 ai_lap 的视角名）；
## - `SHOT_WARMUP_S`：起步后等多久再截图（默认 10 s，让 AI 把车开到有速度）；
## - `SHOT_SETTLE_S`：切视角后等多久让时域上采样收敛（默认 1 s）；
## - `SHOT_FULLSCREEN`：1 = 全屏跑（默认 0，1280×720 窗口）；
## - `SHOT_1TO1`：1 = 全屏时把内容缩放基准改成窗口尺寸，3D 按 1:1 输出
##   （否则工程的 1280×720 stretch 基准会把根视口放大到窗口的 2.25 倍再缩回）；
## - `SHOT_MSAA`：0/2/4（默认 -1 = 沿用 RenderQuality；时域上采样强制 0）；
## - `SHOT_HIDE`：逗号分隔的节点名，截图前隐藏（诊断重复网格用）；
## - `SHOT_DIAG_HIDE`：逗号分隔的节点名，正常截图后再隐藏并补拍 `<view>_hidden.png`；
## - `SHOT_DIR`：PNG 输出目录（默认系统临时目录下的 vettura_shots/upscaler）。
##
## 输出：目录里 `chase.png` / `cockpit.png`（同名覆盖），控制台打印
## `[upscaler-shots] ...` 行；失败 `push_error` + 退出码 1。

const DEFAULT_CAR := "res://mods/trr_porsche_992_gt3r"
const DEFAULT_TRACK := "res://mods/fn_nurburgring"
const DEFAULT_LAYOUT := "nordschleife"
const DEFAULT_SHOT_DIR := "/private/var/folders/1n/jrfz32895pzdm63ht34jz0br0000gn/T/opencode/vettura_shots/upscaler"

## Porsche 992 GT3 R 的 DRIVEREYES + CG 修正，与 ai_lap.gd 的默认值一致。
const EYE_DEFAULT := Vector3(-0.367, 1.000, 0.157)
const CHASE_DISTANCE := 4.5
const CHASE_HEIGHT := 2.0
const CHASE_LOOKAHEAD := 14.0
const CHASE_FOV := 72.0
const COCKPIT_FOV := 70.0

## 这些是时域上采样器；开了 MSAA 会和重投影/历史缓冲冲突，显式关掉。
const TEMPORAL_UPSCALERS := ["fsr2", "fsr3_1", "metalfx_temporal"]

var _car_dir := DEFAULT_CAR
var _track_dir := DEFAULT_TRACK
var _track_layout := DEFAULT_LAYOUT
var _upscaler := "metalfx_temporal"
var _render_scale := 0.8
var _views: PackedStringArray = PackedStringArray(["chase", "cockpit"])
var _warmup_s := 10.0
var _settle_s := 1.0
var _fullscreen := false
var _one_to_one := false
var _msaa := -1
var _hide: PackedStringArray = PackedStringArray()
var _diag_hide: PackedStringArray = PackedStringArray()
var _shot_dir := DEFAULT_SHOT_DIR

var _world: WorldBody
var _car: VehicleBody
var _chase_cam: Camera3D
var _cockpit_cam: Camera3D

var _elapsed := 0.0
var _started := false
var _done := false
var _shot_saved := 0


func _ready() -> void:
	_car_dir = _res_path(_env_text("SHOT_CAR", DEFAULT_CAR))
	_track_dir = _res_path(_env_text("SHOT_TRACK", DEFAULT_TRACK))
	_track_layout = _env_text("SHOT_LAYOUT", DEFAULT_LAYOUT)
	_upscaler = _env_text("SHOT_UPSCALER", "metalfx_temporal")
	_render_scale = clampf(_env_float("SHOT_SCALE", 0.8), 0.1, 1.0)
	_warmup_s = maxf(0.0, _env_float("SHOT_WARMUP_S", 10.0))
	_settle_s = maxf(0.0, _env_float("SHOT_SETTLE_S", 1.0))
	_fullscreen = _env_float("SHOT_FULLSCREEN", 0.0) > 0.5
	_one_to_one = _env_float("SHOT_1TO1", 0.0) > 0.5
	_msaa = int(_env_float("SHOT_MSAA", -1.0))
	_hide = _split_names(_env_text("SHOT_HIDE", ""))
	_diag_hide = _split_names(_env_text("SHOT_DIAG_HIDE", ""))
	_shot_dir = _env_text("SHOT_DIR", DEFAULT_SHOT_DIR)
	var views_text := _env_text("SHOT_VIEWS", "chase,cockpit")
	_views = PackedStringArray()
	for entry in views_text.split(",", false):
		var view := entry.strip_edges().to_lower()
		if not view.is_empty():
			_views.append(view)
	if _views.is_empty():
		_views.append("chase")

	var made := DirAccess.make_dir_recursive_absolute(_shot_dir)
	if made != OK and made != ERR_ALREADY_EXISTS:
		push_error("[upscaler-shots] cannot create shot dir %s (err %d)" % [_shot_dir, made])
		get_tree().quit(1)
		return

	_world = WorldBody.new()
	_world.name = "WorldBody"
	add_child(_world)
	if not _world.load_track(_track_dir, _track_layout):
		push_error("[upscaler-shots] load_track failed: %s [%s]" % [_track_dir, _track_layout])
		get_tree().quit(1)
		return
	print("[upscaler-shots] track=%s layout=%s objects=%d"
			% [_world.track_root(), _world.track_layout(), _world.track_object_count()])

	var ac := AcCar.new()
	_car = null
	if ac.open(_car_dir):
		_car = _world.spawn_vehicle_on_grid(ac, 1)
	if _car == null:
		_car = _world.spawn_vehicle_from_path(_car_dir)
	if _car == null:
		push_error("[upscaler-shots] spawn failed: " + _car_dir)
		get_tree().quit(1)
		return
	_world.set_ai_skill(int(_env_float("SHOT_AI_SKILL", 100.0)))
	_world.set_ai_pace(_env_float("SHOT_AI_PACE", 1.0))
	if not _world.warm_up_vehicle(0):
		push_warning("[upscaler-shots] warm_up_vehicle failed, car will be cold")
	print("[upscaler-shots] car=%s wheels=%d vehicles=%d"
			% [_car_dir, _car.bound_wheels(), _world.vehicle_count()])

	_setup_visual()
	_build_cameras()
	_apply_hidden(_hide)
	_report_glass_nodes()
	_started = true
	_run()


## 与 ai_lap 的窗口模式一致：WeatherRig 负责天空/日光/湿路面，补一盏反向灯
## 避免车身背光面死黑。
func _setup_visual() -> void:
	var rig: Node = load("res://game/render/weather_rig.tscn").instantiate()
	add_child(rig)
	rig.call("set_weather", "dry")
	rig.call("debug_set_wetness", 0.0)
	_world.set_weather({
		"rain": 0.0, "wetness": 0.0, "fog": 0.0,
		"sun": 1.0, "wind": Vector3(2.0, 0.0, 1.0),
	})
	var fill := DirectionalLight3D.new()
	fill.rotation_degrees = Vector3(-18.0, 55.0, 0.0)
	fill.light_energy = 0.7
	fill.shadow_enabled = false
	add_child(fill)


func _build_cameras() -> void:
	_chase_cam = Camera3D.new()
	_chase_cam.name = "ChaseCam"
	_chase_cam.fov = CHASE_FOV
	_chase_cam.near = 0.05
	add_child(_chase_cam)
	_chase_cam.current = true
	_update_chase_cam()

	# 驾驶位相机挂成车身子对象：与车厢刚性，刹车/加速时不会在座椅里漂。
	_cockpit_cam = Camera3D.new()
	_cockpit_cam.name = "CockpitCam"
	_cockpit_cam.fov = COCKPIT_FOV
	_cockpit_cam.near = 0.05
	_cockpit_cam.position = EYE_DEFAULT
	_cockpit_cam.rotation_degrees = Vector3(-2.0, 0.0, 0.0)
	_car.add_child(_cockpit_cam)
	var cabin := OmniLight3D.new()
	cabin.light_energy = 8.0
	cabin.omni_range = 3.0
	cabin.shadow_enabled = false
	cabin.position = EYE_DEFAULT + Vector3(0.0, 0.35, -0.15)
	_car.add_child(cabin)


## 显式配置显示管线。必须晚于 `RenderQuality` 的 deferred `apply_profile()`：
## 否则会被用户设置里的 auto/native 覆盖回去。
func _configure_display() -> void:
	DisplayPipeline.refresh()
	DisplayPipeline.configure({
		"fullscreen": _fullscreen,
		"requested_upscaler": _upscaler,
		"vrs": "off",
		"render_scale": _render_scale,
		"dynamic": false,
		"target_fps": 60,
	})
	var active := DisplayPipeline.active_upscaler()
	if active in TEMPORAL_UPSCALERS:
		# 时域上采样与 MSAA 不兼容；以 DisplayPipeline 解析后的实际值为准。
		get_viewport().msaa_3d = Viewport.MSAA_DISABLED
	elif _msaa >= 0:
		get_viewport().msaa_3d = _msaa_value(_msaa)
	elif _fullscreen:
		# 与游戏全屏档位一致（RenderQuality 按 settings 里的 window_mode 判断，
		# 探针直接切全屏时它仍以为是窗口模式，会留 4x）。
		get_viewport().msaa_3d = Viewport.MSAA_2X
	var state := DisplayPipeline.state()
	var texture_size := get_viewport().get_texture().get_size()
	var scale := float(state.get("scale", 1.0))
	var internal := Vector2(texture_size) * scale
	print("[upscaler-shots] requested=%s active=%s mode=%d scale=%.2f bias=%.2f msaa=%d vrs=%s"
			% [_upscaler, active, int(state.get("upscaler_mode", -1)),
				scale, float(state.get("mipmap_bias", 0.0)),
				get_viewport().msaa_3d, String(state.get("vrs", "off"))])
	print("[upscaler-shots] window=%s viewport_tex=%s 3d_internal≈%dx%d content_scale=%s factor=%.2f mode=%d"
			% [str(DisplayServer.window_get_size()), str(texture_size),
				int(internal.x), int(internal.y),
				str(get_window().content_scale_size),
				get_window().content_scale_factor,
				get_window().content_scale_mode])
	var caps := DisplayPipeline.capabilities()
	print("[upscaler-shots] driver=%s renderer=%s available=%s"
			% [String(caps.get("driver", "")), String(caps.get("renderer", "")),
				",".join(_available_upscalers(caps))])


func _available_upscalers(caps: Dictionary) -> Array:
	var out: Array[String] = []
	var up: Dictionary = caps.get("upscalers", {})
	for id in up:
		if bool(up[id].get("available", false)):
			out.append(String(id))
	return out


func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("[upscaler-shots] headless 下拿不到渲染帧，请用窗口模式运行")
		get_tree().quit(1)
		return
	# 等两帧：autoload 全部 ready，RenderQuality 的 deferred apply_profile 也已执行。
	await get_tree().process_frame
	await get_tree().process_frame
	if _fullscreen:
		# 全屏后再配置显示管线，否则 scaling 会按窗口尺寸算，切全屏又变一次。
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)
		await get_tree().process_frame
		await get_tree().process_frame
		if _one_to_one:
			# `canvas_items` + 1280×720 基准在 2880×1800 全屏下会把根视口
			# 放大到 6480×4050 再缩回窗口（实测），白白多渲染 5 倍像素。
			# 探针没有 HUD，直接把内容缩放基准对齐窗口，3D 就是 1:1。
			get_window().content_scale_size = DisplayServer.window_get_size()
			await get_tree().process_frame
			await get_tree().process_frame
		var screen := DisplayServer.screen_get_size(DisplayServer.window_get_current_screen())
		print("[upscaler-shots] fullscreen window=%s screen=%s"
				% [str(DisplayServer.window_get_size()), str(screen)])
	_configure_display()

	await _wait(_warmup_s)
	for view in _views:
		_select_view(view)
		await _wait(_settle_s)
		await _capture(view)
		if not _diag_hide.is_empty():
			var changed := _apply_hidden(_diag_hide)
			await _wait(0.5)
			await _capture(view + "_hidden")
			for node in changed:
				node.visible = true
	_finish()


func _select_view(view: String) -> void:
	var cockpit := view in ["cockpit", "driver", "onboard"]
	var chase := view in ["chase", "hood", "bumper", "wheel", "drone", "tcam", "orbit"]
	if not cockpit and not chase:
		push_warning("[upscaler-shots] unknown view '%s', falling back to chase" % view)
		chase = true
	_chase_cam.current = chase
	_cockpit_cam.current = cockpit
	print("[upscaler-shots] view=%s camera=%s"
			% [view, "cockpit" if cockpit else "chase"])


func _capture(tag: String) -> void:
	await RenderingServer.frame_post_draw
	var texture := get_viewport().get_texture()
	var image := texture.get_image()
	if image == null:
		push_error("[upscaler-shots] viewport image is null for " + tag)
		return
	var path := _shot_dir.path_join(tag + ".png")
	var err := image.save_png(path)
	if err != OK:
		push_error("[upscaler-shots] save_png failed: %s (err=%d)" % [path, err])
		return
	_shot_saved += 1
	print("[upscaler-shots] shot %s -> %s tex=%s image=%dx%d speed=%.1f km/h fps=%.1f"
			% [tag, path, str(texture.get_size()), image.get_width(), image.get_height(),
				_car.speed_kmh(), Engine.get_frames_per_second()])


func _physics_process(delta: float) -> void:
	if not _started:
		return
	_elapsed += delta
	var input := _world.ai_controls(0)
	if input.size() >= 3:
		_car.set_controls(input[0], input[1], input[2], false, false, true, true)
	_update_chase_cam()


func _update_chase_cam() -> void:
	if _chase_cam == null or _car == null:
		return
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	_chase_cam.global_position = origin + basis.z * CHASE_DISTANCE + Vector3(0.0, CHASE_HEIGHT, 0.0)
	_chase_cam.look_at(origin - basis.z * CHASE_LOOKAHEAD + Vector3(0.0, 1.0, 0.0), Vector3.UP)


func _finish() -> void:
	if _done:
		return
	_done = true
	print("[upscaler-shots] finished shots=%d dir=%s active=%s"
			% [_shot_saved, _shot_dir, DisplayPipeline.active_upscaler()])
	if _shot_saved == 0:
		push_error("[upscaler-shots] no screenshots saved")
		get_tree().quit(1)
		return
	get_tree().quit(0)


func _apply_hidden(names: PackedStringArray) -> Array:
	var changed: Array = []
	for node_name in names:
		var node := _car.find_child(node_name, true, false) as Node3D
		if node == null:
			push_warning("[upscaler-shots] hide target not found: " + node_name)
			continue
		if node.visible:
			node.visible = false
			changed.append(node)
		print("[upscaler-shots] hidden %s (%s)" % [node_name, node.get_path()])
	return changed


## 打印几块玻璃网格的 AABB/材质，确认是否几何重叠（挡风玻璃双份）。
func _report_glass_nodes() -> void:
	for node_name in ["WINDOWS", "WIND_IN", "lights_lined_glass"]:
		var node := _car.find_child(node_name, true, false) as MeshInstance3D
		if node == null:
			continue
		var material := node.get_active_material(0)
		print("[upscaler-shots] glass node=%s visible=%s aabb=%s material=%s"
				% [node_name, str(node.is_visible_in_tree()), str(node.get_aabb()),
					material.resource_name if material != null else "<none>"])


func _split_names(text: String) -> PackedStringArray:
	var out := PackedStringArray()
	for entry in text.split(",", false):
		var item := entry.strip_edges()
		if not item.is_empty():
			out.append(item)
	return out


func _msaa_value(level: int) -> int:
	match level:
		0:
			return Viewport.MSAA_DISABLED
		2:
			return Viewport.MSAA_2X
		_:
			return Viewport.MSAA_4X


func _wait(seconds: float) -> void:
	if seconds <= 0.0:
		return
	await get_tree().create_timer(seconds).timeout


func _env_float(name: String, fallback: float) -> float:
	var text := OS.get_environment(name)
	return text.to_float() if not text.is_empty() else fallback


func _env_text(name: String, fallback: String) -> String:
	var text := OS.get_environment(name)
	return text if not text.is_empty() else fallback


func _res_path(path: String) -> String:
	if path.begins_with("res://"):
		return path
	var clean := path.trim_prefix("/")
	if clean.begins_with("mods/"):
		return "res://" + clean
	return "res://mods/" + clean
