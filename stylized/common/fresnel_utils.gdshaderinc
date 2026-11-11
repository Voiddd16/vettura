// 视角相关工具：菲涅尔系数（玻璃 / 边缘光 / 流光共用）。
// 用法：#include "res://stylized/common/fresnel_utils.gdshaderinc"
// 注意：normal / view 传视图空间即可（spatial fragment 的 NORMAL / VIEW）。

float vfx_fresnel(vec3 normal_vs, vec3 view_vs, float power, float bias) {
	float ndv = abs(dot(normalize(normal_vs), normalize(view_vs)));
	return bias + (1.0 - bias) * pow(clamp(1.0 - ndv, 0.0, 1.0), power);
}
