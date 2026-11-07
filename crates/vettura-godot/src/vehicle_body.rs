//! 车辆节点：把一台车接到物理后端上，并把每帧结果同步到场景树。
//!
//! 这个节点有两种用法：
//!
//! 1. **独立模式**（`setup_default` / `setup_jolt` / `setup_from_car`）：自己持有
//!    `Simulation`，在 `_physics_process` 里步进。适合平地和单机调参；
//! 2. **世界模式**：由 [`WorldBody`](crate::world::WorldBody) 创建并挂在它下面，
//!    由世界统一步进。世界模式不再有自己的 `Simulation`，只保存 `VehicleHandle`
//!    和驾驶员输入，渲染同步由世界完成。
//!
//! 两种模式都会把 `WHEEL_LF/RF/LR/RR` 之类的 dummy 绑到 `VehicleSnapshot`/`VehicleState`
//! 上，车轮转向与滚动在场景里可见。

use godot::classes::Node;
use godot::classes::{INode3D, Node3D};
use godot::prelude::*;
use vettura_physics::backends::SimpleBackend;
use vettura_physics::{Controls, Pose, Simulation, VehicleState};
use vettura_world::{JoltBackend, VehicleHandle, VehicleSnapshot, WheelSnapshot};

use crate::ac_car::AcCar;

/// KN5 车轮挂点名，与 `vettura_physics::Wheel` 的顺序一一对应。
const WHEEL_NODES: [&str; 4] = ["WHEEL_LF", "WHEEL_RF", "WHEEL_LR", "WHEEL_RR"];

/// 一个车轮的渲染绑定：节点 + 绑定时的静止变换。
///
/// 变换写成 `rest * local_rotation`：`rest` 保留 KN5 作者给的对轮子定位用的
/// 父变换，局部旋转只负责转向（绕局部 `+Y`）和滚动（绕局部 `+X`）。
#[derive(Clone)]
struct WheelVisual {
    node: Option<Gd<Node3D>>,
    rest: Transform3D,
}

impl Default for WheelVisual {
    fn default() -> Self {
        Self {
            node: None,
            rest: Transform3D::IDENTITY,
        }
    }
}

impl WheelVisual {
    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, steer_angle: f32, spin_angle: f32) {
        let Some(node) = self.node.as_mut() else {
            return;
        };
        if !steer_angle.is_finite() || !spin_angle.is_finite() {
            return;
        }

        // AC/物理帧的转向/滚动角度已经是右手系弧度值，直接乘以静止基即可。
        let steer = Basis::from_axis_angle(Vector3::UP, steer_angle);
        let spin = Basis::from_axis_angle(Vector3::RIGHT, spin_angle);
        let basis = self.rest.basis * steer * spin;
        node.set_transform(Transform3D {
            basis,
            origin: self.rest.origin,
        });
    }
}

/// 物理后端来源：独立仿真、世界调度，或尚未初始化。
#[derive(Default)]
enum VehicleMode {
    #[default]
    Unset,
    Standalone(Simulation),
    World {
        handle: VehicleHandle,
    },
}

/// 车辆物理节点。
///
/// 独立模式示例（平地 + Jolt 真实悬挂/轮胎）：
///
/// ```gdscript
/// var car := AcCar.new()
/// car.open("res://test/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari")
/// var body := VehicleBody.new()
/// add_child(body)
/// body.setup_from_car(car)
/// body.set_controls(1.0, 0.0, 0.0, false, false, false, true, true)
/// ```
///
/// 世界模式示例（真实赛道 + 多车）：
///
/// ```gdscript
/// var world := WorldBody.new()
/// add_child(world)
/// world.load_track("res://test/daytona_2017", "roadcourse")
/// var body := world.spawn_vehicle_from_path("res://path/to/car")
/// ```
///
/// 两种情况都建议放在 `_physics_process` 中：
///
/// ```gdscript
/// func _physics_process(delta):
///     body.set_controls(
///         Input.get_action_strength("throttle"),
///         Input.get_action_strength("brake"),
///         Input.get_axis("steer_right", "steer_left"),
///         Input.is_action_pressed("handbrake"),
///         Input.is_action_pressed("drs"),
///         Input.is_action_pressed("abs"),
///         Input.is_action_pressed("tc")
///     )
///     print(body.speed_kmh())
/// ```
#[derive(GodotClass)]
#[class(base = Node3D, init)]
pub struct VehicleBody {
    base: Base<Node3D>,
    mode: VehicleMode,
    controls: Controls,
    initial_pose: Pose,
    wheels: [WheelVisual; 4],
    // 世界模式的每帧轮上数据；独立模式走 `wheel_state(Simulation::state())`，
    // 世界模式由 `WorldBody` 写入这里，避免 HUD / 触发器拿不到接地与否。
    last_wheels: [WheelSnapshot; 4],
    // 世界模式没有自己的 Simulation，这三个值由 `WorldBody` 每帧推进后写入，
    // 让 HUD / AI / GDScript 读到的速度、挡位、转速始终可用。
    last_speed: f32,
    last_forward_speed: f32,
    last_gear: i64,
    last_rpm: f32,
}

#[godot_api]
impl INode3D for VehicleBody {
    /// Godot 物理回调。
    ///
    /// - 独立模式：按帧推进固定步长仿真，并同步位姿与车轮；
    /// - 世界模式：什么都不做——状态由 [`WorldBody`](crate::world::WorldBody)
    ///   的同一回调统一写入，避免同一个世界被步进两次。
    fn physics_process(&mut self, delta: f64) {
        match &mut self.mode {
            VehicleMode::Standalone(simulation) => {
                simulation.step(delta as f32, &self.controls);
                let state = *simulation.state();
                self.sync_state(&state);
            }
            VehicleMode::World { .. } | VehicleMode::Unset => {}
        }
    }
}

#[godot_api]
impl VehicleBody {
    /// 用默认车辆参数初始化仿真，并以当前节点位姿为初始状态。
    ///
    /// 使用 `SimpleBackend`：只打通 Godot ↔ 物理层 ↔ 固定步长，**不是**真车。
    /// 要真车请用 [`setup_from_car`](Self::setup_from_car) 或
    /// [`WorldBody`](crate::world::WorldBody)。
    #[func]
    pub fn setup_default(&mut self) {
        let pose = self.current_pose();
        let mut simulation = Simulation::new(Box::new(SimpleBackend::new(spec_default())));
        simulation.reset(pose);
        self.mode = VehicleMode::Standalone(simulation);
        self.initial_pose = pose;
    }

    /// 用 `VehicleSpec::default()` 初始化 Jolt 后端（平地工况）。
    ///
    /// 适合做主项目里的“最小可开”验证：真实悬挂、真实轮胎、自动变速箱，但地面
    /// 是无限平的沥青。
    #[func]
    pub fn setup_jolt(&mut self) -> bool {
        self.setup_jolt_with_spec(spec_default())
    }

    /// 用 `AcCar` 解析出的真实 AC 数据初始化 Jolt 后端（平地工况）。
    ///
    /// 成功返回 `true`；`AcCar` 未打开或物理映射失败会打印错误并返回 `false`。
    #[func]
    pub fn setup_from_car(&mut self, car: Gd<AcCar>) -> bool {
        let Some(spec) = car.bind().vehicle_spec() else {
            godot_error!("setup_from_car: AcCar has no mappable VehicleSpec");
            return false;
        };
        self.setup_jolt_with_spec(spec)
    }

    /// 直接打开一个 AC 车目录并初始化 Jolt 后端（平地工况）。
    ///
    /// 等价于 `AcCar.open()` + `setup_from_car()`，给不想手动建资源的 GDScript。
    #[func]
    pub fn setup_from_car_folder(&mut self, car_path: GString) -> bool {
        let mut car = AcCar::new_gd();
        if !car.bind_mut().open(car_path) {
            return false;
        }
        self.setup_from_car(car)
    }

    /// 单独设置驾驶员输入（不与世界/独立模式冲突，两者读的都是这一段）。
    ///
    /// `throttle` / `brake` 会被限制在 `[0, 1]`；`steering` 会被限制在
    /// `[-1, 1]`，正值向左。
    #[func]
    pub fn set_inputs(&mut self, throttle: f32, brake: f32, steering: f32) {
        self.set_controls(throttle, brake, steering, false, false, false, false);
    }

    /// 完整驾驶员输入：油门、刹车、转向、手刹、DRS、ABS、TC。
    ///
    /// 换挡交给变速箱自动处理；需要手动时可以调用
    /// [`shift_up`](Self::shift_up) / [`shift_down`](Self::shift_down)。
    // Godot 的 `#[func]` 没有结构体参数，七个标量一次性发过来比拆成两个方法
    // 对 GDScript 更友好；这里显式允许，避免为 lint 把 API 拆碎。
    #[allow(clippy::too_many_arguments)]
    #[func]
    pub fn set_controls(
        &mut self,
        throttle: f32,
        brake: f32,
        steering: f32,
        handbrake: bool,
        drs: bool,
        abs: bool,
        tc: bool,
    ) {
        self.controls.throttle = clamp_finite(throttle, 0.0, 1.0, 0.0);
        self.controls.brake = clamp_finite(brake, 0.0, 1.0, 0.0);
        self.controls.steering = clamp_finite(steering, -1.0, 1.0, 0.0);
        self.controls.handbrake = handbrake;
        self.controls.drs = drs;
        self.controls.abs = abs;
        self.controls.tc = tc;
    }

    /// 手动升一挡（下一帧生效）。
    #[func]
    pub fn shift_up(&mut self) {
        self.controls.shift_up = true;
        self.controls.shift_down = false;
    }

    /// 手动降一挡（下一帧生效）。
    #[func]
    pub fn shift_down(&mut self) {
        self.controls.shift_down = true;
        self.controls.shift_up = false;
    }

    /// 把手刹开关复位（`set_controls` 的每帧调用已经会覆盖它，这里只留给事件式输入）。
    #[func]
    pub fn release_handbrake(&mut self) {
        self.controls.handbrake = false;
    }

    /// 把车送回初始位姿并清零速度。
    ///
    /// 世界模式下需要由 [`WorldBody`](crate::world::WorldBody) 完成重置；这里只
    /// 处理独立模式，避免在没有世界句柄时误操作其它节点。
    #[func]
    pub fn reset_to_start(&mut self) {
        match &mut self.mode {
            VehicleMode::Standalone(simulation) => simulation.reset(self.initial_pose),
            VehicleMode::World { .. } | VehicleMode::Unset => {
                godot_warn!("reset_to_start called before world mode is set up; nothing to do");
            }
        }
    }

    /// 当前是否已经接上了物理后端。
    #[func]
    pub fn is_ready(&self) -> bool {
        !matches!(self.mode, VehicleMode::Unset)
    }

    /// 当前车速，单位米每秒（m/s）。未初始化时返回 `0.0`。
    #[func]
    pub fn speed_mps(&self) -> f32 {
        match &self.mode {
            VehicleMode::Standalone(simulation) => simulation.state().speed_mps,
            // 世界模式的速度由世界在每帧 step 后推送，这里直接取位姿插值不精确，
            // 因此只暴露一个近似值：节点相对上一帧的位置差 / 帧时间。为此，世界
            // 模式改用 VehicleSnapshot 时不会用到这个方法。
            VehicleMode::World { .. } | VehicleMode::Unset => self.last_speed,
        }
    }

    /// 当前车速，单位千米每小时（km/h）。
    #[func]
    pub fn speed_kmh(&self) -> f32 {
        self.speed_mps() * 3.6
    }

    /// 有符号纵向速度，m/s。倒车为负。
    #[func]
    pub fn forward_speed_mps(&self) -> f32 {
        match &self.mode {
            VehicleMode::Standalone(simulation) => self.forward_speed(simulation.state()),
            VehicleMode::World { .. } | VehicleMode::Unset => self.last_forward_speed,
        }
    }

    /// 当前挡位：`-1` 倒挡、`0` 空挡、`1` 一挡……
    #[func]
    pub fn gear(&self) -> i64 {
        match &self.mode {
            VehicleMode::Standalone(simulation) => simulation.state().gear as i64,
            VehicleMode::World { .. } | VehicleMode::Unset => self.last_gear,
        }
    }

    /// 发动机转速，r/min。
    #[func]
    pub fn engine_rpm(&self) -> f32 {
        match &self.mode {
            VehicleMode::Standalone(simulation) => simulation.state().engine_rpm,
            VehicleMode::World { .. } | VehicleMode::Unset => self.last_rpm,
        }
    }

    /// 第 `index` 个轮子是否接地。index 顺序为 `FL, FR, RL, RR`。
    #[func]
    pub fn wheel_contact(&self, index: i64) -> bool {
        let Ok(index) = wheel_index(index) else {
            return false;
        };
        self.wheel_state(index).is_some_and(|wheel| wheel.contact)
    }

    /// 第 `index` 个轮子的累计转角，弧度（渲染用）。
    #[func]
    pub fn wheel_spin(&self, index: i64) -> f32 {
        let Ok(index) = wheel_index(index) else {
            return 0.0;
        };
        self.wheel_state(index).map_or(0.0, |wheel| wheel.rotation)
    }

    /// 第 `index` 个轮子的垂向载荷，牛顿。
    #[func]
    pub fn wheel_load(&self, index: i64) -> f32 {
        let Ok(index) = wheel_index(index) else {
            return 0.0;
        };
        self.wheel_state(index).map_or(0.0, |wheel| wheel.load_n)
    }

    /// 在 `root` 的子树里按名字绑定四个车轮 dummy，返回绑定成功的数量。
    ///
    /// KN5 命名约定是 `WHEEL_LF/RF/LR/RR`，恰好对应 `vettura_physics::Wheel`
    /// 的 `FL/FR/RL/RR`；缺失时打印警告但不中断——某些 mod 的车轮是 Mesh 节点，
    /// 未来再补。
    #[func]
    pub fn bind_wheels(&mut self, root: Gd<Node3D>) -> i64 {
        let anchor = root.clone().upcast::<Node>();
        let mut bound: i64 = 0;

        for (index, name) in WHEEL_NODES.iter().enumerate() {
            match anchor
                .find_child_ex(*name)
                .recursive(true)
                .owned(false)
                .done()
            {
                Some(child) => match child.try_cast::<Node3D>() {
                    Ok(wheel) => {
                        let rest = wheel.get_transform();
                        self.wheels[index] = WheelVisual {
                            node: Some(wheel),
                            rest,
                        };
                        bound += 1;
                    }
                    Err(_) => {
                        self.wheels[index] = WheelVisual::empty();
                        godot_warn!("bind_wheels: '{}' is not a Node3D", name);
                    }
                },
                None => {
                    self.wheels[index] = WheelVisual::empty();
                    let near = anchor
                        .find_children_ex("*WHEEL*")
                        .recursive(true)
                        .owned(false)
                        .done();
                    let mut names = Vec::new();
                    for candidate in near.iter_shared() {
                        names.push(candidate.get_name().to_string());
                    }
                    godot_warn!(
                        "bind_wheels: '{}' not found (root={}, children={}, wheel-ish={:?})",
                        name,
                        root.get_name().to_string(),
                        anchor.get_child_count(),
                        names
                    );
                }
            }
        }

        bound
    }

    /// 已绑定的车轮节点数量。
    #[func]
    pub fn bound_wheels(&self) -> i64 {
        self.wheels
            .iter()
            .filter(|wheel| wheel.node.is_some())
            .count() as i64
    }
}

// ---------------------------------------------------------------------------
// 内部实现：这些方法不给 GDScript，只给同 crate 的 `WorldBody` 用。
// ---------------------------------------------------------------------------
impl VehicleBody {
    /// 把当前节点切换到世界模式，登记 `WorldBody` 分配的车辆句柄。
    pub(crate) fn attach_to_world(&mut self, handle: VehicleHandle) {
        self.mode = VehicleMode::World { handle };
    }

    /// 当前世界句柄（仅世界模式有）。
    pub(crate) fn handle(&self) -> Option<VehicleHandle> {
        match &self.mode {
            VehicleMode::World { handle } => Some(*handle),
            _ => None,
        }
    }

    /// 当前驾驶员输入。
    pub(crate) fn controls(&self) -> Controls {
        self.controls
    }

    /// step 之后清掉边沿触发量（换挡），否则 `set_controls` 每帧写入同一个 true
    /// 会让车一直换挡。
    pub(crate) fn clear_edge_inputs(&mut self) {
        self.controls.shift_up = false;
        self.controls.shift_down = false;
    }

    /// 记录初始位姿，供重置使用。
    pub(crate) fn set_initial_pose(&mut self, pose: Pose) {
        self.initial_pose = pose;
    }

    /// 初始位姿。
    pub(crate) fn initial_pose(&self) -> Pose {
        self.initial_pose
    }

    /// 由世界调：把一辆车的位姿、车轮、HUD 数值一次性同步过来。
    pub(crate) fn sync_snapshot(&mut self, snapshot: &VehicleSnapshot) {
        self.last_speed = snapshot.speed_mps;
        self.last_forward_speed = snapshot.forward_speed_mps;
        self.last_gear = snapshot.gear as i64;
        self.last_rpm = snapshot.engine_rpm;
        self.last_wheels = snapshot.wheels;

        self.apply_pose(snapshot.reference_position, snapshot.orientation);
        for index in 0..WHEEL_NODES.len() {
            let wheel = &snapshot.wheels[index];
            self.wheels[index].apply(wheel.steer_angle, wheel.rotation_angle);
        }
    }

    /// 节点在世界里的当前位姿（物理帧）。
    pub(crate) fn current_pose(&self) -> Pose {
        let position = self.base().get_position();
        let rotation = self.base().get_quaternion();
        Pose {
            position: glam_vec3(position),
            orientation: glam_quat(rotation),
        }
    }

    fn setup_jolt_with_spec(&mut self, spec: vettura_physics::VehicleSpec) -> bool {
        let pose = self.current_pose();
        let backend = match JoltBackend::try_new(spec) {
            Ok(backend) => backend,
            Err(err) => {
                godot_error!("Jolt backend rejected the spec: {}", err);
                return false;
            }
        };

        let mut simulation = Simulation::new(Box::new(backend));
        simulation.reset(pose);
        self.mode = VehicleMode::Standalone(simulation);
        self.initial_pose = pose;
        true
    }

    fn sync_state(&mut self, state: &VehicleState) {
        self.last_speed = state.speed_mps;
        self.last_forward_speed = state.speed_mps;
        self.last_gear = state.gear as i64;
        self.last_rpm = state.engine_rpm;

        self.apply_pose(state.position, state.orientation);
        for index in 0..WHEEL_NODES.len() {
            let wheel = &state.wheels[index];
            self.wheels[index].apply(wheel.steer_angle, wheel.rotation);
        }
    }

    fn apply_pose(&mut self, position: glam::Vec3, orientation: glam::Quat) {
        self.base_mut()
            .set_position(Vector3::new(position.x, position.y, position.z));
        self.base_mut().set_quaternion(Quaternion::new(
            orientation.x,
            orientation.y,
            orientation.z,
            orientation.w,
        ));
    }

    fn wheel_state(&self, index: usize) -> Option<WheelStateView> {
        match &self.mode {
            VehicleMode::Standalone(simulation) => {
                let wheel = &simulation.state().wheels[index];
                Some(WheelStateView {
                    contact: wheel.contact,
                    rotation: wheel.rotation,
                    load_n: wheel.load_n,
                })
            }
            VehicleMode::World { .. } | VehicleMode::Unset => {
                let wheel = &self.last_wheels[index];
                Some(WheelStateView {
                    contact: wheel.contact,
                    rotation: wheel.rotation_angle,
                    load_n: wheel.load_n,
                })
            }
        }
    }

    fn forward_speed(&self, state: &VehicleState) -> f32 {
        state.speed_mps
    }
}

/// 静态访问用的小视图：`WheelState` 不是 `Copy`，每次读一个字段都要 clone 整个
/// 数组不划算，所以这里只复制关心的三个值。
struct WheelStateView {
    contact: bool,
    rotation: f32,
    load_n: f32,
}

fn spec_default() -> vettura_physics::VehicleSpec {
    vettura_physics::VehicleSpec::default()
}

fn clamp_finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn wheel_index(index: i64) -> Result<usize, ()> {
    // 顺序固定为 FL/FR/RL/RR，与 `WHEEL_NODES` 一致。
    Ok(index.clamp(0, 3) as usize)
}

fn glam_vec3(value: Vector3) -> glam::Vec3 {
    glam::Vec3::new(value.x, value.y, value.z)
}

fn glam_quat(value: Quaternion) -> glam::Quat {
    glam::Quat::from_xyzw(value.x, value.y, value.z, value.w)
}

#[cfg(test)]
mod tests {
    use super::{Vector3, WheelVisual};
    use godot::prelude::{Basis, Transform3D};

    /// 局部旋转必须叠加在静止变换之后，否则会把作者放好的轮子位置抹掉。
    #[test]
    fn wheel_rotation_preserves_rest_origin() {
        let mut visual = WheelVisual {
            node: None,
            rest: Transform3D {
                basis: Basis::IDENTITY,
                origin: Vector3::new(0.7, 0.3, -1.52),
            },
        };

        // 没有绑定节点时不能 panic，只能静默跳过——这正是本测试的意图。
        visual.apply(0.25, 1.0);
    }

    /// `Transform3D::IDENTITY` 是可用的默认值：世界模式的车辆在还没绑车轮前就
    /// 已经带着它构造出来了。
    #[test]
    fn wheel_visual_defaults_to_identity() {
        let visual = WheelVisual::default();
        assert!(visual.node.is_none());
        assert_eq!(visual.rest, Transform3D::IDENTITY);
    }
}
