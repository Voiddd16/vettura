// 全屏调色：柔性曝光 + 对比/饱和 + 阴影与高光染色 + 暖冷偏移。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear;
uniform float exposure : hint_range(0.2, 3.0) = 1.0;
uniform float contrast : hint_range(0.5, 2.0) = 1.05;
uniform float saturation : hint_range(0.0, 2.0) = 1.10;
uniform vec4 shadows_tint : source_color = vec4(1.0, 1.0, 1.0, 1.0);
uniform vec4 highlights_tint : source_color = vec4(1.0, 1.0, 1.0, 1.0);
uniform float warmth : hint_range(-1.0, 1.0) = 0.0;   // 正=暖，负=冷

void fragment() {
	vec3 c = texture(screen_tex, SCREEN_UV).rgb;
	c = vec3(1.0) - exp(-c * exposure);                    // 柔性曝光
	float lum = dot(c, vec3(0.2126, 0.7152, 0.0722));
	c = mix(vec3(lum), c, saturation);
	c = (c - 0.5) * contrast + 0.5;
	float hi = smoothstep(0.35, 0.85, lum);
	c *= mix(shadows_tint.rgb, highlights_tint.rgb, hi);
	c.r += warmth * 0.06;
	c.b -= warmth * 0.06;
	COLOR = vec4(clamp(c, 0.0, 1.0), 1.0);
}
