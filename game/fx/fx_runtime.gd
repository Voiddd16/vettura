extends Node3D
## 视觉 FX 运行时：在真实赛道场景里自动发现 `WorldBody` / `VehicleBody`，
## 读取 track / car 的 ext_config 绑定记录，并驱动各个 FX。
##
## 用法：把本节点挂到 `WorldBody` 同级（或任意场景根下）即可。
## 它会等待赛道和车辆生成后自动 attach；也可以在外部调用 `attach()`。
##
## v1 先接 RainFX；Grass/Lighting/Particles/BrakeDisc/Tyres/Extra 会按同样方式
## 逐步接入，不需要改 WorldBody。

@export var auto_attach := true
@export var enable_rain := true
@export var enable_grass := true
@export var enable_lighting := true
@export var enable_particles := true
@export var enable_brake_disc := true
@export var enable_tyres := true
@export var enable_extra := true
@export var enable_weather := true
@export var quality := "medium"

var _world: WorldBody
var _cars: Array = []
var _camera: Camera3D

var _rain_fx: Node3D
var _grass_fx: Node3D
var _lighting_fx: Node3D
var _particles_fx: Node3D
var _brake_fx: Node3D
var _tyres_fx: Node3D
var _extra_fx: Node3D
var _weather_fx: Node3D

var _attached := false
var _polls := 0


func _ready() -> void:
	set_process(true)


func _process(_delta: float) -> void:
	if not auto_attach:
		return
	if not _attached:
		_polls += 1
		if _polls % 10 != 0:
			return
		_try_attach()
		return
	if _world != null and is_instance_valid(_world):
		var weather: Dictionary = _world.get_weather()
		if _rain_fx != null and is_instance_valid(_rain_fx):
			_rain_fx.set_weather(
				float(weather.get("rain", 0.0)),
				float(weather.get("wetness", 0.0)),
				weather.get("wind", Vector3.ZERO) as Vector3
			)


func attach(world: WorldBody, cars: Array, camera: Camera3D = null) -> void:
	_world = world
	_cars = cars
	_camera = camera
	_attached = true
	_build_effects()


func _try_attach() -> void:
	var world := _find_world()
	if world == null:
		return
	var cars := _find_cars(world)
	if cars.is_empty():
		return
	attach(world, cars, _find_camera())


func _find_world() -> WorldBody:
	for node in get_tree().root.find_children("*", "WorldBody", true, false):
		if node is WorldBody:
			return node as WorldBody
	return null


func _find_cars(world: WorldBody) -> Array:
	var cars: Array = []
	for node in world.find_children("*", "VehicleBody", true, false):
		if node is VehicleBody:
			cars.append(node)
	return cars


func _find_camera() -> Camera3D:
	return get_viewport().get_camera_3d()


func _build_effects() -> void:
	if _camera == null or not is_instance_valid(_camera):
		_camera = _find_camera()

	var track_root := ""
	var car_roots: Array[String] = []
	if _world != null:
		track_root = String(_world.track_root())
	for car in _cars:
		if car is VehicleBody:
			var root := String(car.car_path())
			if not root.is_empty():
				car_roots.append(root)

	var track_records := _records_for(track_root)
	var car_records: Array = []
	for root in car_roots:
		car_records.append_array(_records_for(root))

	if enable_rain:
		_rain_fx = load("res://game/fx/rain_fx.gd").new()
		_rain_fx.name = "RainFX"
		add_child(_rain_fx)
		_rain_fx.setup(_world, _cars, track_records, _camera)

	if enable_grass:
		_grass_fx = load("res://game/fx/grass_fx.gd").new()
		_grass_fx.name = "GrassFX"
		add_child(_grass_fx)
		_grass_fx.setup(_world, track_records, quality)

	if enable_lighting:
		_lighting_fx = load("res://game/fx/lighting_fx.gd").new()
		_lighting_fx.name = "LightingFX"
		add_child(_lighting_fx)
		_lighting_fx.setup(_world, _cars, track_records, car_records, quality)

	if enable_particles:
		_particles_fx = load("res://game/fx/particles_fx.gd").new()
		_particles_fx.name = "ParticlesFX"
		add_child(_particles_fx)
		_particles_fx.setup(_world, _cars, track_records, car_records, quality)

	if enable_brake_disc:
		_brake_fx = load("res://game/fx/brake_disc_fx.gd").new()
		_brake_fx.name = "BrakeDiscFX"
		add_child(_brake_fx)
		_brake_fx.setup(_cars, car_records)

	if enable_tyres:
		_tyres_fx = load("res://game/fx/tyres_fx.gd").new()
		_tyres_fx.name = "TyresFX"
		add_child(_tyres_fx)
		_tyres_fx.setup(_cars, car_records)

	if enable_extra:
		_extra_fx = load("res://game/fx/extra_fx.gd").new()
		_extra_fx.name = "ExtraFX"
		add_child(_extra_fx)
		_extra_fx.setup(_world, track_records, quality)

	if enable_weather:
		_weather_fx = load("res://game/fx/weather_fx.gd").new()
		_weather_fx.name = "WeatherFX"
		add_child(_weather_fx)
		_weather_fx.setup(_world, track_records)

	print("[fx] attached world=%s cars=%d track_records=%d" % [
		track_root, _cars.size(), track_records.size()])


func _records_for(path: String) -> Array:
	if path.is_empty() or not ClassDB.class_exists("AcVisualCompat"):
		return []
	var compat := AcVisualCompat.new()
	var raw := String(compat.inspect_json(path, 1))
	if raw.is_empty():
		return []
	var parsed = JSON.parse_string(raw)
	if not (parsed is Dictionary):
		return []
	var ext: Dictionary = parsed.get("ext_config", {})
	var records = ext.get("records", [])
	if records is Array:
		return records
	return []
