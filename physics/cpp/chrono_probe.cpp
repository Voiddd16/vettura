/*!
 * @file chrono_probe.cpp —— 诊断探针：把 spec 变成 Chrono 车辆后逐步打印
 * 车身/轮心位置与轮胎力，用来定位「接地」「轮位」类问题。链接真实 Chrono。
 *
 * 编译方式见 docs/build.md §8.3（用 Chrono 的 compile_commands.json 取 include）。
 */

#include "chrono_vehicle/ChVehicleDataPath.h"
#include "chrono_vehicle/terrain/FlatTerrain.h"
#include "chrono_vehicle/wheeled_vehicle/vehicle/WheeledVehicle.h"
#include "vettura_vehicle_spec.h"

#include <cmath>
#include <cstdio>
#include <string>

static vettura_chrono_spec default_spec() {
    vettura_chrono_spec spec{};
    spec.mass_kg = 1200.0;
    spec.wheelbase_m = 2.6;
    spec.cg_height_m = 0.35;
    spec.frontal_area_m2 = 1.8;
    spec.drag_coefficient = 0.35;
    for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
        spec.wheels[i].radius_m = 0.33;
        spec.wheels[i].width_m = 0.24;
        spec.wheels[i].suspension.spring_rate_n_per_m = 80000.0;
        spec.wheels[i].suspension.damper_bump_ns_per_m = 4000.0;
        spec.wheels[i].suspension.damper_rebound_ns_per_m = 6000.0;
        spec.wheels[i].suspension.travel_m = 0.08;
    }
    spec.wheels[0].position = {-0.75, 0.33, -1.3};
    spec.wheels[1].position = {0.75, 0.33, -1.3};
    spec.wheels[2].position = {-0.75, 0.33, 1.3};
    spec.wheels[3].position = {0.75, 0.33, 1.3};
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
    const std::string out_dir = argc > 1 ? argv[1] : "/tmp/vettura-probe";
    std::string vehicle_file;
    if (!vettura::write_vehicle_spec(default_spec(), out_dir, vehicle_file)) {
        std::fprintf(stderr, "write_vehicle_spec failed\n");
        return 1;
    }
    chrono::vehicle::SetVehicleDataPath(out_dir + "/");

    auto terrain = std::make_shared<chrono::vehicle::FlatTerrain>(0.0, 1.4f);
    auto vehicle = std::make_shared<chrono::vehicle::WheeledVehicle>(
        out_dir + "/" + vehicle_file, chrono::ChContactMethod::NSC, true, true);
    if (!vehicle || vehicle->GetChassis() == nullptr) {
        std::fprintf(stderr, "vehicle construction failed\n");
        return 1;
    }
    vehicle->Initialize(chrono::ChCoordsys<>(chrono::ChVector3d(0, 0, 0.335), chrono::QUNIT), 0.0);

    {
        double total = 0.0;
        for (auto& b : vehicle->GetSystem()->GetBodies()) {
            total += b->GetMass();
            std::printf("body %-28s mass=%8.2f\n", b->GetName().c_str(), b->GetMass());
        }
        std::printf("sum of bodies = %.2f, vehicle->GetMass() = %.2f\n", total, vehicle->GetMass());
    }

    const double dt = 1.0 / 600.0;
    double time = 0.0;
    chrono::vehicle::DriverInputs inputs;  // 全零输入

    auto phase = [&](const char* label, int steps, double throttle, double steering) {
        inputs.m_throttle = throttle;
        inputs.m_steering = steering;
        for (int k = 0; k < steps; ++k) {
            time += dt;
            vehicle->Synchronize(time, inputs, *terrain);
            vehicle->Advance(dt);
        }
        const auto pos = vehicle->GetPos();
        const auto rot = vehicle->GetRot();
        const auto eul = rot.GetCardanAnglesXYZ();
        std::printf(
            "%-12s t=%.2f pos=(%+.2f,%+.2f,%+.2f) speed=%+.2f steerFL=%+.3f euler=(%+.3f,%+.3f,%+.3f)\n",
            label, time, pos.x(), pos.y(), pos.z(), vehicle->GetSpeed(),
            vehicle->GetSteeringAngle(0, chrono::vehicle::LEFT), eul.x(), eul.y(), eul.z());
    };

    phase("settle", 900, 0.0, 0.0);
    phase("accelerate", 900, 1.0, 0.0);
    // 转向阶段：每 150 步打印一次，看航向与轮胎侧向力。
    for (int k = 0; k < 6; ++k) {
        phase("steer-left", 150, 0.5, 0.5);
        const auto rot = vehicle->GetRot();
        const auto fwd = rot.Rotate(chrono::ChVector3d(1, 0, 0));
        const auto tire = vehicle->GetTire(0, chrono::vehicle::LEFT);
        const auto tf = tire->ReportTireForce(terrain.get());
        std::printf("    yaw=%+.4f fwd=(%+.3f,%+.3f) tireF=(%+.0f,%+.0f) slip=%.4f/%.4f\n",
                    std::atan2(fwd.y(), fwd.x()), fwd.x(), fwd.y(), tf.force.x(), tf.force.y(),
                    tire->GetLongitudinalSlip(), tire->GetSlipAngle());
    }
    phase("brake", 600, 0.0, 0.0);
    return 0;
}
