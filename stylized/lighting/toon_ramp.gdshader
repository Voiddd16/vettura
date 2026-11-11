// 卡通分档光照：硬切明暗 + 可调色阶数 + 阴影常亮色（风格化渲染用）。
shader_type spatial;
render_mode cull_back;

uniform vec4 base_color : source_color = vec4(0.70, 0.75, 0.85, 1.0);
uniform vec4 shadow_color : source_color = vec4(0.28, 0.30, 0.42, 1.0);
uniform int steps : hint_range(2, 8) = 3;   // 明暗色阶数

void light() {
	float ndl = dot(NORMAL, LIGHT);
	float lit = smoothstep(0.0, 0.2, ndl);
	lit = floor(lit * float(steps)) / max(float(steps) - 1.0, 1.0);
	// 阴影区保留 35% 光色，避免纯黑
	vec3 lc = LIGHT_COLOR * clamp(ATTENUATION, 0.35, 1.0);
	DIFFUSE_LIGHT = mix(shadow_color.rgb, base_color.rgb, lit) * lc;
}
