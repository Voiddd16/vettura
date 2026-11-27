extends Node
## 显示管线冒烟：能力探测、上采样回退、动态分辨率范围、VRS/XR 状态。
##
## ```bash
## godot --headless --path . res://game/display_smoke.tscn
## ```

const UPSCALERS := [
	"auto",
	"native",
	"fsr1",
	"fsr2",
	"fsr3_1",
	"metalfx_spatial",
	"metalfx_temporal",
]


func _ready() -> void:
	var failures: Array[String] = []
	DisplayPipeline.refresh()
	var caps := DisplayPipeline.capabilities()
	var upscalers: Dictionary = caps.get("upscalers", {})

	if not bool(upscalers.get("native", {}).get("available", false)):
		failures.append("native upscaler unavailable")
	if DisplayPipeline.resolve_upscaler("native") != "native":
		failures.append("native resolve failed")

	# 不可用上采样器必须回退到已注册项，而不是返回空/抛错。
	for id in UPSCALERS:
		var resolved := DisplayPipeline.resolve_upscaler(id)
		if not upscalers.has(resolved):
			failures.append("resolve(%s) -> unknown %s" % [id, resolved])

	# 显式配置一个可预测的基线：原生 + 关 VRS + 关动态分辨率。
	DisplayPipeline.configure({
		"fullscreen": false,
		"requested_upscaler": "native",
		"vrs": "off",
		"render_scale": 0.8,
		"dynamic": false,
		"target_fps": 60,
	})
	var state := DisplayPipeline.state()
	if String(state.get("active_upscaler", "")) != "native":
		failures.append("active upscaler = %s" % state.get("active_upscaler"))
	if absf(float(state.get("scale", 0.0)) - 0.8) > 0.001:
		failures.append("scale = %s" % state.get("scale"))
	if bool(state.get("dynamic", true)):
		failures.append("dynamic should be off")
	if String(state.get("vrs", "")) != "off":
		failures.append("vrs = %s" % state.get("vrs"))

	# 打开动态分辨率：初始 scale 必须落在 [min, max] 内。
	DisplayPipeline.configure({
		"fullscreen": true,
		"requested_upscaler": "native",
		"vrs": "off",
		"render_scale": 1.0,
		"dynamic": true,
		"target_fps": 60,
	})
	state = DisplayPipeline.state()
	var scale := float(state.get("scale", 0.0))
	var min_scale := float(state.get("dynamic_min", 0.35))
	var max_scale := float(state.get("dynamic_max", 1.0))
	if scale < min_scale - 0.001 or scale > max_scale + 0.001:
		failures.append("dynamic scale %.2f outside %.2f..%.2f" % [
			scale, min_scale, max_scale])

	# 状态机：低帧率持续 1 s 应降一档，高帧率持续 3 s 应升一档。
	var before := DisplayPipeline.scale()
	DisplayPipeline.debug_feed_fps(20.0)
	DisplayPipeline.debug_feed_fps(20.0)
	var after_down := DisplayPipeline.scale()
	if after_down >= before - 0.001:
		failures.append("dynamic down did not apply (%.2f -> %.2f)" % [before, after_down])
	for _i in 6:
		DisplayPipeline.debug_feed_fps(60.0)
	var after_up := DisplayPipeline.scale()
	if after_up <= after_down + 0.001:
		failures.append("dynamic up did not apply (%.2f -> %.2f)" % [after_down, after_up])

	# manifest 扫描：enabled 但没有桥接 singleton 时只能被识别，不能启用 FSR3.1。
	_check_manifest_scan(failures)

	# 恢复用户设置，避免冒烟场景把临时配置留在 autoload 里。
	DisplayPipeline.apply_profile()

	if failures.is_empty():
		var mods: Array = caps.get("engine_mods", [])
		print("[display-smoke] PASS upscalers=%s vrs=%s xr=%s engine_mods=%d" % [
			",".join(_available_upscalers(upscalers)),
			str(caps.get("vrs", {})),
			str(caps.get("xr", false)),
			mods.size(),
		])
		get_tree().quit(0)
	else:
		for failure in failures:
			push_error("[display-smoke] " + failure)
		print("[display-smoke] FAIL")
		get_tree().quit(1)


func _check_manifest_scan(failures: Array[String]) -> void:
	var mod_root := "user://engine/mods/display_smoke_tmp"
	var manifest_path := mod_root.path_join("manifest.json")
	var error := DirAccess.make_dir_recursive_absolute(mod_root)
	if error != OK:
		failures.append("cannot create manifest test dir: %d" % error)
		return
	var file := FileAccess.open(manifest_path, FileAccess.WRITE)
	if file == null:
		failures.append("cannot write manifest test file")
		DirAccess.remove_absolute(mod_root)
		return
	file.store_string(JSON.stringify({
		"api": 1,
		"id": "display_smoke_tmp",
		"name": "display smoke manifest",
		"enabled": true,
		"provides": ["upscaler.fsr3_1"],
		"bridge": {"singleton": "VetturaEngineModDisplaySmokeMissing"},
	}))
	file.close()

	DisplayPipeline.refresh()
	var caps := DisplayPipeline.capabilities()
	var found := false
	var mods: Array = caps.get("engine_mods", [])
	for mod in mods:
		if String(mod.get("id", "")) == "display_smoke_tmp":
			found = true
			break
	if not found:
		failures.append("manifest scanner did not find display_smoke_tmp")
	if bool(caps.get("upscalers", {}).get("fsr3_1", {}).get("available", true)):
		failures.append("manifest without bridge must not enable fsr3_1")

	DirAccess.remove_absolute(manifest_path)
	DirAccess.remove_absolute(mod_root)
	DisplayPipeline.refresh()


func _available_upscalers(upscalers: Dictionary) -> Array:
	var out: Array[String] = []
	for id in upscalers:
		if bool(upscalers[id].get("available", false)):
			out.append(String(id))
	return out
