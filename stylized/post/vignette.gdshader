// 全屏暗角。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear;
uniform float strength : hint_range(0.0, 1.0) = 0.35;
uniform float radius : hint_range(0.1, 1.0) = 0.55;
uniform float softness : hint_range(0.01, 1.0) = 0.40;
uniform vec4 tint : source_color = vec4(0.00, 0.00, 0.05, 1.0);

void fragment() {
	float d = length(SCREEN_UV - vec2(0.5));
	float v = smoothstep(radius - softness, radius + softness, d) * strength;
	vec3 col = mix(texture(screen_tex, SCREEN_UV).rgb, tint.rgb, v);
	COLOR = vec4(col, 1.0);
}
