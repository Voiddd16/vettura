/*!
 * @file bridge.cpp
 *
 * vettura-physics (Rust) ←→ Chrono::Vehicle (C++) 的 C ABI 胶水实现。
 *
 * 契约是 `chrono_bridge.h`（纯 Pod + 不透明句柄 + status code），没有用 cxx：
 * 见该头文件的理由。
 *
 * 职责划分：
 * - 本文件只做「车辆」部分：Chrono::Vehicle 的 `WheeledVehicle` + `FlatTerrain`
 *   （平地）+ parking brake，不接 SCM/Granular/FEA 地形、不接传感器/可视化；
 * - spec → Chrono JSON 的翻译在 `vettura_vehicle_spec.cpp`（纯 C++，可脱离
 *   Chrono 单测）；
 * - Rust 侧（`src/backends/chrono/`）负责固定步长调度、输入钳制、单位/坐标系
 *   转换和状态回填的取舍。
 *
 * 坐标系：Chrono ISO 帧是 Z 上、X 前、Y 左；我们的物理帧是 +X 右、+Y 上、-Z 前。
 * 转换集中在 `to_chrono` / `from_chrono`： (cx,cy,cz) = (-z, -x, y)。
 */

#include "chrono_bridge.h"
#include "vettura_vehicle_spec.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <memory>
#include <string>
#include <vector>

#if !defined(_WIN32)
#include <unistd.h>  // getpid
#endif

#include "chrono/core/ChCoordsys.h"
#include "chrono/core/ChQuaternion.h"
#include "chrono/core/ChVector3.h"

#include "chrono_vehicle/ChTerrain.h"
#include "chrono_vehicle/ChVehicleDataPath.h"
#include "chrono_vehicle/terrain/FlatTerrain.h"
#include "chrono_vehicle/wheeled_vehicle/vehicle/WheeledVehicle.h"

namespace {

// 顶层/子系统统一的 status code。
constexpr int32_t kOk = VETTURA_CHRONO_OK;
constexpr int32_t kInvalidArgument = VETTURA_CHRONO_ERR_INVALID_ARGUMENT;
constexpr int32_t kInvalidSpec = VETTURA_CHRONO_ERR_INVALID_SPEC;
constexpr int32_t kSolver = VETTURA_CHRONO_ERR_SOLVER;

thread_local std::string g_last_error;

void set_last_error(const std::string& message) { g_last_error = message; }

bool is_finite(double v) { return std::isfinite(v); }

bool spec_is_valid(const vettura_chrono_spec* spec) {
    if (spec == nullptr)
        return false;
    const double scalars[] = {spec->mass_kg,
                              spec->wheelbase_m,
                              spec->cg_height_m,
                              spec->frontal_area_m2,
                              spec->drag_coefficient,
                              spec->engine.idle_rpm,
                              spec->engine.redline_rpm,
                              spec->engine.peak_torque_nm,
                              spec->engine.final_drive,
                              spec->drivetrain.efficiency,
                              spec->tyres.peak_friction};
    for (double v : scalars) {
        if (!is_finite(v))
            return false;
    }
    for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
        const vettura_chrono_wheel& wheel = spec->wheels[i];
        if (!is_finite(wheel.radius_m) || wheel.radius_m <= 0.0)
            return false;
        if (!is_finite(wheel.position.x) || !is_finite(wheel.position.y) ||
            !is_finite(wheel.position.z))
            return false;
    }
    return true;
}

bool controls_are_valid(const vettura_chrono_controls* controls) {
    if (controls == nullptr)
        return false;
    const double inputs[] = {controls->throttle, controls->brake, controls->clutch, controls->steering};
    for (double v : inputs) {
        if (!is_finite(v))
            return false;
    }
    return controls->throttle >= 0.0 && controls->throttle <= 1.0 && controls->brake >= 0.0 &&
           controls->brake <= 1.0 && controls->clutch >= 0.0 && controls->clutch <= 1.0 &&
           controls->steering >= -1.0 && controls->steering <= 1.0;
}

/* ---------------- 坐标转换 ----------------
 *
 * 物理帧（与 Godot 一致）：+X 右、+Y 上、-Z 前。
 * Chrono ISO 帧：+X 前、+Y 左、+Z 上。
 *
 * 向量换算就是换基： (cx,cy,cz) = (-z,-x,y)。
 * 四元数换算同理，但要写成 q' = qM · q · qM^-1（qM = 我们的基 → Chrono 基）。
 *
 * 四元数数学全部本地实现：Chrono 10.0 起 ChQuaternion 是类模板，
 * Q_from_AngAxis 之类的自由函数已被移除，不依赖它们更稳。
 */

// 物理帧 → Chrono ISO 帧：(cx, cy, cz) = (-z, -x, y)。
chrono::ChVector3d to_chrono(const vettura_chrono_vec3& v) {
    return chrono::ChVector3d(-v.z, -v.x, v.y);
}

// Chrono ISO 帧 → 物理帧：(px, py, pz) = (-cy, cz, -cx)。
vettura_chrono_vec3 from_chrono(const chrono::ChVector3d& v) {
    vettura_chrono_vec3 out;
    out.x = -v.y();
    out.y = v.z();
    out.z = -v.x();
    return out;
}

// 就地算四元数（x,y,z,w 顺序，与 ABI 一致）。
struct LocalQuat {
    double x;
    double y;
    double z;
    double w;
};

LocalQuat quat_mul(const LocalQuat& a, const LocalQuat& b) {
    LocalQuat r;
    r.w = a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z;
    r.x = a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y;
    r.y = a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x;
    r.z = a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w;
    return r;
}

LocalQuat quat_conjugate(const LocalQuat& q) { return LocalQuat{-q.x, -q.y, -q.z, q.w}; }

LocalQuat quat_normalize(const LocalQuat& q) {
    const double n = std::sqrt(q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w);
    if (n <= 0.0)
        return LocalQuat{0.0, 0.0, 0.0, 1.0};
    return LocalQuat{q.x / n, q.y / n, q.z / n, q.w / n};
}

LocalQuat quat_from_axis_angle(const chrono::ChVector3d& axis_unit, double angle) {
    const double half = 0.5 * angle;
    const double s = std::sin(half);
    LocalQuat q;
    q.x = axis_unit.x() * s;
    q.y = axis_unit.y() * s;
    q.z = axis_unit.z() * s;
    q.w = std::cos(half);
    return q;
}

// qM：把「我们的基」旋转到「Chrono 基」。推导见 bridge.cpp 顶部注释：
// 轴 (-1,1,1)/√3，转角 120°。
const LocalQuat& basis_change_quat() {
    static const LocalQuat kQm = [] {
        // 基映射：x̂→-ŷ、ŷ→ẑ、ẑ→-x̂（det = +1，纯旋转）。这个矩阵的轴角表示是
        // 「绕 (1,-1,-1)/√3 转 120°」；写成 (-1,1,1) 会得到转置（逆旋转），
        // 平移不出错、但姿态会被镜像 —— 表现为转向输入正确、上报的 yaw 反号。
        LocalQuat q = quat_from_axis_angle(
            chrono::ChVector3d(1.0, -1.0, -1.0) / std::sqrt(3.0), 2.0 * M_PI / 3.0);
        return quat_normalize(q);
    }();
    return kQm;
}

chrono::ChQuaterniond to_chrono_quat(const vettura_chrono_quat& q) {
    const LocalQuat q_ours{q.x, q.y, q.z, q.w};
    const LocalQuat converted =
        quat_mul(quat_mul(basis_change_quat(), q_ours), quat_conjugate(basis_change_quat()));
    // Chrono 的 e0 是标量部分（= w），e1..e3 是向量部分。
    return chrono::ChQuaterniond(converted.w, converted.x, converted.y, converted.z);
}

vettura_chrono_quat from_chrono_quat(const chrono::ChQuaterniond& q) {
    const LocalQuat q_chrono{q.e1(), q.e2(), q.e3(), q.e0()};
    const LocalQuat ours =
        quat_mul(quat_mul(quat_conjugate(basis_change_quat()), q_chrono), basis_change_quat());
    vettura_chrono_quat out;
    out.w = ours.w;
    out.x = ours.x;
    out.y = ours.y;
    out.z = ours.z;
    return out;
}

/* ---------------- 车辆句柄 ---------------- */

class VetturaVehicleImpl {
  public:
    // 生成 JSON 并构造 Chrono 车辆；失败时返回 nullptr 并设置 last_error。
    static VetturaVehicleImpl* create(const vettura_chrono_spec* spec) {
        std::unique_ptr<VetturaVehicleImpl> impl(new VetturaVehicleImpl());
        if (!impl->setup(spec))
            return nullptr;
        return impl.release();
    }

    ~VetturaVehicleImpl() {
        // 清掉本实例写出的 Chrono JSON。用 error_code 版本：析构里不能抛异常。
        if (!m_temp_dir.empty()) {
            std::error_code ec;
            std::filesystem::remove_all(m_temp_dir, ec);
        }
    }

    int32_t reset(const vettura_chrono_pose* pose) {
        try {
            if (pose == nullptr)
                return kInvalidArgument;
            auto chassis = m_vehicle->GetChassisBody();

            // Chrono 10.0 起 ChVehicle::Relocate 已移除，改为直接摆放底盘参考帧
            // （ChBodyAuxRef::SetFrameRefToAbs，与 Chrono 自己的 ChChassis::Initialize
            // 走同一条路径）。注意不能用 ChassisBody()->SetPos()：那是 **COG 帧**，
            // 比参考帧高 cg_height，会把车整体插进地面。
            //
            // 只保留绕世界竖轴的 yaw（Chrono 的悬架/转向都以竖直姿态构建），
            // pitch/roll 丢弃——车总是摆回平地上。
            chrono::ChVector3d ref = to_chrono(pose->position);
            const chrono::ChQuaterniond q_chrono = to_chrono_quat(pose->orientation);
            const chrono::ChVector3d fwd = q_chrono.Rotate(chrono::ChVector3d(1, 0, 0));
            const double yaw = std::atan2(fwd.y(), fwd.x());
            const LocalQuat yaw_q =
                quat_normalize(LocalQuat{0.0, 0.0, std::sin(0.5 * yaw), std::cos(0.5 * yaw)});

            // 盘心不能低于地面（Chrono 会判定无接触 → 自由落体），所以参考帧的
            // 高度有下限：轮半径 + 间隙。低于下限就抬起来，让车落回悬架上。
            const double min_ref_z = spawn_height_m(&m_spec);
            if (ref.z() < min_ref_z)
                ref.z() = min_ref_z;

            chassis->SetFrameRefToAbs(chrono::ChFrame<>(
                ref, chrono::ChQuaterniond(yaw_q.w, yaw_q.x, yaw_q.y, yaw_q.z)));

            // 清零所有刚体速度（底盘 + 车轮/轮胎），避免残留动量。
            // 悬架/轮胎内部状态（滑移率、胎温等）不回溯：本后端的状态量都很短，
            // 几帧内就重新收敛；要彻底归零只能重建整辆车，代价太高。
            if (auto* system = m_vehicle->GetSystem()) {
                for (auto& body : system->GetBodies()) {
                    if (body.get() == nullptr)
                        continue;
                    body->SetPosDt(chrono::VNULL);
                    body->SetAngVelLocal(chrono::VNULL);
                }
            }
            m_time = 0.0;
            return kOk;
        } catch (const std::exception& e) {
            set_last_error(std::string("reset failed: ") + e.what());
            return kSolver;
        }
    }

    int32_t step(double dt, const vettura_chrono_controls* controls) {
        try {
            if (dt <= 0.0 || !is_finite(dt))
                return kInvalidArgument;
            if (!controls_are_valid(controls))
                return kInvalidArgument;

            chrono::vehicle::DriverInputs inputs;  // { m_steering, m_throttle, m_braking, m_clutch }
            inputs.m_steering = controls->steering;
            inputs.m_throttle = controls->throttle;
            inputs.m_braking = controls->brake;
            inputs.m_clutch = controls->clutch;
            // handbrake：挂驻车制动（锁所有制动器）。
            if (m_parking != static_cast<bool>(controls->handbrake)) {
                m_vehicle->ApplyParkingBrake(static_cast<bool>(controls->handbrake));
                m_parking = static_cast<bool>(controls->handbrake);
            }

            m_time += dt;
            m_vehicle->Synchronize(m_time, inputs, *m_terrain);
            m_vehicle->Advance(dt);

            // 累计车轮转角（供可视化/里程表用）。Chrono 不直接暴露转角，
            // 用角速度积分；GetSpindleOmega 的正方向就是前进方向。
            constexpr double kTwoPi = 6.283185307179586476925286766559;
            for (int axle = 0; axle < 2; ++axle) {
                for (int side = 0; side < 2; ++side) {
                    const auto vs = (side == 0) ? chrono::vehicle::LEFT : chrono::vehicle::RIGHT;
                    const int index = axle * 2 + side;
                    double angle =
                        m_wheel_rotation[index] + m_vehicle->GetSpindleOmega(axle, vs) * dt;
                    // 与 SimpleBackend 一致：取 [0, 2π) 的回绕。
                    angle -= kTwoPi * std::floor(angle / kTwoPi);
                    m_wheel_rotation[index] = angle;
                }
            }
            return kOk;
        } catch (const std::exception& e) {
            set_last_error(std::string("step failed: ") + e.what());
            return kSolver;
        }
    }

    int32_t read_state(vettura_chrono_state* out_state) {
        try {
            if (out_state == nullptr)
                return kInvalidArgument;
            std::memset(out_state, 0, sizeof(*out_state));

            auto chassis = m_vehicle->GetChassisBody();
            out_state->position = from_chrono(chassis->GetPos());
            out_state->orientation = from_chrono_quat(chassis->GetRot());
            out_state->linear_velocity = from_chrono(chassis->GetPosDt());
            // 世界系角速度（不是车体局部系）：与 SimpleBackend 的
            // `omega = (0, yaw_rate, 0)` 契约一致。
            out_state->angular_velocity = from_chrono(chassis->GetAngVelParent());
            out_state->speed_mps = static_cast<double>(m_vehicle->GetSpeed());

            if (m_engine != nullptr) {
                // GetMotorSpeed() 是 **rad/s**（ChEngineSimpleMap 直接缓存传动轴转速）。
                // 转成 rpm 后再夹到怠速：Chrono 的 AutomaticTransmissionSimpleMap 在
                // 停车时把发动机转速算成 0（模型里没有怠速控制器/液力变矩器蠕变），
                // 而真实的内燃机怠速时不会为 0。这里只影响上报值，不影响物理。
                constexpr double kRadPerSecToRpm = 60.0 / 6.283185307179586476925286766559;
                const double rpm = m_engine->GetMotorSpeed() * kRadPerSecToRpm;
                out_state->engine_rpm = std::max(rpm, m_spec.engine.idle_rpm);
                if (m_transmission != nullptr)
                    out_state->gear = static_cast<int>(m_transmission->GetCurrentGear());
            }

            for (int axle = 0; axle < 2; ++axle) {
                for (int side = 0; side < 2; ++side) {
                    // VehicleSide: LEFT = 0, RIGHT = 1
                    const chrono::vehicle::VehicleSide vs =
                        (side == 0) ? chrono::vehicle::LEFT : chrono::vehicle::RIGHT;
                    // 我们 wheels[] 的顺序：FL, FR, RL, RR
                    const int index = axle * 2 + side;

                    vettura_chrono_wheel_state& w = out_state->wheels[index];
                    w.angular_velocity = static_cast<double>(m_vehicle->GetSpindleOmega(axle, vs));
                    w.steer_angle = static_cast<double>(m_vehicle->GetSteeringAngle(axle, vs));
                    // 累计转角在 step() 里按 dt 累加（read_state 必须无副作用，
                    // 否则每帧多读一次就会多转一圈）。
                    w.rotation = m_wheel_rotation[index];

                    const auto tire = m_vehicle->GetTire(axle, vs);
                    if (tire != nullptr) {
                        w.slip_ratio = static_cast<double>(tire->GetLongitudinalSlip());
                        w.slip_angle = static_cast<double>(tire->GetSlipAngle());
                        chrono::vehicle::TerrainForce force = tire->ReportTireForce(m_terrain.get());
                        w.load_n = static_cast<double>(force.force.z());
                        // 触地判定：法向力大于单轮静载的 0.1%。
                        w.contact = (w.load_n > 1e-3 * m_spec.mass_kg * 9.81 / 4.0) ? 1 : 0;
                    }
                }
            }
            return kOk;
        } catch (const std::exception& e) {
            set_last_error(std::string("read_state failed: ") + e.what());
            return kSolver;
        }
    }

  private:
    VetturaVehicleImpl() = default;

    bool setup(const vettura_chrono_spec* spec) {
        m_spec = *spec;

        // ---- 生成 Chrono JSON 并定位 vehicle data path ----
        if (m_temp_dir.empty()) {
            std::error_code ec;
            auto base = std::filesystem::temp_directory_path(ec);
            if (ec)
                base = std::filesystem::path("/tmp");
            // **每个实例一个目录**：Chrono 的 vehicle data path 是进程级全局状态，
            // 两个 spec 不同的实例共用目录会互相覆盖 JSON。用实例地址做后缀
            // （不引入全局计数器），析构时整个删掉。
            const auto tag = reinterpret_cast<std::uintptr_t>(this);
            m_temp_dir = (base / ("vettura-chrono-" + std::to_string(get_pid()) + "-" +
                                  std::to_string(tag)))
                             .string();
        }
        std::error_code ec;
        std::filesystem::create_directories(m_temp_dir, ec);
        if (ec) {
            set_last_error("cannot create temp dir " + m_temp_dir + ": " + ec.message());
            return false;
        }

        std::string vehicle_file;
        if (!vettura::write_vehicle_spec(*spec, m_temp_dir, vehicle_file)) {
            set_last_error("cannot translate vettura_chrono_spec into Chrono vehicle JSON "
                           "(AWD / invalid gear ratios are not supported yet)");
            return false;
        }

        // 子系统的相对路径由 GetVehicleDataFile 拼 vehicle data path 解析
        // （Chrono 是裸字符串拼接，末尾的 '/' 必须带上，否则路径会粘连）。
        chrono::vehicle::SetVehicleDataPath(m_temp_dir + "/");

        // ---- 地形：平地，摩擦系数来自 spec ----
        m_terrain = std::make_shared<chrono::vehicle::FlatTerrain>(0.0, static_cast<float>(spec->tyres.peak_friction));

        // ---- 车辆：JSON 驱动，powertrain/tires 都由 JSON 就位 ----
        // 注意：WheeledVehicle::Create 对**顶层**文件是直接 ifstream 打开，不经过
        // data path（只有子系统文件才过），所以这里必须给绝对路径。
        const std::string abs_vehicle_json = m_temp_dir + "/" + vehicle_file;
        m_vehicle = std::make_shared<chrono::vehicle::WheeledVehicle>(
            abs_vehicle_json, chrono::ChContactMethod::NSC, /*create_powertrain=*/true,
            /*create_tires=*/true);

        // JSON 读失败时 WheeledVehicle::Create 会提前返回，留下一个空壳：
        // m_chassis 是 nullptr，后面 Initialize/Synchronize 会直接崩溃。
        // 这里拦下来，返回结构化错误而不是段错误。
        if (m_vehicle == nullptr || m_vehicle->GetChassis() == nullptr) {
            set_last_error("Chrono could not load the generated vehicle spec (" +
                           m_temp_dir + "/" + vehicle_file + "); see build.rs / the JSON writer");
            return false;
        }

        // ---- 初始化：从 spawn 高度出发 ----
        // 车轮中心不能低于地面：Chrono 的 DiscTerrainCollision 把「盘心在地形以下」
        // 判定为无接触，整车会直接自由落体。所以按 轮半径 + 5mm 间隙 抬起来，
        // 让轮胎几乎无冲击地贴上去（Chrono 自带车辆 demo 也是这个思路）。
        const double spawn_height = spawn_height_m(spec);
        m_vehicle->Initialize(chrono::ChCoordsys<>(chrono::ChVector3d(0, 0, spawn_height), chrono::QUNIT), 0.0);

        m_engine = m_vehicle->GetEngine();
        m_transmission = m_vehicle->GetTransmission();
        m_time = 0.0;
        return true;
    }

    static int get_pid() {
#if defined(_WIN32)
        return static_cast<int>(::_getpid());
#else
        return static_cast<int>(::getpid());
#endif
    }

    /// 出生高度：轮半径 + 一点点间隙。见 setup() 里的说明。
    ///
    /// 间隙只留 5 mm：留太大（比如整个悬架行程）会让整车自由落体砸到地上，
    /// 触地瞬间的法向冲击能把车弹起来，测试里就表现为载荷长时间震荡。
    static double spawn_height_m(const vettura_chrono_spec* spec) {
        double radius = 0.33;
        for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
            if (spec->wheels[i].radius_m > 0.0)
                radius = std::max(radius, spec->wheels[i].radius_m);
        }
        return radius + 0.005;
    }

    vettura_chrono_spec m_spec{};
    std::string m_temp_dir;
    std::shared_ptr<chrono::vehicle::WheeledVehicle> m_vehicle;
    std::shared_ptr<chrono::vehicle::ChTerrain> m_terrain;
    std::shared_ptr<chrono::vehicle::ChEngine> m_engine;
    std::shared_ptr<chrono::vehicle::ChTransmission> m_transmission;
    double m_time = 0.0;
    bool m_parking = false;
    double m_wheel_rotation[VETTURA_CHRONO_WHEEL_COUNT] = {0, 0, 0, 0};
};

}  // namespace

/* ---------------- C ABI 导出 ---------------- */

extern "C" int32_t vettura_chrono_abi_version(void) {
    return VETTURA_CHRONO_ABI_VERSION;
}

extern "C" void vettura_chrono_build_id(char* out, int32_t out_len) {
    if (out == nullptr || out_len <= 0)
        return;
    std::string id = "vettura-chrono/";
#if defined(_MSC_VER)
    id += "msvc=" + std::to_string(_MSC_VER) + " ";
#elif defined(__clang__)
    id += "clang=" + std::to_string(__clang_major__) + "." + std::to_string(__clang_minor__) + "." +
          std::to_string(__clang_patchlevel__) + " ";
#elif defined(__GNUC__)
    id += "gcc=" + std::to_string(__GNUC__) + "." + std::to_string(__GNUC_MINOR__) + " ";
#endif
#ifdef CHRONO_VERSION
    id += "chrono=" CHRONO_VERSION " ";
#else
    id += "chrono=unknown ";
#endif
    // FP 契约（见 docs/build.md §8.2；不一致会让不同编译器结果分叉）。
#if defined(__clang__) || defined(__GNUC__)
    id += "fp=strict-no-contract ";
#elif defined(_MSC_VER)
    id += "fp=strict ";
#endif
    std::strncpy(out, id.c_str(), static_cast<size_t>(out_len - 1));
    out[out_len - 1] = '\0';
}

extern "C" int32_t vettura_chrono_create(const vettura_chrono_spec* spec,
                                         vettura_chrono_vehicle** out) {
    if (out == nullptr)
        return kInvalidArgument;
    *out = nullptr;
    if (!spec_is_valid(spec))
        return kInvalidArgument;
    if (vettura_chrono_abi_version() != VETTURA_CHRONO_ABI_VERSION) {
        // ABI 版本不符：绝不跨版本链接。
        set_last_error("bridge ABI version mismatch");
        return kInvalidArgument;
    }
    VetturaVehicleImpl* impl = VetturaVehicleImpl::create(spec);
    if (impl == nullptr) {
        // last_error 已在 create() 内设置。
        return kInvalidSpec;
    }
    *out = reinterpret_cast<vettura_chrono_vehicle*>(impl);
    return kOk;
}

extern "C" void vettura_chrono_destroy(vettura_chrono_vehicle* vehicle) {
    delete reinterpret_cast<VetturaVehicleImpl*>(vehicle);
}

extern "C" int32_t vettura_chrono_reset(vettura_chrono_vehicle* vehicle, const vettura_chrono_pose* pose) {
    if (vehicle == nullptr || pose == nullptr)
        return kInvalidArgument;
    return reinterpret_cast<VetturaVehicleImpl*>(vehicle)->reset(pose);
}

extern "C" int32_t vettura_chrono_step(vettura_chrono_vehicle* vehicle,
                                       double dt,
                                       const vettura_chrono_controls* controls) {
    if (vehicle == nullptr || dt <= 0.0 || controls == nullptr)
        return kInvalidArgument;
    return reinterpret_cast<VetturaVehicleImpl*>(vehicle)->step(dt, controls);
}

extern "C" int32_t vettura_chrono_read_state(const vettura_chrono_vehicle* vehicle,
                                             vettura_chrono_state* out_state) {
    if (out_state == nullptr)
        return kInvalidArgument;
    if (vehicle == nullptr)
        return kInvalidArgument;
    // read_state 不改车辆状态，但 Chrono 的 ReportTireForce 不是 const。
    auto* impl = const_cast<VetturaVehicleImpl*>(reinterpret_cast<const VetturaVehicleImpl*>(vehicle));
    return impl->read_state(out_state);
}

extern "C" const char* vettura_chrono_last_error(void) {
    return g_last_error.empty() ? nullptr : g_last_error.c_str();
}
