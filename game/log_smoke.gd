extends Node
## 日志冒烟：验证 `VetturaLogger` GDScript API 与 ring / reload 链路，
## 不依赖 `mods/`（ring 的内容来自 GDExtension 启动 banner）。
##
## 用法：
##
## ```bash
## /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/log_smoke.tscn
## ```
##
## 期望最后一行 `[log-smoke] PASS`，退出码 0。
##
## panic 转储验证（会触发一次真实 Rust panic，进程由布好的定时器退出）：
##
## ```bash
## LOG_SMOKE_PANIC=1 /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://game/log_smoke.tscn
## grep "simulated panic" "$HOME/Library/Application Support/Godot/app_userdata/Vettura/logs/last-crash.log"
## ```
##
## 期望 last-crash.log 里有 `simulated panic`、backtrace 与最近日志段。

var _ok := true

func _ready() -> void:
	var logger := VetturaLogger.new()
	_check_ring(logger)
	_check_set_level(logger)
	_check_dump_to_file(logger)
	_check_reload(logger)

	if OS.get_environment("LOG_SMOKE_PANIC") == "1":
		_trigger_panic(logger)  # panic 会中止本函数；退出由其中布好的定时器负责
		return

	if _ok:
		print("[log-smoke] PASS")
		get_tree().quit(0)
	else:
		push_error("[log-smoke] FAIL")
		get_tree().quit(1)

# ---------------------------------------------------------------- 检查

func _check_ring(logger: VetturaLogger) -> void:
	# GDExtension 启动时已发过 "vettura log online" banner → ring 非空。
	var tail := logger.recent(5)
	var dump := logger.dump()
	var dir := logger.log_dir()
	_ok = _ok and tail.size() >= 1 and dump.contains("vettura log online") and not dir.is_empty()
	if tail.size() == 0:
		push_error("log-smoke: recent(5) 为空，ring 没收到 banner")
	if not dump.contains("vettura log online"):
		push_error("log-smoke: dump() 缺少启动 banner")
	if dir.is_empty():
		push_error("log-smoke: log_dir() 为空")

# set_level：合法级别返回 true，非法级别返回 false。
func _check_set_level(logger: VetturaLogger) -> void:
	var applied := logger.set_level("vettura_kn5", "debug")
	var rejected := logger.set_level("vettura_kn5", "verbose")
	_ok = _ok and applied and not rejected
	if not applied:
		push_error("log-smoke: set_level 合法级别返回 false")
	if rejected:
		push_error("log-smoke: set_level 非法级别返回 true")

func _check_dump_to_file(logger: VetturaLogger) -> void:
	const DUMP_PATH := "user://log-smoke-dump.txt"
	var wrote := logger.dump_to_file(DUMP_PATH)
	_ok = _ok and wrote and FileAccess.file_exists(DUMP_PATH)
	if not wrote:
		push_error("log-smoke: dump_to_file 返回 false")
	elif not FileAccess.file_exists(DUMP_PATH):
		push_error("log-smoke: 转储文件不存在")

# reload：缺文件时 warn + 保留当前配置，返回 true；级别覆盖重载后仍生效。
func _check_reload(logger: VetturaLogger) -> void:
	var reloaded := logger.reload()
	var tail := logger.recent(5)
	_ok = _ok and reloaded and tail.size() >= 1
	if not reloaded:
		push_error("log-smoke: reload 返回 false（subscriber 未安装？）")
	if tail.size() == 0:
		push_error("log-smoke: reload 后 ring 被清空")

# ---------------------------------------------------------------- panic 测试

func _trigger_panic(logger: VetturaLogger) -> void:
	# 先布置退出定时器：simulate_panic 的 panic 被 godot-rust 捕获后，
	# 调用它的 GDScript 函数会中止，但引擎侧定时器照常触发。
	get_tree().create_timer(1.0).timeout.connect(func() -> void:
		print("[log-smoke] PANIC-TEST-DONE (last-crash.log 应已写出)")
		get_tree().quit(0)
	)
	print("[log-smoke] PANIC-TEST-BEGIN")
	logger.simulate_panic()
	print("[log-smoke] 不应到达这里（panic 应已中止本函数）")
