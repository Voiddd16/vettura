// HUD 面板底板：圆角盒 + 发光描边（挂到 ColorRect / Panel.material）。
// 注意：panel_size 要设成该 Control 的像素尺寸，圆角与描边才不随长宽比变形。
shader_type canvas_item;

uniform vec2 panel_size = vec2(360.0, 140.0);                    // 像素
uniform vec4 fill : source_color = vec4(0.04, 0.07, 0.11, 0.82);
uniform vec4 border_color : source_color = vec4(0.20, 0.80, 1.00, 1.00);
uniform float border_width : hint_range(0.0, 16.0) = 2.0;        // 像素
uniform float corner_radius : hint_range(0.0, 64.0) = 14.0;      // 像素
uniform float border_glow : hint_range(0.0, 4.0) = 1.5;

float vfx_rounded_box(vec2 p, vec2 b, float r) {
	vec2 q = abs(p) - b + r;
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void fragment() {
	vec2 b = panel_size * 0.5;
	float d = vfx_rounded_box(UV * panel_size - b, b, corner_radius);
	float inside = 1.0 - smoothstep(0.0, 1.0, d);
	float edge = 1.0 - smoothstep(border_width - 1.0, border_width + 1.0, abs(d));
	float glow = exp(-max(abs(d) - border_width, 0.0) * 0.35) * border_glow;
	vec4 col = fill;
	col.rgb = mix(col.rgb, border_color.rgb, edge);
	col.a = max(col.a * inside, max(edge, glow * 0.5) * border_color.a);
	COLOR = col;
}
