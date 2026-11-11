// 色差（径向 RGB 分离）：速度感/镜头感叠层。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear;
uniform float strength : hint_range(0.0, 1.0) = 0.20;
uniform vec2 center = vec2(0.5, 0.5);

void fragment() {
	vec2 dir = SCREEN_UV - center;
	vec2 off = dir * dot(dir, dir) * 0.03 * strength;
	float r = texture(screen_tex, SCREEN_UV - off).r;
	float g = texture(screen_tex, SCREEN_UV).g;
	float b = texture(screen_tex, SCREEN_UV + off).b;
	COLOR = vec4(r, g, b, 1.0);
}
