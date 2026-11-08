/*!
 * @file vettura_vehicle_spec.h
 *
 * vettura_chrono_spec → Chrono::Vehicle JSON 生成器（纯标准 C++，不依赖 Chrono）。
 *
 * 见 `vettura_vehicle_spec.cpp` 的头部注释：Chrono 的 wheeled vehicle 是 JSON
 * 驱动的，本翻译器把 backend-neutral spec 落到那套 schema 上。
 */

#ifndef VETTURA_VEHICLE_SPEC_H
#define VETTURA_VEHICLE_SPEC_H

#include "chrono_bridge.h"

#include <string>

namespace vettura {

/*!
 * 在 `data_dir` 下生成完整的 Chrono 车辆 JSON（顶层 + 各子系统）。
 *
 * 成功时返回 true，并把 `vehicle_file_out` 设为**相对 vehicle data path** 的顶层
 * 车辆 JSON 路径（即 `vehicle/vettura_vehicle.json`）——bridge 直接把它交给
 * `chrono::vehicle::WheeledVehicle`。
 *
 * 返回 false 表示 spec 无法忠实表达（例如 AWD：还没有 ShaftsDriveline4WD 的
 * schema），此时 `data_dir` 可能已被创建但没有完整内容。
 */
bool write_vehicle_spec(const vettura_chrono_spec& spec, const std::string& data_dir,
                        std::string& vehicle_file_out);

}  // namespace vettura

#endif /* VETTURA_VEHICLE_SPEC_H */
