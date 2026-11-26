// GrassFX 草卡片（MultiMesh billboard）。
//
// 参考：
// - `mods/imitate/acc-shaders/recreated/ksGrass_ps.fx`：txVariation 世界色斑、
//   RAINFX_WET、根部/叶片颜色层次；
// - `mods/imitate/acc-shaders/custom_objects/common/smoothA2C.hlsl`：A2C 软边；
// - GPUOpen 程序化草：保长弯曲、远处加宽补偿、法线朝天软化、根部自阴影。
//
// 硬约束：草属于 CSP ROUGH 类。wetness 只允许变暗 + 轻微贴伏，不允许
// 增加 roughness/specular 来“反光”。wetness 来自 project.godot 的 global
// uniform，由 WeatherRig 统一广播。
shader_type spatial;
render_mode cull_disabled, depth_draw_opaque, alpha_to_coverage;

uniform sampler2D grass_atlas : source_color, filter_linear_mipmap, repeat_disable;
uniform vec4 tint : source_color = vec4(1.0, 1.0, 1.0, 1.0);
uniform float wind_strength : hint_range(0.0, 0.4) = 0.055;
uniform float wind_speed : hint_range(0.0, 4.0) = 1.4;
uniform float fade_start : hint_range(0.0, 200.0) = 46.0;
uniform float fade_end : hint_range(1.0, 250.0) = 74.0;
uniform float variation_strength : hint_range(0.0, 0.5) = 0.14;
uniform float root_darkness : hint_range(0.0, 0.8) = 0.55;
uniform float clump_strength : hint_range(0.0, 1.0) = 0.55;

global uniform float wetness;

varying vec2 v_uv;
varying vec3 v_color;
varying vec3 v_world;
varying float v_height;

float hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

float value_noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	float a = hash12(i);
	float b = hash12(i + vec2(1.0, 0.0));
	float c = hash12(i + vec2(0.0, 1.0));
	float d = hash12(i + vec2(1.0, 1.0));
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 低频团块 + 高频细节；不需要贴图，CPU 散布那层仍负责真正的实例数量。
float clump_noise(vec2 p) {
	return value_noise(p * 0.24) * 0.72 + value_noise(p * 0.73 + 17.0) * 0.28;
}

void vertex() {
	vec3 instance_scale = vec3(
		length(MODEL_MATRIX[0].xyz),
		length(MODEL_MATRIX[1].xyz),
		length(MODEL_MATRIX[2].xyz)
	);
	vec3 world_pos = MODEL_MATRIX[3].xyz;
	vec2 world_xz = world_pos.xz;
	float dist = length((VIEW_MATRIX * vec4(world_pos, 1.0)).xyz);

	// 远处减少每株的视觉贡献，同时把剩余叶片加宽补偿密度；不是直接缩小。
	float far = smoothstep(28.0, 66.0, dist);
	float width_compensation = mix(1.0, 1.85, far);

	// 面向相机；保留实例位置与缩放。
	MODELVIEW_MATRIX = VIEW_MATRIX * mat4(
		vec4(INV_VIEW_MATRIX[0].xyz, 0.0),
		vec4(INV_VIEW_MATRIX[1].xyz, 0.0),
		vec4(INV_VIEW_MATRIX[2].xyz, 0.0),
		vec4(world_pos, 1.0)
	);
	VERTEX *= instance_scale;

	// 世界空间团块：同一片草地有疏有密，株高也有轻微变化。
	float clump = clump_noise(world_xz);
	float clump_mask = smoothstep(0.22, 0.72, clump);
	VERTEX.y *= mix(0.70, 1.15, clump);
	VERTEX.x *= width_compensation;

	// 叶片从根到尖收窄，避免“上下同宽”的纸片。
	float height = UV.y;
	VERTEX.x *= mix(1.0, 0.32, height);

	// 世界空间风场：两个不同频率的噪声方向 + 每株 hash 相位 + 尖端高频域扭曲。
	float phase = hash12(world_xz * 1.71 + 0.37) * 6.2831853;
	float t = TIME * wind_speed;
	vec2 wind_a = vec2(
		value_noise(world_xz * 0.09 + vec2(t * 0.07, 0.0)) - 0.5,
		value_noise(world_xz * 0.11 + vec2(0.0, t * 0.06)) - 0.5
	);
	vec2 wind_b = vec2(
		value_noise(world_xz * 0.31 - vec2(t * 0.11, 0.0)) - 0.5,
		value_noise(world_xz * 0.27 + vec2(0.0, -t * 0.09)) - 0.5
	);
	vec2 wind = (wind_a * 1.6 + wind_b * 0.7) * wind_strength * 2.0;
	float tip_turbulence = (value_noise(world_xz * 1.7 + vec2(t * 0.55, t * 0.31)) - 0.5)
		* wind_strength * height * height;
	float bend = (wind.x + tip_turbulence) * height * height * 2.2;
	// 根部压住、只让上半段弯；wetness 让草轻微贴伏下垂，不是整体缩放。
	float wet = clamp(wetness, 0.0, 1.0);
	float wet_lean = wet * 0.16 * height * height;
	VERTEX.x += bend + wet_lean;
	VERTEX.z += wind.y * height * height * 0.75;
	VERTEX.y -= wet * 0.10 * height * height;

	v_uv = UV;
	v_color = COLOR.rgb;
	v_world = world_pos;
	v_height = height;
}

void fragment() {
	vec4 tex = texture(grass_atlas, v_uv);
	float camera_distance = length(VERTEX);
	float fade = 1.0 - smoothstep(fade_start, fade_end, camera_distance);

	// 世界空间团块决定“这块地长不长草”，避免整片均匀插秧。
	float clump = clump_noise(v_world.xz);
	float clump_mask = smoothstep(0.20, 0.68, clump);
	float coverage = tex.a * fade * mix(1.0 - clump_strength, 1.0, clump_mask);
	ALPHA = coverage;

	// 世界空间低频色斑（ksGrass 的 txVariation 近似）+ 少量枯草/土色。
	float variation = 1.0 + variation_strength * 0.5
		* sin(v_world.x * 0.37 + v_world.z * 0.21)
		* sin(v_world.x * 0.11 - v_world.z * 0.29 + 1.7);
	float dry = smoothstep(0.70, 0.96, value_noise(v_world.xz * 0.63 + 9.7));
	vec3 base = tex.rgb * tint.rgb * v_color * variation;
	vec3 root_col = base * (1.0 - root_darkness);
	vec3 tip_col = base * vec3(1.05, 0.94, 0.75);
	vec3 color = mix(root_col, tip_col, smoothstep(0.0, 1.0, v_height));
	color = mix(color, vec3(0.25, 0.21, 0.11) * (0.7 + 0.3 * variation), dry * 0.32);

	float wet = clamp(wetness, 0.0, 1.0);
	// 去饱和：草不能是荧光绿；保留一点色相但让明度主导。
	float luma = dot(color, vec3(0.299, 0.587, 0.114));
	color = mix(vec3(luma), color, 0.82);
	// ROUGH 类：只变暗，不提高高光/金属度。
	color *= (1.0 - 0.20 * wet);
	ALBEDO = color;

	// 草叶不是薄平面：世界 up 当受光法线，再和卡片法线柔和混合。
	vec3 up_view = normalize((VIEW_MATRIX * vec4(0.0, 1.0, 0.0, 0.0)).xyz);
	NORMAL = normalize(mix(up_view, NORMAL, 0.25));
	BACKLIGHT = tex.rgb * v_color * vec3(0.08, 0.13, 0.04);
	ROUGHNESS = 1.0;
	SPECULAR = 0.03;
}
