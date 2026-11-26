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
// 形状：横向二次方软剖面 + 纵向亮头/渐隐拖尾；每滴用实例 scale 和世界位置
// 散列改变长宽/亮度，避免同一粒径档的雨滴完全一样（纸屑感）。
//
// 渲染：blend_mix、unshaded、不写深度；片元无光照计算。
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_disabled, unshaded;

// 与草/湿路面读取同一个全局值（WeatherRig 广播），雨不再自己持有天气。
global uniform float wetness;

uniform vec3 fall_direction = vec3(0.0, -1.0, 0.0);
uniform vec3 drop_color : source_color = vec3(0.72, 0.82, 1.0);
uniform float brightness = 1.0;
uniform float exposure_gain = 1.0;
uniform float shutter_time = 0.0166667;
uniform float frame_time = 0.0166667;
uniform float drop_diameter_mm = 1.5;
uniform float terminal_velocity = 6.0;
uniform float drop_width_scale = 1.0;
uniform float drop_length_scale = 1.0;
uniform float jitter_strength = 0.18;
uniform float min_screen_width_px = 1.1;
uniform float max_distance = 70.0;
uniform vec2 viewport_size = vec2(1920.0, 1080.0);
// 参考 `accRainDrops_ps.fx` 的 calculateSoft：雨丝靠近车/路面时按深度淡出，
// 避免一根根白线硬切进几何体。Forward+ / Mobile 都支持 depth texture。
uniform sampler2D depth_texture : hint_depth_texture, repeat_disable, filter_nearest;
uniform float depth_softness = 0.35;
// 参考 accRainDrops_ps.fx 的 normal/fresnel 环境反射：雨丝边缘更亮、中心
// 更接近背景折射色，而不是整条纯白。颜色由 RainSystem 从环境雾色同步。
uniform vec3 sky_color : source_color = vec3(0.62, 0.72, 0.85);
uniform vec3 ground_color : source_color = vec3(0.10, 0.11, 0.13);

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
varying float v_jitter;
varying float v_shaft;
varying vec3 v_world;

void vertex() {
    vec3 pos = MODEL_MATRIX[3].xyz;
    float dist = distance(CAMERA_POSITION_WORLD, pos);
    // 每个实例的 scale（0.65–1.35）与位置散列一起决定这一滴的长/宽/亮度，
    // 避免同粒径档所有雨滴长得一模一样——这是“纸屑感”的主要来源。
    float instance_scale = clamp(length(MODEL_MATRIX[0].xyz), 0.4, 2.5);
    float shape_hash = fract(sin(dot(pos.xz, vec2(12.9898, 78.233))) * 43758.5453);
    float length_jitter = mix(0.72, 1.32, shape_hash) * mix(0.85, 1.15, clamp(instance_scale, 0.5, 1.5));
    float width_jitter = mix(0.82, 1.22, fract(shape_hash * 7.13 + 0.37));

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
    width = clamp(width, 0.0004, 0.020);
    // v3 的物理长度由 |v|·T 决定；`drop_length`/`stretch_multiplier` 作为
    // 旧版形状倍率保留（默认值归一到 1.0），避免 Inspector 参数变成死旋钮。
    length = clamp(length * drop_length_scale * length_jitter, 0.02, 1.1);

    // 近景更宽更虚；远景点宽更小，更多交给雾。
    width *= mix(1.0, 1.35 + defocus_amount, near_factor) * width_jitter;
    length *= mix(1.0, 1.18, near_factor);
    // 抗锯齿用的最小屏幕宽度会把细长拖影撑成“白方块”：限制长宽比，
    // 让这一帧的四边形永远保持雨丝形状（至少 2:1）。
    width = min(width, length * 0.5);
    width = max(width, 0.0004);

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
        * smoothstep(2.0, 5.0, dist);
    v_phase = phase;
    // 每滴亮度抖动（旧版 jitter）：用世界位置散列，逐滴不同、同滴稳定。
    float jitter_hash = fract(sin(dot(pos.xz, vec2(12.9898, 78.233))) * 43758.5453);
    v_jitter = 1.0 - jitter_strength * 0.5 + jitter_strength * jitter_hash;

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
    // 横向软剖面：中间亮、两边按二次方衰减。旧版 smoothstep 在 80% 宽度内
    // 都是满 alpha，细线看起来像白色马克笔；现在是一条有中心亮度的柔和雨丝。
    float cross_x = abs(v_uv.x - 0.5) * 2.0;
    float edge = pow(max(0.0, 1.0 - cross_x * cross_x), mix(1.7, 0.75, soft));
    // 纵向：底部是雨滴头，向上是运动模糊拖尾。旧版两端都 fade、中间一段
    // 满亮，看起来像一根短白棍；现在改成“头部亮、向上单调衰减”。
    float head = smoothstep(0.0, 0.055, v_uv.y);
    float tail = pow(clamp(1.0 - v_uv.y, 0.0, 1.0), mix(0.85, 1.55, soft));
    float ends = head * tail;

    // 内部只保留极轻的纵向条纹 + 一个靠近头部的弱高光，去掉旧版 1–3 个
    // 明亮光斑——那会让每一滴看起来像发光的胶囊。
    float grain = 0.86 + 0.14 * sin(v_uv.y * 37.0 + v_phase * 5.0);
    float glint = exp(-pow((v_uv.y - 0.16) / 0.11, 2.0)) * 0.22 * highlight_count;
    float pattern = mix(1.0, grain + glint, internal_pattern_strength * mix(0.2, 1.0, v_near));
    pattern = mix(pattern, 1.0, v_far * 0.9);

    // 真实雨丝不是不断头的线：每滴沿长度随机断 1~2 次，断点位置/长度都由
    // 世界位置散列决定。这是“雨是一截截水柱”最便宜也最有效的近似。
    float seed = fract(sin(dot(v_world.xz + v_uv.y * 17.0, vec2(12.9898, 78.233))) * 43758.5453);
    float b1c = 0.16 + 0.58 * fract(seed * 13.17);
    float b1l = 0.035 + 0.080 * fract(seed * 7.71);
    float b1 = smoothstep(b1l, 0.0, abs(v_uv.y - b1c));
    float b2c = 0.14 + 0.62 * fract(seed * 29.31 + 0.41);
    float b2l = 0.025 + 0.065 * fract(seed * 11.93 + 0.20);
    float b2 = smoothstep(b2l, 0.0, abs(v_uv.y - b2c));
    float breaks = (1.0 - 0.85 * b1) * (1.0 - 0.70 * b2);
    pattern *= breaks;

    // Garg–Nayar 近似：ΔI = τ(Ē_d − E_bg)。这里用 light/bg 的
    // 解析近似替代逐像素背景采样；亮背景前 delta<=0，雨滴自动隐去。
    float T = max(shutter_time, frame_time);
    float tau = clamp((drop_diameter_mm * 0.001) / max(terminal_velocity, 0.1), 0.00002, T);
    float exposure = tau / max(T, 0.0001);
    // 深度软相交：当前雨丝片元在视空间 z 比场景表面更靠近相机时 soft->1；
    // 贴近/切入几何体时淡出，而不是被深度测试切出一条硬边。
    float scene_depth = texture(depth_texture, SCREEN_UV).r;
    vec4 scene_view = INV_PROJECTION_MATRIX * vec4(SCREEN_UV * 2.0 - 1.0, scene_depth, 1.0);
    float scene_z = scene_view.z / scene_view.w;
    float soft_intersection = smoothstep(0.0, max(depth_softness, 0.001), VERTEX.z - scene_z);

    float delta = max(light - bg * background_response, 0.0);
    float wet = clamp(wetness, 0.0, 1.0);
    float alpha = edge * ends * pattern * delta * (0.20 + 0.80 * exposure)
        * exposure_gain * v_fade * v_shaft * v_jitter * brightness * soft_intersection
        * mix(0.60, 1.0, wet);
    // 近景大滴不要变成半透明白条；远景已经交给雾和 v_fade。
    alpha *= mix(1.0, 0.72, v_near * 0.85);
    alpha = clamp(alpha, 0.0, 0.58);

    // 雨滴主要透出背景的冷灰，不应该是纯蓝白；保留一点天空的冷色。
    // 薄水膜/雨滴的 fresnel：边缘反射天空，中心偏向背景折射色。
    float signed_x = v_uv.x * 2.0 - 1.0;
    float facing = sqrt(max(0.0, 1.0 - signed_x * signed_x));
    vec3 n_view = normalize(vec3(signed_x, 0.0, facing));
    vec3 to_cam_view = normalize(-VERTEX);
    float fresnel = pow(1.0 - clamp(dot(n_view, to_cam_view), 0.0, 1.0), 3.0);
    vec3 bg_color = mix(ground_color, sky_color, smoothstep(0.18, 0.82, SCREEN_UV.y));
    vec3 water = mix(bg_color * 0.45, bg_color * (0.85 + 0.35 * light), fresnel);
    vec3 tinted = mix(drop_color * (0.80 + 0.45 * light), water, 0.35);
    ALBEDO = tinted;
    ALPHA = alpha;
}

