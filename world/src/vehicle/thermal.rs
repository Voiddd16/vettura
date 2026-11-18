//! Tyre and brake temperature models.
//!
//! Pure functions: spec, current temperatures, the energy injected this step and
//! the current speed in, the next temperatures out. The vehicle side computes
//! the heat and hands it over; the coolant/oil pacing constants live here too.

use vettura_physics::{BrakeSpec, TyreThermalSpec};

/// 冷却液向节温器设定点松弛的速率，1/s（时间常数约 4 s）。真车的节温器就
/// 是这样：任何负荷下都把工作温度守住，而不是让平衡点随散热量漂移。
pub(crate) const THERMOSTAT_RATE_PER_S: f32 = 0.25;

/// 收油门时冷却液向环境温度冷却的速率，1/s（比暖机慢：车体还有余温）。
pub(crate) const ENGINE_COOLING_RATE_PER_S: f32 = 0.02;

/// 三层轮胎温度（胎面接触区 / 胎体 / 核心）的内部状态。
///
/// 转子级别的刹车模型不需要分层，所以窗口不考虑刹车；无分层数据的车保持
/// 老行为——三个温度都是同一个胎体。
pub(crate) struct TyreTemperatures {
    /// Friction-surface temperature, °C: where the heat is generated.
    pub(crate) surface: f32,
    /// Under-tread temperature, °C: what the grip curve reads.
    pub(crate) patch: f32,
    /// Carcass temperature, °C: the bulk that lags everything.
    pub(crate) core: f32,
}

/// Advance the three tyre temperatures by `heat_j` over `step` seconds at
/// `speed_mps` towards `ambient_c`.
pub(crate) fn advance_tyre_temperatures(
    spec: &TyreThermalSpec,
    state: TyreTemperatures,
    heat_j: f32,
    speed_mps: f32,
    ambient_c: f32,
    step: f32,
) -> TyreTemperatures {
    let cooling = ((spec.cooling_per_s + (speed_mps.abs() / 60.0).clamp(0.0, 3.0) * 0.06) * step)
        .clamp(0.0, 0.9);
    if !spec.has_layers() {
        // Single mass: everything happens in the carcass, as before.
        let next = state.core + (heat_j / spec.heat_capacity_j_per_k.max(1.0)) * (1.0 - cooling);
        let target = ambient_c;
        let core = (next + target * cooling).clamp(target, 1_400.0);
        return TyreTemperatures {
            surface: core,
            patch: core,
            core,
        };
    }

    let surface_capacity = spec.surface_capacity_j_per_k.max(1.0);
    let patch_capacity = spec.patch_capacity_j_per_k.max(1.0);
    let core_capacity = spec.heat_capacity_j_per_k.max(1.0);
    let ambient = ambient_c;
    let mut surface = (state.surface + heat_j / surface_capacity).clamp(ambient, 1_400.0);
    let mut patch = state.patch;
    let mut core = state.core;

    // Surface to air: the fastest channel, and it grows with the air flowing
    // over the tread.
    surface -= (surface - ambient) * cooling;

    // Down the chain. Each transfer is a fraction of the temperature
    // difference per second, so it is stable at any step size.
    let to_patch = spec.surface_transfer_per_s.max(0.0) * step;
    if to_patch > 0.0 {
        let flow = (surface - patch).clamp(-1.0e3, 1.0e3);
        surface -= flow * to_patch;
        patch += flow * to_patch * (surface_capacity / patch_capacity);
    }
    let to_core = spec.patch_transfer_per_s.max(0.0) * step;
    if to_core > 0.0 {
        let flow = (patch - core).clamp(-1.0e3, 1.0e3);
        patch -= flow * to_core;
        core += flow * to_core * (patch_capacity / core_capacity);
    }

    TyreTemperatures {
        surface: surface.clamp(ambient, 1_400.0),
        patch: patch.clamp(ambient, 1_400.0),
        core: core.clamp(ambient, 1_400.0),
    }
}

/// One step of the brake thermal model, °C.
///
/// Returns `(surface, core)` temperatures. The surface is the friction face the
/// pads see, which is what the efficiency curve is indexed by; the core is the
/// bulk of the rotor, which soaks heat in slowly and hands it back quickly, so
/// the pads keep cooking after the pedal is released. `ambient_c` is the air
/// the disc exchanges with and may be below zero.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_brake_temperatures(
    spec: &BrakeSpec,
    surface_in: f32,
    core_in: f32,
    energy_j: f32,
    speed_mps: f32,
    ambient_c: f32,
    step: f32,
) -> (f32, f32) {
    let rise = if spec.thermal_mass_j_per_k > 1.0 {
        (energy_j * spec.heat_per_joule / spec.thermal_mass_j_per_k).max(0.0)
    } else {
        0.0
    };
    // Cooling: ambient exchange plus the air flowing over the disc.
    let speed_factor = (speed_mps / 60.0).clamp(0.05, 4.0);
    let cooling = (spec.cooling_per_s * speed_factor * step).clamp(0.0, 0.95);
    let difference = surface_in - ambient_c;
    let drop = difference * cooling;
    let mut surface = surface_in + rise - drop;

    let mut core = core_in;
    let surface_capacity = spec.thermal_mass_j_per_k.max(1.0);
    let core_capacity = spec.core_capacity_j_per_k.max(1.0);
    if core_capacity > 1.0 {
        // Heat always flows down the gradient, but AC models the two directions
        // with different rates, so the conductance depends on which side is
        // hotter: soaks in slowly, comes back out quickly.
        let conductance = if surface > core {
            surface_capacity * spec.core_transfer_in_per_s.max(0.0)
        } else {
            core_capacity * spec.core_transfer_out_per_s.max(0.0)
        };
        let flow = conductance * (surface - core).clamp(-1.0e4, 1.0e4);
        surface -= flow * step / surface_capacity;
        core += flow * step / core_capacity;
        let air_flow = core_capacity * spec.core_transfer_air_per_s.max(0.0) * (core - ambient_c);
        core -= air_flow * step / core_capacity;
    }

    (
        surface.clamp(ambient_c, 1_400.0),
        core.clamp(ambient_c, 1_400.0),
    )
}
