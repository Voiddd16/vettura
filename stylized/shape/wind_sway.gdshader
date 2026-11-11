// 植被/旗帜风摆：世界空间顶点位移，摆幅随高度增长（根部不动）。
// 位移按世界坐标计算，网格按“底边在 y=0”摆放即可。
shader_type spatial;
render_mode world_vertex_coords, blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx;

#include "res://stylized/common/noise.gdshaderinc"

uniform float sway_strength : hint_range(0.0, 1.0) = 0.15; // 顶部最大摆幅（米）
uniform float sway_speed = 1.6;
uniform vec2 wind_direction = vec2(1.0, 0.3);              // 世界 XZ 风向
uniform float wind_strength : hint_range(0.0, 4.0) = 1.0;
uniform float noise_scale = 0.35;

void vertex() {
	float h = clamp(VERTEX.y, 0.0, 2.0);
	float phase = TIME * sway_speed + vfx_fbm(VERTEX.xz * noise_scale, 2) * 6.2831853;
	VERTEX.xz += normalize(wind_direction) * sin(phase) * sway_strength * h * wind_strength;
}
