// 赛车玻璃：可调透射色 + 菲涅尔反射 + 边缘辉光。
// 用作车窗/风挡的 material_override；移动端安全（不读屏幕纹理）。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#include "res://stylized/common/fresnel_utils.gdshaderinc"

uniform vec4 tint : source_color = vec4(0.05, 0.07, 0.10, 0.35); // 玻璃本体色与不透明度
uniform float reflection_power : hint_range(0.5, 8.0) = 3.0;     // 菲涅尔锐度（越小越泛白）
uniform float reflection_strength : hint_range(0.0, 1.0) = 0.55;
uniform vec3 reflection_color : source_color = vec3(0.60, 0.70, 0.90); // 假天空反射色
uniform float fresnel_bias : hint_range(0.0, 1.0) = 0.04;
uniform float edge_glow : hint_range(0.0, 2.0) = 0.0;            // 轮廓电光（概念车风格）
uniform vec3 edge_glow_color : source_color = vec3(0.30, 0.70, 1.00);

void fragment() {
	float f = vfx_fresnel(NORMAL, VIEW, reflection_power, fresnel_bias);
	vec3 refl = reflection_color * (0.4 + 0.6 * f);
	ALBEDO = mix(tint.rgb, refl, reflection_strength * f);
	ALPHA = clamp(tint.a + f * (1.0 - tint.a) * reflection_strength, 0.0, 1.0);
	ROUGHNESS = mix(0.02, 0.18, f);
	METALLIC = 0.0;
	EMISSION = edge_glow_color * edge_glow * f;
}
