// 座舱/屏幕雨滴叠层：低成本的程序化雨滴 + 雨丝，供 RainFx 的 CanvasLayer 使用。
// intensity 0..1 由 RainFx 每帧写入；不读取屏幕纹理，移动端安全。
shader_type canvas_item;
render_mode blend_mix, unshaded;

uniform float intensity : hint_range(0.0, 1.0) = 0.0;
uniform float speed : hint_range(0.0, 4.0) = 1.0;

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void fragment() {
    vec2 uv = UV;
    float alpha = 0.0;

    // 三层不同尺度的雨滴，向下滚动。
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        vec2 p = uv * vec2(16.0 + fi * 9.0, 8.0 + fi * 4.0);
        p.y += TIME * speed * (0.6 + fi * 0.35);
        vec2 cell = floor(p);
        vec2 f = fract(p) - 0.5;
        float h = hash12(cell + fi * 19.0);
        if (h > 0.80) {
            float radius = 0.018 + 0.040 * hash12(cell + 7.0);
            float d = length(f * vec2(1.0, 1.35));
            alpha += smoothstep(radius, radius * 0.45, d) * (0.25 + 0.35 * h);
        }
    }

    // 细小雨丝。
    float streak = smoothstep(0.965, 1.0, sin(uv.x * 90.0 + uv.y * 18.0 + TIME * speed * 1.7));
    alpha += streak * 0.08;

    COLOR = vec4(vec3(0.72, 0.84, 1.0), clamp(alpha * intensity, 0.0, 0.28));
}
