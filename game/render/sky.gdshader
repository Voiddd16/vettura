// 程序化天空：大气渐变 + 太阳/月亮 + 两层云 + 夜间星空。
//
// 目标不是复刻 CSP 的 `clouds.fx`，而是给 IBL 一份“有结构”的辐射源：
// 纯渐变天空会让车漆/湿路面/玻璃只剩一条死板的反射带，云层和太阳盘能把
// 环境反射做出层次。参考 `mods/imitate/acc-weatherfx-base`：
// - 天色/云量/雾由天气驱动（`weather_visuals.gd` 写 uniform）；
// - 云层有亮面/暗面（用朝太阳方向的第二次噪声采样近似自阴影）；
// - 夜间压暗天空并叠星点，避免夜景变成纯黑剪影。
//
// 性能约定：
// - **不使用 `TIME`**。Godot 的 sky 着色器只要引用 TIME，辐射 cubemap 会
//   每帧重建；这里把云层漂移做成 `cloud_offset` uniform，由 GDScript 每帧
//   推进，并用 Sky 的 INCREMENTAL 模式分摊 cubemap 更新。
// - 云层噪声来自预生成的 `cloud_noise` 纹理，不在每像素里跑多层 value noise。
// - 云层只在 `EYEDIR.y > 0` 时计算；地平线以下直接走地面色。
// - 星星用三次哈希点阵，不做逐像素循环。
shader_type sky;

// 天顶/地平线/地面颜色。源色是 sRGB，Godot 会转线性。
uniform vec3 sky_top_color : source_color = vec3(0.10, 0.26, 0.58);
uniform vec3 sky_horizon_color : source_color = vec3(0.52, 0.68, 0.86);
uniform vec3 ground_color : source_color = vec3(0.10, 0.10, 0.11);
uniform vec3 sun_color : source_color = vec3(1.0, 0.93, 0.80);
uniform vec3 cloud_lit_color : source_color = vec3(0.95, 0.96, 0.98);
uniform vec3 cloud_shadow_color : source_color = vec3(0.36, 0.40, 0.47);
uniform vec3 moon_color : source_color = vec3(0.72, 0.80, 1.0);

// 云的覆盖度/厚度/尺度。覆盖度 0 是晴空，1 是满云。
uniform float cloud_coverage : hint_range(0.0, 1.0) = 0.32;
uniform float cloud_opacity : hint_range(0.0, 1.0) = 0.92;
uniform float cloud_sharpness : hint_range(0.01, 0.8) = 0.22;
uniform float cloud_scale : hint_range(0.2, 4.0) = 1.0;
uniform float cloud_altitude : hint_range(0.5, 3.0) = 1.0;
uniform vec2 cloud_offset = vec2(0.0);
// 风的次级层（高层卷云）相对主层的速度和尺度。
uniform float cloud_second_layer : hint_range(0.0, 1.0) = 0.45;
// 云层质量：1 = 桌面（多层采样 + 自阴影），0 = 移动端（单层采样）。
// 由 `game/render/render_quality.gd` 按后端写进天空材质。
uniform float cloud_quality : hint_range(0.0, 1.0) = 1.0;
// 预生成的 fBm 噪声纹理（NoiseTexture2D，seamless）。天空 shader 不再在
// 每个像素里跑 5 层 value noise——那是之前云层卡顿的主因。
uniform sampler2D cloud_noise : repeat_enable, filter_linear_mipmap;
uniform float cloud_noise_scale : hint_range(0.02, 1.0) = 0.16;

// 太阳盘/光晕强度，以及夜间要素。
uniform float sun_disc_intensity : hint_range(0.0, 4.0) = 1.0;
uniform float sun_glow_intensity : hint_range(0.0, 4.0) = 0.8;
uniform float night : hint_range(0.0, 1.0) = 0.0;
uniform float star_intensity : hint_range(0.0, 2.0) = 0.0;
uniform float moon_intensity : hint_range(0.0, 4.0) = 0.0;
uniform vec3 moon_direction = vec3(0.35, 0.45, -0.82);
uniform float horizon_haze : hint_range(0.0, 1.0) = 0.45;
uniform float sky_exposure : hint_range(0.1, 4.0) = 1.0;

float hash13(vec3 p) {
	p = fract(p * 0.1031);
	p += dot(p, p.yzx + 33.33);
	return fract((p.x + p.y) * p.z);
}

// 云层噪声：一次纹理采样代替过去的 5 层 value noise。
// `cloud_noise` 本身已是 FastNoiseLite 的 fBm（5 octaves），再叠 2~3 个
// 不同尺度的采样就有足够的细节。
float noise_at(vec2 p) {
	return texture(cloud_noise, p * cloud_noise_scale).r;
}

// 方向点阵星场：把方向量化到格子里，每格最多一颗星。
float stars(vec3 dir, float scale, float threshold) {
	vec3 p = dir * scale;
	vec3 cell = floor(p);
	vec3 local = fract(p) - 0.5;
	float h = hash13(cell);
	float seed = hash13(cell + 19.19);
	float present = step(threshold, h);
	// 越靠近格心越亮，形成点状而不是方块。
	float core = 1.0 - smoothstep(0.02, 0.42, length(local));
	return present * core * core * (0.35 + 0.65 * seed);
}

vec3 sky_gradient(vec3 dir, vec3 sun_dir) {
	float height = clamp(dir.y, -1.0, 1.0);
	// 地平线附近色带变化最快，pow < 1 让天顶过渡更自然。
	float up = pow(clamp(height, 0.0, 1.0), 0.42);
	vec3 color = mix(sky_horizon_color, sky_top_color, up);

	// 太阳侧的地平线偏暖（瑞利散射的廉价近似）。
	vec2 flat_dir = normalize(vec2(dir.x, dir.z) + vec2(0.0001));
	vec2 flat_sun = normalize(vec2(sun_dir.x, sun_dir.z) + vec2(0.0001));
	float sun_side = pow(max(dot(flat_dir, flat_sun), 0.0), 3.0);
	float horizon_band = exp(-abs(height) * 9.0);
	color = mix(color, sun_color, sun_side * horizon_band * 0.22 * (1.0 - night));

	// 地平线雾带：远山/建筑与天空的接缝不再是一条硬色差。
	color = mix(color, sky_horizon_color, horizon_band * horizon_haze);
	// 地平线以下用地面色，避免镜头俯视时出现一片突兀的亮色。
	// （反向 smoothstep 在 Metal 上是未定义行为，这里写正向再取反。）
	color = mix(color, ground_color, 1.0 - smoothstep(-0.12, 0.01, height));

	// 夜空整体压暗、偏蓝。
	color = mix(color, color * vec3(0.16, 0.20, 0.34), night * 0.85);
	return color;
}

vec3 cloud_layer(vec3 dir, vec3 sun_dir) {
	if (dir.y <= 0.002) {
		return vec3(0.0);
	}
	// 把视线投影到一块云平面：y 越大投影越远，形成自然的透视压缩。
	float altitude = max(dir.y, 0.02) * cloud_altitude;
	vec2 p = dir.xz / (altitude + 0.10) * cloud_scale + cloud_offset;
	float threshold = mix(0.80, 0.28, clamp(cloud_coverage, 0.0, 1.0));

	// 移动端：单次纹理采样，不做第二次自阴影采样。
	if (cloud_quality < 0.5) {
		float base = noise_at(p);
		float coverage = smoothstep(threshold, threshold + max(cloud_sharpness, 0.02), base);
		vec2 sun_flat = normalize(vec2(sun_dir.x, sun_dir.z) + vec2(0.0001));
		float rim = pow(max(dot(normalize(vec2(dir.x, dir.z) + vec2(0.0001)), sun_flat), 0.0), 6.0);
		float light_amount = clamp(0.55 + (base - 0.5) * 0.9, 0.0, 1.0)
			* (0.35 + 0.65 * max(sun_dir.y, 0.0));
		vec3 cloud = mix(cloud_shadow_color, cloud_lit_color, light_amount);
		cloud = mix(cloud, cloud_lit_color * sun_color, rim * light_amount * 0.4);
		cloud = mix(cloud, cloud * vec3(0.20, 0.24, 0.34), night * 0.9);
		float horizon_fade = smoothstep(0.0, 0.09, dir.y);
		return cloud * coverage * horizon_fade * cloud_opacity;
	}

	float base = noise_at(p);
	float detail = noise_at(p * 2.65 + vec2(13.0, 7.0)) * 0.35;
	float density = base * 0.92 + detail;
	float coverage = smoothstep(threshold, threshold + max(cloud_sharpness, 0.02), density);

	// 朝太阳方向再采一次：差值近似云团的自阴影，比固定颜色更有体积感。
	vec2 sun_flat = normalize(vec2(sun_dir.x, sun_dir.z) + vec2(0.0001));
	float toward_sun = noise_at(p + sun_flat * 0.055 + vec2(0.0, 0.03));
	float light_amount = clamp(0.55 + (base - toward_sun) * 3.2, 0.0, 1.0);
	light_amount *= 0.35 + 0.65 * max(sun_dir.y, 0.0);

	// 高层卷云：同一套噪声的不同尺度/相位，覆盖率低，补一点丝状结构。
	float high = noise_at(p * 0.42 + cloud_offset * 0.35 + vec2(31.0, 17.0));
	float high_coverage = smoothstep(0.52, 0.92, high) * cloud_second_layer * 0.45;

	vec3 cloud = mix(cloud_shadow_color, cloud_lit_color, light_amount);
	// 太阳方向的云边缘染色。
	vec2 cloud_dir_flat = normalize(vec2(dir.x, dir.z) + vec2(0.0001));
	float rim = pow(max(dot(cloud_dir_flat, sun_flat), 0.0), 6.0);
	cloud = mix(cloud, cloud_lit_color * sun_color, rim * light_amount * 0.45);
	// 夜间云层变暗，保留一点月光边缘。
	cloud = mix(cloud, cloud * vec3(0.20, 0.24, 0.34), night * 0.9);
	cloud += moon_color * rim * night * moon_intensity * 0.12;

	float horizon_fade = smoothstep(0.0, 0.09, dir.y);
	float amount = clamp(coverage + high_coverage, 0.0, 1.0) * horizon_fade * cloud_opacity;
	return cloud * amount;
}

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 sun_dir = LIGHT0_ENABLED ? normalize(LIGHT0_DIRECTION) : normalize(vec3(0.32, 0.58, -0.75));

	vec3 color = sky_gradient(dir, sun_dir);

	// 星空：只在夜间可见，三层不同尺度避免颗粒感太机械。
	if (night > 0.01 && star_intensity > 0.001) {
		float star = stars(dir, 380.0, 0.986);
		star += stars(dir, 640.0, 0.992) * 0.6;
		star += stars(dir, 220.0, 0.980) * 1.2;
		// 银河：一条略亮的噪声带，权重压得很低，只是给夜空一点结构。
		float band = noise_at(vec2(atan(dir.x, dir.z) * 16.0, dir.y * 42.0));
		float milky = smoothstep(0.62, 0.92, band) * smoothstep(0.0, 0.25, dir.y) * 0.18;
		color += (vec3(0.72, 0.80, 1.0) * star * 1.6 + vec3(0.45, 0.52, 0.72) * milky)
			* star_intensity * night;
	}

	// 太阳：光晕 + 圆盘。LIGHT0_SIZE 是角直径（弧度），默认 0.5°。
	if (LIGHT0_ENABLED && !AT_CUBEMAP_PASS) {
		float sun_dot = clamp(dot(dir, sun_dir), -1.0, 1.0);
		float sun_angle = acos(sun_dot);
		// LIGHT0_SIZE 是太阳的角直径；画圆盘要用半径，别把直径当半径（会大一倍）。
		float disc_radius = max(LIGHT0_SIZE * 0.5, 0.00435);
		float disc = 1.0 - smoothstep(disc_radius * 0.72, disc_radius * 1.35, sun_angle);
		float glow = pow(max(sun_dot, 0.0), 12.0) * 0.22 + pow(max(sun_dot, 0.0), 90.0) * 0.55;
		vec3 sun_radiance = sun_color * LIGHT0_COLOR * max(LIGHT0_ENERGY, 0.0);
		color += sun_radiance * (disc * 9.0 * sun_disc_intensity + glow * sun_glow_intensity);
	} else if (AT_CUBEMAP_PASS && LIGHT0_ENABLED) {
		// cubemap 里也放一个柔和光晕，车漆/水面的反射才有方向性高光。
		float sun_dot = clamp(dot(dir, sun_dir), -1.0, 1.0);
		color += sun_color * LIGHT0_COLOR * max(LIGHT0_ENERGY, 0.0)
			* pow(max(sun_dot, 0.0), 18.0) * 0.55 * sun_glow_intensity;
	}

	// 月亮：固定方向的小圆盘，与太阳分开，夜间的“第二光源”。
	if (night > 0.01 && moon_intensity > 0.001) {
		vec3 moon_dir = normalize(moon_direction);
		float moon_dot = clamp(dot(dir, moon_dir), -1.0, 1.0);
		float moon_angle = acos(moon_dot);
		float moon_disc = 1.0 - smoothstep(0.010, 0.016, moon_angle);
		float moon_halo = pow(max(moon_dot, 0.0), 160.0) * 0.35;
		color += moon_color * (moon_disc * 2.2 + moon_halo) * moon_intensity * night;
	}

	// 云层叠在渐变与太阳光晕之上；云层自身已经按天光调过色。
	color += cloud_layer(dir, sun_dir);

	COLOR = max(color, vec3(0.0)) * sky_exposure;
}
