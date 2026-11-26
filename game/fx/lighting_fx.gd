extends Node3D
## LightingFX：把 ext_config 的 LIGHT_SERIES / EMISSIVE_LIGHT / LIGHT_POLLUTION
## 映射成真实 OmniLight3D / SpotLight3D。
##
## 支持：
## - `MESHES` / `MATERIALS` / `NAME` 选择器；
## - COLOR / RANGE / SPOT / ENERGY / OFFSET；
## - 预算上限，移动端减少阴影；
## - 无匹配时只记录 warning，不阻断场景。

const MAX_LIGHTS := 48

var enabled := true
var _lights: Array[Light3D] = []
var _base_energy: Array[float] = []
var _world: WorldBody
var _quality := "medium"
## 太阳强度只需要以秒级刷新；缓存后如果变化小于死区，就不写 48 盏灯的
## `light_energy`（每次赋值都会标脏场景灯光参数）。
const SUN_POLL_S := 0.25
const SUN_EPSILON := 0.01
var _poll_timer := 0.0
var _sun_level := -1.0
var _last_applied_sun := -2.0


func setup(world: WorldBody, cars: Array, track_records: Array, car_records: Array, quality: String) -> void:
	_quality = quality
	_world = world
	if world == null:
		return
	# 赛道记录只搜赛道，车记录只搜车：旧实现把两边合在一起搜，车上的
	# `NAME=LED` 会匹配到起点附近同名/同材质的赛道网格，生成一盏留在起跑线
	# 的点光源。
	var track_roots: Array[Node] = [world]
	var car_roots: Array[Node] = []
	for car in cars:
		if car is Node:
			car_roots.append(car)
	var count := 0
	for record in track_records:
		if _lights.size() >= MAX_LIGHTS:
			break
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.lighting":
			continue
		count += 1
		_apply_record(record, track_roots, false)
	for record in car_records:
		if _lights.size() >= MAX_LIGHTS:
			break
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.lighting":
			continue
		count += 1
		_apply_record(record, car_roots, true)
	print("[lighting] lights=%d records=%d" % [_lights.size(), count])


func _apply_record(record: Dictionary, roots: Array[Node], follow: bool) -> void:
	var values: Dictionary = record.get("values", {})
	if not _active(values):
		return
	var selectors := _selectors(values)
	var nodes := _find_matching(roots, selectors)
	if nodes.is_empty() and selectors.is_empty():
		nodes = _find_named(roots, str(record.get("name", "")))
	if nodes.is_empty():
		return
	# `LIGHT_SERIES` 本来就是沿赛道一排灯；车辆上的 EMISSIVE_LIGHT 只取一个
	# 代表节点。F1 的 `NAME=LED` 会在车内匹配几十个网格，旧实现给每个网格都
	# 放一盏 8m 点光，起跑点附近因此出现一团/一盏残留光。
	var series := String(record.get("name", "")).to_upper().contains("LIGHT_SERIES")
	if follow and not series and nodes.size() > 1:
		nodes = [nodes[0]]
	if OS.get_environment("VETTURA_LIGHT_DEBUG") == "1":
		print("[lighting-debug] %s series=%s nodes=%d follow=%s first=%s" % [
			record.get("name", ""), str(series), nodes.size(), str(follow),
			str(nodes[0].global_position) if not nodes.is_empty() else "-"])

	var color := _color(values.get("color", "1,1,1"))
	var energy := _float(values.get("energy", values.get("specular_mult", 1.0)), 1.0)
	# 车灯（LED/刹车灯）用更小的默认范围，避免一个车内 LED 照亮整条起跑线。
	var default_range := 2.0 if follow else 8.0
	var range_value := maxf(1.0, _float(values.get("range", default_range), default_range))
	var spot := _float(values.get("spot", 0.0), 0.0)
	var offset := _vector(values.get("offset", "0,0,0")) * Vector3(1, 1, 1)
	var cast_shadow := (_quality == "high" or _quality == "ultra") and not follow

	for node in nodes:
		if _lights.size() >= MAX_LIGHTS:
			break
		var light: Light3D
		if spot > 0.0:
			var spot_light := SpotLight3D.new()
			spot_light.spot_range = range_value
			spot_light.spot_angle = clampf(spot, 1.0, 89.0)
			spot_light.spot_angle_attenuation = 1.0
			light = spot_light
		else:
			var omni := OmniLight3D.new()
			omni.omni_range = range_value
			omni.omni_attenuation = 1.2
			light = omni
		light.name = "FxLight_%s_%d" % [record.get("name", "light"), _lights.size()]
		light.light_color = color
		light.light_energy = clampf(energy, 0.05, 3.0)
		light.shadow_enabled = cast_shadow
		light.distance_fade_enabled = true
		light.distance_fade_begin = range_value * 1.5
		light.distance_fade_length = range_value
		if follow:
			# 车灯跟着网格走。旧实现把光源挂在世界原点（创建时的全局位置），
			# 车开走后灯留在起跑线，雨天/夜晚就会看到一盏漂浮的点光源。
			(node as Node3D).add_child(light)
			light.position = offset
		else:
			add_child(light)
			light.global_position = (node as Node3D).global_position + offset
		_lights.append(light)
		_base_energy.append(light.light_energy)


func _process(delta: float) -> void:
	if _world == null or _lights.is_empty():
		return
	# 白天/强日照时衰减灯光能量，避免 LIGHT_SERIES 在路面上烧出白斑。
	_poll_timer -= delta
	if _poll_timer <= 0.0:
		_poll_timer = SUN_POLL_S
		if is_instance_valid(_world):
			_sun_level = clampf(float(_world.get_weather().get("sun", 1.0)), 0.0, 1.0)
	if absf(_sun_level - _last_applied_sun) < SUN_EPSILON:
		return
	_last_applied_sun = _sun_level
	var scale := lerpf(1.0, 0.28, _sun_level)
	for index in _lights.size():
		var light := _lights[index]
		if light != null and index < _base_energy.size():
			light.light_energy = _base_energy[index] * scale


func _active(values: Dictionary) -> bool:
	if values.has("active"):
		var raw := str(values["active"]).to_lower()
		return raw != "0" and raw != "false" and raw != "off"
	return true


func _selectors(values: Dictionary) -> Array[String]:
	var out: Array[String] = []
	for key in ["meshes", "materials", "name"]:
		if not values.has(key):
			continue
		for part in _parts(values[key]):
			var text := str(part).strip_edges()
			if not text.is_empty():
				out.append(text)
	return out


func _find_matching(roots: Array[Node], selectors: Array[String]) -> Array[Node3D]:
	var out: Array[Node3D] = []
	if selectors.is_empty():
		return out
	for root in roots:
		for node in root.find_children("*", "MeshInstance3D", true, false):
			var mesh := node as MeshInstance3D
			if mesh == null:
				continue
			var haystack := String(mesh.name).to_lower()
			var material_haystack := _material_names(mesh).to_lower()
			for selector in selectors:
				var needle := selector.to_lower().trim_prefix("*").trim_suffix("*")
				if haystack.contains(needle) or material_haystack.contains(needle):
					out.append(mesh)
					break
	return out


func _find_named(roots: Array[Node], name: String) -> Array[Node3D]:
	var out: Array[Node3D] = []
	if name.is_empty():
		return out
	for root in roots:
		for node in root.find_children("*", "Node3D", true, false):
			if String(node.name).to_lower().contains(name.to_lower()):
				out.append(node as Node3D)
	return out


func _material_names(mesh: MeshInstance3D) -> String:
	var names: Array[String] = []
	if mesh.mesh != null:
		for i in mesh.mesh.get_surface_count():
			var material := mesh.get_surface_override_material(i)
			if material == null:
				material = mesh.mesh.surface_get_material(i)
			if material != null:
				names.append(String(material.resource_name))
	return " ".join(names)


func _parts(value: Variant) -> Array:
	if value is Array:
		return value
	return str(value).split(",", false)


func _float(value: Variant, fallback: float) -> float:
	if value is Array:
		if value.is_empty():
			return fallback
		value = value[0]
	var text := str(value).strip_edges()
	var parsed := text.to_float()
	return parsed if text.is_valid_float() else fallback


func _vector(value: Variant) -> Vector3:
	var parts := _parts(value)
	if parts.size() < 3:
		return Vector3.ZERO
	# ext_config 使用 AC 坐标；Godot 的 X/Z 轴镜像。
	return Vector3(-str(parts[0]).to_float(), str(parts[1]).to_float(), -str(parts[2]).to_float())


func _color(value: Variant) -> Color:
	var parts := _parts(value)
	var r := 1.0
	var g := 1.0
	var b := 1.0
	if parts.size() >= 3:
		r = str(parts[0]).to_float()
		g = str(parts[1]).to_float()
		b = str(parts[2]).to_float()
	# ext_config 的 COLOR 是 HDR 值（25,25,25 很常见）；归一化到 0..1，
	# 否则 44 盏灯叠加会在地面上形成过曝白块。
	var max_value := maxf(r, maxf(g, b))
	if max_value > 0.001:
		r /= max_value
		g /= max_value
		b /= max_value
	return Color(clampf(r, 0.0, 1.0), clampf(g, 0.0, 1.0), clampf(b, 0.0, 1.0), 1.0)
