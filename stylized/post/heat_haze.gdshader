// 热浪扭曲：地平线区域屏幕扰动（区域/强度可调）。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear_mipmap;
uniform float strength : hint_range(0.0, 1.0) = 0.35;
uniform vec2 region_center = vec2(0.50, 0.62);   // 默认地平线附近
uniform vec2 region_size = vec2(0.90, 0.28);
uniform float speed = 2.0;

#include "res://stylized/common/noise.gdshaderinc"

void fragment() {
	vec2 uv = SCREEN_UV;
	vec2 q = (uv - region_center) / region_size;
	float mask = 1.0 - smoothstep(0.35, 1.0, length(q));
	float n1 = vfx_fbm(uv * vec2(16.0, 10.0) + vec2(TIME * speed, 0.0), 2);
	float n2 = vfx_fbm(uv * vec2(16.0, 10.0) + vec2(-TIME * speed * 0.7, 1.7), 2);
	vec2 off = (vec2(n1, n2) - 0.5) * 0.025 * strength * mask;
	COLOR = vec4(texture(screen_tex, uv + off).rgb, 1.0);
}
