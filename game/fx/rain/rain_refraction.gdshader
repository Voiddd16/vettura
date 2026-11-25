// 可选镜头水珠折射层：hint_screen_texture + BackBufferCopy 局部矩形。
// 默认关闭；只在 HIGH/ULTRA 且 screen_refraction_enabled=true 时启用。
// 这是一个共享的全屏层，不对每颗 3D 雨滴逐滴采样屏幕；因此不会出现
// “透明材质互相采样”的递归问题。Mobile 上这次 Viewport copy 单独计费。
shader_type canvas_item;
render_mode blend_mix, unshaded;

uniform sampler2D screen_texture : hint_screen_texture, filter_linear_mipmap;
uniform float intensity : hint_range(0.0, 1.0) = 0.0;
uniform float gravity_speed : hint_range(0.0, 2.0) = 0.25;

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void fragment() {
    vec2 uv = SCREEN_UV;
    vec2 p = uv * vec2(15.0, 8.0);
    p.y += TIME * gravity_speed;
    vec2 cell = floor(p);
    vec2 f = fract(p) - 0.5;
    float h = hash12(cell);
    float mask = 0.0;
    vec2 offset = vec2(0.0);
    if (h > 0.84) {
        float radius = 0.06 + 0.07 * hash12(cell + 7.0);
        float d = length(f * vec2(1.0, 1.35));
        mask = smoothstep(radius, radius * 0.25, d) * (0.35 + 0.65 * h);
        // 透镜倒像 + 轻微色散：只做一次屏幕采样，不做逐滴循环。
        offset = normalize(f + vec2(0.0001)) * (0.0015 + radius * 0.004);
    }
    if (mask <= 0.001) {
        COLOR = vec4(0.0);
    } else {
        vec3 refr = texture(screen_texture, uv + offset).rgb;
        COLOR = vec4(refr, mask * intensity);
    }
}
