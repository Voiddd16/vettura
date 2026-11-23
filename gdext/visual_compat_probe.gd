extends Node
## 验证 `AcVisualCompat` 已注册，并能对真实 mod 目录输出兼容报告。
##
## 运行：
##   /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://gdext/visual_compat_probe.tscn

func _ready() -> void:
	var exists := ClassDB.class_exists("AcVisualCompat")
	print("[visual-compat] class_exists=", exists)
	if not exists:
		get_tree().quit(1)
		return

	var compat := AcVisualCompat.new()
	var text: String = compat.inspect_text("res://mods/GT7.HUD.-.v1.3.1", 1)
	print("[visual-compat] report:\n", text.substr(0, 800))
	get_tree().quit(0)
