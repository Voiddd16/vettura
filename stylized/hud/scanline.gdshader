// HUD 扫描线/故障风叠层：全屏 ColorRect 覆盖（读屏幕纹理）。
// 放在 HUD CanvasLayer 最后；glow/glitch 由脚本驱动，平时保持 0。
shader_type canvas_item;

uniform sampler2D screen_tex : hint_screen_texture, filter_linear;
uniform float strength : hint_range(0.0, 1.0) = 0.25;
uniform float line_density = 420.0;   // UV 空间行密度
uniform float band_speed = 2.0;       // 慢速亮带滚动
uniform float glitch : hint_range(0.0, 1.0) = 0.0;

#include "res://stylized/common/noise.gdshaderinc"

void fragment() {
	vec2 uv = SCREEN_UV;
	float row = floor(uv.y * line_density);
	float line = 0.5 + 0.5 * sin(uv.y * line_density * 6.2831853);
	float band = 0.5 + 0.5 * sin(uv.y * 4.0 - TIME * band_speed);
	float hit = step(1.0 - glitch * 0.02, vfx_hash12(vec2(row, floor(TIME * 12.0))));
	float ox = (vfx_hash12(vec2(row, floor(TIME * 12.0)) + 7.7) - 0.5) * 0.06 * hit;
	vec3 col = texture(screen_tex, uv + vec2(ox, 0.0)).rgb;
	col *= 1.0 - strength * (0.30 * line + 0.35 * band);
	COLOR = vec4(col, 1.0);
}
