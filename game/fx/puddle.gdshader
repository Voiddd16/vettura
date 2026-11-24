// 低洼积水片：程序化噪声遮罩 + 低粗糙度，贴地绘制时不会是一整块方形灰片。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, diffuse_burley;

uniform vec4 water_color : source_color = vec4(0.035, 0.055, 0.08, 1.0);
uniform float wetness : hint_range(0.0, 1.0) = 0.8;
uniform float patch_scale : hint_range(1.0, 20.0) = 5.0;

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void fragment() {
    vec2 uv = UV * patch_scale;
    float n = value_noise(uv) * 0.55
        + value_noise(uv * 2.1 + 17.0) * 0.28
        + value_noise(uv * 4.3 + 41.0) * 0.17;
    float mask = smoothstep(0.42, 0.68, n);
    ALBEDO = water_color.rgb;
    ALPHA = mask * wetness * 0.62;
    ROUGHNESS = 0.035;
    METALLIC = 0.75;
    SPECULAR = 1.0;
}
