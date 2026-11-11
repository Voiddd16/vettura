// 流光假反射车漆：程序化“天空条带”反射，无 cubemap，移动端友好。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#include "res://stylized/common/noise.gdshaderinc"

uniform vec4 base_color : source_color = vec4(0.15, 0.16, 0.18, 1.0);
uniform vec3 sky_top : source_color = vec3(0.25, 0.45, 0.75);
uniform vec3 sky_bottom : source_color = vec3(0.90, 0.85, 0.80);
uniform float streak_strength : hint_range(0.0, 2.0) = 0.60;
uniform float streak_scale : hint_range(0.5, 12.0) = 6.0;
uniform float metallic : hint_range(0.0, 1.0) = 0.80;
uniform float roughness : hint_range(0.0, 1.0) = 0.18;

void fragment() {
	vec3 r = reflect(-VIEW, normalize(NORMAL));
	vec3 sky = mix(sky_bottom, sky_top, clamp(r.y * 0.5 + 0.5, 0.0, 1.0));
	float streak = 0.65 + 0.35 * vfx_fbm(vec2(atan(r.z, r.x) * 2.0, r.y * streak_scale), 3);
	ALBEDO = base_color.rgb + sky * streak * streak_strength * (1.0 - roughness);
	METALLIC = metallic;
	ROUGHNESS = roughness;
}
