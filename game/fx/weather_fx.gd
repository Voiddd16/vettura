extends Node3D
## WeatherFX：复用项目已有的 `weather_visuals.gd`，把 WorldBody 的物理天气
## 同步到天空/雾/太阳/雨丝。ext_config 的 WIND/WEATHER_FX 记录先保留，
## 后续接入时间/风/云参数；v1 不重复实现第二套天气系统。

var _visuals: Node
var _world: WorldBody


func setup(world: WorldBody, _records: Array) -> void:
	_world = world
	var root := get_tree().current_scene
	if root != null:
		for node in root.find_children("*", "", true, false):
			if node.has_method("sync_from_world"):
				_visuals = node
				break
	if _visuals == null:
		var script := load("res://gdext/weather_visuals.gd")
		if script == null:
			push_warning("[weather-fx] weather_visuals.gd 缺失，跳过天气视觉")
			return
		_visuals = script.new()
		_visuals.name = "WeatherVisuals"
		add_child(_visuals)
	if _world_ready():
		_visuals.sync_from_world(_world)


func _process(_delta: float) -> void:
	if _visuals != null and _world_ready():
		_visuals.sync_from_world(_world)


func _world_ready() -> bool:
	return _world != null and is_instance_valid(_world)
