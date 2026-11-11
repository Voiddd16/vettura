// 边缘光（rim light）叠层：推荐作为原材质的 next_pass。
// 沿法线微外扩，避免与底层几何 z-fighting；加色混合只增不遮。
shader_type spatial;
render_mode blend_add, unshaded, cull_back, depth_draw_never;

uniform vec4 color : source_color = vec4(0.30, 0.80, 1.00, 1.0);
uniform float power : hint_range(0.5, 8.0) = 3.0;
uniform float strength : hint_range(0.0, 4.0) = 1.0;
uniform float offset : hint_range(0.0, 0.05) = 0.01;

void vertex() {
	VERTEX += NORMAL * offset;
}

void fragment() {
	float f = pow(clamp(1.0 - abs(dot(normalize(NORMAL), normalize(VIEW))), 0.0, 1.0), power);
	ALBEDO = color.rgb * f * strength;
}
