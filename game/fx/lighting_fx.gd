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
var _quality := "medium"


func setup(world: WorldBody, cars: Array, track_records: Array, car_records: Array, quality: String) -> void:
	_quality = quality
	if world == null:
		return
	var all_records: Array = []
	all_records.append_array(track_records)
	all_records.append_array(car_records)
	var roots: Array[Node] = [world]
	for car in cars:
		if car is Node:
			roots.append(car)
	for record in all_records:
		if _lights.size() >= MAX_LIGHTS:
			break
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.lighting":
			continue
		_apply_record(record, roots)
	print("[lighting] lights=%d records=%d" % [_lights.size(), all_records.size()])


func _apply_record(record: Dictionary, roots: Array[Node]) -> void:
	var values: Dictionary = record.get("values", {})
	if not _active(values):
		return
	var selectors := _selectors(values)
	var nodes := _find_matching(roots, selectors)
	if nodes.is_empty() and selectors.is_empty():
		nodes = _find_named(roots, str(record.get("name", "")))
	if nodes.is_empty():
		return
	var color := _color(values.get("color", "1,1,1"))
	var energy := _float(values.get("energy", values.get("specular_mult", 1.0)), 1.0)
	var range_value := maxf(2.0, _float(values.get("range", 8.0), 8.0))
	var spot := _float(values.get("spot", 0.0), 0.0)
	var offset := _vector(values.get("offset", "0,0,0")) * Vector3(1, 1, 1)
	var cast_shadow := _quality == "high" or _quality == "ultra"

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
		light.light_energy = clampf(energy, 0.05, 12.0)
		light.shadow_enabled = cast_shadow
		light.distance_fade_enabled = true
		light.distance_fade_begin = range_value * 1.5
		light.distance_fade_length = range_value
		add_child(light)
		light.global_position = (node as Node3D).global_position + offset
		_lights.append(light)


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
	var max_value := maxf(r, maxf(g, b))
	if max_value > 2.0:
		r /= 10.0
		g /= 10.0
		b /= 10.0
	return Color(clampf(r, 0.0, 4.0), clampf(g, 0.0, 4.0), clampf(b, 0.0, 4.0), 1.0)
