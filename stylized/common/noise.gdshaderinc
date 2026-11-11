// 通用噪声工具（2D）：hash / value noise / fbm。
// 所有函数带 vfx_ 前缀，避免与使用方 shader 内的函数冲突。
// 用法：#include "res://stylized/common/noise.gdshaderinc"

float vfx_hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

float vfx_noise2d(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	float a = vfx_hash12(i);
	float b = vfx_hash12(i + vec2(1.0, 0.0));
	float c = vfx_hash12(i + vec2(0.0, 1.0));
	float d = vfx_hash12(i + vec2(1.0, 1.0));
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 分形叠加；octaves 建议 1..4
float vfx_fbm(vec2 p, int octaves) {
	float v = 0.0;
	float amp = 0.5;
	for (int i = 0; i < octaves; i++) {
		v += amp * vfx_noise2d(p);
		p = p * 2.0 + vec2(17.7);
		amp *= 0.5;
	}
	return v;
}
