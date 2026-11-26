// 湿路面：单一 global `wetness` 驱动的 PBR 地面材质。
//
// 参考 CSP RainFX 的三类表面语义（SOAKING / SMOOTH / ROUGH）：
// - 沥青/混凝土走 SOAKING：albedo 平方变暗，roughness 降到水膜区间；
// - 白线/黄线同样接收 wetness，避免雨天仍纯白；
// - 草的 ROUGH 由 grass_blade.gdshader 自己处理，不在这里。
//
// 材质由 WeatherRig 在运行时从原 StandardMaterial3D 复制贴图/颜色生成；
// 因此这里不做 AC shader 语义，只保留 albedo/normal/AO 与 PBR 参数。
shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

global uniform float wetness;

uniform sampler2D albedo_texture : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_texture : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D ao_texture : hint_default_white, filter_linear_mipmap, repeat_enable;
uniform sampler2D wetness_mask : hint_default_white, filter_linear_mipmap, repeat_enable;

uniform vec4 albedo_color : source_color = vec4(1.0);
uniform vec2 uv_scale = vec2(1.0);
uniform float normal_scale = 1.0;
uniform float base_roughness = 0.85;
uniform float base_specular = 0.20;
uniform float ao_strength = 1.0;
uniform float mask_scale = 0.055;
uniform float is_line = 0.0;

void fragment() {
	vec2 uv = UV * uv_scale;
	vec4 tex = texture(albedo_texture, uv);
	vec3 albedo = tex.rgb * albedo_color.rgb;

	// 低洼/车辙遮罩：R 通道高 = 更容易积水。没有 mask 时 hint_default_white
	// 会让全场均匀变湿，至少不会出现“只有局部像塑料”的断层。
	float mask = texture(wetness_mask, UV * mask_scale).r;
	mask = smoothstep(0.30, 0.78, mask);
	float w = clamp(wetness * mix(0.55, 1.0, mask), 0.0, 1.0);

	// CSP SOAKING：湿沥青不是简单乘 0.8，而是反照率整体压暗（平方项）。
	vec3 wet_albedo = albedo * albedo;
	// 白/黄线的 albedo 接近 1，平方后仍是 1；额外按亮度压一档，保证
	// 雨天标线不会保持纯白。暗沥青不受影响。
	float peak = max(albedo.r, max(albedo.g, albedo.b));
	float bright = smoothstep(0.55, 0.95, peak);
	ALBEDO = mix(albedo, wet_albedo, smoothstep(0.05, 0.90, w));
	ALBEDO *= mix(1.0, mix(1.0, 0.55, bright), smoothstep(0.05, 0.85, w));

	// 水膜把微观法线压平；积水区几乎镜面。
	vec3 n = texture(normal_texture, uv).rgb;
	vec3 flat_normal = vec3(0.5, 0.5, 1.0);
	NORMAL_MAP = mix(n, flat_normal, smoothstep(0.10, 0.75, w));
	NORMAL_MAP_DEPTH = mix(normal_scale, 0.12, smoothstep(0.10, 0.85, w));

	float puddle = smoothstep(0.45, 0.90, mask);
	float wet_roughness = mix(0.24, 0.14, puddle);
	ROUGHNESS = mix(base_roughness, wet_roughness, smoothstep(0.15, 0.90, w));
	SPECULAR = mix(base_specular, 0.72, smoothstep(0.10, 0.85, w));
	METALLIC = 0.0;
	// 湿沥青的反射沿车辙/道路方向拉长，而不是一团圆斑。
	ANISOTROPY = mix(0.0, 0.88, smoothstep(0.15, 0.90, w));
	ANISOTROPY_FLOW = vec2(0.0, 1.0);

	// 雨天的 AO 仍保留接触阴影，但水膜会填平微观凹陷，逐步抬到 1.0。
	float ao = texture(ao_texture, UV).r;
	AO = mix(mix(1.0, ao, ao_strength), 1.0, smoothstep(0.20, 0.95, w));
	AO_LIGHT_AFFECT = mix(0.0, 0.35, w);

	// 标线在高湿度下也要变滑：给它们一个额外的低粗糙度偏置，避免纯白边。
	if (is_line > 0.5) {
		ROUGHNESS = min(ROUGHNESS, mix(0.30, 0.10, w));
	}
}
