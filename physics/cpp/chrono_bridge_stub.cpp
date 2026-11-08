/*!
 * @file chrono_bridge_stub.cpp
 *
 * Fallback implementation of the C ABI from `chrono_bridge.h`, used when the
 * Chrono C++ sources are not available (`third_party/chrono` is empty and no
 * `CHRONO_ROOT` is set). It lets `cargo build -p vettura-physics --features
 * chrono` link and run the workspace tests while reporting
 * `VETTURA_CHRONO_ERR_NOT_LINKED`, so callers fall back to `SimpleBackend`.
 *
 * The real implementation is `bridge.cpp`, which `build.rs` compiles instead
 * whenever Chrono is found. Both files implement the exact same ABI, so only
 * one of them is ever linked.
 */

#include "chrono_bridge.h"

#include <cstring>
#include <string>

namespace {

thread_local std::string g_last_error;

void set_last_error(const char* message) { g_last_error = message ? message : ""; }

}  // namespace

extern "C" int32_t vettura_chrono_abi_version(void) {
    return VETTURA_CHRONO_ABI_VERSION;
}

extern "C" void vettura_chrono_build_id(char* out, int32_t out_len) {
    static const char kId[] = "stub (Chrono not linked; see third_party/chrono/README.md)";
    if (out == nullptr || out_len <= 0)
        return;
    std::strncpy(out, kId, static_cast<size_t>(out_len - 1));
    out[out_len - 1] = '\0';
}

extern "C" int32_t vettura_chrono_create(const vettura_chrono_spec* spec,
                                         vettura_chrono_vehicle** out) {
    (void)spec;
    if (out != nullptr)
        *out = nullptr;
    set_last_error("Chrono is not linked: put the sources in third_party/chrono or set CHRONO_ROOT");
    return VETTURA_CHRONO_ERR_NOT_LINKED;
}

extern "C" void vettura_chrono_destroy(vettura_chrono_vehicle* vehicle) { (void)vehicle; }

extern "C" int32_t vettura_chrono_reset(vettura_chrono_vehicle* vehicle, const vettura_chrono_pose* pose) {
    (void)vehicle;
    (void)pose;
    set_last_error("Chrono is not linked");
    return VETTURA_CHRONO_ERR_NOT_LINKED;
}

extern "C" int32_t vettura_chrono_step(vettura_chrono_vehicle* vehicle,
                                       double dt,
                                       const vettura_chrono_controls* controls) {
    (void)vehicle;
    (void)dt;
    (void)controls;
    set_last_error("Chrono is not linked");
    return VETTURA_CHRONO_ERR_NOT_LINKED;
}

extern "C" int32_t vettura_chrono_read_state(const vettura_chrono_vehicle* vehicle,
                                             vettura_chrono_state* out_state) {
    (void)vehicle;
    if (out_state == nullptr)
        return VETTURA_CHRONO_ERR_INVALID_ARGUMENT;
    set_last_error("Chrono is not linked");
    return VETTURA_CHRONO_ERR_NOT_LINKED;
}

extern "C" const char* vettura_chrono_last_error(void) {
    return g_last_error.empty() ? nullptr : g_last_error.c_str();
}
