// 车漆换色：HSV 平移（色相/饱和/明度），不依赖贴图，清漆质感。
// 用作车身 material_override，或由 gdext 管线生成换色变体材质。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#include "res://stylized/common/color_utils.gdshaderinc"

uniform vec4 base_color : source_color = vec4(0.80, 0.10, 0.10, 1.0);
uniform float hue_shift : hint_range(-0.5, 0.5) = 0.0;
uniform float saturation : hint_range(0.0, 2.0) = 1.0;
uniform float value_scale : hint_range(0.0, 2.0) = 1.0;
uniform float metallic : hint_range(0.0, 1.0) = 0.70;
uniform float roughness : hint_range(0.0, 1.0) = 0.25;
uniform float clearcoat : hint_range(0.0, 1.0) = 0.40;

void fragment() {
	vec3 hsv = vfx_rgb2hsv(base_color.rgb);
	hsv.x = fract(hsv.x + hue_shift);
	hsv.y = clamp(hsv.y * saturation, 0.0, 1.0);
	hsv.z = clamp(hsv.z * value_scale, 0.0, 1.0);
	ALBEDO = vfx_hsv2rgb(hsv);
	METALLIC = metallic;
	ROUGHNESS = roughness;
	CLEARCOAT = clearcoat;
}
