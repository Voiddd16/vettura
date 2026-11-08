extends Node3D
## 工程自带的最小可用性冒烟场景。
##
## 它只回答一个问题：**主项目能不能通过 GDExtension 把一台真实 AC 车放到真实
## AC 赛道上，并让 Jolt 真的把它推着走？** 因此故意不用 Input、相机、HUD，控制量
## 直接写在脚本里，进程退出码就是验收结果。
##
## 用法（macOS；Godot 4.7）：
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --headless \
##     --path /path/to/your/godot/project \
##     res://gdext/baseline.tscn
## ```
##
## 期望：控制台输出 `[baseline] ...` 前缀的行，最后一行 `[baseline] PASS`，
## 退出码 `0`。任一步失败都会 `push_error` + `quit(1)`。

const CAR_DIR := "res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
const TRACK_DIR := "res://test/daytona_2017"
const TRACK_LAYOUT := "roadcourse"

## 跑多少秒后收工。F1 从静止起步，20 秒足够拉开数百米；本场景不追求整圈。
const RUN_S := 20.0

## 全程开着 ABS / TC：这里验证“车能不能开”，不是把辅助关掉考验物理边界。
const DRIVER_AIDS := [true, true]

var _world: WorldBody
var _car: VehicleBody
var _elapsed := 0.0
var _start: Vector3
var _max_contacts := 0

func _ready() -> void:
    _world = WorldBody.new()
    add_child(_world)

    if not _world.load_track(TRACK_DIR, TRACK_LAYOUT):
        push_error("[baseline] load_track failed")
        get_tree().quit(1)
        return
    print("[baseline] track=", _world.track_root(), " layout=", _world.track_layout(),
            " objects=", _world.track_object_count())

    _car = _world.spawn_vehicle_from_path(CAR_DIR)
    if _car == null:
        push_error("[baseline] spawn_vehicle_from_path failed")
        get_tree().quit(1)
        return
    print("[baseline] spawned=", _car.name, " wheels=", _car.bound_wheels(),
            " vehicles=", _world.vehicle_count())

    _start = _car.position

func _physics_process(delta: float) -> void:
    if _car == null or _world == null:
        return

    _elapsed += delta
    _max_contacts = max(_max_contacts, _contacts())

    # 全油门 + 方向盘打正：验证“能跑”，不验证“能跑好”。
    _car.set_controls(1.0, 0.0, 0.0, false, false, DRIVER_AIDS[0], DRIVER_AIDS[1])

    # 每秒一行 telemetry，既能看出“卡住”，也能看出它在往哪个方向跑。
    if int(_elapsed) != int(_elapsed - delta):
        print("[baseline] t=%.1f pos=%s speed=%.1f km/h gear=%d rpm=%.0f contacts=%d lap=%.2f"
                % [_elapsed, str(_car.position), _car.speed_kmh(), _car.gear(),
                   _car.engine_rpm(), _contacts(), _world.lap_time_s()])

    if _elapsed >= RUN_S:
        _finish()

func _contacts() -> int:
    var count := 0
    for index in 4:
        if _car.wheel_contact(index):
            count += 1
    return count

func _finish() -> void:
    var moved := _start.distance_to(_car.position)
    print("[baseline] finished moved=%.1f m max_contacts=%d speed=%.1f km/h laps=%d last=%.2f"
            % [moved, _max_contacts, _car.speed_kmh(), _world.laps(), _world.last_lap_s()])

    var ok := true
    if moved < 50.0:
        push_error("[baseline] car moved only %.1f m" % moved)
        ok = false
    if _max_contacts < 2:
        push_error("[baseline] car lost the track (max contacts=%d)" % _max_contacts)
        ok = false
    if not is_finite(_car.position.x) or not is_finite(_car.position.y) or not is_finite(_car.position.z):
        push_error("[baseline] car position is not finite")
        ok = false
    if _world.laps() == 0:
        # 一圈未必能在 20 秒内完成，只提示不作为失败条件——本场景验“能开”，
        # 圈速的正确性交给 `_world.lap_time_s()` / `laps()` 与单独测试。
        push_warning("[baseline] no full lap within the smoke window")

    if ok:
        print("[baseline] PASS")
        get_tree().quit(0)
    else:
        push_error("[baseline] FAIL")
        get_tree().quit(1)
