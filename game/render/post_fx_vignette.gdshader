// Mobile 廉价镜头暗角：不读屏幕纹理，只是一层径向透明黑渐变。
//
// 桌面用 `post_fx.gdshader` 做色散/颗粒/锐化；移动端 tile GPU 对全屏
// screen texture 读写很敏感，所以只保留“相机感”里最便宜的一项：暗角。
// 这层只有一个全屏 quad + 一个 length/smoothstep，没有额外 framebuffer copy。
shader_type canvas_item;
render_mode unshaded;

uniform vec4 tint : source_color = vec4(0.01, 0.012, 0.016, 1.0);
uniform float strength : hint_range(0.0, 1.0) = 0.18;
uniform float softness : hint_range(0.05, 1.0) = 0.72;
uniform float aspect = 1.7777778;

void fragment() {
	vec2 centered = UV - 0.5;
	vec2 scaled = centered * vec2(aspect, 1.0);
	float radius = length(scaled) / max(length(vec2(aspect, 1.0)) * 0.5, 0.0001);
	float edge = smoothstep(1.0 - softness, 1.0, radius);
	COLOR = vec4(tint.rgb, edge * strength);
}
