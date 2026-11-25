// 地面涟漪：MultiMesh 水平 quad + 程序化圆环。
// 用 MultiMesh 而不是 Decal：Mobile 下每个 Mesh 最多 8 个 Decal，且 Decal
// 会额外走一遍投影；这里同屏最多 6 个实例，CPU 更新，成本可控。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform vec3 ripple_color : source_color = vec3(0.68, 0.82, 1.0);

varying vec2 v_uv;

void vertex() {
    v_uv = UV;
}

void fragment() {
    vec2 p = v_uv * 2.0 - 1.0;
    float r = length(p);
    float ring = smoothstep(0.55, 0.72, r) * (1.0 - smoothstep(0.78, 0.98, r));
    float inner = smoothstep(0.10, 0.20, r) * (1.0 - smoothstep(0.24, 0.36, r));
    ALBEDO = ripple_color;
    ALPHA = (ring * 0.35 + inner * 0.12) * COLOR.a;
}
