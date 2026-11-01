/*!
 * @file chrono_bridge.h
 *
 * C ABI contract between `vettura-physics` (Rust) and the Chrono::Vehicle
 * adapter (C++).
 *
 * ## Why a plain C ABI instead of cxx
 *
 * Everything we need to cross the boundary is POD (spec / pose / controls /
 * state) plus one opaque handle, so cxx's code generation buys nothing.
 * A C ABI is the one contract every compiler and platform agrees on, and it
 * keeps Chrono (which needs a C++ toolchain and CMake) buildable by any
 * compiler the user happens to have. This header is the single reviewable
 * contract; both sides compile it (`extern "C"` guard below).
 *
 * ## Cross-compiler determinism rules
 *
 * These are not style preferences — violating any of them makes results
 * differ between clang / gcc / MSVC. See `docs/build.md` §8 for the exact
 * compiler flags that enforce them.
 *
 * 1.  **POD only, fixed-width.** `double` and `int32_t` exclusively. Never
 *     `float` (Rust upcasts its `f32` at the boundary), never `long double`
 *     (x87 vs SSE vs MSVC all differ), never `bool` / `size_t` / `enum`
 *     (their ABI size is compiler/ABI dependent).
 * 2.  **No exceptions escape.** Chrono throws; every entry point wraps its
 *     body in `try { ... } catch (...) { return status; }`.
 * 3.  **No global mutable state, no threads.** Chrono is built with OpenMP
 *     disabled so thread count cannot change the result.
 * 4.  **Symmetric ownership.** Whatever `vettura_chrono_create` allocates is
 *     released by `vettura_chrono_destroy`; the Rust side never frees.
 * 5.  **Strict IEEE, no contraction.** No fast-math, no FMA contraction, no
 *     `-march=native` **in our bridge code**. Only then do the lanes of a
 *     vectorized expression stay bit-identical across compilers. Chrono itself
 *     is built by its own CMake and `FindSIMD` appends `-march=native`, so the
 *     bit-exact guarantee is scoped to "same machine, different compilers";
 *     cross-machine replay additionally needs a baseline ISA (see
 *     `docs/build.md` §8.2).
 *
 * ## Units and frames
 *
 * SI throughout: metres, kilograms, seconds, newtons, radians, rpm.
 * Physics frame: `+X` right, `+Y` up, `-Z` forward (same as
 * `vettura_physics::VehicleSpec`). Quaternions are `(x, y, z, w)`, normalized.
 */

#ifndef VETTURA_CHRONO_BRIDGE_H
#define VETTURA_CHRONO_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Bumped on every incompatible change to the structs/functions below; the
 * Rust side asserts it matches at link time (golden replay test). */
#define VETTURA_CHRONO_ABI_VERSION 1

/* Number of wheels mirrored from `vettura_physics::Wheel::ALL`. */
#define VETTURA_CHRONO_WHEEL_COUNT 4

/* Upper bound for `gear_ratios`; keeps the spec struct a fixed-size POD so
 * its layout never shifts. Mirrors `EngineSpec::gear_ratios`. */
#define VETTURA_CHRONO_MAX_GEAR_COUNT 16

/* Mirrors `vettura_physics::DrivetrainLayout` discriminants. */
#define VETTURA_CHRONO_LAYOUT_RWD 0
#define VETTURA_CHRONO_LAYOUT_FWD 1
#define VETTURA_CHRONO_LAYOUT_AWD 2

/* Status codes. `success` is 0; everything else is an error. */
#define VETTURA_CHRONO_OK 0
#define VETTURA_CHRONO_ERR_INVALID_ARGUMENT 1
#define VETTURA_CHRONO_ERR_INVALID_SPEC 2
#define VETTURA_CHRONO_ERR_NOT_LINKED 3
#define VETTURA_CHRONO_ERR_SOLVER 4

/* Opaque handle to a `chrono::vehicle::ChVehicle` (plus terrain). Defined in
 * `bridge.cpp` only; Rust never dereferences it. */
typedef struct vettura_chrono_vehicle vettura_chrono_vehicle;

typedef struct vettura_chrono_vec3 {
    double x;
    double y;
    double z;
} vettura_chrono_vec3;

typedef struct vettura_chrono_quat {
    double x;
    double y;
    double z;
    double w;
} vettura_chrono_quat;

/* Mirrors `vettura_physics::SuspensionSpec`. */
typedef struct vettura_chrono_suspension {
    double spring_rate_n_per_m;
    double damper_bump_ns_per_m;
    double damper_rebound_ns_per_m;
    double travel_m;
    double anti_roll_n_per_m;
} vettura_chrono_suspension;

/* Mirrors `vettura_physics::WheelSpec`. `position` is the hard point in the
 * physics frame, matching `WheelSpec::position`. */
typedef struct vettura_chrono_wheel {
    vettura_chrono_vec3 position;
    double radius_m;
    double width_m;
    int32_t steerable;
    int32_t driven;
    vettura_chrono_suspension suspension;
} vettura_chrono_wheel;

/* Mirrors `vettura_physics::EngineSpec`. */
typedef struct vettura_chrono_engine {
    double idle_rpm;
    double redline_rpm;
    double peak_torque_nm;
    int32_t gear_count;
    double gear_ratios[VETTURA_CHRONO_MAX_GEAR_COUNT]; /* descending */
    double final_drive;
} vettura_chrono_engine;

/* Mirrors `vettura_physics::DrivetrainSpec`; `layout` uses the LAYOUT_* codes. */
typedef struct vettura_chrono_drivetrain {
    int32_t layout;
    double efficiency;
} vettura_chrono_drivetrain;

/* Mirrors `vettura_physics::TyreSpec`. Chrono-specific tyre parameters
 * (stiffness, relaxation length) go into a separate extension struct later so
 * this file stays backend-neutral. */
typedef struct vettura_chrono_tyre {
    double peak_friction;
} vettura_chrono_tyre;

/* Mirrors `vettura_physics::VehicleSpec`.
 * `VehicleSpec::name` is metadata and deliberately absent. */
typedef struct vettura_chrono_spec {
    double mass_kg;
    double wheelbase_m;
    double cg_height_m;
    double frontal_area_m2;
    double drag_coefficient;
    vettura_chrono_wheel wheels[VETTURA_CHRONO_WHEEL_COUNT];
    vettura_chrono_engine engine;
    vettura_chrono_drivetrain drivetrain;
    vettura_chrono_tyre tyres;
} vettura_chrono_spec;

/* Mirrors `vettura_physics::Pose`. */
typedef struct vettura_chrono_pose {
    vettura_chrono_vec3 position;
    vettura_chrono_quat orientation;
} vettura_chrono_pose;

/* Mirrors `vettura_physics::Controls`. Axes are normalized exactly like the
 * Rust struct; `clamped()` is applied on the Rust side, the bridge assumes
 * in-range input but still rejects NaN. */
typedef struct vettura_chrono_controls {
    double throttle;
    double brake;
    double clutch;
    double steering;
    int32_t handbrake;
    int32_t shift_up;
    int32_t shift_down;
} vettura_chrono_controls;

/* Mirrors `vettura_physics::WheelState`. */
typedef struct vettura_chrono_wheel_state {
    double rotation;
    double steer_angle;
    double angular_velocity;
    double slip_ratio;
    double slip_angle;
    double load_n;
    int32_t contact;
} vettura_chrono_wheel_state;

/* Mirrors `vettura_physics::VehicleState`. */
typedef struct vettura_chrono_state {
    vettura_chrono_vec3 position;
    vettura_chrono_quat orientation;
    vettura_chrono_vec3 linear_velocity;
    vettura_chrono_vec3 angular_velocity;
    double speed_mps;
    vettura_chrono_wheel_state wheels[VETTURA_CHRONO_WHEEL_COUNT];
    double engine_rpm;
    int32_t gear;
} vettura_chrono_state;

/* ABI revision implemented by the shared library. */
int32_t vettura_chrono_abi_version(void);

/* Human-readable build identity (compiler, flags, Chrono commit). Written into
 * `out` (always NUL-terminated when `out_len > 0`); intended for the golden
 * replay test so a mismatch is diagnosable. */
void vettura_chrono_build_id(char* out, int32_t out_len);

/* Build a vehicle from a backend-neutral spec. On success `*out` owns the
 * handle and must be released with `vettura_chrono_destroy`.
 *
 * `spec.mass_kg` is the **whole vehicle** mass. Chrono models wheels, tyres and
 * uprights as separate bodies, so the chassis body only gets the sprung mass
 * that remains after subtracting the wheel-end budget (see
 * `vettura_vehicle_spec.cpp`, `mass_budget`). */
int32_t vettura_chrono_create(
    const vettura_chrono_spec* spec,
    vettura_chrono_vehicle** out);

/* Release a handle created above. NULL is a no-op. */
void vettura_chrono_destroy(vettura_chrono_vehicle* vehicle);

/* Teleport the vehicle to `pose` and zero every body velocity.
 *
 * `pose.position` is the chassis **reference frame** position (the frame the
 * vehicle is built around, not the centre of mass). Only the heading (yaw
 * about the world up axis) is kept; pitch/roll are discarded so the vehicle
 * always comes back upright on the ground plane.
 *
 * The reference frame is lifted to at least "tyre radius + clearance" above
 * the ground. Chrono treats a wheel centre below the terrain as "no contact",
 * so spawning lower than that would make the car fall through instead of
 * settling. */
int32_t vettura_chrono_reset(
    vettura_chrono_vehicle* vehicle,
    const vettura_chrono_pose* pose);

/* Advance exactly `dt` seconds (no variable step, no substepping here — that
 * belongs to `vettura_physics::Simulation`). */
int32_t vettura_chrono_step(
    vettura_chrono_vehicle* vehicle,
    double dt,
    const vettura_chrono_controls* controls);

/* Copy the current state into `out_state`.
 *
 * `position` is the chassis **centre of mass** (Chrono's body frame), which
 * sits `cg_height_m` above the reference frame used by `vettura_chrono_reset`.
 * `orientation` is the full chassis attitude.
 *
 * `wheels[i].rotation` is an angle accumulated from the spindle speed (wrapped
 * to `[0, 2π)`), meant for visual wheel spin. `engine_rpm` is the physical
 * engine speed clamped to the spec idle speed: Chrono's automatic transmission
 * reports 0 rpm when the car is stationary even though a real engine would be
 * idling. */
int32_t vettura_chrono_read_state(
    const vettura_chrono_vehicle* vehicle,
    vettura_chrono_state* out_state);

/* Message for the most recent error on the calling thread. Valid until the
 * next bridge call; never NULL after an error, may be NULL otherwise. */
const char* vettura_chrono_last_error(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* VETTURA_CHRONO_BRIDGE_H */
