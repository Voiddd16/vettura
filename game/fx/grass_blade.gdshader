// GrassFX 草卡片：billboard + 图集 UV + 每实例颜色 + 顶点风摆。
// MultiMesh 的 instance color 通过 COLOR 传入；图集由脚本生成/加载。
shader_type spatial;
render_mode cull_disabled, depth_draw_opaque, unshaded;

uniform sampler2D grass_atlas : source_color, filter_linear_mipmap, repeat_disable;
uniform vec4 tint : source_color = vec4(1.0, 1.0, 1.0, 1.0);
uniform float wind_strength : hint_range(0.0, 0.4) = 0.055;
uniform float wind_speed : hint_range(0.0, 4.0) = 1.4;

varying vec2 v_uv;
varying vec3 v_color;

void vertex() {
    vec3 instance_scale = vec3(
        length(MODEL_MATRIX[0].xyz),
        length(MODEL_MATRIX[1].xyz),
        length(MODEL_MATRIX[2].xyz)
    );
    // 面向相机；保留实例位置与缩放。
    MODELVIEW_MATRIX = VIEW_MATRIX * mat4(
        vec4(INV_VIEW_MATRIX[0].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[1].xyz, 0.0),
        vec4(INV_VIEW_MATRIX[2].xyz, 0.0),
        vec4(MODEL_MATRIX[3].xyz, 1.0)
    );
    VERTEX *= instance_scale;

    float phase = MODEL_MATRIX[3].x * 0.17 + MODEL_MATRIX[3].z * 0.13;
    float sway = sin(TIME * wind_speed + phase) * wind_strength;
    VERTEX.x += sway * UV.y * UV.y;
    VERTEX.z += sway * 0.35 * UV.y * UV.y;

    v_uv = UV;
    v_color = COLOR.rgb;
}

void fragment() {
    vec4 tex = texture(grass_atlas, v_uv);
    if (tex.a < 0.34) {
        discard;
    }
    ALBEDO = tex.rgb * tint.rgb * v_color;
    ALPHA = tex.a;
}
