// 贴地薄雾/溅射水雾带：少量大尺寸软 billboard，GPU 粒子，不写深度。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform vec3 mist_color : source_color = vec3(0.55, 0.62, 0.72);
uniform float intensity = 0.035;

varying vec2 v_uv;

void vertex() {
    vec3 instance_scale = vec3(
        length(MODEL_MATRIX[0].xyz),
        length(MODEL_MATRIX[1].xyz),
        length(MODEL_MATRIX[2].xyz)
    );
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
    float radial = smoothstep(1.0, 0.15, length(p));
    float vertical = smoothstep(0.0, 0.35, v_uv.y) * smoothstep(1.0, 0.55, v_uv.y);
    ALBEDO = mist_color;
    ALPHA = radial * vertical * intensity;
}
