/* 布局探针：实测 cpp/chrono_bridge.h 的 sizeof/offsetof，供 Rust 侧
 * `layout_matches_the_c_header` 测试使用。
 *
 * 运行：
 *   clang -std=c99 physics/cpp/layout_probe.c -o /tmp/layout && /tmp/layout
 */
#include <stdio.h>
#include <stddef.h>
#include "chrono_bridge.h"

#define P(type, field)                                                                             \
    printf("%-28s size=%-4zu align=%-3zu %-24s off=%zu\n", #type, sizeof(type), _Alignof(type), #field, \
           offsetof(type, field))

int main(void) {
    P(vettura_chrono_vec3, x);
    P(vettura_chrono_quat, w);
    P(vettura_chrono_suspension, spring_rate_n_per_m);
    P(vettura_chrono_wheel, position);
    P(vettura_chrono_wheel, radius_m);
    P(vettura_chrono_wheel, steerable);
    P(vettura_chrono_wheel, suspension);
    P(vettura_chrono_engine, idle_rpm);
    P(vettura_chrono_engine, gear_count);
    P(vettura_chrono_engine, gear_ratios);
    P(vettura_chrono_engine, final_drive);
    P(vettura_chrono_drivetrain, layout);
    P(vettura_chrono_tyre, peak_friction);
    P(vettura_chrono_spec, mass_kg);
    P(vettura_chrono_spec, wheels);
    P(vettura_chrono_spec, engine);
    P(vettura_chrono_spec, tyres);
    P(vettura_chrono_pose, orientation);
    P(vettura_chrono_controls, handbrake);
    P(vettura_chrono_wheel_state, rotation);
    P(vettura_chrono_wheel_state, contact);
    P(vettura_chrono_state, position);
    P(vettura_chrono_state, speed_mps);
    P(vettura_chrono_state, wheels);
    P(vettura_chrono_state, gear);
    return 0;
}
