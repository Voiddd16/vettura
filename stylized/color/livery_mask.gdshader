// 涂装换色：按遮罩贴图分区改色（R=主色区，G=辅色区，其余=底色）。
// 接真车时遮罩需按车身 UV 对齐；可由 gdext 管线作为 override 材质下发。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform sampler2D mask_tex : source_color, filter_linear;
uniform vec4 base_color : source_color = vec4(0.12, 0.12, 0.14, 1.0);
uniform vec4 primary : source_color = vec4(0.10, 0.35, 0.95, 1.0);
uniform vec4 secondary : source_color = vec4(0.95, 0.85, 0.10, 1.0);
uniform float metallic : hint_range(0.0, 1.0) = 0.50;
uniform float roughness : hint_range(0.0, 1.0) = 0.35;

void fragment() {
	vec3 m = texture(mask_tex, UV).rgb;
	float cover = clamp(m.r + m.g, 0.0, 1.0);
	ALBEDO = primary.rgb * m.r + secondary.rgb * m.g + base_color.rgb * (1.0 - cover);
	METALLIC = metallic;
	ROUGHNESS = roughness;
}
