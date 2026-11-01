use godot::classes::{INode3D, Node3D};
use godot::prelude::*;
use vettura_physics::backends::SimpleBackend;
use vettura_physics::{Controls, Simulation, VehicleSpec};

/// 车辆物理节点。
///
/// 以固定步长驱动 `Simulation`，并把结果同步到当前节点的位姿。当前使用
/// `SimpleBackend`；以后切换到 Chrono 等真实后端时，只需要替换
/// `Simulation::new` 传入的 backend。
///
/// 建议放在 `_physics_process` 中调用：
///
/// ```gdscript
/// func _physics_process(_delta):
///     $VehicleBody.set_inputs(
///         Input.get_action_strength("throttle"),
///         Input.get_action_strength("brake"),
///         Input.get_axis("steer_right", "steer_left")
///     )
///     print($VehicleBody.speed_mps())
/// ```
#[derive(GodotClass)]
#[class(base = Node3D, init)]
pub struct VehicleBody {
    base: Base<Node3D>,
    simulation: Option<Simulation>,
    controls: Controls,
}

#[godot_api]
impl INode3D for VehicleBody {
    /// Godot 物理回调：按帧推进固定步长仿真，并同步节点位姿。
    ///
    /// 未调用 `setup_default()` 时节点不会参与物理步进。
    fn physics_process(&mut self, delta: f64) {
        let Some(simulation) = self.simulation.as_mut() else {
            return;
        };

        simulation.step(delta as f32, &self.controls);

        let state = simulation.state();
        let position = state.position;
        let orientation = state.orientation;
        self.base_mut()
            .set_position(Vector3::new(position.x, position.y, position.z));
        self.base_mut().set_quaternion(Quaternion::new(
            orientation.x,
            orientation.y,
            orientation.z,
            orientation.w,
        ));
    }
}

#[godot_api]
impl VehicleBody {
    /// 用默认车辆参数初始化仿真，并以当前节点位姿为初始状态。
    ///
    /// 在读取车身输入前调用一次即可。
    #[func]
    pub fn setup_default(&mut self) {
        let position = self.base().get_position();
        let rotation = self.base().get_quaternion();
        let pose = vettura_physics::Pose {
            position: glam_vec3(position),
            orientation: glam::Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w),
        };

        let spec = VehicleSpec::default();
        self.simulation = Some(Simulation::new(Box::new(SimpleBackend::new(spec))));
        if let Some(simulation) = self.simulation.as_mut() {
            simulation.reset(pose);
        }
    }

    /// 设置驾驶员输入。
    ///
    /// `throttle` 和 `brake` 会被限制在 `[0, 1]`；`steering` 会被限制在
    /// `[-1, 1]`。
    #[func]
    pub fn set_inputs(&mut self, throttle: f32, brake: f32, steering: f32) {
        self.controls.throttle = throttle.clamp(0.0, 1.0);
        self.controls.brake = brake.clamp(0.0, 1.0);
        self.controls.steering = steering.clamp(-1.0, 1.0);
    }

    /// 当前车速，单位米每秒（m/s）。未初始化时返回 `0.0`。
    #[func]
    pub fn speed_mps(&self) -> f32 {
        self.simulation
            .as_ref()
            .map_or(0.0, |simulation| simulation.state().speed_mps)
    }
}

fn glam_vec3(value: Vector3) -> glam::Vec3 {
    glam::Vec3::new(value.x, value.y, value.z)
}
