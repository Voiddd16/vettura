// 圆环仪表：270° 弧形进度环（转速/速度/电量），挂到方形 ColorRect。
shader_type canvas_item;

uniform float fill_ratio : hint_range(0.0, 1.0) = 0.5;   // 进度 0..1（脚本驱动）
uniform vec4 track_color : source_color = vec4(1.00, 1.00, 1.00, 0.12);
uniform vec4 fill_color : source_color = vec4(0.20, 0.90, 1.00, 1.00);
uniform vec4 warn_color : source_color = vec4(1.00, 0.25, 0.20, 1.00);
uniform float warn_from : hint_range(0.0, 1.0) = 0.85;   // 超过此比例变警告色
uniform float ring_radius : hint_range(0.1, 0.95) = 0.78;
uniform float ring_width : hint_range(0.01, 0.5) = 0.10;
uniform float start_angle = 2.3561945;   // 弧度（135°：左下起）
uniform float sweep_angle = 4.7123890;   // 弧度（270° 顺时针到右下）

void fragment() {
	vec2 p = UV - 0.5;
	float dist = abs(length(p) - ring_radius);
	float ring = 1.0 - smoothstep(ring_width - 0.02, ring_width + 0.02, dist);
	float ang = atan(p.y, p.x);
	if (ang < 0.0) {
		ang += 6.2831853;
	}
	// 屏幕坐标 y 向下：角度顺时针增长，135° 起扫过顶部到 405° 正好是标准仪表弧
	float in_window = step(start_angle, ang);
	float end = start_angle + sweep_angle;
	if (end < 6.2831853) {
		in_window *= step(ang, end);
	}
	float t = clamp((ang - start_angle) / sweep_angle, 0.0, 1.0);
	float fill = ring * in_window * step(t, fill_ratio);
	vec3 fc = fill_color.rgb;
	if (fill_ratio >= warn_from) {
		fc = warn_color.rgb;
	}
	vec4 col = track_color * ring * in_window;
	col.rgb = mix(col.rgb, fc, fill);
	col.a = max(col.a, fill);
	COLOR = col;
}
