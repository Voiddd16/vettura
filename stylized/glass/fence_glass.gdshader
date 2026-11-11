// 护栏/围挡玻璃：淡色透射 + 可调网格线（近似 AC ksGlassFence）。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back;

#include "res://stylized/common/fresnel_utils.gdshaderinc"

uniform vec4 tint : source_color = vec4(0.60, 0.90, 0.85, 0.22);
uniform vec2 grid_count = vec2(24.0, 4.0);        // UV 空间网格数
uniform float grid_width : hint_range(0.0, 0.5) = 0.05;
uniform vec3 grid_color : source_color = vec3(0.85, 0.95, 0.95);
uniform float reflection_strength : hint_range(0.0, 1.0) = 0.35;

void fragment() {
	float f = vfx_fresnel(NORMAL, VIEW, 3.0, 0.04);
	vec2 g = abs(fract(UV * grid_count) - 0.5);
	float line = 1.0 - smoothstep(grid_width - 0.02, grid_width + 0.02, min(g.x, g.y));
	vec3 body = mix(tint.rgb, vec3(0.7, 0.85, 0.9), f * reflection_strength);
	ALBEDO = mix(body, grid_color, line * 0.7);
	ALPHA = clamp(tint.a + line * 0.35 + f * 0.25, 0.0, 1.0);
	ROUGHNESS = 0.1;
}
