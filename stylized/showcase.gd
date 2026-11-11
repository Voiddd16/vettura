## 风格化渲染框架演示页：六大类效果各一个展台 + HUD 仪表 + 全屏滤镜链。
## 不依赖 mods/ 真车资产，纯合成几何。
##
## 运行（桌面窗口，看实际效果）：
##   /Applications/Godot.app/Contents/MacOS/Godot --path . res://stylized/showcase.tscn --quit-after 600
## 冒烟（无窗口，验证资源/脚本/场景树）：
##   /Applications/Godot.app/Contents/MacOS/Godot --headless --path . res://stylized/showcase.tscn --quit-after 60
extends Node3D

const FxLayer := preload("res://stylized/post/fx_layer.gd")
const RACING_GLASS := preload("res://stylized/glass/racing_glass.gdshader")
const FENCE_GLASS := preload("res://stylized/glass/fence_glass.gdshader")
const HUD_PANEL := preload("res://stylized/hud/hud_panel.gdshader")
const GAUGE_RING := preload("res://stylized/hud/gauge_ring.gdshader")
const SCANLINE := preload("res://stylized/hud/scanline.gdshader")
const PAINT_HUE := preload("res://stylized/color/paint_hue.gdshader")
const LIVERY_MASK := preload("res://stylized/color/livery_mask.gdshader")
const OUTLINE := preload("res://stylized/shape/outline.gdshader")
const WIND_SWAY := preload("res://stylized/shape/wind_sway.gdshader")
const RIM_LIGHT := preload("res://stylized/lighting/rim_light.gdshader")
const TOON_RAMP := preload("res://stylized/lighting/toon_ramp.gdshader")
const FAKE_REFLECTION := preload("res://stylized/lighting/fake_reflection.gdshader")
const VIGNETTE := preload("res://stylized/post/vignette.gdshader")
const COLOR_GRADE := preload("res://stylized/post/color_grade.gdshader")

var _time := 0.0
var _paint_mat: ShaderMaterial
var _gauge_mat: ShaderMaterial
var _outline_mat: ShaderMaterial

func _ready() -> void:
	_setup_sky_and_light()
	_setup_ground()
	_setup_podiums()
	_setup_hud()
	_setup_post()

func _process(delta: float) -> void:
	_time += delta
	if _paint_mat:
		_paint_mat.set_shader_parameter("hue_shift", 0.5 * sin(_time * 0.4))
	if _gauge_mat:
		_gauge_mat.set_shader_parameter("fill_ratio", 0.5 + 0.45 * sin(_time * 0.8))
	if _outline_mat:
		_outline_mat.set_shader_parameter("width", 0.02 + 0.012 * sin(_time * 2.0))

func _sm(shader: Shader, params: Dictionary = {}) -> ShaderMaterial:
	var mat := ShaderMaterial.new()
	mat.shader = shader
	for key in params:
		mat.set_shader_parameter(key, params[key])
	return mat

func _place(mesh: Mesh, mat: Material, pos: Vector3) -> MeshInstance3D:
	var mi := MeshInstance3D.new()
	mi.mesh = mesh
	mi.material_override = mat
	mi.position = pos
	add_child(mi)
	return mi

func _setup_sky_and_light() -> void:
	var env := Environment.new()
	env.background_mode = Environment.BG_SKY
	var sky_mat := ProceduralSkyMaterial.new()
	sky_mat.sky_top_color = Color(0.25, 0.42, 0.70)
	sky_mat.sky_horizon_color = Color(0.72, 0.78, 0.85)
	sky_mat.ground_bottom_color = Color(0.12, 0.13, 0.15)
	sky_mat.ground_horizon_color = Color(0.60, 0.65, 0.70)
	var sky := Sky.new()
	sky.sky_material = sky_mat
	env.sky = sky
	env.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	env.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	var we := WorldEnvironment.new()
	we.environment = env
	add_child(we)

	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-55.0, 30.0, 0.0)
	sun.shadow_enabled = true
	add_child(sun)

	var cam := Camera3D.new()
	cam.position = Vector3(0.0, 2.4, 7.5)
	add_child(cam)
	cam.look_at(Vector3(0.0, 0.9, 0.0))
	cam.current = true

func _setup_ground() -> void:
	var plane := PlaneMesh.new()
	plane.size = Vector2(24.0, 24.0)
	var mat := StandardMaterial3D.new()
	mat.albedo_color = Color(0.16, 0.17, 0.20)
	mat.roughness = 0.9
	_place(plane, mat, Vector3.ZERO)

func _setup_podiums() -> void:
	# 1) glass/：赛车玻璃球 + 护栏玻璃板
	_place(SphereMesh.new(), _sm(RACING_GLASS), Vector3(-4.5, 1.0, 0.0)).mesh.radius = 0.7
	var fence := BoxMesh.new()
	fence.size = Vector3(2.2, 1.6, 0.06)
	_place(fence, _sm(FENCE_GLASS), Vector3(-2.7, 1.0, 0.0))

	# 2) color/：HSV 车漆立方体（色相动画）+ 遮罩涂装板（程序生成遮罩）
	var cube := BoxMesh.new()
	cube.size = Vector3(1.1, 1.1, 1.1)
	_paint_mat = _sm(PAINT_HUE)
	_place(cube, _paint_mat, Vector3(-0.9, 0.75, 0.0))

	var livery := QuadMesh.new()
	livery.size = Vector2(1.5, 1.1)
	var livery_mat := _sm(LIVERY_MASK)
	livery_mat.set_shader_parameter("mask_tex", _make_mask())
	_place(livery, livery_mat, Vector3(0.9, 1.0, 0.0))

	# 3) shape/：描边（挂在前机盖"概念车"胶囊上，作为 next_pass）
	var hero := CapsuleMesh.new()
	hero.radius = 0.45
	hero.height = 1.6
	var hero_mat := StandardMaterial3D.new()
	hero_mat.albedo_color = Color(0.85, 0.30, 0.20)
	hero_mat.roughness = 0.3
	_outline_mat = _sm(OUTLINE)
	hero_mat.next_pass = _outline_mat
	_place(hero, hero_mat, Vector3(0.0, 1.0, -2.2))

	# 4) shape/：风摆草丛
	var blade := BoxMesh.new()
	blade.size = Vector3(0.05, 1.1, 0.05)
	var sway_mat := _sm(WIND_SWAY)
	for i in 7:
		var off := Vector3(4.5 + 0.5 * sin(float(i) * 1.7), 0.55, 0.5 * cos(float(i) * 2.3))
		_place(blade, sway_mat, off)

	# 5) lighting/：卡通分档球 + 边缘光（next_pass 叠加）
	var toon_mat := _sm(TOON_RAMP, {"steps": 3})
	toon_mat.next_pass = _sm(RIM_LIGHT)
	var toon_sphere := SphereMesh.new()
	toon_sphere.radius = 0.75
	toon_sphere.height = 1.5
	_place(toon_sphere, toon_mat, Vector3(2.7, 0.95, 0.0))

	# 6) lighting/：流光假反射车漆板
	var streak := BoxMesh.new()
	streak.size = Vector3(1.6, 1.1, 0.4)
	_place(streak, _sm(FAKE_REFLECTION), Vector3(4.5, 0.95, -0.6))

func _make_mask() -> ImageTexture:
	# R=主色区 / G=辅色区；真车使用时按车身 UV 绘制遮罩
	var img := Image.create_empty(256, 128, false, Image.FORMAT_RGB8)
	img.fill(Color(0, 0, 0))
	img.fill_rect(Rect2i(0, 0, 110, 128), Color(1, 0, 0))
	img.fill_rect(Rect2i(160, 0, 96, 128), Color(0, 1, 0))
	img.fill_rect(Rect2i(120, 40, 16, 48), Color(0, 1, 0))
	return ImageTexture.create_from_image(img)

func _setup_hud() -> void:
	var hud := CanvasLayer.new()
	hud.layer = 10
	add_child(hud)

	# 面板底板 + 标题
	var panel := ColorRect.new()
	panel.position = Vector2(36.0, 36.0)
	panel.size = Vector2(340.0, 120.0)
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var panel_mat := _sm(HUD_PANEL)
	panel_mat.set_shader_parameter("panel_size", panel.size)
	panel.material = panel_mat
	hud.add_child(panel)

	var label := Label.new()
	label.text = "FX SHOWCASE"
	label.position = Vector2(24.0, 16.0)
	label.add_theme_font_size_override("font_size", 26)
	label.add_theme_color_override("font_color", Color(0.85, 0.95, 1.0))
	panel.add_child(label)

	# 右上角圆环仪表（fill_ratio 在 _process 里动画）
	var gauge := ColorRect.new()
	gauge.anchor_left = 1.0
	gauge.anchor_right = 1.0
	gauge.offset_left = -230.0
	gauge.offset_right = -70.0
	gauge.offset_top = 36.0
	gauge.offset_bottom = 196.0
	gauge.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_gauge_mat = _sm(GAUGE_RING)
	gauge.material = _gauge_mat
	hud.add_child(gauge)

	# 扫描线叠层（最后画，盖住 3D + 上面两项）
	var scan := ColorRect.new()
	scan.set_anchors_preset(Control.PRESET_FULL_RECT)
	scan.mouse_filter = Control.MOUSE_FILTER_IGNORE
	scan.material = _sm(SCANLINE, {"strength": 0.18})
	hud.add_child(scan)

func _setup_post() -> void:
	var fx := FxLayer.new()
	fx.layer = 100
	add_child(fx)
	fx.push(COLOR_GRADE, {"saturation": 1.15, "warmth": 0.10})
	fx.push(VIGNETTE, {"strength": 0.30})
