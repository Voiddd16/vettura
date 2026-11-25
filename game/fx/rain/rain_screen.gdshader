// 可选的屏幕雨痕：CanvasLayer 全屏程序化叠加，不读取屏幕纹理。
// 默认关闭；低画质强制关闭。用于桌面/高画质近景增强，不参与 3D 粒子模拟。
shader_type canvas_item;
render_mode blend_mix, unshaded;

uniform float intensity : hint_range(0.0, 1.0) = 0.0;
uniform vec2 wind = vec2(0.0, 0.0);
uniform vec2 viewport_size = vec2(1920.0, 1080.0);

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void fragment() {
    if (intensity <= 0.001) {
        COLOR = vec4(0.0);
    } else {
        vec2 uv = UV;
        vec2 slant = vec2(wind.x * 0.08, wind.y * 0.08);
        float alpha = 0.0;

        // 三层不同速度/密度的斜雨丝；越靠上越淡，模拟镜头近处雨痕。
        for (int i = 0; i < 3; i++) {
            float fi = float(i);
            vec2 p = vec2(uv.x * (80.0 + fi * 35.0), uv.y * (18.0 + fi * 9.0));
            p.x += uv.y * slant.x * 20.0;
            p.y += TIME * (0.8 + fi * 0.35);
            vec2 cell = floor(p);
            vec2 f = fract(p) - 0.5;
            float h = hash12(cell + fi * 19.0);
            if (h > 0.88) {
                float width = 0.018 + 0.022 * hash12(cell + 3.0);
                float line = smoothstep(width, 0.0, abs(f.x));
                float ends = smoothstep(0.5, 0.2, abs(f.y));
                alpha += line * ends * (0.05 + 0.08 * h);
            }
        }

        // 轻微的镜头水珠；不做屏幕纹理采样。
        vec2 p = uv * vec2(24.0, 12.0);
        vec2 cell = floor(p);
        vec2 f = fract(p) - 0.5;
        float h = hash12(cell + 7.0);
        if (h > 0.86) {
            float r = 0.045 + 0.06 * hash12(cell + 11.0);
            alpha += smoothstep(r, r * 0.35, length(f * vec2(1.0, 1.4))) * 0.10;
        }

        COLOR = vec4(vec3(0.72, 0.84, 1.0), clamp(alpha * intensity, 0.0, 0.12));
    }
}
