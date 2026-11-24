extends Node3D
## TyresFX：ext_config 的 TYRES_FX_CUSTOMTEXTURE_* → 车模轮胎材质贴图。
##
## 支持：
## - MATERIAL / TXDIFFUSE / TXNORMAL；
## - 直接文件路径（PNG/JPG/DDS，取决于 Godot Image 支持）；
## - `cars\...zip::file.dds` 这种 AC 打包路径：v1 只报告，不强行解压。
##
## 贴图替换按“材质资源”去重，避免同一个 tread 被替换几十次。

var enabled := true
var _applied := {}
var _warnings := {}


func setup(cars: Array, car_records: Array) -> void:
	for record in car_records:
		if not (record is Dictionary):
			continue
		if String(record.get("module", "")) != "vettura.fx.tyres":
			continue
		_apply_record(record, cars)
	print("[tyres] applied=%d records=%d" % [_applied.size(), car_records.size()])


func _apply_record(record: Dictionary, cars: Array) -> void:
	var values: Dictionary = record.get("values", {})
	var material_name := String(values.get("material", "")).strip_edges()
	if material_name.is_empty():
		return
	var diffuse := String(values.get("txdiffuse", "")).strip_edges()
	if diffuse.is_empty():
		return

	for car in cars:
		if not (car is VehicleBody):
			continue
		for node in car.find_children("*", "MeshInstance3D", true, false):
			var mesh := node as MeshInstance3D
			if mesh == null or mesh.mesh == null:
				continue
			for surface in mesh.mesh.get_surface_count():
				var source := mesh.get_surface_override_material(surface)
				if source == null:
					source = mesh.mesh.surface_get_material(surface)
				if not (source is StandardMaterial3D):
					continue
				if not String(source.resource_name).to_lower().contains(material_name.to_lower()):
					continue
				var key := "%s:%s" % [car.get_instance_id(), source.get_instance_id()]
				if _applied.has(key):
					continue
				var texture := _load_texture(diffuse, car)
				if texture == null:
					continue
				var dup := (source as StandardMaterial3D).duplicate() as StandardMaterial3D
				dup.albedo_texture = texture
				mesh.set_surface_override_material(surface, dup)
				_applied[key] = true


func _load_texture(path: String, car: VehicleBody) -> Texture2D:
	var normalized := path.replace("\\", "/")
	if normalized.contains("::"):
		return _load_zip_texture(normalized, car)
	if not normalized.begins_with("res://") and not normalized.begins_with("user://"):
		if not normalized.begins_with("/"):
			var car_root := String(car.car_path())
			normalized = car_root.path_join(normalized)
	if not FileAccess.file_exists(normalized):
		_warn_once(normalized, "贴图不存在：%s" % normalized)
		return null
	var image := Image.load_from_file(normalized)
	if image == null or image.is_empty():
		return null
	return ImageTexture.create_from_image(image)


func _load_zip_texture(path: String, car: VehicleBody) -> Texture2D:
	var parts := path.split("::", false)
	if parts.size() < 2:
		return null
	var zip_rel := String(parts[0]).trim_prefix("/")
	var inner := String(parts[1])
	var car_root := String(car.car_path())
	var candidates: Array[String] = []
	var cursor := car_root
	for _i in 6:
		candidates.append(cursor.path_join(zip_rel))
		candidates.append(cursor.path_join("extension/textures").path_join(zip_rel))
		var parent := cursor.get_base_dir()
		if parent == cursor or parent.is_empty():
			break
		cursor = parent
	var zip_path := ""
	for candidate in candidates:
		if FileAccess.file_exists(candidate):
			zip_path = candidate
			break
	if zip_path.is_empty():
		_warn_once(path, "zip 不存在：%s" % zip_rel)
		return null
	var reader := ZIPReader.new()
	if reader.open(zip_path) != OK:
		_warn_once(path, "zip 打不开：%s" % zip_path)
		return null
	var bytes := reader.read_file(inner)
	reader.close()
	if bytes.is_empty():
		_warn_once(path, "zip 内文件不存在：%s" % inner)
		return null
	var image := Image.new()
	if image.load_dds_from_buffer(bytes) != OK:
		if image.load_png_from_buffer(bytes) != OK:
			_warn_once(path, "zip 内贴图格式不支持：%s" % inner)
			return null
	return ImageTexture.create_from_image(image)


func _warn_once(key: String, message: String) -> void:
	if _warnings.has(key):
		return
	_warnings[key] = true
	push_warning("[tyres-fx] " + message)
