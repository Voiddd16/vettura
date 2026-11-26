extends Node3D
## 桌面可视化探针：两阶段检查「加载车辆模组」的完整效果。
## 需要 mods/ 真车真赛道 + 桌面窗口（headless 拿不到截图，只出遥测）。
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --path . res://gdext/desktop_probe.tscn
## ```
##
## 阶段 1（浮空预览页 `main.tscn`）：载车 → 四个视角各截一张 →
## 油门看车轮转起来 → 刹车看车轮停下来。
## 阶段 2（真实赛道 `WorldBody`）：Daytona + F1 → 追逐相机 → AI 沿赛道线跑
## 16 s（会自己过弯，不会直道撞墙）→ 重刹 6 s → 评估，逐秒遥测 + 截图。
##
## 截图存 `SHOT_DIR`；结束打印 `[desktop-probe] PASS` / `FAIL`，退出码即验收结果。

const CAR_DIR := "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const TRACK_DIR := "res://mods/daytona_2017"
const TRACK_LAYOUT := "roadcourse"
const MAIN_SCENE := "res://game/main.tscn"
const SHOT_DIR := "/private/var/folders/1n/jrfz32895pzdm63ht34jz0br0000gn/T/opencode/vettura_shots"

## 每步时长（s）。阶段 1 共 7 步（14 s），阶段 2 共 12 步（24 s）。
const STEP := 2.0
const TOTAL_TIMEOUT := 150.0

var _main: Node3D
var _world: WorldBody
var _car: VehicleBody
var _cam: Camera3D

var _phase := 0       # 0=等载车 1=预览检查 2=赛道检查
var _t := 0.0         # 阶段内计时（遥测/步进）
var _step := 0
var _total := 0.0

var _start_pos: Vector3
var _max_speed := 0.0
var _max_contacts := 0
var _speed_at_brake := 0.0

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(SHOT_DIR)
	if DisplayServer.get_name() == "headless":
		push_warning("[desktop-probe] headless 下截图不可用，只输出遥测")
	_main = load(MAIN_SCENE).instantiate() as Node3D
	add_child(_main)

func _process(delta: float) -> void:
	_total += delta
	_t += delta
	if _total > TOTAL_TIMEOUT:
		push_error("[desktop-probe] 超时未完成（%.0f s）" % _total)
		get_tree().quit(1)
		return
	match _phase:
		0:
			if _main != null and _main.has_car():
				_phase = 1
				_t = 0.0
				_step = 0
				_preview_step()
			elif _t > 20.0:
				push_error("[desktop-probe] 车模 20s 未加载（mods/ 缺失或 GDExtension 未注册）")
				get_tree().quit(1)
		1:
			if _t >= STEP:
				_t = 0.0
				_step += 1
				_preview_step()
		2:
			if _t >= STEP:
				_t = 0.0
				_step += 1
				_track_step()

## 阶段 1：直接驱动 main.gd 的视角/输入处理器（下划线方法，GDScript 不设防）。
func _preview_step() -> void:
	match _step:
		0:
			_main._camera_mode = 0
			_shot("p1_cam0_behind")
		1:
			_main._on_throttle_pressed()
			_shot("p1_cam0_throttle")
		2:
			_main._camera_mode = 1
			_shot("p1_cam1_driver")
		3:
			_main._camera_mode = 2
			_shot("p1_cam2_orbit")
		4:
			_main._camera_mode = 3
			_shot("p1_cam3_hood")
		5:
			_main._on_throttle_released()
			_main._on_brake_pressed()
			_shot("p1_brake")
		6:
			print("[desktop-probe] phase1 wheel_spin=%.1f rad/s（油门后应 > 0）" % _main._wheel_spin)
			_main._on_brake_released()
			_start_track_phase()
		_:
			pass

## 阶段 2 入口：释放预览页，装载真赛道 + 真车 + 追逐相机。
func _start_track_phase() -> void:
	_main.queue_free()
	_main = null
	_phase = 9  # 过渡：装载是同步阻塞的，本帧之后直接进阶段 2

	_world = WorldBody.new()
	add_child(_world)
	var t0 := Time.get_ticks_msec()
	if not _world.load_track(TRACK_DIR, TRACK_LAYOUT):
		push_error("[desktop-probe] load_track failed")
		get_tree().quit(1)
		return
	print("[desktop-probe] track loaded in %.1f s, objects=%d, layout=%s"
			% [(Time.get_ticks_msec() - t0) / 1000.0, _world.track_object_count(),
				_world.track_layout()])

	t0 = Time.get_ticks_msec()
	_car = _world.spawn_vehicle_from_path(CAR_DIR)
	if _car == null:
		push_error("[desktop-probe] spawn_vehicle_from_path failed")
		get_tree().quit(1)
		return
	print("[desktop-probe] vehicle spawned in %.1f s, wheels=%d, vehicles=%d"
			% [(Time.get_ticks_msec() - t0) / 1000.0, _car.bound_wheels(),
				_world.vehicle_count()])

	_start_pos = _car.position
	_setup_environment()
	_cam = Camera3D.new()
	add_child(_cam)
	_cam.current = true
	_phase = 2
	_t = 0.0
	_step = 0
	_track_step()

## 赛道与车模都没有自带灯光：给一段日光 + 工程默认环境（天空/环境光/后处理），
## 否则路面只有一片死黑，截图根本看不出车模贴图与路面。
##
## 不再手搓 `AMBIENT_SOURCE_SKY` 的 ProceduralSky：Godot 4.7 里这条路径实测
## 不产生环境光，探针会拍到一张比游戏暗得多的图，失去对比价值。
func _setup_environment() -> void:
	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-48.0, -125.0, 0.0)
	sun.light_energy = 2.2
	sun.light_angular_distance = 0.5
	sun.shadow_enabled = true
	sun.directional_shadow_mode = DirectionalLight3D.SHADOW_PARALLEL_4_SPLITS
	sun.directional_shadow_max_distance = 120.0
	add_child(sun)

	var base := load("res://game_environment.tres") as Environment
	var env := base.duplicate(true) as Environment
	var world_env := WorldEnvironment.new()
	world_env.environment = env
	add_child(world_env)

## 阶段 2 步进：0–7 AI 驾驶（16 s，沿 `fast_lane.ai` 跑）、8–10 重刹（6 s）、
## 11 收工评估。
func _track_step() -> void:
	match _step:
		0:
			# 截图前先把相机摆到位，否则第一帧拍到的是相机默认位置。
			_update_chase_cam()
			_shot("p2_t0_start")
		7:
			_shot("p2_accel")
		8:
			_speed_at_brake = _car.speed_kmh()
		9:
			_shot("p2_braking")
		11:
			_shot("p2_after_brake")
			_finish_track()
		_:
			pass

func _physics_process(delta: float) -> void:
	if _phase != 2 or _car == null:
		return
	if _step < 8:
		# AI 沿赛道线驾驶：油门/刹车/转向都由 `fast_lane.ai` 决定。
		var ai := _world.ai_controls(0)
		if ai.size() >= 3:
			_car.set_controls(ai[0], ai[1], ai[2], false, false, true, true)
		else:
			# 没有 AI 线时退回全油门，好歹能看到加速表现。
			_car.set_controls(1.0, 0.0, 0.0, false, false, true, true)
	else:
		_car.set_controls(0.0, 1.0, 0.0, false, false, true, true)
	_update_chase_cam()

	_max_speed = max(_max_speed, _car.speed_kmh())
	_max_contacts = max(_max_contacts, _contacts())
	if int(_t) != int(_t - delta):
		print("[desktop-probe] t=%.1f speed=%.1f km/h gear=%d rpm=%.0f contacts=%d lap=%.2f"
				% [_t, _car.speed_kmh(), _car.gear(), _car.engine_rpm(),
					_contacts(), _world.lap_time_s()])

## 追逐相机：跟在车后（物理帧 -Z 为前，所以 +Z 是车尾方向）。
func _update_chase_cam() -> void:
	var basis := _car.global_transform.basis
	var origin := _car.global_position
	_cam.global_position = origin + basis.z * 4.0 + Vector3(0.0, 1.8, 0.0)
	_cam.look_at(origin - basis.z * 12.0 + Vector3(0.0, 1.0, 0.0), Vector3.UP)

func _contacts() -> int:
	var count := 0
	for index in 4:
		if _car.wheel_contact(index):
			count += 1
	return count

func _finish_track() -> void:
	var moved := _start_pos.distance_to(_car.position)
	var end_speed := _car.speed_kmh()
	print("[desktop-probe] finished moved=%.1f m max_speed=%.1f km/h"
			% [moved, _max_speed]
			+ " brake=%.1f→%.1f km/h max_contacts=%d laps=%d last_lap=%.2f"
			% [_speed_at_brake, end_speed, _max_contacts, _world.laps(),
				_world.last_lap_s()])
	var ok := true
	if _max_speed < 90.0:
		push_error("[desktop-probe] 全油门 16 s 只到 %.1f km/h（< 90）" % _max_speed)
		ok = false
	if _max_contacts < 2:
		push_error("[desktop-probe] 车轮最大接地数 %d（< 2），车掉出赛道" % _max_contacts)
		ok = false
	if not is_finite(_car.position.x) or not is_finite(_car.position.y) \
			or not is_finite(_car.position.z):
		push_error("[desktop-probe] 车辆位置非有限值")
		ok = false
	if _speed_at_brake > 0.0 and end_speed > 0.6 * _speed_at_brake:
		push_warning("[desktop-probe] 重刹 6 s 减速不明显（%.0f → %.0f km/h）"
				% [_speed_at_brake, end_speed])
	if ok:
		print("[desktop-probe] PASS")
		get_tree().quit(0)
	else:
		push_error("[desktop-probe] FAIL")
		get_tree().quit(1)

## 截一帧存 SHOT_DIR（窗口模式才有意义）。
## 注意：必须等 `frame_post_draw`，否则 get_image() 拿到的是没画完的空缓冲（全黑）。
func _shot(tag: String) -> void:
	if DisplayServer.get_name() == "headless":
		return
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := "%s/%s.png" % [SHOT_DIR, tag]
	var err := img.save_png(path)
	if err != OK:
		push_warning("[desktop-probe] screenshot failed: %s (err=%d)" % [path, err])
	else:
		print("[desktop-probe] shot: ", path)
