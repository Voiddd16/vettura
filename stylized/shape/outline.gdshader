// 轮廓描边：反壳法（沿法线外扩 + 正面剔除），适合选中框/目标标记。
// 推荐用法：作为原材质的 next_pass 叠加，不必替换原材质。
shader_type spatial;
render_mode cull_front, unshaded;

uniform vec4 color : source_color = vec4(0.10, 1.00, 0.90, 1.0);
uniform float width : hint_range(0.0, 0.20) = 0.02;   // 物体本地空间单位

void vertex() {
	VERTEX += NORMAL * width;
}

void fragment() {
	ALBEDO = color.rgb;
}
