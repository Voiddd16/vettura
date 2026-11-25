// Mobile 雨丝：曝光积分 + 背景亮度响应 + 光源响应 + 三层景深。
//
// 光度模型（Garg & Nayar 2003, “When Does a Camera See Rain?”）：
//   I_d = τ·Ē_d + (T−τ)·E_bg
//   ΔI  = τ(Ē_d − E_bg) = α − β·I_bg
// 这里不逐像素读背景，而是用屏幕高度 + sky/ground uniform 近似 E_bg；
// 这是 Mobile 下替代 screen texture 逐像素采样的廉价折中。
//
// streak 长度不是固定比例，而是运动模糊积分：L = |v|·T，T = max(shutter, frame_dt)。
// 亮度按停留时间 τ/T 摊薄；低帧率时 T 变大，拖影变长、变淡。
//
// 渲染：blend_mix、unshaded、不写深度；片元无光照计算。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

uniform vec3 fall_direction = vec3(0.0, -1.0, 0.0);
uniform vec3 drop_color : source_color = vec3(0.72, 0.82, 1.0);
uniform float brightness = 1.0;
uniform float exposure_gain = 1.0;
uniform float shutter_time = 0.0166667;
uniform float frame_time = 0.0166667;
uniform float drop_diameter_mm = 1.5;
uniform float terminal_velocity = 6.0;
uniform float drop_width_scale = 1.0;
uniform float min_screen_width_px = 1.1;
uniform float max_distance = 70.0;
uniform vec2 viewport_size = vec2(1920.0, 1080.0);

uniform float layer_softness = 0.35;
uniform float internal_pattern_strength = 0.55;
uniform float oscillation_freq = 2.4;
uniform float highlight_count = 2.0;
uniform float dof_near = 5.0;
uniform float dof_mid = 25.0;
uniform float dof_far = 65.0;
uniform float defocus_amount = 0.0;

uniform float sky_brightness = 0.9;
uniform float ground_brightness = 0.25;
uniform float background_response = 0.85;
uniform vec3 sun_direction = vec3(0.3, -0.6, 0.2);
uniform float backlit_boost = 1.0;
uniform vec3 local_light_pos_0 = vec3(0.0);
uniform vec3 local_light_pos_1 = vec3(0.0);
uniform vec3 local_light_pos_2 = vec3(0.0);
uniform vec3 local_light_color_0 = vec3(0.0);
uniform vec3 local_light_color_1 = vec3(0.0);
uniform vec3 local_light_color_2 = vec3(0.0);

uniform float gust_strength = 0.25;
uniform float gust_period = 3.0;
uniform float shaft_scale = 0.08;
uniform float shaft_contrast = 0.35;

varying vec2 v_uv;
varying float v_fade;
varying float v_near;
varying float v_far;
varying float v_light;
varying float v_bg;
varying float v_phase;
varying float v_shaft;
varying vec3 v_world;

void vertex() {
    vec3 pos = MODEL_MATRIX[3].xyz;
    float dist = distance(CAMERA_POSITION_WORLD, pos);

    // 三层景深：近景更大更虚，中景最清晰，远景交给雾/低对比。
    float near_factor = 1.0 - smoothstep(dof_near, dof_mid, dist);
    float far_factor = smoothstep(dof_mid, dof_far, dist);
    v_near = near_factor;
    v_far = far_factor;

    vec3 fall = normalize(fall_direction);
    vec3 to_cam = normalize(CAMERA_POSITION_WORLD - pos);
    vec3 x_axis = cross(fall, to_cam);
    if (length(x_axis) < 0.0001) {
        x_axis = vec3(1.0, 0.0, 0.0);
    }
    x_axis = normalize(x_axis);
    vec3 z_axis = normalize(cross(x_axis, fall));

    MODELVIEW_MATRIX = VIEW_MATRIX * mat4(
        vec4(INV_VIEW_MATRIX[0].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[1].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[2].xyz, 0.0),
        vec4(MODEL_MATRIX[3].xyz, 1.0)
    );

    // 曝光积分：低帧率帧时间变大，拖影自然变长，亮度由 τ/T 摊薄。
    float T = max(shutter_time, frame_time);
    float speed = max(terminal_velocity, 0.1);
    float length = clamp(speed * T * (0.7 + 0.6 * UV.y), 0.02, 1.5);
    float diameter = max(drop_diameter_mm * 0.001, 0.0002);
    float width = max(diameter * drop_width_scale, 0.0005);
    width = max(width, min_screen_width_px / max(viewport_size.y * max(PROJECTION_MATRIX[1][1], 0.0001) / max(2.0 * dist, 0.0001), 0.0001));
    width = clamp(width, 0.0004, 0.045);
    length = clamp(length, 0.02, 1.1);

    // 近景更宽更虚；远景点宽更小，更多交给雾。
    width *= mix(1.0, 1.8 + defocus_amount, near_factor);
    length *= mix(1.0, 1.18, near_factor);

    vec2 local = vec2(VERTEX.x * width, VERTEX.y * length);
    vec3 fall_view = normalize((VIEW_MATRIX * vec4(fall_direction, 0.0)).xyz);
    float angle = atan(fall_view.x, fall_view.y);
    mat2 rot = mat2(vec2(cos(angle), sin(angle)), vec2(-sin(angle), cos(angle)));
    local = rot * local;

    // 雨滴下落时的轻微摆动/弯曲；中远景趋于直线。
    float phase = MODEL_MATRIX[3].x * 0.37 + MODEL_MATRIX[3].z * 0.21;
    float bend = sin(UV.y * 3.14159 + TIME * oscillation_freq + phase) * 0.004 * near_factor;
    local.x += bend * UV.y;

    VERTEX = vec3(local, 0.0);
    v_uv = UV;
    v_world = pos;
    v_fade = clamp(1.0 - dist / max_distance, 0.0, 1.0)
        * smoothstep(1.3, 3.8, dist);
    v_phase = phase;

    // 雨幕条带：低频空间密度调制，随阵风包络移动；不是全屏均匀雨。
    float gust = 1.0 + gust_strength * sin(TIME * 6.2831 / max(gust_period, 0.1));
    v_shaft = mix(1.0 - shaft_contrast, 1.0 + shaft_contrast,
        smoothstep(-0.4, 0.4, sin(pos.x * shaft_scale + pos.z * shaft_scale * 0.7 + TIME * 0.18) * gust));
}

float local_light_contribution(vec3 position, vec3 color) {
    if (dot(color, color) < 0.001) {
        return 0.0;
    }
    float d2 = dot(position - v_world, position - v_world);
    return dot(color, vec3(0.33)) * (1.0 / (1.0 + d2 * 0.06));
}

void fragment() {
    float bg = mix(sky_brightness, ground_brightness, smoothstep(0.18, 0.82, SCREEN_UV.y));
    v_bg = bg;

    // 逆光/方向光：只有朝向光源看时才给亮边；不是恒定白色。
    vec3 view_dir = normalize(CAMERA_POSITION_WORLD - v_world);
    float backlit = pow(max(dot(view_dir, -normalize(sun_direction)), 0.0), 4.0) * backlit_boost;
    float light = 0.35 + backlit;

    // 最多 3 个局部光源；只在灯附近产生局部高光/颜色沾染。
    light += local_light_contribution(local_light_pos_0, local_light_color_0);
    light += local_light_contribution(local_light_pos_1, local_light_color_1);
    light += local_light_contribution(local_light_pos_2, local_light_color_2);
    v_light = light;

    float soft = mix(layer_softness, 1.0, v_near);
    float edge = smoothstep(0.0, 0.10 + soft * 0.32, v_uv.x)
        * smoothstep(1.0, 0.90 - soft * 0.32, v_uv.x);
    float ends = smoothstep(0.0, 0.08 + soft * 0.15, v_uv.y)
        * smoothstep(1.0, 0.92 - soft * 0.15, v_uv.y);

    // 内部非均匀亮度：散斑 + 1–3 个拖尾高光 + 轻微振荡。
    float grain = 0.72 + 0.28 * sin(v_uv.y * 43.0 + v_phase * 5.0) * sin(v_uv.x * 19.0 + v_phase);
    float highlight = 0.0;
    float bands = max(highlight_count, 1.0);
    for (int i = 0; i < 3; i++) {
        if (float(i) < bands) {
            float center = 0.22 + 0.56 * fract(0.37 + float(i) * 0.29);
            float w = 0.035 + 0.025 * float(i);
            highlight += exp(-pow((v_uv.y - center) / w, 2.0)) * (0.35 - 0.08 * float(i));
        }
    }
    float pattern = mix(1.0, mix(grain, grain + highlight, 0.65), internal_pattern_strength * mix(0.25, 1.0, v_near));
    pattern = mix(pattern, 1.0, v_far * 0.85);

    // Garg–Nayar 近似：ΔI = τ(Ē_d − E_bg)。这里用 light/bg 的
    // 解析近似替代逐像素背景采样；亮背景前 delta<=0，雨滴自动隐去。
    float T = max(shutter_time, frame_time);
    float tau = clamp((drop_diameter_mm * 0.001) / max(terminal_velocity, 0.1), 0.00002, T);
    float exposure = tau / max(T, 0.0001);
    float delta = max(light - bg * background_response, 0.0);
    float alpha = edge * ends * pattern * delta * (0.25 + 0.75 * exposure)
        * exposure_gain * v_fade * v_shaft * brightness;
    alpha = clamp(alpha, 0.0, 0.72);

    vec3 tinted = drop_color * (0.55 + 0.65 * light);
    ALBEDO = tinted;
    ALPHA = alpha;
}

