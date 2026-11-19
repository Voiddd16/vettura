//! Chrono::Vehicle backend.
//!
//! 该模块取代了原先独立的 `vettura-physics-chrono` crate：拆分得过细，而 Chrono
//! 的 C++ 源码统一放在仓库根的 `third_party/chrono`，由本 crate 的 `chrono`
//! feature 门控链接。
//!
//! # Glue 层：C ABI，不是 cxx
//!
//! 边界是 `cpp/chrono_bridge.h` 声明的 `extern "C"` 接口，Rust 侧手写
//! `#[repr(C)]` 镜像（[`ffi`]，带布局测试）。选 C ABI 的理由：只要能跨界的全是
//! Pod 加一个不透明句柄，cxx 的代码生成买不到安全；而 C ABI 是唯一所有编译器
//! 都共识的契约，Chrono 可以用任意 C++ 工具链编出来。
//!
//! # C++ 侧职责（`cpp/`）
//!
//! - [`bridge.cpp`](../../../cpp/bridge.cpp)：`WheeledVehicle` + `FlatTerrain`
//!   + 固定步长步进 + 状态回填；
//! - [`vettura_vehicle_spec.cpp`](../../../cpp/vettura_vehicle_spec.cpp)：
//!   spec → Chrono JSON（Chrono 的 wheeled vehicle 是 JSON 驱动的），纯标准
//!   C++，可脱离 Chrono 单测；
//! - [`chrono_bridge_stub.cpp`](../../../cpp/chrono_bridge_stub.cpp)：Chrono
//!   没就位时的替代实现，返回 `NotLinked`，让 workspace 依然能编译/测试。
//!
//! # 只接 vehicle 部分
//!
//! 我们只做车辆动力学：平地地形 (`FlatTerrain`)、底盘/悬架/轮胎/传动/转向、
//! 驻车制动。SCM/Granular/FEA 地形、传感器、可视化都不接。
//!
//! # 线程契约
//!
//! Chrono 以 `ENABLE_OPENMP=OFF` 构建，bridge 侧无全局状态；`ChronoBackend`
//! 因此是 `Send`，但**不是** `Sync`：多线程同时 step 同一个实例没有意义也没有
//! 实现。`Simulation` 单持有者，正好匹配。

pub mod ffi;

use thiserror::Error;

use crate::{Controls, Pose, VehicleBackend, VehicleSpec, VehicleState};

pub use ffi::{
    VetturaChronoControls, VetturaChronoVehicle, to_ffi_controls, to_ffi_pose, to_ffi_spec,
};

#[derive(Debug, Error)]
pub enum ChronoError {
    #[error("Chrono::Vehicle is not linked in this build; see third_party/README.md")]
    NotLinked,

    #[error("invalid vehicle specification: {0}")]
    InvalidSpec(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("solver error inside Chrono::Vehicle: {0}")]
    Solver(String),
}

/// Chrono::Vehicle backend, driven through the C ABI in `cpp/chrono_bridge.h`.
///
/// `try_new` 返回 [`ChronoError::NotLinked`] 时（Chrono 源码没就位，或链接到
/// 的是 stub），调用方应回退到 [`crate::backends::SimpleBackend`]。
#[derive(Debug)]
pub struct ChronoBackend {
    /// Opaque handle owned by the bridge; null until `create` succeeds.
    handle: Option<*mut VetturaChronoVehicle>,
    spec: VehicleSpec,
    state: VehicleState,
    last_error_message: String,
}

impl ChronoBackend {
    /// Build a Chrono vehicle from a backend-neutral spec.
    ///
    /// 失败时保留 `NotLinked` / `InvalidSpec` 的区分，调用方（或日志）可以据此
    /// 决定回退策略；`vettura_chrono_last_error()` 的原文通过
    /// [`ChronoBackend::last_error`] 暴露。
    pub fn try_new(spec: VehicleSpec) -> Result<Self, ChronoError> {
        // 创建前先校验 ABI 版本：跨版本链接一律拒绝，比崩溃好。
        // SAFETY: 纯读取，不依赖指针参数。
        let abi = unsafe { ffi::vettura_chrono_abi_version() };
        if abi != ffi::VETTURA_CHRONO_ABI_VERSION {
            return Err(ChronoError::InvalidArgument(format!(
                "bridge ABI version {abi} != expected {}",
                ffi::VETTURA_CHRONO_ABI_VERSION
            )));
        }

        let mut handle: *mut VetturaChronoVehicle = std::ptr::null_mut();
        let ffi_spec = ffi::to_ffi_spec(&spec);
        // SAFETY: spec 是栈上 Pod，生命周期覆盖本次调用；handle 由 bridge 分配。
        let status = unsafe { ffi::vettura_chrono_create(&ffi_spec, &mut handle as *mut *mut _) };
        let message = ffi::last_error();
        match status {
            ffi::VETTURA_CHRONO_OK => Ok(Self {
                handle: Some(handle),
                spec,
                state: VehicleState::default(),
                last_error_message: message,
            }),
            ffi::VETTURA_CHRONO_ERR_NOT_LINKED => Err(ChronoError::NotLinked),
            ffi::VETTURA_CHRONO_ERR_INVALID_SPEC => Err(ChronoError::InvalidSpec(message)),
            ffi::VETTURA_CHRONO_ERR_INVALID_ARGUMENT => Err(ChronoError::InvalidArgument(message)),
            other => Err(ChronoError::Solver(format!("status {other}: {message}"))),
        }
    }

    /// Bridge 侧最后一次错误的原文（用于诊断；`create` 失败时也有效）。
    pub fn last_error(&self) -> &str {
        &self.last_error_message
    }

    /// Whether the bridge linked real Chrono (`false` means the stub is
    /// linked and every call returns `NotLinked`).
    pub fn is_linked(&self) -> bool {
        self.handle.is_some()
    }

    /// ABI/编译器/flags 标识（golden replay 测试用它定位 divergence）。
    pub fn bridge_build_id() -> String {
        ffi::build_id()
    }
}

impl VehicleBackend for ChronoBackend {
    fn spec(&self) -> &VehicleSpec {
        &self.spec
    }

    fn state(&self) -> &VehicleState {
        &self.state
    }

    fn reset(&mut self, pose: Pose) {
        let Some(handle) = self.handle else {
            // 没链接时保持 SimpleBackend 相同语义：原地归零。
            self.state = VehicleState::default();
            self.state.position = pose.position;
            self.state.orientation = pose.orientation;
            return;
        };
        let ffi_pose = ffi::to_ffi_pose(&pose);
        // SAFETY: handle 由本实例持有；pose 生命周期覆盖调用。
        let status = unsafe { ffi::vettura_chrono_reset(handle, &ffi_pose) };
        debug_assert_eq!(
            status,
            ffi::VETTURA_CHRONO_OK,
            "chrono reset failed: {}",
            ffi::last_error()
        );
    }

    fn step(&mut self, dt: f32, controls: &Controls) {
        let Some(handle) = self.handle else {
            self.state = VehicleState::default();
            return;
        };
        let mut ffi_state = unsafe { std::mem::zeroed::<ffi::VetturaChronoState>() };
        // SAFETY: handle 由本实例持有；controls 在调用期间有效；dt 已由
        // Simulation 钳到合理量级。
        let ffi_controls = ffi::to_ffi_controls(controls);
        let status = unsafe {
            ffi::vettura_chrono_step(handle, dt as f64, &ffi_controls);
            // bridge 的 read_state 需要非 const 句柄（ReportTireForce 不是 const）。
            ffi::vettura_chrono_read_state(handle, &mut ffi_state)
        };
        debug_assert_eq!(
            status,
            ffi::VETTURA_CHRONO_OK,
            "chrono read_state failed: {}",
            ffi::last_error()
        );
        if status == ffi::VETTURA_CHRONO_OK {
            self.state = ffi::from_ffi_state(&ffi_state);
        }
    }
}

impl Drop for ChronoBackend {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // SAFETY: 句柄只归还给 bridge 一次；destroy(NULL) 是空操作。
            unsafe { ffi::vettura_chrono_destroy(handle) };
        }
    }
}

// SAFETY: bridge 无全局状态、Chrono 以单线程构建；句柄随本实例一起被 drop，
// 因此可以跨线程移动。多线程并发 step 同一实例是被禁止的（无 Sync）。
unsafe impl Send for ChronoBackend {}
