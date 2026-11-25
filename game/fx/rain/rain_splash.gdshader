// 地面水花：GPU 粒子的 billboard 小片，unshaded / 不写深度。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform vec3 splash_color : source_color = vec3(0.72, 0.82, 1.0);
uniform float brightness = 0.8;

varying vec2 v_uv;

void vertex() {
    vec3 instance_scale = clamp(vec3(
        length(MODEL_MATRIX[0].xyz),
        length(MODEL_MATRIX[1].xyz),
        length(MODEL_MATRIX[2].xyz)
    ), vec3(0.05), vec3(2.0));
    MODELVIEW_MATRIX = VIEW_MATRIX * mat4(
        vec4(INV_VIEW_MATRIX[0].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[1].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[2].xyz, 0.0),
        vec4(MODEL_MATRIX[3].xyz, 1.0)
    );
    VERTEX *= instance_scale;
    v_uv = UV;
}

void fragment() {
    vec2 p = v_uv * 2.0 - 1.0;
    float d = length(p);
    float core = smoothstep(0.55, 0.15, d);
    float ring = smoothstep(0.15, 0.35, d) * (1.0 - smoothstep(0.55, 0.85, d));
    float alpha = (core * 0.12 + ring * 0.05) * brightness;
    ALBEDO = splash_color;
    ALPHA = clamp(alpha, 0.0, 0.18);
}
