extends Node3D
## BrakeDiscFX：刹车盘温度 → 发光/发红。
##
## 通过 `VehicleBody.brake_temperature_c()` 读取四轮刹车盘表面温度，找到车模里
## 名字/材质带 brake/disc/rotor 的 MeshInstance3D，复制材质并写 emission。
## 温度阈值来自 ext_config（BRAKE_DISC_FX / BRAKE_DISC_*）。

var enabled := true
var glow_threshold := 380.0
var max_emissive := 1.0
var _entries: Array = []


func setup(cars: Array, car_records: Array) -> void:
	_apply_records(car_records)
	for car in cars:
		if not (car is VehicleBody):
			continue
		_collect_car(car)
	print("[brake-disc] materials=%d" % _entries.size())


func _apply_records(records: Array) -> void:
	for record in records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.brake_disc":
			continue
		var values: Dictionary = record.get("values", {})
		if values.has("glow_threshold"):
			glow_threshold = _float(values["glow_threshold"], glow_threshold)
		if values.has("max_emissive"):
			max_emissive = _float(values["max_emissive"], max_emissive)


func _collect_car(car: VehicleBody) -> void:
	for node in car.find_children("*", "MeshInstance3D", true, false):
		var mesh := node as MeshInstance3D
		if mesh == null:
			continue
		var haystack := (String(mesh.name) + " " + _material_names(mesh)).to_lower()
		if not (haystack.contains("brake") or haystack.contains("disc") or haystack.contains("rotor")):
			continue
		if mesh.mesh == null:
			continue
		for surface in mesh.mesh.get_surface_count():
			var source := mesh.get_surface_override_material(surface)
			if source == null:
				source = mesh.mesh.surface_get_material(surface)
			if not (source is StandardMaterial3D):
				continue
			var material := (source as StandardMaterial3D).duplicate() as StandardMaterial3D
			material.emission_enabled = true
			material.emission = Color(0, 0, 0)
			material.emission_energy_multiplier = 0.0
			mesh.set_surface_override_material(surface, material)
			_entries.append({"car": car, "material": material})


func _process(delta: float) -> void:
	if not enabled or _entries.is_empty():
		return
	for entry in _entries:
		var car: VehicleBody = entry["car"]
		var material: StandardMaterial3D = entry["material"]
		if not is_instance_valid(car) or material == null:
			continue
		var temperature := 0.0
		for wheel in 4:
			temperature = maxf(temperature, car.brake_temperature_c(wheel))
		var excess := maxf(0.0, temperature - glow_threshold)
		var glow := clampf(excess / 420.0, 0.0, 1.0)
		if glow <= 0.001:
			material.emission_enabled = false
			continue
		material.emission_enabled = true
		# 低温暗红 → 高温橙黄。
		material.emission = Color(1.0, lerpf(0.12, 0.62, glow), lerpf(0.02, 0.18, glow), 1.0)
		material.emission_energy_multiplier = glow * max_emissive * 4.0


func _material_names(mesh: MeshInstance3D) -> String:
	var names: Array[String] = []
	if mesh.mesh == null:
		return ""
	for i in mesh.mesh.get_surface_count():
		var material := mesh.get_surface_override_material(i)
		if material == null:
			material = mesh.mesh.surface_get_material(i)
		if material != null:
			names.append(String(material.resource_name))
	return " ".join(names)


func _float(value: Variant, fallback: float) -> float:
	var text := String(value).strip_edges()
	return text.to_float() if text.is_valid_float() else fallback


func _ready() -> void:
	set_process(true)
