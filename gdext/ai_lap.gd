extends Node3D
## AI 遥控整圈探针：验证「赛道 AI 线 → AiDriver → 物理 → 计时 → 遥测」全链路。
##
## 与 baseline.tscn 的差别：baseline 只验「车能不能被推着走 20 秒」，本场景
## 让 `WorldBody.ai_controls()`（fast_lane.ai 纯追踪控制器）遥控一台真车跑
## **完整一圈**，期间遥测（`user://telemetry.cfg`，UDP）与日志（`user://logs/`）
## 全程开着，供外部脚本抓帧分析。声效走 `FmodSystem`：按视角循环播放车模的
## engine_ext / engine_int，并把 rpm / 油门 / 车速 与 3D 听者每帧喂给 FMOD；
## 没编 `--features fmod`、没 bank 或声效缺失时只警告，不影响圈速验证。
##
## 用法（macOS；Godot 4.7）：
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --headless \
##     --path /path/to/vettura res://gdext/ai_lap.tscn
## ```
##
## 期望：输出 `[ai-lap] ...` 前缀的行，最后一行 `[ai-lap] PASS`，退出码 `0`。
## 任一步失败 `push_error` + `quit(1)`。
##
## 录视频（窗口模式 + 追逐相机 + 日光，跑完 AI_LAP_MAX_S 后退出）：
##
## ```bash
## AI_LAP_CAM=1 AI_LAP_REQUIRE_LAP=0 AI_LAP_MAX_S=25 \
## /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/ai_lap.tscn
## ```
##
## 抓整圈视频帧（配合 scripts/record_ai_lap.py；每 AI_LAP_CAPTURE_EVERY 帧存一张
## JPEG，跑完后由 ffmpeg 编码成 mp4）：
##
## ```bash
## AI_LAP_CAPTURE=1 AI_LAP_CAM=1 \
## /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/ai_lap.tscn --fixed-fps 60
## ```
##
## 环境变量：AI_LAP_MAX_S（圈时上限/最长跑动秒数）、AI_LAP_LAPS（目标圈数）、
## AI_LAP_CAM=1（窗口模式下加相机+日光）、AI_LAP_REQUIRE_LAP=0（短跑录视频
## 时不要求完成整圈）、AI_LAP_CAPTURE=1（抓帧）、AI_LAP_CAPTURE_EVERY（隔几帧
## 抓一次，默认 2 → 30fps 视频）、AI_LAP_CAPTURE_DIR（帧输出目录，默认
## user://ai_lap_frames）、AI_LAP_VIEW（视角，见下）、
## AI_LAP_CAR/AI_LAP_TRACK/AI_LAP_LAYOUT（车与赛道，支持裸模组名或 res:// 路径）、
## AI_LAP_SKILL（0..=110）、AI_LAP_PACE（配速倍率，默认 1.0）、
## AI_LAP_SPAWN_GRID（发车格序号，0=起跑线，默认 1）、
## AI_LAP_EYE_X/Y/Z（驾驶位相机偏移）、
## AI_LAP_AUDIO=0（关闭 FMOD 声效；默认开启，SDK/声效缺失时自动降级为警告）。
## AI_LAP_UPSCALER（auto/native/fsr1/fsr2/fsr3_1/metalfx_spatial/metalfx_temporal，
## 留空不改显示设置）、AI_LAP_UPSCALER_SCALE（3D 比例，默认 0.8）、
## AI_LAP_FULLSCREEN=1（全屏）、AI_LAP_WINDOW=1920x1080（窗口尺寸）、
## AI_LAP_1TO1=1（内容缩放基准对齐窗口，避免 1280×720 stretch 基准的超采样）。
##
## 视角（AI_LAP_VIEW）：
##   chase   车后追随（默认）
##   cockpit 驾驶位（座舱内，局部刚性）
##   hood    前机盖（引擎盖上方往前看，局部刚性）
##   bumper  前保险杠低机位（贴地车头视角，局部刚性）
##   wheel   轮胎特写（看单侧后轮/前轮，局部刚性；AI_LAP_WHEEL=lf/rf/lr/rr）
##   drone   无人机追踪（高空滞后跟随，带惯性平滑）
##   tcam    定点摄像机追踪（赛道边固定机位，车靠近时切下一个机位）
##   orbit   环绕跟拍（绕车慢速旋转的电影镜头）
## 别名：bonnet→hood、tyre/tire→wheel、tv/fixed/trackside→tcam、
## onboard/driver→cockpit。未知视角会警告并回退 chase。
##
## 各视角镜头参数也可用环境变量覆盖：AI_LAP_HOOD_X/Y/Z + AI_LAP_HOOD_FOV、
## AI_LAP_BUMPER_X/Y/Z + AI_LAP_BUMPER_FOV、AI_LAP_WHEEL_FOV、
## AI_LAP_DRONE_DISTANCE/_HEIGHT/_FOV、AI_LAP_TCAM_DISTANCE/_SIDE/_HEIGHT/
## _RANGE/_FOV、AI_LAP_ORBIT_RADIUS/_HEIGHT/_SPEED/_FOV。

const CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const TRACK_DIR := "res://mods/daytona_2017"
const TRACK_LAYOUT := "roadcourse"

## 驾驶位相机的默认眼睛位置。AC 的 `DRIVEREYES` 相对车辆 CG、+X 为左、+Z 为前；
## 物理帧 X/Z 取反、Y 要加 CG 高度。默认按 Porsche 992 GT3 R：
## DRIVEREYES=(0.367, 0.910, -0.157)，CG=0.320（前轮半径 0.340 + BASEY -0.020）。
## Y 在 DRIVEREYES+CG（1.23）之上再抬 0.22 m 越过车头。换车时可用
## AI_LAP_EYE_X/Y/Z 覆盖。
const EYE_X_DEFAULT := -0.367
const EYE_Y_DEFAULT := 1.000
const EYE_Z_DEFAULT := 0.157

## 前机盖 / 保险杠镜头的后备局部锚点（车体坐标系：+X 右、+Y 上、-Z 前）。
## 正常会由 `_auto_fit_body_cams()` 按车模包围盒自动适配；这里只是拿不到
## 网格包围盒时的兜底。显式设了 AI_LAP_HOOD_X/Y/Z 或 AI_LAP_BUMPER_X/Y/Z
## 时以环境变量为准。
const HOOD_DEFAULT := Vector3(0.0, 0.72, -0.75)
const BUMPER_DEFAULT := Vector3(0.0, 0.34, -1.60)

## 支持的全部视角。别名在 `_normalize_view()` 里折叠到这些名字。
const VIEWS := ["chase", "cockpit", "hood", "bumper", "wheel", "drone", "tcam", "orbit"]
const VIEW_ALIASES := {
	"bonnet": "hood",
	"tyre": "wheel",
	"tire": "wheel",
	"tv": "tcam",
	"fixed": "tcam",
	"trackside": "tcam",
	"onboard": "cockpit",
	"driver": "cockpit",
}
## 这些视角的相机是车身子对象：位姿随车身刚性移动，不需要每帧重算世界位姿。
const RIGID_VIEWS := ["cockpit", "hood", "bumper", "wheel"]

## 时域上采样与 MSAA 不兼容；配置显示管线时显式关掉 3D MSAA。
const TEMPORAL_UPSCALERS := ["fsr2", "fsr3_1", "metalfx_temporal"]

## 圈时上限（秒）。Daytona roadcourse ≈ 5.7 km，AI 目标速度受 fast_lane.ai
## 调速；正常 F1 圈时 100–150 s。超过这个值判定 AI 没能完成圈。
## 可用环境变量 AI_LAP_MAX_S 覆盖，缩短诊断迭代。
const MAX_LAP_S_DEFAULT := 300.0

## 完成几圈收工（默认一圈；可用 AI_LAP_LAPS 覆盖）。
const TARGET_LAPS_DEFAULT := 1
const TARGET_LAPS := 1

## AI 难度：AC 的 AI level，0..=110（100 = 职业，默认）。
const DEFAULT_AI_SKILL := 100

## FMOD 声效事件：AC 车模通用命名，驾驶位用车内声，其余视角用车外声。
const ENGINE_EXT := "engine_ext"
const ENGINE_INT := "engine_int"

## 全程 ABS / TC 开：验的是「链路」，不是物理边界。
const ABS_ON := true
const TC_ON := true

var _world: WorldBody
var _car: VehicleBody
var _elapsed := 0.0
var _max_lap_s := 300.0
var _target_laps := 1
var _start: Vector3
var _last_logged_s := -1.0
var _reported_laps := 0
var _ai_alive := false
var _max_speed_kmh := 0.0
var _last_input := [0.0, 0.0, 0.0]

## FMOD 声效：start() + load_car_path() 都成功才真正播放。
var _audio: FmodSystem
var _audio_ready := false
var _engine_event := ""

## 最长连续 0 轮接触时长（秒）。F1 高速过 crest 会短暂腾空（实测 150 km/h
## 腾空 ~2–3 s 属正常），所以不能用「瞬时 min contacts」判丢赛道；真丢赛道
## （掉出碰撞网、卡进地形）会连续 0 接触很久，用连续时长判更准。
const MAX_AIRBORNE_S := 4.0
var _airborne_time := 0.0
var _max_airborne_s := 0.0

## 卡死自救：连续 STUCK_S 秒速度 < STUCK_V m/s 就 reset 一次，上限 MAX_RESCUES 次。
## 防 AI 卡死在护栏/草地里时探针永远跑不到时间上限。
const STUCK_V_MPS := 0.5
const STUCK_S := 5.0
const MAX_RESCUES := 3
var _stuck_time := 0.0
var _rescues := 0
var _last_reset_s := -10.0

## 窗口模式视觉：AI_LAP_CAM=1 且非 headless 时加追逐相机 + 日光 + 天空环境。
var _cam: Camera3D
var _require_lap := true
## AI_LAP_VIEW 折叠后的视角名（见 VIEWS），默认 "chase"。
var _view := "chase"
var _eye_offset := Vector3(-0.367, 1.000, 0.157)
var _hood_local := HOOD_DEFAULT
var _bumper_local := BUMPER_DEFAULT

## 相机参数（环境变量可覆盖，默认与旧行为一致）：
##   AI_LAP_CHASE_DISTANCE / _HEIGHT / _LOOKAHEAD / _FOV  车后镜头
##   AI_LAP_COCKPIT_FOV                                  驾驶位镜头
##   AI_LAP_HOOD_FOV / AI_LAP_BUMPER_FOV                 机盖 / 保险杠镜头
##   AI_LAP_WHEEL_FOV + AI_LAP_WHEEL                     轮胎镜头
##   AI_LAP_DRONE_* / AI_LAP_TCAM_* / AI_LAP_ORBIT_*     无人机 / 定点 / 环绕
var _chase_distance := 4.5
var _chase_height := 2.0
var _chase_lookahead := 14.0
var _chase_fov := 72.0
var _cockpit_fov := 70.0
var _hood_fov := 78.0
var _bumper_fov := 84.0
var _wheel_fov := 62.0
var _wheel_key := "rr"
var _drone_distance := 7.0
var _drone_height := 4.0
var _drone_fov := 78.0
var _drone_lag := 5.0
var _tcam_distance := 45.0
var _tcam_side := 14.0
var _tcam_height := 5.0
var _tcam_range := 90.0
var _tcam_fov := 45.0
var _orbit_radius := 7.5
var _orbit_height := 2.4
var _orbit_speed := 0.45
var _orbit_fov := 70.0

## 动态镜头状态：无人机平滑位置、定点机位与环绕角。
var _drone_position := Vector3.ZERO
var _drone_ready := false
var _tcam_position := Vector3.ZERO
var _tcam_side_sign := 1.0
var _tcam_armed := false
var _orbit_angle := 0.0
## 轮胎镜头：车轮 dummy 在车体坐标系里的位置 + 相机相对它的偏移。
var _wheel_local := Vector3.ZERO
var _wheel_offset := Vector3.ZERO

## 可由环境变量指定的资源路径 / AI 难度。
var _car_dir := CAR_DIR
var _track_dir := TRACK_DIR
var _track_layout := TRACK_LAYOUT

## 可选上采样配置（AI_LAP_UPSCALER 为空时完全不动全局显示状态）。
var _upscaler := ""
var _upscaler_scale := 0.8
var _upscaler_fullscreen := false
var _upscaler_1to1 := false
var _upscaler_window := Vector2i.ZERO

## 录视频抓帧：AI_LAP_CAPTURE=1 时每个渲染帧存一张 JPEG，由外部 ffmpeg 编码。
var _capture_dir := ""
var _capture_every := 2
var _capture_frame := 0
var _capture_saved := 0

## JPEG 编码/写盘放后台线程，主线程只做 GPU 回读。否则 1080p 下每 2 帧一次
## 同步 save_jpg 会卡住渲染循环，窗口看起来"停住"。
const CAPTURE_QUEUE_MAX := 300
var _capture_thread: Thread
var _capture_mutex := Mutex.new()
var _capture_sem := Semaphore.new()
var _capture_queue: Array = []
var _capture_written := 0
var _capture_errors := 0
var _capture_backlog_warned := false

func _ready() -> void:
	_max_lap_s = _env_float("AI_LAP_MAX_S", MAX_LAP_S_DEFAULT)
	_target_laps = int(_env_float("AI_LAP_LAPS", TARGET_LAPS_DEFAULT))
	_require_lap = _env_float("AI_LAP_REQUIRE_LAP", 1.0) > 0.5
	_car_dir = _res_path(_env_text("AI_LAP_CAR", CAR_DIR))
	_track_dir = _res_path(_env_text("AI_LAP_TRACK", TRACK_DIR))
	_track_layout = _env_text("AI_LAP_LAYOUT", TRACK_LAYOUT)
	_upscaler = _env_text("AI_LAP_UPSCALER", "")
	_upscaler_scale = clampf(_env_float("AI_LAP_UPSCALER_SCALE", 0.8), 0.1, 1.0)
	_upscaler_fullscreen = _env_float("AI_LAP_FULLSCREEN", 0.0) > 0.5
	_upscaler_1to1 = _env_float("AI_LAP_1TO1", 0.0) > 0.5
	_upscaler_window = _parse_size(_env_text("AI_LAP_WINDOW", ""))
	_view = _normalize_view(_env_text("AI_LAP_VIEW", "chase"))
	_eye_offset = Vector3(
		_env_float("AI_LAP_EYE_X", EYE_X_DEFAULT),
		_env_float("AI_LAP_EYE_Y", EYE_Y_DEFAULT),
		_env_float("AI_LAP_EYE_Z", EYE_Z_DEFAULT)
	)
	_hood_local = Vector3(
		_env_float("AI_LAP_HOOD_X", HOOD_DEFAULT.x),
		_env_float("AI_LAP_HOOD_Y", HOOD_DEFAULT.y),
		_env_float("AI_LAP_HOOD_Z", HOOD_DEFAULT.z)
	)
	_bumper_local = Vector3(
		_env_float("AI_LAP_BUMPER_X", BUMPER_DEFAULT.x),
		_env_float("AI_LAP_BUMPER_Y", BUMPER_DEFAULT.y),
		_env_float("AI_LAP_BUMPER_Z", BUMPER_DEFAULT.z)
	)
	_chase_distance = _env_float("AI_LAP_CHASE_DISTANCE", _chase_distance)
	_chase_height = _env_float("AI_LAP_CHASE_HEIGHT", _chase_height)
	_chase_lookahead = _env_float("AI_LAP_CHASE_LOOKAHEAD", _chase_lookahead)
	_chase_fov = _env_float("AI_LAP_CHASE_FOV", _chase_fov)
	_cockpit_fov = _env_float("AI_LAP_COCKPIT_FOV", _cockpit_fov)
	_hood_fov = _env_float("AI_LAP_HOOD_FOV", _hood_fov)
	_bumper_fov = _env_float("AI_LAP_BUMPER_FOV", _bumper_fov)
	_wheel_fov = _env_float("AI_LAP_WHEEL_FOV", _wheel_fov)
	_wheel_key = _env_text("AI_LAP_WHEEL", _wheel_key).to_lower()
	_drone_distance = _env_float("AI_LAP_DRONE_DISTANCE", _drone_distance)
	_drone_height = _env_float("AI_LAP_DRONE_HEIGHT", _drone_height)
	_drone_fov = _env_float("AI_LAP_DRONE_FOV", _drone_fov)
	_drone_lag = _env_float("AI_LAP_DRONE_LAG", _drone_lag)
	_tcam_distance = _env_float("AI_LAP_TCAM_DISTANCE", _tcam_distance)
	_tcam_side = _env_float("AI_LAP_TCAM_SIDE", _tcam_side)
	_tcam_height = _env_float("AI_LAP_TCAM_HEIGHT", _tcam_height)
	_tcam_range = _env_float("AI_LAP_TCAM_RANGE", _tcam_range)
	_tcam_fov = _env_float("AI_LAP_TCAM_FOV", _tcam_fov)
	_orbit_radius = _env_float("AI_LAP_ORBIT_RADIUS", _orbit_radius)
	_orbit_height = _env_float("AI_LAP_ORBIT_HEIGHT", _orbit_height)
	_orbit_speed = _env_float("AI_LAP_ORBIT_SPEED", _orbit_speed)
	_orbit_fov = _env_float("AI_LAP_ORBIT_FOV", _orbit_fov)

	_world = WorldBody.new()
	add_child(_world)

	if not _world.load_track(_track_dir, _track_layout):
		push_error("[ai-lap] load_track failed")
		get_tree().quit(1)
		return
	print("[ai-lap] track=", _world.track_root(), " layout=", _world.track_layout(),
			" objects=", _world.track_object_count(),
			" sectors=", _world.sector_count())
	print("[ai-lap] telemetry_enabled=", _world.is_telemetry_enabled())

	# AI_LAP_SPAWN_GRID：在发车格生成（0 = 起跑线，1/2… 依次向后 8 m/16 m）。
	# 默认 1：出生点贴在起跑线上时，首次越线要等整圈之后，第一圈不计时；
	# 在线后 8 m 出生可以让计时在起步后立刻开始。设为 -1 回到起跑线。
	var grid_index := int(_env_float("AI_LAP_SPAWN_GRID", 1.0))
	_car = null
	if grid_index >= 0:
		var ac_car := AcCar.new()
		if ac_car.open(_car_dir):
			_car = _world.spawn_vehicle_on_grid(ac_car, grid_index)
	if _car == null:
		_car = _world.spawn_vehicle_from_path(_car_dir)
	if _car == null:
		push_error("[ai-lap] spawn failed: " + _car_dir)
		get_tree().quit(1)
		return
	_world.set_ai_skill(int(_env_float("AI_LAP_SKILL", DEFAULT_AI_SKILL)))
	# 配速倍率：110 档自带约 1.65，这里再乘外部倍率（1.0 = 不改动）。
	_world.set_ai_pace(_env_float("AI_LAP_PACE", 1.0))
	print("[ai-lap] car=", _car_dir, " ai_skill=", _world.ai_skill(),
			" ai_pace=", _world.ai_pace())
	# 比赛温度：AI 代表的是「已经暖过胎的职业车手」，不是冷车起步。
	# 不暖胎时实测圈速慢 20%+（冷机 ECU 限扭到 55% 功率、冷胎掉一档抓地，
	# 变速箱还会因为冷胎高滑移拒绝升挡）。
	if not _world.warm_up_vehicle(0):
		push_warning("[ai-lap] warm_up_vehicle failed, lap will be cold")
	print("[ai-lap] spawned=", _car.name, " wheels=", _car.bound_wheels(),
			" vehicles=", _world.vehicle_count())
	_start = _car.position
	_setup_audio()

	if _env_float("AI_LAP_CAM", 0.0) > 0.5 and DisplayServer.get_name() != "headless":
		_setup_visual()

	if not _upscaler.is_empty():
		if DisplayServer.get_name() == "headless":
			push_warning("[ai-lap] headless 下无法使用上采样器 %s，保持默认" % _upscaler)
		else:
			_apply_upscaler_settings()

	if _env_float("AI_LAP_CAPTURE", 0.0) > 0.5:
		_capture_dir = OS.get_environment("AI_LAP_CAPTURE_DIR")
		if _capture_dir.is_empty():
			_capture_dir = "user://ai_lap_frames"
		_capture_every = maxi(1, int(_env_float("AI_LAP_CAPTURE_EVERY", 2.0)))
		# 清掉上一批帧，避免新旧混在一起。
		var absolute := ProjectSettings.globalize_path(_capture_dir)
		var dir := DirAccess.open(absolute)
		if dir == null:
			var made := DirAccess.make_dir_recursive_absolute(absolute)
			if made != OK:
				push_error("[ai-lap] cannot create capture dir %s (err %d)" % [absolute, made])
				_capture_dir = ""
		else:
			for file in dir.get_files():
				if file.ends_with(".jpg"):
					dir.remove(file)
		if not _capture_dir.is_empty():
			_capture_thread = Thread.new()
			_capture_thread.start(_capture_worker)
			print("[ai-lap] capture frames -> %s every %d frame (async writer)"
					% [absolute, _capture_every])

## 可选：给整圈运行显式指定上采样器。必须晚于 `RenderQuality` 的 deferred
## `apply_profile()`，否则会被用户设置里的 auto/native 覆盖回去。
func _apply_upscaler_settings() -> void:
	await get_tree().process_frame
	await get_tree().process_frame
	if _upscaler_fullscreen:
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)
		await get_tree().process_frame
		await get_tree().process_frame
	if _upscaler_window.x > 0 and _upscaler_window.y > 0:
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
		DisplayServer.window_set_size(_upscaler_window)
		await get_tree().process_frame
		await get_tree().process_frame
	if _upscaler_1to1:
		get_window().content_scale_size = DisplayServer.window_get_size()
		await get_tree().process_frame
		await get_tree().process_frame
	DisplayPipeline.refresh()
	DisplayPipeline.configure({
		"fullscreen": _upscaler_fullscreen,
		"requested_upscaler": _upscaler,
		"vrs": "off",
		"render_scale": _upscaler_scale,
		"dynamic": false,
		"target_fps": 60,
	})
	var active := DisplayPipeline.active_upscaler()
	if active in TEMPORAL_UPSCALERS:
		get_viewport().msaa_3d = Viewport.MSAA_DISABLED
	elif _upscaler_fullscreen:
		get_viewport().msaa_3d = Viewport.MSAA_2X
	var state := DisplayPipeline.state()
	var texture_size := get_viewport().get_texture().get_size()
	print("[ai-lap] display requested=%s active=%s mode=%d scale=%.2f bias=%.2f msaa=%d window=%s viewport=%s"
			% [_upscaler, active, int(state.get("upscaler_mode", -1)),
				float(state.get("scale", 0.0)), float(state.get("mipmap_bias", 0.0)),
				get_viewport().msaa_3d,
				str(DisplayServer.window_get_size()), str(texture_size)])


## 赛道与车模都不自带灯光：窗口模式下补一盏日光即可。
## 天空与环境光走工程默认环境（`game_environment.tres` +
## project.godot 的 `environment/defaults/default_environment`）——以前这里自己
## 用代码建 `AMBIENT_SOURCE_SKY` 的环境，实测不产生任何环境光，太阳照不到的
## 地方（看台内壁、车身底面）全是一片死黑。
##
## 录视频用两盏灯：主光（带阴影）+ 反向补光。单盏日光下车身背光的一面、
## 看台内壁、轮胎全在阴影里，录出来的画面一半是黑的；补光只影响这个探针的
## 窗口模式，不动工程默认环境。
func _setup_visual() -> void:
	# 天气视觉：统一由 WeatherRig 驱动（单 wetness、8s 渐变、湿路面 shader、
	# ReflectionProbe/灯光）。AI_LAP_WEATHER 映射到 WeatherRig 的 preset。
	var rig: Node = load("res://game/render/weather_rig.tscn").instantiate()
	add_child(rig)
	var weather_preset: String = OS.get_environment("AI_LAP_WEATHER").to_lower()
	var want_rain := weather_preset == "rain" or weather_preset == "storm"
	if want_rain:
		rig.call("set_weather", "storm")
		# 录视频要整段都是雨天，不等 8s 过渡。
		rig.call("debug_set_wetness", 1.0)
		# 界面之外，物理天气也要同步：否则视频里下雨但轮胎仍是干地抓地。
		_world.set_weather({
			"rain": 0.85, "wetness": 0.90, "fog": 0.25,
			"sun": 0.35, "wind": Vector3(4.0, 0.0, 2.0),
		})
		var rain_scene: PackedScene = load("res://game/fx/rain/rain_system.tscn")
		if rain_scene != null:
			var rain := rain_scene.instantiate()
			rain.name = "RainSystem"
			add_child(rain)
	elif weather_preset == "overcast":
		rig.call("set_weather", "drizzle")
	elif weather_preset == "fog":
		rig.call("set_weather", "storm")
		rig.call("debug_set_wetness", 0.65)
	else:
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

	_cam = Camera3D.new()
	_cam.current = true
	# 各视角用的镜头焦段不同：座舱/机盖偏窄，保险杠/无人机偏广。
	_cam.fov = _view_fov()
	_cam.near = 0.05
	_auto_fit_body_cams()
	if _view in RIGID_VIEWS and _car != null:
		# 驾驶位/机盖/保险杠/轮胎相机挂成车身节点的子对象，位姿固定：这样它和
		# 车厢完全刚性，加速/刹车俯仰时不会相对座椅前后漂移，也不会穿出车顶。
		# 相机默认朝局部 -Z（车头），无需每帧重算世界位姿。
		_car.add_child(_cam)
		match _view:
			"cockpit":
				_cam.position = _eye_offset
				_cam.rotation_degrees = Vector3(-2.0, 0.0, 0.0)
				# 座舱里日光被车顶挡住，仪表/方向盘全黑。给车节点挂一盏短程点光源，
				# 只照亮驾驶位附近的内饰（范围 3 m），窗外路面不受影响。
				var cabin := OmniLight3D.new()
				cabin.light_energy = 8.0
				cabin.omni_range = 3.0
				cabin.shadow_enabled = false
				cabin.position = _eye_offset + Vector3(0.0, 0.35, -0.15)
				_car.add_child(cabin)
			"hood":
				_cam.position = _hood_local
				_cam.rotation_degrees = Vector3(-1.5, 0.0, 0.0)
			"bumper":
				_cam.position = _bumper_local
				_cam.rotation_degrees = Vector3.ZERO
			"wheel":
				_wheel_local = _find_wheel_local()
				var side := signf(_wheel_local.x)
				if is_zero_approx(side):
					side = 1.0
				# 车外、略靠后上方：既看得到轮胎/悬挂，也带一点路面与车身侧面。
				_wheel_offset = Vector3(side * 0.62, 0.30, 0.85)
				_update_wheel_cam()
	else:
		add_child(_cam)
	# 立刻摆好第一帧机位，避免开局一帧相机还在原点。
	_update_camera(0.0)
	print("[ai-lap] visual mode: %s cam + WeatherRig (%s)" % [_view, weather_preset])

## FMOD 声效：加载当前车的 bank 并起引擎循环声。
##
## 这一层是"有声音更好"而不是探针的通过条件：没编 `--features fmod`、没 SDK、
## 车没 sfx/ 或 bank 不兼容时都只打 warning，圈速 / 遥测链路照常验证。
func _setup_audio() -> void:
	if _env_float("AI_LAP_AUDIO", 1.0) < 0.5:
		return
	if not ClassDB.class_exists("FmodSystem"):
		push_warning("[ai-lap] FmodSystem 未注册，本圈无声")
		return

	_audio = FmodSystem.new()
	add_child(_audio)
	if not _audio.start(64):
		push_warning("[ai-lap] FMOD 未启动（%s），本圈无声" % _audio.last_error())
		return
	if not _audio.load_car_path(_car_dir):
		push_warning("[ai-lap] 声效 bank 加载失败（%s），本圈无声" % _audio.last_error())
		return

	# 驾驶位用车内声，其余视角用车外声；某个事件缺失时退回另一个。
	var events := _audio.event_names()
	if _view == "cockpit" and events.has(ENGINE_INT):
		_engine_event = ENGINE_INT
	else:
		_engine_event = ENGINE_EXT
	if not events.has(_engine_event):
		push_warning("[ai-lap] 车模没有 %s / %s，本圈无声" % [ENGINE_EXT, ENGINE_INT])
		return
	if not _audio.play(_engine_event):
		push_warning("[ai-lap] 播放 %s 失败（%s），本圈无声" % [_engine_event, _audio.last_error()])
		return

	_audio_ready = true
	print("[ai-lap] audio on: event=%s bank=%s" % [_engine_event, _audio.soundbank_path()])
	print("[ai-lap] audio fallback: %s" % _audio.fallback_summary())

## 每帧把车辆状态喂给 FMOD 的引擎事件。
##
## 这里只写参数、不设 3D 声源/听者：FMOD 的默认听者与事件都在原点，引擎声就会
## 以固定音量持续播放（`fmod_probe` 也是这么验的）。参数名由 FmodSystem 侧用
## 候选名逐个尝试，缺失的直接跳过，这里只管给值。
func _update_audio(_delta: float) -> void:
	if not _audio_ready:
		return

	var rpm := _car.engine_rpm()
	var throttle: float = _last_input[0]
	# 没有独立的发动机负载信号，用油门配合转速做近似：高转全油门负载也略降。
	var load := clampf(throttle * (0.4 + 0.6 * (1.0 - clampf(rpm / 9000.0, 0.0, 1.0))), 0.0, 1.0)
	_audio.set_vehicle_parameters(
		_engine_event, rpm, throttle, load, _car.speed_kmh(),
		1.0 if _view == "cockpit" else 0.0,
		_car.gear(), _car.max_wheel_slip_angle()
	)
	# WAV 录制：每个物理帧推进一步混音，让音轨时长与仿真/视频时间轴一致。
	if not OS.get_environment("VETTURA_AUDIO_WAV").is_empty():
		_audio.step_audio()

## 每帧按视角更新相机。刚性视角（座舱/机盖/保险杠/轮胎）挂在车身下，
## 已经随车体移动，这里不需要重算；其余视角各自求世界位姿。
func _update_camera(delta: float) -> void:
	if _cam == null or _car == null:
		return
	match _view:
		"chase":
			_update_chase_cam()
		"drone":
			_update_drone_cam(delta)
		"tcam":
			_update_tcam_cam()
		"orbit":
			_update_orbit_cam(delta)
		"wheel":
			_update_wheel_cam()
		_:
			pass

## 追逐相机：跟在车后上方（物理帧 -Z 为车头，+Z 是车尾方向）。
func _update_chase_cam() -> void:
	if _cam == null or _car == null:
		return
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	_cam.global_position = origin + basis.z * _chase_distance + Vector3(0.0, _chase_height, 0.0)
	_cam.look_at(origin - basis.z * _chase_lookahead + Vector3(0.0, 1.0, 0.0), Vector3.UP)

## 无人机追踪：高空、滞后跟随。位置用指数平滑逼近目标点，帧率无关；
## 视线落在车头前方一点，像航拍机把车压在画面下三分之一。
func _update_drone_cam(delta: float) -> void:
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	var target := origin + basis.z * _drone_distance + Vector3.UP * _drone_height
	if not _drone_ready:
		_drone_position = target
		_drone_ready = true
	else:
		var weight := 1.0 - exp(-_drone_lag * maxf(delta, 0.0))
		_drone_position = _drone_position.lerp(target, weight)
	_cam.global_position = _drone_position
	_cam.look_at(origin - basis.z * 6.0 + Vector3.UP * 0.6, Vector3.UP)

## 定点摄像机追踪：机位固定在世界坐标，只转动镜头跟拍；车即将到达时把机位
## 预置到前方赛道外，跑过或离得太远就切下一个机位（相邻机位左右交替，
## 模拟赛道边一排转播机位）。
func _update_tcam_cam() -> void:
	if _cam == null or _car == null:
		return
	var origin := _car.global_position
	var forward := -_car.global_transform.basis.z
	forward.y = 0.0
	if forward.length_squared() < 1.0e-6:
		forward = Vector3.FORWARD
	forward = forward.normalized()
	var right := forward.cross(Vector3.UP).normalized()
	# to_car 沿前进方向为正 = 车已经跑过机位；距离过远说明重置/低速卡住。
	var to_car := origin - _tcam_position
	var passed := _tcam_armed and to_car.dot(forward) > 30.0
	var too_far := _tcam_armed and _tcam_position.distance_to(origin) > _tcam_range
	if not _tcam_armed or passed or too_far:
		_tcam_side_sign = -_tcam_side_sign
		var spot := (
			origin
			+ forward * _tcam_distance
			+ right * (_tcam_side_sign * _tcam_side)
		)
		spot.y = _tcam_ground_y(spot) + _tcam_height
		_tcam_position = spot
		_tcam_armed = true
	_cam.global_position = _tcam_position
	_cam.look_at(origin + Vector3.UP * 0.5, Vector3.UP)

## 定点机位的地面高度：向下打一条射线，射不到（赛道外没有碰撞网）时用车身
## 当前高度。只在切机位时调用，不进每帧热路径。
func _tcam_ground_y(spot: Vector3) -> float:
	var space := get_world_3d().direct_space_state
	if space == null:
		return _car.global_position.y
	var query := PhysicsRayQueryParameters3D.create(
		spot + Vector3.UP * 30.0, spot - Vector3.UP * 60.0)
	var hit := space.intersect_ray(query)
	if hit.is_empty():
		return _car.global_position.y
	var hit_position: Vector3 = hit["position"]
	return hit_position.y

## 轮胎特写：相机挂在车身上，看选定的车轮 dummy（AI_LAP_WHEEL=lf/rf/lr/rr）。
func _update_wheel_cam() -> void:
	if _cam == null or _car == null:
		return
	var wheel_world := _car.to_global(_wheel_local)
	_cam.global_position = _car.to_global(_wheel_local + _wheel_offset)
	_cam.look_at(wheel_world + Vector3.UP * 0.15, Vector3.UP)

## 环绕跟拍：相机绕车慢速转圈，半径/高度/角速度可调，适合做展示镜头。
func _update_orbit_cam(delta: float) -> void:
	_orbit_angle = fmod(_orbit_angle + _orbit_speed * delta, TAU)
	var basis := _car.global_transform.basis
	var offset := basis * Vector3(
		cos(_orbit_angle) * _orbit_radius, 0.0, sin(_orbit_angle) * _orbit_radius)
	_cam.global_position = _car.global_position + offset + Vector3.UP * _orbit_height
	_cam.look_at(_car.global_position + Vector3.UP * 0.6, Vector3.UP)

## 车体局部包围盒：遍历车模的 MeshInstance3D，把各自的 AABB 变换回车身坐标系
## 再合并。只在建镜头时算一次。
func _car_body_aabb() -> AABB:
	var bounds := AABB()
	var first := true
	var inverse := _car.global_transform.affine_inverse()
	for node in _car.find_children("*", "MeshInstance3D", true, false):
		var mesh_instance := node as MeshInstance3D
		if mesh_instance == null or mesh_instance.mesh == null:
			continue
		var local := inverse * mesh_instance.global_transform * mesh_instance.get_aabb()
		if first:
			bounds = local
			first = false
		else:
			bounds = bounds.merge(local)
	return bounds

## 没有显式覆盖时，按车模包围盒自动放机盖 / 保险杠机位。不同车型车头长度
## 差别很大（F1 前翼距 CG ~2.6 m，GT 车 ~1.4 m），硬编码会把镜头塞进车头。
## 三个轴的环境变量都没设才自动适配，只要设了任意一个就完全交给使用者。
func _auto_fit_body_cams() -> void:
	if _car == null:
		return
	var box := _car_body_aabb()
	if box.size == Vector3.ZERO:
		return
	if _no_env(["AI_LAP_HOOD_X", "AI_LAP_HOOD_Y", "AI_LAP_HOOD_Z"]):
		_hood_local = Vector3(
			0.0,
			box.position.y + box.size.y * 0.68,
			box.position.z + box.size.z * 0.32
		)
	if _no_env(["AI_LAP_BUMPER_X", "AI_LAP_BUMPER_Y", "AI_LAP_BUMPER_Z"]):
		# 放在车头最前沿之外一点：贴着前翼/保险杠但不穿模，画面上方不会
		# 悬着自家前翼。
		_bumper_local = Vector3(
			0.0,
			box.position.y + box.size.y * 0.30,
			box.position.z - box.size.z * 0.02
		)
	print("[ai-lap] body cams: hood=%s bumper=%s (aabb=%s)"
			% [str(_hood_local), str(_bumper_local), str(box)])

## 这些环境变量是否一个都没设。
func _no_env(names: Array) -> bool:
	for name in names:
		if not OS.get_environment(name).is_empty():
			return false
	return true

## 当前视角的视野角。
func _view_fov() -> float:
	match _view:
		"cockpit":
			return _cockpit_fov
		"hood":
			return _hood_fov
		"bumper":
			return _bumper_fov
		"wheel":
			return _wheel_fov
		"drone":
			return _drone_fov
		"tcam":
			return _tcam_fov
		"orbit":
			return _orbit_fov
		_:
			return _chase_fov

## 视角名归一化：别名折叠 + 未知值回退 chase。
func _normalize_view(name: String) -> String:
	var view: String = VIEW_ALIASES.get(name.to_lower(), name.to_lower())
	if not view in VIEWS:
		push_warning("[ai-lap] unknown AI_LAP_VIEW '%s', falling back to chase" % name)
		return "chase"
	return view

## 找到 AI_LAP_WHEEL 指定的车轮 dummy，返回它在车体坐标系里的位置。
## 车模模型是车身节点的子对象，`bind_wheels()` 已确认这些 dummy 存在。
func _find_wheel_local() -> Vector3:
	var names := {"lf": "WHEEL_LF", "rf": "WHEEL_RF", "lr": "WHEEL_LR", "rr": "WHEEL_RR"}
	var node_name: String = names.get(_wheel_key, "WHEEL_RR")
	var wheel := _car.find_child(node_name, true, false) as Node3D
	if wheel == null:
		# 极少数 mod 的车轮不是 WHEEL_* 节点：退回车尾右侧的通用机位，
		# 至少不会把镜头留在车身原点里。
		push_warning("[ai-lap] %s not found, wheel cam uses a generic rear side" % node_name)
		return Vector3(1.0, 0.35, 1.4)
	return _car.to_local(wheel.global_position)

## 每个渲染帧抓一张图。放在 _process 而不是 _physics_process：渲染帧率决定的
## 是视频帧率，物理帧率（60 Hz）决定的是车怎么动，两者本来就不必一致。
## GPU 回读必须留在主线程，JPEG 编码 + 写盘丢给后台线程，避免卡渲染循环。
func _process(_delta: float) -> void:
	if _capture_dir.is_empty():
		return
	if _capture_frame % _capture_every != 0:
		_capture_frame += 1
		return
	# 后台积压过多时**等待写线程排空**，不能丢帧：orbit 这类便宜视角渲染
	# 速度可能远快于 1080p JPEG 写盘，丢帧会把 9 分钟的圈压成 1.5 分钟的
	# “快进”视频（实测 orbit 只存下 2588/16351 帧）。阻塞主线程让仿真等
	# 待，视频时间轴才和圈速一致。
	_capture_mutex.lock()
	var backlog := _capture_queue.size()
	_capture_mutex.unlock()
	while backlog >= CAPTURE_QUEUE_MAX:
		if not _capture_backlog_warned:
			_capture_backlog_warned = true
			push_warning("[ai-lap] capture writer backlog, waiting (frames are not dropped)")
		OS.delay_msec(2)
		_capture_mutex.lock()
		backlog = _capture_queue.size()
		_capture_mutex.unlock()
	var image := get_viewport().get_texture().get_image()
	if image == null:
		_capture_frame += 1
		return
	# 用「已保存张数」编号而不是渲染帧号：每隔几帧抓一次会跳号，而 ffmpeg
	# 的序列解码器一遇到断号就停（实测 5250 张只编出 1 帧）。
	var path := "%s/f%05d.jpg" % [_capture_dir, _capture_saved]
	_capture_mutex.lock()
	_capture_queue.push_back([ProjectSettings.globalize_path(path), image])
	_capture_mutex.unlock()
	_capture_sem.post()
	_capture_saved += 1
	_capture_frame += 1

## 后台写帧：等待信号量，取队列里的 (路径, Image) 写 JPEG。
## 停止时主线程推入 `null` 作为哨兵。
func _capture_worker() -> void:
	while true:
		_capture_sem.wait()
		_capture_mutex.lock()
		var item: Variant = _capture_queue.pop_front() if not _capture_queue.is_empty() else null
		_capture_mutex.unlock()
		if item == null:
			return
		var image: Image = item[1]
		var err := image.save_jpg(item[0], 0.85)
		if err == OK:
			_capture_written += 1
		else:
			# 线程里不调用 push_error（非线程安全），只计数，由 _finish 统一报。
			_capture_errors += 1

## 停掉后台写线程并等它把队列写完。
func _stop_capture() -> void:
	if _capture_thread == null:
		return
	_capture_mutex.lock()
	_capture_queue.push_back(null)
	_capture_mutex.unlock()
	_capture_sem.post()
	_capture_thread.wait_to_finish()
	_capture_thread = null

func _exit_tree() -> void:
	_stop_capture()

func _physics_process(delta: float) -> void:
	if _car == null or _world == null:
		return

	_elapsed += delta

	# 遥控：AI 控制器每帧求一次 [油门, 刹车, 转向]，空数组 = 没有 AI 线。
	var input := _world.ai_controls(0)
	if input.size() == 3:
		_ai_alive = true
		_last_input = [input[0], input[1], input[2]]
		_car.set_controls(input[0], input[1], input[2], false, false, ABS_ON, TC_ON)
	elif not _ai_alive and _elapsed > 2.0:
		push_error("[ai-lap] ai_controls gave no input (no fast_lane.ai?)")
		_finish()
		return

	var contacts := _contacts()
	# reset 后车身瞬时离地属正常，连续 0 接触计时在宽限期内暂停。
	if _elapsed - _last_reset_s > 1.0:
		if contacts == 0:
			_airborne_time += delta
			_max_airborne_s = max(_max_airborne_s, _airborne_time)
		else:
			_airborne_time = 0.0
	_max_speed_kmh = max(_max_speed_kmh, _car.speed_kmh())
	_update_audio(delta)

	# 卡死自救：速度持续过低 → reset 回赛道，次数有限。
	if _car.speed_kmh() < STUCK_V_MPS * 3.6:
		_stuck_time += delta
		if _stuck_time >= STUCK_S:
			_stuck_time = 0.0
			_rescues += 1
			if _rescues <= MAX_RESCUES:
				print("[ai-lap] STUCK (t=%.1f, rescues=%d/%d) → reset_vehicle" % [_elapsed, _rescues, MAX_RESCUES])
				_world.reset_vehicle(_car)
				_last_reset_s = _elapsed
			else:
				push_error("[ai-lap] stuck too many times (%d), giving up" % _rescues)
				_finish()
				return
	else:
		_stuck_time = 0.0

	# 每秒一行状态：圈 / 圈速 / 位姿 / 动力 / 轮接触 / AI 输入。
	if int(_elapsed) != int(_elapsed - delta):
		print("[ai-lap] t=%6.1f lap=%d lapT=%6.2f sec=%d pos=%s spd=%7.1f km/h gear=%d rpm=%5.0f ct=%d ai=[%.2f %.2f %.2f]"
				% [_elapsed, _world.laps(), _world.lap_time_s(), _world.current_sector(),
					str(_car.position), _car.speed_kmh(), _car.gear(), _car.engine_rpm(),
					contacts, _last_input[0], _last_input[1], _last_input[2]])

	# 完成圈时打印圈速详情（遥测里另有 JSON 帧）。
	if _world.laps() > _reported_laps:
		_reported_laps = _world.laps()
		print("[ai-lap] LAP %d done: last=%.2f s best=%.2f s sectors=%s valid(last)=%s"
				% [_world.laps(), _world.last_lap_s(), _world.best_lap_s(),
					str(_world.last_sector_times_s()), str(_world.last_lap_valid())])

	# 驾驶位/机盖/保险杠/轮胎相机是车身子对象，局部变换固定；其余视角每帧求位姿。
	_update_camera(delta)

	if _world.laps() >= _target_laps or _elapsed >= _max_lap_s:
		_finish()

func _contacts() -> int:
	var count := 0
	for index in 4:
		if _car.wheel_contact(index):
			count += 1
	return count

func _env_float(name: String, fallback: float) -> float:
	var text := OS.get_environment(name)
	if text.is_empty():
		return fallback
	return text.to_float()

func _env_text(name: String, fallback: String) -> String:
	var text := OS.get_environment(name)
	return text if not text.is_empty() else fallback


## 解析 `1920x1080` / `1920X1080`；非法值返回 (0,0)。
func _parse_size(text: String) -> Vector2i:
	var lower := text.to_lower()
	var parts := lower.split("x", false)
	if parts.size() != 2:
		return Vector2i.ZERO
	var width := int(parts[0].strip_edges())
	var height := int(parts[1].strip_edges())
	if width <= 0 or height <= 0:
		return Vector2i.ZERO
	return Vector2i(width, height)

## 允许 --car/--track 传 `res://mods/...`、`mods/...` 或裸模组名 `daytona_2017`。
func _res_path(path: String) -> String:
	if path.begins_with("res://"):
		return path
	var clean := path.trim_prefix("/")
	if clean.begins_with("mods/"):
		return "res://" + clean
	return "res://mods/" + clean

func _finish() -> void:
	var moved := _start.distance_to(_car.position)
	var ok := true
	print("[ai-lap] finished t=%.1f moved=%.1f m max_speed=%.1f km/h laps=%d last=%.2f best=%.2f max_airborne=%.2f s"
			% [_elapsed, moved, _max_speed_kmh, _world.laps(), _world.last_lap_s(),
				_world.best_lap_s(), _max_airborne_s])
	if not _capture_dir.is_empty():
		_capture_mutex.lock()
		var queued := _capture_queue.size()
		_capture_mutex.unlock()
		print("[ai-lap] capture queued=%d writing..." % [queued])
		_stop_capture()
		print("[ai-lap] captured frames=%d (every %d, errors=%d) dir=%s"
				% [_capture_written, _capture_every, _capture_errors,
					ProjectSettings.globalize_path(_capture_dir)])

	if _require_lap and _world.laps() < _target_laps:
		push_error("[ai-lap] AI failed to complete %d lap(s) in %.0f s (rescues=%d)" % [_target_laps, _max_lap_s, _rescues])
		ok = false
	if not _ai_alive:
		push_error("[ai-lap] ai_controls never produced input")
		ok = false
	if _max_speed_kmh < 80.0:
		push_error("[ai-lap] car never went faster than %.1f km/h" % _max_speed_kmh)
		ok = false
	if _max_airborne_s > MAX_AIRBORNE_S:
		push_error("[ai-lap] car lost the track (airborne %.1f s straight)" % _max_airborne_s)
		ok = false
	if not is_finite(_car.position.x) or not is_finite(_car.position.y) or not is_finite(_car.position.z):
		push_error("[ai-lap] car position is not finite")
		ok = false

	if ok:
		print("[ai-lap] PASS")
		get_tree().quit(0)
	else:
		push_error("[ai-lap] FAIL")
		get_tree().quit(1)
