## 全屏滤镜层：把 stylized/post/*.gdshader 以叠层方式挂到画面上。
##
## 每个 push 出来的滤镜是全屏 ColorRect（忽略鼠标）；后推的滤镜
## 会采样到前面滤镜画完的画面，因此按顺序链式生效。
##
## 用法：
##   const FxLayer := preload("res://stylized/post/fx_layer.gd")
##   var fx := FxLayer.new()
##   fx.layer = 100
##   add_child(fx)
##   var rect := fx.push(preload("res://stylized/post/vignette.gdshader"), {"strength": 0.4})
##   (rect.material as ShaderMaterial).set_shader_parameter("strength", 0.6)
##   fx.pop()      # 移除最后推入的滤镜
##   fx.clear()    # 清空
extends CanvasLayer

## 叠加一个滤镜，返回其全屏 ColorRect（可继续调参）。
func push(shader: Shader, params: Dictionary = {}) -> ColorRect:
	var rect := ColorRect.new()
	rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	rect.set_anchors_preset(Control.PRESET_FULL_RECT)
	var mat := ShaderMaterial.new()
	mat.shader = shader
	for key in params:
		mat.set_shader_parameter(key, params[key])
	rect.material = mat
	add_child(rect)
	return rect

## 移除最后推入的滤镜。
func pop() -> void:
	var children := get_children()
	if not children.is_empty():
		children[-1].queue_free()

## 清空所有滤镜。
func clear() -> void:
	for child in get_children():
		child.queue_free()
