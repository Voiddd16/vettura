// 玻璃/镜头雨滴层（独立全屏 pass）。
//
// 参考语义：
// - godotshaders.com/shader/rain-on-glass/：sdEgg SDF、静态滴 + 慢痕 + 快滴
//   三层、SCREEN_TEXTURE 折射、textureLod 动态模糊；
// - `acc-shaders/custom/rain/accRainWindscreen_*`：水膜/折射/雨刷分层。
//
// 许可注意：本文件是按上述公开资料重写的实现，不复制其 HLSL/GLSL 源码；
// 若后续直接引入第三方代码，必须逐帖核对 CC0/MIT/GPLv3 并写进
// THIRD_PARTY_NOTICES。GPLv3 代码不得进入闭源构建。
//
// 该层默认关闭；由 RainSystem 的 screen_refraction_enabled 控制。移动端
// 走低配变体（只开两层、降低 textureLod 级别），且必须使用 COPY_MODE_VIEWPORT
// 的 BackBufferCopy，避免旧 Rect 在分辨率变化时采到边缘暖色（右下糊斑根因）。
shader_type canvas_item;
render_mode blend_mix, unshaded;

uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_linear_mipmap;
uniform float intensity : hint_range(0.0, 1.0) = 0.0;
uniform float gravity_speed : hint_range(0.0, 2.0) = 0.25;
uniform float wind_slide : hint_range(-1.0, 1.0) = 0.0;
uniform float blur_strength : hint_range(0.0, 4.0) = 1.0;
uniform int quality : hint_range(0, 2) = 1;

float hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

// 蛋形 SDF：上窄下宽 + 泪尾，比圆点更像真实镜头水滴。
float sdEgg(vec2 p, float r) {
	p.y += 0.34 * p.x * p.x;
	float d = length(p) - r;
	return d;
}

vec2 layer_uv(vec2 uv, vec2 cells, vec2 speed, float wind) {
	vec2 p = uv * cells;
	p.y += TIME * speed.y;
	p.x += TIME * speed.x + uv.y * wind;
	return p;
}

float layer_mask(vec2 uv, vec2 cells, float radius_min, float radius_max,
		float threshold, float slide, out vec2 local) {
	vec2 p = layer_uv(uv, cells, vec2(0.0, slide), wind_slide * 0.08);
	vec2 cell = floor(p);
	vec2 f = fract(p) - 0.5;
	float h = hash12(cell + cells.x * 3.1);
	if (h < threshold) {
		local = vec2(0.0);
		return 0.0;
	}
	float r = mix(radius_min, radius_max, hash12(cell + 7.3));
	vec2 q = f * vec2(1.0, 1.25);
	// 滴尾：沿 y 方向拉长，底部比顶部窄。
	q.y += 0.16 * q.x * q.x;
	float d = sdEgg(q, r);
	local = q;
	return smoothstep(r * 0.85, r * 0.15, d) * mix(0.35, 1.0, h);
}

void fragment() {
	vec2 uv = clamp(UV, vec2(0.0), vec2(1.0));
	if (intensity <= 0.001) {
		COLOR = vec4(0.0);
	} else {
		vec2 local = vec2(0.0);
		// 静态滴：大、慢；慢痕：细长；快滴：小、滑得快。移动端只开前两层。
		float mask = layer_mask(uv, vec2(17.0, 9.0), 0.09, 0.20, 0.80, gravity_speed * 0.15, local);
		float streak = layer_mask(uv, vec2(9.0, 3.0), 0.035, 0.075, 0.72,
			gravity_speed * 0.55, local);
		if (quality >= 1) {
			streak = max(streak, layer_mask(uv, vec2(28.0, 16.0), 0.02, 0.05, 0.86,
				gravity_speed * 1.25, local) * 0.65);
		}
		float total = clamp(mask + streak, 0.0, 1.0);
		if (total <= 0.001) {
			COLOR = vec4(0.0);
		} else {
			// 法线近似：滴心向外偏移背景；边缘为负，形成透镜倒像。
			vec2 offset = normalize(local + vec2(0.0001)) * (0.0018 + 0.0045 * total);
			// 动态模糊：水痕越密/越快，mip 越高，避免高频雨滴闪烁。
			float lod = clamp(mix(0.0, 1.6, total) * blur_strength, 0.0, 2.0);
			vec2 sample_uv = clamp(uv + offset, vec2(0.0), vec2(1.0));
			vec3 refr = textureLod(screen_texture, sample_uv, lod).rgb;
			COLOR = vec4(refr, total * intensity);
		}
	}
}
