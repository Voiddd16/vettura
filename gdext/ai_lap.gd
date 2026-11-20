extends Node3D
## AI 遥控整圈探针：验证「赛道 AI 线 → AiDriver → 物理 → 计时 → 遥测」全链路。
##
## 与 baseline.tscn 的差别：baseline 只验「车能不能被推着走 20 秒」，本场景
## 让 `WorldBody.ai_controls()`（fast_lane.ai 纯追踪控制器）遥控一台真车跑
## **完整一圈**，期间遥测（`user://telemetry.cfg`，UDP）与日志（`user://logs/`）
## 全程开着，供外部脚本抓帧分析。
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
## user://ai_lap_frames）、AI_LAP_VIEW（chase=车后/cockpit=驾驶位）、
## AI_LAP_CAR/AI_LAP_TRACK/AI_LAP_LAYOUT（车与赛道，支持裸模组名或 res:// 路径）、
## AI_LAP_SKILL（0..=110）、AI_LAP_EYE_X/Y/Z（驾驶位相机偏移）。

const CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const TRACK_DIR := "res://mods/daytona_2017"
const TRACK_LAYOUT := "roadcourse"

## 驾驶位相机的默认眼睛位置。AC 的 `DRIVEREYES` 相对车辆 CG、+X 为左、+Z 为前；
## 物理帧 X/Z 取反、Y 要加 CG 高度。默认按 Porsche 992 GT3 R：
## DRIVEREYES=(0.367, 0.910, -0.157)，CG=0.320（前轮半径 0.340 + BASEY -0.020）。
## Y 在 DRIVEREYES+CG（1.23）之上再抬 0.22 m 越过车头。换车时可用
## AI_LAP_EYE_X/Y/Z 覆盖。
const EYE_X_DEFAULT := -0.367
const EYE_Y_DEFAULT := 1.450
const EYE_Z_DEFAULT := 0.157

## 圈时上限（秒）。Daytona roadcourse ≈ 5.7 km，AI 目标速度受 fast_lane.ai
## 调速；正常 F1 圈时 100–150 s。超过这个值判定 AI 没能完成圈。
## 可用环境变量 AI_LAP_MAX_S 覆盖，缩短诊断迭代。
const MAX_LAP_S_DEFAULT := 300.0

## 完成几圈收工（默认一圈；可用 AI_LAP_LAPS 覆盖）。
const TARGET_LAPS_DEFAULT := 1
const TARGET_LAPS := 1

## AI 难度：AC 的 AI level，0..=110（100 = 职业，默认）。
const DEFAULT_AI_SKILL := 100

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
## AI_LAP_VIEW: "chase"（车后常规视角，默认）或 "cockpit"（驾驶位）。
var _view := "chase"
var _eye_offset := Vector3(-0.367, 1.450, 0.157)

## 可由环境变量指定的资源路径 / AI 难度。
var _car_dir := CAR_DIR
var _track_dir := TRACK_DIR
var _track_layout := TRACK_LAYOUT

## 录视频抓帧：AI_LAP_CAPTURE=1 时每个渲染帧存一张 JPEG，由外部 ffmpeg 编码。
var _capture_dir := ""
var _capture_every := 2
var _capture_frame := 0
var _capture_saved := 0

func _ready() -> void:
	_max_lap_s = _env_float("AI_LAP_MAX_S", MAX_LAP_S_DEFAULT)
	_target_laps = int(_env_float("AI_LAP_LAPS", TARGET_LAPS_DEFAULT))
	_require_lap = _env_float("AI_LAP_REQUIRE_LAP", 1.0) > 0.5
	_car_dir = _res_path(_env_text("AI_LAP_CAR", CAR_DIR))
	_track_dir = _res_path(_env_text("AI_LAP_TRACK", TRACK_DIR))
	_track_layout = _env_text("AI_LAP_LAYOUT", TRACK_LAYOUT)
	_view = _env_text("AI_LAP_VIEW", "chase").to_lower()
	_eye_offset = Vector3(
		_env_float("AI_LAP_EYE_X", EYE_X_DEFAULT),
		_env_float("AI_LAP_EYE_Y", EYE_Y_DEFAULT),
		_env_float("AI_LAP_EYE_Z", EYE_Z_DEFAULT)
	)

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

	_car = _world.spawn_vehicle_from_path(_car_dir)
	if _car == null:
		push_error("[ai-lap] spawn_vehicle_from_path failed: " + _car_dir)
		get_tree().quit(1)
		return
	_world.set_ai_skill(int(_env_float("AI_LAP_SKILL", DEFAULT_AI_SKILL)))
	print("[ai-lap] car=", _car_dir, " ai_skill=", _world.ai_skill())
	# 比赛温度：AI 代表的是「已经暖过胎的职业车手」，不是冷车起步。
	# 不暖胎时实测圈速慢 20%+（冷机 ECU 限扭到 55% 功率、冷胎掉一档抓地，
	# 变速箱还会因为冷胎高滑移拒绝升挡）。
	if not _world.warm_up_vehicle(0):
		push_warning("[ai-lap] warm_up_vehicle failed, lap will be cold")
	print("[ai-lap] spawned=", _car.name, " wheels=", _car.bound_wheels(),
			" vehicles=", _world.vehicle_count())
	_start = _car.position

	if _env_float("AI_LAP_CAM", 0.0) > 0.5 and DisplayServer.get_name() != "headless":
		_setup_visual()

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
			print("[ai-lap] capture frames -> %s every %d frame" % [absolute, _capture_every])

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
	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-48.0, -125.0, 0.0)
	sun.light_energy = 2.2
	sun.shadow_enabled = true
	add_child(sun)

	var fill := DirectionalLight3D.new()
	fill.rotation_degrees = Vector3(-18.0, 55.0, 0.0)
	fill.light_energy = 0.7
	fill.shadow_enabled = false
	add_child(fill)

	_cam = Camera3D.new()
	add_child(_cam)
	_cam.current = true
	# 驾驶位视角窄一点更符合真实镜头；追逐视角保持原来的广角。
	_cam.fov = 70.0 if _view == "cockpit" else 72.0
	_cam.near = 0.05
	print("[ai-lap] visual mode: %s cam + sun + fill (sky/ambient from project default_environment)" % _view)

## 追逐相机：跟在车后上方（物理帧 -Z 为车头，+Z 是车尾方向）。
func _update_chase_cam() -> void:
	if _cam == null or _car == null:
		return
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	_cam.global_position = origin + basis.z * 4.5 + Vector3(0.0, 2.0, 0.0)
	_cam.look_at(origin - basis.z * 14.0 + Vector3(0.0, 1.0, 0.0), Vector3.UP)

## 驾驶位相机：眼睛固定在车身坐标系的 DRIVEREYES 处（略抬高以越过车头），
## 朝车头（-Z）看。up 用世界 Y：人的地平感不随车身侧倾，倾斜赛道上画面更可读。
func _update_cockpit_cam() -> void:
	if _cam == null or _car == null:
		return
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	_cam.global_position = origin + basis * _eye_offset
	_cam.look_at(origin + basis * Vector3(_eye_offset.x, _eye_offset.y, -20.0), Vector3.UP)

## 每个渲染帧抓一张图。放在 _process 而不是 _physics_process：渲染帧率决定的
## 是视频帧率，物理帧率（60 Hz）决定的是车怎么动，两者本来就不必一致。
func _process(_delta: float) -> void:
	if _capture_dir.is_empty():
		return
	if _capture_frame % _capture_every != 0:
		_capture_frame += 1
		return
	var image := get_viewport().get_texture().get_image()
	if image == null:
		_capture_frame += 1
		return
	# 用「已保存张数」编号而不是渲染帧号：每隔几帧抓一次会跳号，而 ffmpeg
	# 的序列解码器一遇到断号就停（实测 5250 张只编出 1 帧）。
	var path := "%s/f%05d.jpg" % [_capture_dir, _capture_saved]
	var err := image.save_jpg(ProjectSettings.globalize_path(path), 0.85)
	if err != OK:
		if _capture_saved == 0:
			push_error("[ai-lap] cannot save frame %s (err %d)" % [path, err])
	else:
		_capture_saved += 1
	_capture_frame += 1

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

	if _view == "cockpit":
		_update_cockpit_cam()
	else:
		_update_chase_cam()

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
		print("[ai-lap] captured frames=%d (every %d) dir=%s"
				% [_capture_saved, _capture_every, ProjectSettings.globalize_path(_capture_dir)])

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
