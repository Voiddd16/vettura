/*!
 * @file bridge_dump.cpp
 *
 * 独立小工具：把一个 spec 打印成 Chrono JSON，并落盘到 <out>。
 * 用法：
 *   vettura_bridge_dump <out-dir>
 *
 * 不用 Chrono，只用 `vettura_vehicle_spec.cpp` + `chrono_bridge.h`，
 * 目的是让「spec → Chrono JSON」这层可以脱离 C++ 大依赖单独编译和验证：
 * ```bash
 * clang++ -std=c++17 -I cpp cpp/bridge_dump.cpp cpp/vettura_vehicle_spec.cpp \
 *     -o /tmp/bridge_dump
 * /tmp/bridge_dump /tmp/chrono-json && python3 -m json.tool /tmp/chrono-json/vehicle/vettura_vehicle.json
 * ```
 */

#include "chrono_bridge.h"
#include "vettura_vehicle_spec.h"

#include <cstdio>
#include <string>

static vettura_chrono_spec default_spec() {
    vettura_chrono_spec spec{};
    spec.mass_kg = 1200.0;
    spec.wheelbase_m = 2.6;
    spec.cg_height_m = 0.35;
    spec.frontal_area_m2 = 1.8;
    spec.drag_coefficient = 0.35;

    // 与 vettura-physics 的 VehicleSpec::default() 一致：FL/FR 转向，RL/RR 驱动。
    const double radius = 0.33;
    const double half_wb = 1.3;
    for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
        spec.wheels[i].radius_m = radius;
        spec.wheels[i].width_m = 0.24;
        spec.wheels[i].suspension.spring_rate_n_per_m = 80000.0;
        spec.wheels[i].suspension.damper_bump_ns_per_m = 4000.0;
        spec.wheels[i].suspension.damper_rebound_ns_per_m = 6000.0;
        spec.wheels[i].suspension.travel_m = 0.08;
        spec.wheels[i].suspension.anti_roll_n_per_m = 0.0;
    }
    spec.wheels[0].position = {-0.75, radius, -half_wb};
    spec.wheels[1].position = {0.75, radius, -half_wb};
    spec.wheels[2].position = {-0.75, radius, half_wb};
    spec.wheels[3].position = {0.75, radius, half_wb};
    spec.wheels[0].steerable = 1;
    spec.wheels[1].steerable = 1;
    spec.wheels[2].driven = 1;
    spec.wheels[3].driven = 1;

    spec.engine.idle_rpm = 1000.0;
    spec.engine.redline_rpm = 8000.0;
    spec.engine.peak_torque_nm = 350.0;
    spec.engine.gear_count = 6;
    const double ratios[6] = {3.2, 2.1, 1.6, 1.3, 1.1, 0.95};
    for (int i = 0; i < 6; ++i)
        spec.engine.gear_ratios[i] = ratios[i];
    spec.engine.final_drive = 3.7;

    spec.drivetrain.layout = VETTURA_CHRONO_LAYOUT_RWD;
    spec.drivetrain.efficiency = 0.9;
    spec.tyres.peak_friction = 1.4;
    return spec;
}

int main(int argc, char** argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: %s <out-dir>\n", argv[0]);
        return 2;
    }
    const std::string out_dir = argv[1];
    const vettura_chrono_spec spec = default_spec();

    std::string vehicle_file;
    if (!vettura::write_vehicle_spec(spec, out_dir, vehicle_file)) {
        std::fprintf(stderr, "write_vehicle_spec failed (spec not representable?)\n");
        return 1;
    }
    std::printf("%s\n", vehicle_file.c_str());
    return 0;
}
