// 3D 雨丝：面向相机的细长 quad，沿全局下落方向在屏幕空间倾斜。
// - billboard 在 view space 完成，避免 world-space 顶点与粒子 transform 叠乘。
// - 软 alpha 边缘 + 最小屏幕宽度，抑制亚像素细线闪烁。
// - Mobile：unshaded、不写深度、不投射阴影；片元只做两次 smoothstep。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform vec3 fall_direction = vec3(0.0, -1.0, 0.0);
uniform vec3 drop_color : source_color = vec3(0.72, 0.82, 1.0);
uniform float brightness = 1.0;
uniform float drop_length = 0.38;
uniform float base_width = 0.012;
uniform float min_screen_width_px = 1.25;
uniform float max_distance = 70.0;
uniform vec2 viewport_size = vec2(1920.0, 1080.0);
uniform float jitter_strength = 0.18;

varying vec2 v_uv;
varying float v_fade;
varying float v_jitter;

void vertex() {
    vec3 instance_scale = clamp(vec3(
        length(MODEL_MATRIX[0].xyz),
        length(MODEL_MATRIX[1].xyz),
        length(MODEL_MATRIX[2].xyz)
    ), vec3(0.15), vec3(2.0));
    float dist = distance(CAMERA_POSITION_WORLD, MODEL_MATRIX[3].xyz);

    // 标准 billboard：位置仍用实例世界坐标，基向量换成相机方向。
    MODELVIEW_MATRIX = VIEW_MATRIX * mat4(
        vec4(INV_VIEW_MATRIX[0].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[1].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[2].xyz, 0.0),
        vec4(MODEL_MATRIX[3].xyz, 1.0)
    );

    // 一个像素在距离 dist 处对应的世界宽度：h / (2*d*cot(fov/2))。
    float cot_half_fov = max(PROJECTION_MATRIX[1][1], 0.0001);
    float pixels_per_world = viewport_size.y * cot_half_fov / max(2.0 * dist, 0.0001);
    float min_world_width = clamp(min_screen_width_px / max(pixels_per_world, 0.0001), 0.001, 0.25);

    float width = clamp(max(base_width * instance_scale.x, min_world_width), 0.001, 0.09);
    float length = clamp(drop_length * instance_scale.y, 0.03, 0.70);
    vec2 local = vec2(VERTEX.x * width, VERTEX.y * length);

    // 在屏幕空间把 quad 的本地 Y 轴转到“下落速度 + 风”的合速度方向。
    vec3 fall_view = normalize((VIEW_MATRIX * vec4(fall_direction, 0.0)).xyz);
    float angle = atan(fall_view.x, fall_view.y);
    mat2 rot = mat2(vec2(cos(angle), sin(angle)), vec2(-sin(angle), cos(angle)));
    local = rot * local;

    VERTEX = vec3(local, 0.0);
    v_uv = UV;
    // 太靠近相机的雨滴会在屏幕上变成一块大白板：近处淡出，
    // 同时配合发射盒避免相机内部生成可见雨滴。
    v_fade = clamp(1.0 - dist / max_distance, 0.0, 1.0)
        * smoothstep(0.8, 2.8, dist);
    v_jitter = 1.0 - jitter_strength * 0.5
        + jitter_strength * fract(sin(dot(MODEL_MATRIX[3].xz, vec2(12.9898, 78.233))) * 43758.5453);
}

void fragment() {
    float edge = smoothstep(0.0, 0.32, v_uv.x) * smoothstep(1.0, 0.68, v_uv.x);
    float ends = smoothstep(0.0, 0.12, v_uv.y) * smoothstep(1.0, 0.88, v_uv.y);
    float alpha = pow(edge, 1.35) * ends * v_fade * v_jitter * brightness;
    ALBEDO = drop_color;
    ALPHA = clamp(alpha, 0.0, 0.82);
}
