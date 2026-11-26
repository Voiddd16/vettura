extends Node3D
## WeatherFX：优先复用场景里的 `WeatherRig`（任何带 `sync_from_world` 的节点），
## 把 WorldBody 的物理天气同步到天空/雾/太阳/雨丝；没有 WeatherRig 的旧场景
## （ai_lap / desktop_probe）才创建 `weather_visuals.gd` 兜底。
## ext_config 的 WIND/WEATHER_FX 记录先保留，后续接入时间/风/云参数；
## v1 不重复实现第二套天气系统。
##
## 同步频率说明：物理天气变化以秒计（湿度/雨量），不需要每帧读一次
## `WorldBody.get_weather()`（每次都会构造一个 GDScript 字典）。这里用
## 5 Hz 轮询 + `sync_from_world` 内部的值死区；天气真正变化时最多晚
## 200 ms 生效，肉眼不可见，但省掉大量跨语言调用与字典分配。

const SYNC_INTERVAL_S := 0.2

var _visuals: Node
var _world: WorldBody
var _sync_timer := 0.0


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


func _process(delta: float) -> void:
	if _visuals == null or not _world_ready():
		return
	_sync_timer -= delta
	if _sync_timer > 0.0:
		return
	_sync_timer = SYNC_INTERVAL_S
	_visuals.sync_from_world(_world)


func _world_ready() -> bool:
	return _world != null and is_instance_valid(_world)
