// 车底接触阴影：一个跟随车辆的世界空间软圆盘。
//
// 阴雨时 DirectionalLight3D 的 shadow_opacity 被压到 0.1~0.2，真实的软阴影
// 几乎不可见；用这个低成本 blob 保持“车身贴地”。它不是方向光阴影的替代，
// 只是 Mobile 无 SSAO 时的接触阴影兜底；桌面 SSAO 仍开着。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform float strength : hint_range(0.0, 1.0) = 0.2;
uniform vec3 shadow_color : source_color = vec3(0.0, 0.0, 0.0);

void fragment() {
	vec2 p = UV * 2.0 - 1.0;
	// 长轴沿车身方向，短轴横向；二次衰减比硬圆更像接触阴影。
	float d = length(p * vec2(1.35, 0.85));
	float a = pow(max(0.0, 1.0 - d), 1.7);
	ALBEDO = shadow_color;
	ALPHA = a * strength;
}
