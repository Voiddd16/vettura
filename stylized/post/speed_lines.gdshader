// 速度线：径向条纹随“车速”渐显（speed 0..1 由游戏注入）。
// 注：噪声在角度维采样，π 接缝处条纹不连续，风格化场景可接受。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear;
uniform float speed : hint_range(0.0, 1.0) = 0.5;   // 车速感（游戏侧驱动）
uniform float strength : hint_range(0.0, 1.0) = 0.60;
uniform vec2 center = vec2(0.5, 0.5);

#include "res://stylized/common/noise.gdshaderinc"

void fragment() {
	vec2 p = SCREEN_UV - center;
	float d = length(p);
	float a = atan(p.y, p.x);
	float n = vfx_fbm(vec2(a * 40.0, TIME * 6.0), 3);
	float streak = smoothstep(0.55, 0.95, n) * smoothstep(0.30, 0.85, d) * speed * strength;
	vec3 col = texture(screen_tex, SCREEN_UV).rgb;
	col += streak * vec3(0.75, 0.85, 1.00) * 0.6;
	COLOR = vec4(col, 1.0);
}
