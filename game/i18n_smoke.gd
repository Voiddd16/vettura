extends Node
## i18n 无头冒烟：中文原文 key、英文映射、运行时切换和恢复。
##
## ```bash
## godot --headless --path . res://game/i18n_smoke.tscn
## ```

var _ok := true


func _ready() -> void:
	var original := I18n.language_setting()

	I18n.set_language("zh_CN")
	TranslationServer.set_locale("zh_CN")
	_expect(I18n.t("开始驾驶"), "开始驾驶", "zh 动态翻译")
	_expect(tr("开始驾驶"), "开始驾驶", "zh auto_translate")

	I18n.set_language("en")
	_expect(I18n.t("开始驾驶"), "Start Driving", "en 动态翻译")
	_expect(tr("设置"), "Settings", "en auto_translate")
	_expect(I18n.t("车辆：%s", ["Ferrari"]), "Car: Ferrari", "en 格式化")
	_expect(I18n.t("无"), "No", "en 短词")

	# 所有映射都应该注册成功；中英相同的词（English/ABS/…）跳过。
	var unmapped: Array[String] = []
	for key: String in I18n.EN:
		if I18n.EN[key] == key:
			continue
		if tr(key) == key:
			unmapped.append(key)
	if not unmapped.is_empty():
		_fail("以下 key 没有注册英文翻译：%s" % str(unmapped.slice(0, 8)))

	I18n.set_language("zh_CN")
	_expect(I18n.t("开始驾驶"), "开始驾驶", "切回中文")

	# 恢复用户原来的设置，避免冒烟测试改写语言偏好。
	I18n.set_language(original)

	if _ok:
		print("[i18n-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[i18n-smoke] FAIL")
		get_tree().quit(1)


func _expect(actual: String, expected: String, label: String) -> void:
	if actual != expected:
		_fail("%s: 期望 %s，实际 %s" % [label, expected, actual])


func _fail(message: String) -> void:
	push_error("[i18n-smoke] " + message)
	_ok = false
