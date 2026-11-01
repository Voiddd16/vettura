/*!
 * @file vettura_vehicle_spec.cpp
 *
 * vettura_chrono_spec → Chrono::Vehicle JSON 规范生成器。
 *
 * Chrono 的 wheeled vehicle 是 **JSON 驱动**的（见
 * `chrono_vehicle/wheeled_vehicle/vehicle/WheeledVehicle::Create`）：顶层车辆
 * JSON 引用若干子系统 JSON（chassis / suspension / wheel / brake / steering /
 * driveline / powertrain / tire）。这个翻译器的职责就是把我们的 backend-neutral
 * `vettura_chrono_spec` 落到这套 schema 上。
 *
 * 数据落盘成文件是必须的：`WheeledVehicle` 按文件名读取，且子系统路径由
 * `chrono::vehicle::GetVehicleDataFile` 相对 *vehicle data path* 解析，
 * 所以生成的文件挂在 `data_dir/vehicle/` 下（bridge 用
 * `SetChronoDataPath(data_dir)` 指向它）。
 *
 * 只依赖标准 C++：这份代码不需要 Chrono 就能编译和单测。
 */

#include "vettura_vehicle_spec.h"

#include <algorithm>

#include <cerrno>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <sys/stat.h>
#include <sys/types.h>
#include <vector>

namespace vettura {
namespace {

// Chrono vehicle data path 下的相对位置；bridge 会 SetChronoDataPath(data_dir)，
// 顶层车辆文件必须落在 <data_dir>/vehicle/ 下（WheeledVehicle::Create 里
// 子系统路径由 GetVehicleDataFile 解析，它就是拼 data path）。
constexpr const char* kVehicleJsonPath = "vehicle/vettura_vehicle.json";

// 与 vettura_chrono_spec::gear_ratios 对应（index 0 = 一档）。
constexpr int kMaxGears = VETTURA_CHRONO_MAX_GEAR_COUNT;

/* ---------------- 极简 JSON 输出助手 ----------------
 * 只生成、不解析；数字用最短往返表示，保证 double 读写一致。
 *
 * 缩进 = 2 空格 × 当前容器深度。容器状态用一个栈显式维护，避免「写完之后
 * 才发现字符串不闭合」这类问题。 */

class JsonWriter {
  public:
    void begin_object() {
        value_open("{");
        stack_.push_back(kObject);
        needs_separator_ = false;
    }

    void end_object() {
        if (stack_.empty() || stack_.back() != kObject)
            return;
        stack_.pop_back();
        if (needs_separator_) {
            out_ << "\n";
            indent();
        }
        out_ << "}";
        set_separator();
    }

    void begin_array() {
        value_open("[");
        stack_.push_back(kArray);
        needs_separator_ = false;
    }

    void end_array() {
        if (stack_.empty() || stack_.back() != kArray)
            return;
        stack_.pop_back();
        if (needs_separator_) {
            out_ << "\n";
            indent();
        }
        out_ << "]";
        set_separator();
    }

    void key(const std::string& name) { pending_key_ = "\"" + name + "\""; }

    void number(double v) { value_open(number_to_string(v)); }

    void integer(int v) { value_open(std::to_string(v)); }

    void boolean(bool v) { value_open(v ? "true" : "false"); }

    void string(const std::string& v) { value_open(quote(v)); }

    void vector3(double x, double y, double z) {
        begin_array();
        number(x);
        number(y);
        number(z);
        end_array();
    }

    /// Chrono 的四元数 JSON 字段是 4 元数组 [w, x, y, z]（ReadQuaternionJSON 会
    /// 断言 Size()==4）。Release 构建下断言被关掉，写成 3 元数组会越界读到垃圾，
    /// 得到非法朝向 —— 表现是车身完全无法偏航。恒等四元数统一走这里。
    void quaternion_identity() {
        begin_array();
        number(1.0);
        number(0.0);
        number(0.0);
        number(0.0);
        end_array();
    }

    std::string str() const { return out_.str(); }

    // 把缓冲写进文件；返回 false 时说明打不开/写不动。
    bool write_to(const std::string& path) const {
        std::ofstream f(path, std::ios::binary | std::ios::trunc);
        if (!f)
            return false;
        f << out_.str();
        return static_cast<bool>(f);
    }

  private:
    enum Kind { kObject, kArray };

    void indent() {
        for (size_t i = 0; i < stack_.size(); ++i)
            out_ << "  ";
    }

    // 写入一个「值」：负责逗号、换行、缩进、以及可选的 `key:` 前缀。
    void value_open(const std::string& literal) {
        if (needs_separator_)
            out_ << ",";
        out_ << "\n";
        indent();
        if (!pending_key_.empty()) {
            out_ << pending_key_ << ": ";
            pending_key_.clear();
        }
        out_ << literal;
        set_separator();
    }

    // 只有非空容器在写下一个成员前需要逗号；空容器不需要
    // （{ "a": 1, "b": {} } 合法，{ , } 不合法）。
    void set_separator() { needs_separator_ = !stack_.empty(); }

    static std::string quote(const std::string& v) {
        std::string escaped = "\"";
        for (char c : v) {
            switch (c) {
                case '"': escaped += "\\\""; break;
                case '\\': escaped += "\\\\"; break;
                case '\n': escaped += "\\n"; break;
                case '\t': escaped += "\\t"; break;
                default: escaped += c;
            }
        }
        escaped += "\"";
        return escaped;
    }

    static std::string number_to_string(double v) {
        char buf[64];
        // %.17g 一定往返成功，但难读：从头开始找最短的往返表示。
        for (int precision = 1; precision <= 17; ++precision) {
            int n = std::snprintf(buf, sizeof(buf), "%.*g", precision, v);
            if (n > 0 && static_cast<size_t>(n) < sizeof(buf) && std::strtod(buf, nullptr) == v) {
                return std::string(buf, static_cast<size_t>(n));
            }
        }
        return std::string(buf);
    }

    std::ostringstream out_;
    std::string pending_key_;
    bool needs_separator_ = false;
    std::vector<int> stack_;
};

std::string path_join(const std::string& dir, const std::string& rel) {
    if (dir.empty())
        return rel;
    if (dir.back() == '/')
        return dir + rel;
    return dir + "/" + rel;
}

bool ensure_dir(const std::string& path) {
    if (path.empty())
        return true;
    std::string current;
    size_t i = 0;
    if (path[0] == '/') {
        current = "/";
        i = 1;
    }
    while (i < path.size()) {
        size_t next = path.find('/', i);
        if (next == std::string::npos)
            next = path.size();
        if (next > i) {
            if (!current.empty() && current.back() != '/')
                current += '/';
            current += path.substr(i, next - i);
            if (::mkdir(current.c_str(), 0755) != 0 && errno != EEXIST)
                return false;
        }
        i = next + 1;
    }
    return true;
}

/* ---------------- 子系统 JSON ----------------
 * 每个子系统一个文件；schema 都是从上游 Chrono 的 data/vehicle/hmmwv 目录
 * 里的 JSON 搬来的，字段名必须与之一致。 */

// ---- 质量预算 ----
//
// `vettura_chrono_spec::mass_kg` 是**整车**质量（含车轮/轮胎/转向节），但 Chrono
// 里这些是独立刚体：底盘 JSON 的 Mass 只表示簧上质量。如果底盘直接写整车质量，
// 总质量会多出一整套轮端的重量（实测 1200 kg 的车变成 1578 kg，轮荷全部偏高
// 30%，胎压/轮胎参数也跟着偏）。
//
// 这里按真实乘用车的比例把整车质量拆开：轮端四件（Spindle/Upright/Wheel/Tire）
// 每个角合计约 1.7%，转向拉杆 0.5%，其余都给底盘。总和≈spec.mass_kg。
struct MassBudget {
    double spindle;       ///< 每个转向节
    double upright;       ///< 每个立柱
    double wheel;         ///< 每个轮辋
    double tire;          ///< 每条轮胎
    double steering_link; ///< 转向拉杆（整根）
    double chassis;       ///< 簧上质量（底盘 JSON 的 Mass）
};

MassBudget mass_budget(const vettura_chrono_spec& spec) {
    const double m = spec.mass_kg;
    MassBudget b;
    b.spindle = 0.004 * m;
    b.upright = 0.003 * m;
    b.wheel = 0.004 * m;
    b.tire = 0.006 * m;
    b.steering_link = 0.005 * m;
    b.chassis = m - 4.0 * (b.spindle + b.upright + b.wheel + b.tire) - b.steering_link;
    // 极端 spec（质量很小、轮端比例固定）时兜底，保证底盘质量为正。
    b.chassis = std::max(b.chassis, 0.5 * m);
    return b;
}

// Chassis: Template "RigidChassis"
void write_chassis(const vettura_chrono_spec& spec, JsonWriter& w) {
    w.begin_object();
    w.key("Name");
    w.string("vettura chassis");
    w.key("Type");
    w.string("Chassis");
    w.key("Template");
    w.string("RigidChassis");

    w.key("Components");
    w.begin_array();
    w.begin_object();
    w.key("Centroidal Frame");
    w.begin_object();
    w.key("Location");
    w.vector3(0.0, 0.0, spec.cg_height_m);
    w.key("Orientation");
    w.quaternion_identity();  // [w, x, y, z] 单位四元数 = 恒等
    w.end_object();
    // 底盘只承担簧上质量：轮端（Spindle/Upright/Wheel/Tire）在 Chrono 里是独立
    // 刚体，若这里直接写整车质量，总质量会多出一整套轮端（见 mass_budget）。
    w.key("Mass");
    w.number(mass_budget(spec).chassis);
    // 把整车近似成长方体算惯性矩：x 滚动、y 俯仰、z 横摆。
    const double length = spec.wheelbase_m;
    const double width = std::fabs(spec.wheels[0].position.x) + std::fabs(spec.wheels[1].position.x) + 2e-3;
    const double height = 2.0 * spec.cg_height_m + 1e-3;
    const double ixx = spec.mass_kg * (width * width + height * height) / 12.0;
    const double iyy = spec.mass_kg * (length * length + height * height) / 12.0;
    const double izz = spec.mass_kg * (length * length + width * width) / 12.0;
    w.key("Moments of Inertia");
    w.vector3(ixx, iyy, izz);
    w.key("Products of Inertia");
    w.vector3(0.0, 0.0, 0.0);
    w.key("Void");
    w.boolean(false);
    w.end_object();
    w.end_array();

    w.key("Driver Position");
    w.begin_object();
    w.key("Location");
    w.vector3(0.3 * spec.wheelbase_m, 0.0, spec.cg_height_m + 0.6);
    w.key("Orientation");
    w.quaternion_identity();
    w.end_object();
    w.key("Rear Connector Location");
    w.vector3(-0.7 * spec.wheelbase_m, 0.0, -0.2 * spec.cg_height_m);
    w.end_object();
}

// Suspension: Template "DoubleWishboneReduced"
// 我们的 spec 只有弹簧/阻尼/行程，双摇臂硬点用与 HMMWV 成比例的默认几何，
// 需要的更多细节（硬点、衬套）走后续的 ChronoSpec 扩展，不污染 VehicleSpec。
void write_suspension(const vettura_chrono_spec& spec, JsonWriter& w) {
    // HMMWV DWA-reduced（未装载半径 0.4699 m）等比缩放到当前胎半径。
    const double k = spec.wheels[0].radius_m / 0.4699;
    const MassBudget budget = mass_budget(spec);
    const double spindle_mass = budget.spindle;
    const double upright_mass = budget.upright;

    w.begin_object();
    w.key("Name");
    w.string("vettura suspension");
    w.key("Type");
    w.string("Suspension");
    w.key("Template");
    w.string("DoubleWishboneReduced");

    w.key("Camber Angle (deg)");
    w.number(0.0);
    w.key("Toe Angle (deg)");
    w.number(0.0);

    w.key("Spindle");
    w.begin_object();
    w.key("Mass");
    w.number(spindle_mass);
    w.key("COM");
    w.vector3(-0.040 * k, 0.910 * k, -0.026 * k);
    w.key("Inertia");
    w.vector3(spindle_mass * 0.001, spindle_mass * 0.002, spindle_mass * 0.001);
    w.key("Radius");
    w.number(0.15 * k);
    w.key("Width");
    w.number(0.06 * k);
    w.end_object();

    w.key("Upright");
    w.begin_object();
    w.key("Mass");
    w.number(upright_mass);
    w.key("COM");
    w.vector3(-0.040 * k, 0.751 * k, -0.026 * k);
    w.key("Inertia");
    w.vector3(upright_mass * 0.001, upright_mass * 0.001, upright_mass * 0.001);
    w.key("Radius");
    w.number(0.025 * k);
    w.end_object();

    w.key("Upper Control Arm");
    w.begin_object();
    w.key("Location Chassis Front");
    w.vector3(-0.048 * k, 0.446 * k, 0.245 * k);
    w.key("Location Chassis Back");
    w.vector3(-0.268 * k, 0.478 * k, 0.196 * k);
    w.key("Location Upright");
    w.vector3(-0.053 * k, 0.716 * k, 0.215 * k);
    w.end_object();

    w.key("Lower Control Arm");
    w.begin_object();
    w.key("Location Chassis Front");
    w.vector3(0.223 * k, 0.307 * k, 0.0);
    w.key("Location Chassis Back");
    w.vector3(-0.223 * k, 0.307 * k, 0.0);
    w.key("Location Upright");
    w.vector3(-0.036 * k, 0.787 * k, -0.118 * k);
    w.end_object();

    w.key("Tierod");
    w.begin_object();
    w.key("Location Chassis");
    w.vector3(-0.250 * k, 0.448 * k, 0.054 * k);
    w.key("Location Upright");
    w.vector3(-0.176 * k, 0.821 * k, -0.016 * k);
    w.end_object();

    w.key("Shock");
    w.begin_object();
    w.key("Location Chassis");
    w.vector3(0.104 * k, 0.498 * k, 0.323 * k);
    w.key("Location Upright");
    w.vector3(0.097 * k, 0.543 * k, -0.047 * k);
    w.key("Free Length");
    w.number(0.339 * k);
    w.key("Spring Coefficient");
    w.number(spec.wheels[0].suspension.spring_rate_n_per_m);
    w.key("Damping Coefficient");
    const double damping = (spec.wheels[0].suspension.damper_bump_ns_per_m +
                            spec.wheels[0].suspension.damper_rebound_ns_per_m) /
                           2.0;
    w.number(damping);
    w.end_object();

    w.key("Axle");
    w.begin_object();
    w.key("Inertia");
    w.number(0.4);
    w.end_object();
    w.end_object();
}

// Wheel: Template "Wheel"
void write_wheel(const vettura_chrono_spec& spec, JsonWriter& w) {
    const double mass = mass_budget(spec).wheel;
    const double inertia = 0.5 * mass * spec.wheels[0].radius_m * spec.wheels[0].radius_m;
    w.begin_object();
    w.key("Name");
    w.string("vettura wheel");
    w.key("Type");
    w.string("Wheel");
    w.key("Template");
    w.string("Wheel");
    w.key("Mass");
    w.number(mass);
    w.key("Inertia");
    w.vector3(inertia, inertia, inertia);
    w.key("Visualization");
    w.begin_object();
    w.key("Mesh Filename");
    w.string("");  // 没有网格文件：不渲染，纯物理
    w.key("Radius");
    w.number(spec.wheels[0].radius_m);
    w.key("Width");
    w.number(spec.wheels[0].width_m);
    w.end_object();
    w.end_object();
}

// Brake: Template "BrakeSimple"
void write_brake(const vettura_chrono_spec& spec, JsonWriter& w) {
    // 单轮峰值制动力矩 ≈ 静载 × 摩擦系数 × 半径；这是上限值，实际由输入调制。
    const double load = spec.mass_kg * 9.81 / 4.0;
    w.begin_object();
    w.key("Name");
    w.string("vettura brake");
    w.key("Type");
    w.string("Brake");
    w.key("Template");
    w.string("BrakeSimple");
    w.key("Maximum Torque");
    w.number(load * spec.tyres.peak_friction * spec.wheels[0].radius_m);
    w.end_object();
}

// Steering: Template "RackPinion"
//
// 齿条行程 = 小齿轮半径 × 最大转角，再通过转向节臂转成车轮转角。我们的悬架
// 硬点是从 HMMWV 等比缩放的（k = 胎半径 / 0.4699），所以转向几何也按同一 k 缩放：
// 等比缩放下车轮转角保持不变，生成的转向特性与 HMMWV 一致（满舵约 26°）。
// 之前用固定值 0.1*胎半径 当小齿轮半径，满舵只有 ~10°，转向明显偏钝。
void write_steering(const vettura_chrono_spec& spec, JsonWriter& w) {
    const double k = spec.wheels[0].radius_m / 0.4699;
    const double link_mass = mass_budget(spec).steering_link;
    w.begin_object();
    w.key("Name");
    w.string("vettura steering");
    w.key("Type");
    w.string("Steering");
    w.key("Template");
    w.string("RackPinion");
    w.key("Steering Link");
    w.begin_object();
    w.key("Mass");
    w.number(link_mass);
    w.key("COM");
    w.number(0.0);
    w.key("Inertia");
    w.vector3(1.0, 1.0, 1.0);
    w.key("Radius");
    w.number(0.03 * k);
    w.key("Length");
    w.number(0.896 * k);
    w.end_object();
    w.key("Pinion");
    w.begin_object();
    w.key("Radius");
    w.number(0.1 * k);
    w.key("Maximum Angle (deg)");
    w.number(50.0);
    w.end_object();
    w.end_object();
}

// Driveline: Template "ShaftsDriveline2WD"
void write_driveline(const vettura_chrono_spec& spec, JsonWriter& w) {
    w.begin_object();
    w.key("Name");
    w.string("vettura driveline");
    w.key("Type");
    w.string("Driveline");
    w.key("Template");
    w.string("ShaftsDriveline2WD");
    w.key("Shaft Direction");
    w.begin_object();
    w.key("Motor Block");
    w.vector3(1.0, 0.0, 0.0);
    w.key("Axle");
    w.vector3(0.0, 1.0, 0.0);
    w.end_object();
    w.key("Shaft Inertia");
    w.begin_object();
    w.key("Driveshaft");
    w.number(0.5);
    w.key("Differential Box");
    w.number(0.6);
    w.end_object();
    w.key("Gear Ratio");
    w.begin_object();
    w.key("Conical Gear");
    // Chrono 用 output/input，spec 的 final_drive 是 input/output，取倒数。
    w.number(1.0 / spec.engine.final_drive);
    w.end_object();
    w.key("Axle Differential Locking Limit");
    w.number(100.0);
    w.end_object();
}

// Engine: Template "EngineSimpleMap"（怠速 → 红线之间的一条简化扭矩曲线）
void write_engine(const vettura_chrono_spec& spec, JsonWriter& w) {
    const double idle = spec.engine.idle_rpm;
    const double redline = spec.engine.redline_rpm;
    const double peak_rpm = idle + 0.72 * (redline - idle);

    w.begin_object();
    w.key("Name");
    w.string("vettura engine");
    w.key("Type");
    w.string("Engine");
    w.key("Template");
    w.string("EngineSimpleMap");
    w.key("Maximal Engine Speed RPM");
    w.number(redline);

    w.key("Map Full Throttle");
    w.begin_array();
    // [rpm, torque(Nm)]；线性包络：idle→峰值→redline。
    for (int i = 0; i <= 8; ++i) {
        const double rpm = idle + (redline - idle) * i / 8.0;
        const double frac = (rpm - idle) / std::max(redline - idle, 1e-9);
        double torque;
        if (rpm < peak_rpm)
            torque = 0.45 + 0.55 * frac / 0.72;
        else
            torque = 1.0 - 0.65 * (frac - 0.72) / 0.28;
        w.begin_array();
        w.number(rpm);
        w.number(spec.engine.peak_torque_nm * torque);
        w.end_array();
    }
    w.end_array();

    w.key("Map Zero Throttle");
    w.begin_array();
    w.begin_array();
    w.number(-100.0);
    w.number(0.0);
    w.end_array();
    w.begin_array();
    w.number(0.0);
    w.number(0.0);
    w.end_array();
    w.begin_array();
    w.number(idle);
    w.number(-0.05 * spec.engine.peak_torque_nm);
    w.end_array();
    w.begin_array();
    w.number(redline);
    w.number(-0.3 * spec.engine.peak_torque_nm);
    w.end_array();
    w.end_array();
    w.end_object();
}

// Transmission: Template "AutomaticTransmissionSimpleMap"
void write_transmission(const vettura_chrono_spec& spec, JsonWriter& w) {
    w.begin_object();
    w.key("Name");
    w.string("vettura transmission");
    w.key("Type");
    w.string("Transmission");
    w.key("Template");
    w.string("AutomaticTransmissionSimpleMap");
    w.key("Gear Box");
    w.begin_object();
    w.key("Reverse Gear Ratio");
    w.number(-0.3 / spec.engine.gear_ratios[0]);
    w.key("Forward Gear Ratios");
    w.begin_array();
    for (int i = 0; i < spec.engine.gear_count; ++i)
        // spec 的齿比是 input/output，Chrono 要 output/input。
        w.number(1.0 / spec.engine.gear_ratios[i]);
    w.end_array();
    w.key("Shift Points Map RPM");
    w.begin_array();
    for (int i = 0; i < spec.engine.gear_count; ++i) {
        w.begin_array();
        w.number(spec.engine.idle_rpm);
        // 最后一档的升档点贴近红线，其余档位按怠速的 2.2 倍升档。
        w.number((i == spec.engine.gear_count - 1) ? 0.95 * spec.engine.redline_rpm
                                                   : 2.2 * spec.engine.idle_rpm);
        w.end_array();
    }
    w.end_array();
    w.end_object();
    w.end_object();
}

// Tire: Template "TMsimpleTire"（handling tire，只能配非可变形地形）
//
// 必须走 "Parameters"（全参数化）分支：另外两条分支（Load Index /
// Maximum Bearing Capacity）都会用承载能力**重算轮胎半径**，把 Design 里的
// "Unloaded Radius" 覆盖掉 —— 我们给出的 spec 半径 0.33 会被改成 0.223，
// 轮心于是落到地面以下，Chrono 直接判定「盘心在地形下 → 无接触」而整车自由落体。
//
// 系数照抄 Chrono 自带的乘用车轮胎 Guess* 缺省值（ChTMsimpleTire.cpp 里那张
// Pacejka 手册系数表），只把纵/横向最大力按 spec 的峰值摩擦系数缩放。
void write_tire(const vettura_chrono_spec& spec, JsonWriter& w) {
    const double radius = spec.wheels[0].radius_m;
    const double width = spec.wheels[0].width_m;
    const double mass = mass_budget(spec).tire;
    const double rim_radius = 0.65 * radius;

    // 单轮名义载荷与轮胎垂向特性
    const double pn = spec.mass_kg * 9.81 / 4.0;
    const double defl_max = 0.16 * (radius - rim_radius);  // Chrono 同款的静挠度比例
    const double cz_tire = pn / std::max(defl_max, 1e-3);
    // DWA 模板没有弹簧/减振器，垂向柔度全部由轮胎承担。要让 spec 的弹簧刚度
    // 真正起作用，这里用「悬架弹簧 + 轮胎」串联的等效刚度：簧越软，车越低、
    // 侧倾越大。同时给静挠度设上限（不超过行程的一半），避免软簧把车坐进轮拱。
    const double cz_susp = spec.wheels[0].suspension.spring_rate_n_per_m;
    double cz = (cz_susp > 0.0) ? (cz_tire * cz_susp) / (cz_tire + cz_susp) : cz_tire;
    const double cz_min =
        pn / std::max(0.5 * spec.wheels[0].suspension.travel_m, 0.02);
    cz = std::max(cz, cz_min);
    // 阻尼直接用 spec 的压缩/回弹平均值（N·s/m），不再从轮胎质量反推。
    const double dz = 0.5 * (spec.wheels[0].suspension.damper_bump_ns_per_m +
                             spec.wheels[0].suspension.damper_rebound_ns_per_m);
    const double dz_safe = std::max(dz, 100.0);
    // 摩擦缩放：Chrono 缺省按 mu_0 = 0.8 给纵/横向力上限。
    const double fric = spec.tyres.peak_friction / 0.8;

    w.begin_object();
    w.key("Name");
    w.string("vettura tire");
    w.key("Type");
    w.string("Tire");
    w.key("Template");
    w.string("TMsimpleTire");

    w.key("Design");
    w.begin_object();
    w.key("Unloaded Radius [m]");
    w.number(radius);
    w.key("Mass [kg]");
    w.number(mass);
    w.key("Inertia [kg.m2]");
    w.vector3(0.5 * mass * radius * radius, mass * radius * radius, 0.5 * mass * radius * radius);
    w.key("Width [m]");
    w.number(width);
    w.key("Rim Radius [m]");
    w.number(rim_radius);
    w.end_object();

    w.key("Coefficient of Friction");
    w.number(spec.tyres.peak_friction);
    w.key("Rolling Resistance Coefficient");
    w.number(0.015);

    w.key("Parameters");
    w.begin_object();
    w.key("Vertical");
    w.begin_object();
    w.key("Nominal Vertical Force [N]");
    w.number(pn);
    w.key("Maximum Vertical Force [N]");
    w.number(3.5 * pn);
    w.key("Vertical Tire Stiffness [N/m]");
    w.number(cz);
    w.key("Vertical Tire Damping [Ns/m]");
    w.number(dz_safe);
    w.end_object();

    w.key("Longitudinal");
    w.begin_object();
    w.key("Initial Slopes dFx/dsx [N]");
    w.vector3(19.9774 * pn, 20.8424 * 2.0 * pn, 0.0);
    w.key("Maximum Fx Load [N]");
    w.vector3(1.1404 * pn * fric, 1.1045 * 2.0 * pn * fric, 0.0);
    w.key("Sliding Fx Load [N]");
    w.vector3(0.84482 * pn * fric, 0.82041 * 2.0 * pn * fric, 0.0);
    w.end_object();

    w.key("Lateral");
    w.begin_object();
    w.key("Initial Slopes dFy/dsy [N]");
    w.vector3(16.7895 * pn, 12.9866 * 2.0 * pn, 0.0);
    w.key("Maximum Fy Load [N]");
    w.vector3(1.0107 * pn * fric, 0.91902 * 2.0 * pn * fric, 0.0);
    w.key("Sliding Fy Load [N]");
    w.vector3(0.84864 * pn * fric, 0.78702 * 2.0 * pn * fric, 0.0);
    w.end_object();

    w.end_object();  // Parameters
    w.end_object();
}

}  // namespace

/* ---------------- 入口 ---------------- */

bool write_vehicle_spec(const vettura_chrono_spec& spec, const std::string& data_dir,
                        std::string& vehicle_file_out) {
    // ---- 校验：只生成我们能忠实表达的组合 ----
    if (spec.mass_kg <= 0.0 || spec.wheelbase_m <= 0.0)
        return false;
    for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
        if (spec.wheels[i].radius_m <= 0.0)
            return false;
    }
    if (spec.engine.gear_count < 1 || spec.engine.gear_count > kMaxGears)
        return false;
    for (int i = 0; i < spec.engine.gear_count; ++i) {
        if (spec.engine.gear_ratios[i] == 0.0)
            return false;
    }
    if (spec.engine.final_drive == 0.0)
        return false;
    // AWD 需要 ShaftsDriveline4WD，schema 还没接。
    if (spec.drivetrain.layout == VETTURA_CHRONO_LAYOUT_AWD)
        return false;
    // 悬架参数左右一致（生成器只写一份 per-axle 的悬架 JSON）。
    for (int i = 0; i < VETTURA_CHRONO_WHEEL_COUNT; ++i) {
        if (spec.wheels[i].suspension.spring_rate_n_per_m !=
                spec.wheels[0].suspension.spring_rate_n_per_m ||
            spec.wheels[i].suspension.travel_m != spec.wheels[0].suspension.travel_m)
            return false;
    }

    const std::string base = path_join(data_dir, "vettura");
    const std::string vehicle_dir = path_join(data_dir, "vehicle");
    for (const std::string& dir : {base + "/chassis", base + "/suspension", base + "/wheel",
                                   base + "/brake", base + "/steering", base + "/driveline",
                                   base + "/powertrain", base + "/tire", vehicle_dir}) {
        if (!ensure_dir(dir))
            return false;
    }

    struct File {
        const char* rel;
        void (*write)(const vettura_chrono_spec&, JsonWriter&);
    };
    const File files[] = {
        {"vettura/chassis/chassis.json", write_chassis},
        {"vettura/suspension/front.json", write_suspension},
        {"vettura/suspension/rear.json", write_suspension},
        {"vettura/wheel/wheel.json", write_wheel},
        {"vettura/brake/front.json", write_brake},
        {"vettura/brake/rear.json", write_brake},
        {"vettura/steering/steering.json", write_steering},
        {"vettura/driveline/driveline.json", write_driveline},
        {"vettura/powertrain/engine.json", write_engine},
        {"vettura/powertrain/transmission.json", write_transmission},
        {"vettura/tire/tire.json", write_tire},
    };
    for (const File& file : files) {
        JsonWriter w;
        file.write(spec, w);
        if (!w.write_to(path_join(data_dir, file.rel)))
            return false;
    }

    // ---- 顶层车辆 JSON ----
    const bool driven_axle_is_rear = (spec.drivetrain.layout == VETTURA_CHRONO_LAYOUT_RWD);
    const int driven_index = driven_axle_is_rear ? 1 : 0;  // 0 = 前轴, 1 = 后轴
    const double max_steer_rad = 30.0 * M_PI / 180.0;
    const double min_turn_radius = spec.wheelbase_m / std::tan(max_steer_rad);

    JsonWriter w;
    w.begin_object();
    w.key("Name");
    w.string("vettura vehicle");
    w.key("Type");
    w.string("Vehicle");
    w.key("Template");
    w.string("WheeledVehicle");

    w.key("Chassis");
    w.begin_object();
    w.key("Input File");
    w.string("vettura/chassis/chassis.json");
    w.end_object();

    w.key("Axles");
    w.begin_array();
    // 前轴（可转向）：wheels[0]/[1]
    w.begin_object();
    w.key("Suspension Input File");
    w.string("vettura/suspension/front.json");
    w.key("Suspension Location");
    w.vector3(0.5 * spec.wheelbase_m, 0.0, 0.0);
    w.key("Steering Index");
    w.integer(0);
    w.key("Left Wheel Input File");
    w.string("vettura/wheel/wheel.json");
    w.key("Right Wheel Input File");
    w.string("vettura/wheel/wheel.json");
    w.key("Left Brake Input File");
    w.string("vettura/brake/front.json");
    w.key("Right Brake Input File");
    w.string("vettura/brake/front.json");
    w.key("Tire Input File");
    w.string("vettura/tire/tire.json");
    w.end_object();
    // 后轴
    w.begin_object();
    w.key("Suspension Input File");
    w.string("vettura/suspension/rear.json");
    w.key("Suspension Location");
    w.vector3(-0.5 * spec.wheelbase_m, 0.0, 0.0);
    w.key("Left Wheel Input File");
    w.string("vettura/wheel/wheel.json");
    w.key("Right Wheel Input File");
    w.string("vettura/wheel/wheel.json");
    w.key("Left Brake Input File");
    w.string("vettura/brake/rear.json");
    w.key("Right Brake Input File");
    w.string("vettura/brake/rear.json");
    w.key("Tire Input File");
    w.string("vettura/tire/tire.json");
    w.end_object();
    w.end_array();

    w.key("Steering Subsystems");
    w.begin_array();
    w.begin_object();
    w.key("Input File");
    w.string("vettura/steering/steering.json");
    w.key("Location");
    w.vector3(0.5 * spec.wheelbase_m, 0.0, 0.0);
    w.key("Orientation");
    w.quaternion_identity();  // 恒等
    w.end_object();
    w.end_array();

    w.key("Wheelbase");
    w.number(spec.wheelbase_m);
    w.key("Minimum Turning Radius");
    w.number(min_turn_radius);
    w.key("Maximum Steering Angle (deg)");
    w.number(30.0);

    w.key("Driveline");
    w.begin_object();
    w.key("Input File");
    w.string("vettura/driveline/driveline.json");
    w.key("Suspension Indexes");
    w.begin_array();
    w.integer(driven_index);
    w.end_array();
    w.end_object();

    w.key("Powertrain");
    w.begin_object();
    w.key("Engine Input File");
    w.string("vettura/powertrain/engine.json");
    w.key("Transmission Input File");
    w.string("vettura/powertrain/transmission.json");
    w.end_object();

    w.end_object();

    if (!w.write_to(path_join(data_dir, kVehicleJsonPath)))
        return false;
    vehicle_file_out = kVehicleJsonPath;
    return true;
}

}  // namespace vettura
