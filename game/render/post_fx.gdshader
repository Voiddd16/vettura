// 镜头/传感器后处理：暗角、径向色散、胶片颗粒、轻锐化。
//
// 参考 `mods/imitate/acc-weatherfx-base/shaders/pp_final.fx` 的 ppfilter 配方
// （vignette / chromatic aberration / lens distortion / grain / sharpen），
// 但只保留“没有贴图资产也能跑”的部分：
// - 色彩/曝光/tonemap 交给 `Environment`（ACES + glow），这里不再重复；
// - 效果默认很轻，作为“相机感”的最后一层，不是风格化滤镜；
// - mobile 渲染后端默认关闭（由 `post_fx.gd` 控制），避免带宽浪费。
//
// 注意：这是 canvas_item shader，挂在 3D 之后、UI 之前的 CanvasLayer 上，
// 所以不会把 HUD/按钮一起做暗角。`screen_texture` 用 hint_screen_texture
// 让引擎自动提供当前帧。
//
// `filter_linear`（不是 filter_linear_mipmap）：这里所有采样都固定 LOD 0，
// 请求 mipmap 只会让引擎每帧额外生成一份屏幕纹理 mip 链，画面完全一样。
shader_type canvas_item;
render_mode unshaded;

uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_linear;

// 暗角：0 关闭；0.2~0.35 是“镜头自然衰减”的区间。
uniform float vignette_strength : hint_range(0.0, 1.0) = 0.22;
uniform float vignette_softness : hint_range(0.05, 1.0) = 0.62;

// 径向色散（边缘 RGB 分离）。真实镜头约 0.0002~0.0015；超过 0.003 会像故障艺术。
uniform float chromatic_aberration : hint_range(0.0, 0.01) = 0.0006;

// 胶片颗粒强度（线性亮度上的扰动）；0.01~0.02 隐约可见即可。
uniform float film_grain : hint_range(0.0, 0.1) = 0.012;
uniform float grain_size : hint_range(0.5, 4.0) = 1.25;

// 轻锐化：0 关闭；0.1~0.25 让远景边缘不糊，又不出现白边。
uniform float sharpen : hint_range(0.0, 1.0) = 0.14;

// 桶形畸变（广角镜头）。默认 0，赛车内视角可开到 0.02~0.05。
uniform float lens_distortion : hint_range(-0.2, 0.2) = 0.0;

uniform vec2 viewport_size = vec2(1280.0, 720.0);

float hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

vec3 sample_screen(vec2 uv) {
	return textureLod(screen_texture, clamp(uv, vec2(0.0), vec2(1.0)), 0.0).rgb;
}

void fragment() {
	vec2 uv = SCREEN_UV;
	vec2 centered = uv - 0.5;

	// 桶形/枕形畸变：把采样坐标按半径二次项拉伸。
	float radius2 = dot(centered, centered);
	if (abs(lens_distortion) > 0.00001) {
		vec2 corrected = centered * (1.0 + lens_distortion * radius2);
		uv = corrected + 0.5;
	}

	// 色散按半径增大：中心几乎为零，画面边缘最明显。
	vec2 ca_offset = centered * chromatic_aberration * (0.35 + radius2 * 2.2);
	vec3 color;
	if (chromatic_aberration > 0.00001) {
		color.r = sample_screen(uv + ca_offset).r;
		color.g = sample_screen(uv).g;
		color.b = sample_screen(uv - ca_offset).b;
	} else {
		color = sample_screen(uv);
	}

	// 轻锐化（unsharp mask）：中心 - 四邻域平均，负值裁掉避免振铃。
	if (sharpen > 0.0001) {
		vec2 texel = 1.0 / max(viewport_size, vec2(1.0));
		vec3 blur = (
			sample_screen(uv + vec2(texel.x, 0.0)) +
			sample_screen(uv - vec2(texel.x, 0.0)) +
			sample_screen(uv + vec2(0.0, texel.y)) +
			sample_screen(uv - vec2(0.0, texel.y))
		) * 0.25;
		color = max(color + (color - blur) * sharpen, vec3(0.0));
	}

	// 暗角：半径归一化到画面角落 = 1，softness 控制过渡宽度。
	float aspect = viewport_size.x / max(viewport_size.y, 1.0);
	vec2 vignette_uv = centered * vec2(aspect, 1.0);
	float vignette_radius = length(vignette_uv) / max(length(vec2(aspect, 1.0)) * 0.5, 0.0001);
	float vignette = 1.0 - vignette_strength
		* smoothstep(1.0 - vignette_softness, 1.0, vignette_radius);
	color *= vignette;

	// 胶片颗粒：按屏幕像素量化，随时间滚动；亮度越低颗粒越明显一点。
	if (film_grain > 0.0001) {
		vec2 grain_uv = floor(uv * viewport_size / max(grain_size, 0.5));
		float noise = hash12(grain_uv + vec2(TIME * 37.0, TIME * 17.0)) - 0.5;
		float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
		float weight = mix(1.25, 0.45, smoothstep(0.0, 0.8, luma));
		color += noise * film_grain * weight;
	}

	COLOR = vec4(max(color, vec3(0.0)), 1.0);
}
