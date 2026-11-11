//! Jolt 世界节点：主项目进入赛车模拟的入口。
//!
//! 它做四件事，别的什么都不做：
//!
//! 1. **加载赛道**：`models*.ini` / `surfaces.ini` / `ai/fast_lane.ai` /
//!    `data/sections.ini` / `groove.ini`，把碰撞体装进 Jolt 静态网格，并把同一份
//!    几何转成一张低模 `MeshInstance3D`（见 [`track_mesh`]）；
//! 2. **生成车辆**：从 [`AcCar`](crate::ac_car::AcCar) 拿真实 `VehicleSpec`，
//!    落到 `start_pose` 或发车格上，把车模实例化到 [`VehicleBody`] 下并绑轮；
//! 3. **统一步进**：在自己的 `_physics_process` 里按固定步长推进 Jolt，再把
//!    `VehicleSnapshot` 推给每个 `VehicleBody`——子节点的 `_physics_process`
//!    不会重复步进物理；
//! 4. **计时**：用 `AiSpline` + `sections.ini` 驱动 `LapTimer`，对外只暴露
//!    圈速 / 分段 / 有效性这些标量。
//!
//! 之所以放在 `vettura-godot` 而不是 `vettura-world`，是因为它需要依赖节点树与
//! `ProjectSettings`；`vettura-world` 保持 headless，仍可被 CLI 与测试直接复用。

use godot::classes::{INode3D, MeshInstance3D, Node, Node3D};
use godot::prelude::*;
use vettura_physics::Pose;
use vettura_world::{LapTimer, TrackModule, VehicleHandle, World};

use crate::ac_car::AcCar;
use crate::paths::native_path;
use crate::vehicle_body::VehicleBody;

mod track_mesh;

/// 发车格长度（米）。`TrackModule` 自己会做坡度、净空、路面检查，这里只是告诉它
/// 想要几排。
const GRID_SLOTS: usize = 16;
/// 发车间隔（米）。
const GRID_SPACING_M: f32 = 8.0;

/// 赛车世界节点。
///
/// ```gdscript
/// var world := WorldBody.new()
/// add_child(world)
/// if world.load_track("res://mods/daytona_2017", "roadcourse"):
///     var car := world.spawn_vehicle_from_path(
///         "res://mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari"
///     )
///     car.set_controls(1.0, 0.0, 0.0, false, false, false, true, true)
/// ```
#[derive(GodotClass)]
#[class(base = Node3D, init)]
pub struct WorldBody {
    base: Base<Node3D>,
    world: Option<World>,
    track: Option<TrackModule>,
    track_mesh: Option<Gd<MeshInstance3D>>,
    lap_timer: Option<LapTimer>,
    start_pose: Option<Pose>,
    grid_poses: Vec<Pose>,
    vehicles: Vec<Gd<VehicleBody>>,
    primary: Option<VehicleHandle>,
    elapsed_s: f32,
}

#[godot_api]
impl INode3D for WorldBody {
    /// Godot 物理回调：把所有车的输入推进世界，再统一同步渲染。
    fn physics_process(&mut self, delta: f64) {
        let Some(world) = self.world.as_mut() else {
            return;
        };

        // 先收输入：`VehicleBody` 存的是这一帧的 `Controls`，`set_controls` 会转成
        // Jolt 的 `DriverInput`。布尔量（换挡/DRS/ABS/TC）是边沿触发，所以在
        // step 之后要清掉，否则会一直换挡。
        for vehicle in &mut self.vehicles {
            let Some(handle) = vehicle.bind().handle() else {
                continue;
            };
            let controls = vehicle.bind().controls();
            if world.set_controls(handle, controls).is_err() {
                continue;
            }
            vehicle.bind_mut().clear_edge_inputs();
        }

        self.elapsed_s += delta as f32;
        if let Err(err) = world.step(delta as f32) {
            godot_error!("WorldBody step failed: {}", err);
            return;
        }

        for vehicle in &mut self.vehicles {
            let Some(handle) = vehicle.bind().handle() else {
                continue;
            };
            if let Some(snapshot) = world.snapshot(handle) {
                let snapshot = snapshot.clone();
                vehicle.bind_mut().sync_snapshot(&snapshot);
            }
        }

        if let (Some(timer), Some(handle)) = (self.lap_timer.as_mut(), self.primary)
            && let Some(snapshot) = world.snapshot(handle)
        {
            timer.update(snapshot.reference_position, self.elapsed_s);
        }
    }
}

#[godot_api]
impl WorldBody {
    /// 加载一个 AC 赛道并创建 Jolt 世界。
    ///
    /// - `path` 可以是 `res://`（内部会 `globalize_path`）或文件系统路径；
    /// - `layout` 为空时使用 `TrackModule::load`（`models.ini` / 第一个
    ///   `models_*.ini`）； Daytona 这种多布局赛道请传 `"roadcourse"` / `"oval"`。
    ///
    /// 返回 `true` 表示赛道与碰撞体都装进了世界，可以直接 `spawn_vehicle*`。
    #[func]
    pub fn load_track(&mut self, path: GString, layout: GString) -> bool {
        self.load_track_with_options(path, layout, true)
    }

    /// 与 [`load_track`](Self::load_track) 相同，但可以选择不创建低模路面网格。
    ///
    /// 主项目自己渲染赛道时传 `false`：物理与计时完全不受影响。
    #[func]
    pub fn load_track_with_options(
        &mut self,
        path: GString,
        layout: GString,
        build_visual_mesh: bool,
    ) -> bool {
        self.unload();

        let root = native_path(&path);
        let layout = layout.to_string();
        let track = if layout.trim().is_empty() {
            TrackModule::load(&root)
        } else {
            TrackModule::load_layout(&root, &layout)
        };
        let track = match track {
            Ok(track) => track,
            Err(err) => {
                godot_error!("WorldBody.load_track: {}", err);
                return false;
            }
        };

        let world = match track.create_world() {
            Ok(world) => world,
            Err(err) => {
                godot_error!(
                    "WorldBody.load_track: Jolt world rejected the track: {}",
                    err
                );
                return false;
            }
        };

        let lap_timer = track.lap_timer().ok().flatten();
        let start_pose = track.start_pose().ok().flatten();
        let grid_poses = track
            .spawn_poses(GRID_SLOTS, GRID_SPACING_M)
            .unwrap_or_default();

        if build_visual_mesh {
            self.track_mesh = track_mesh::collider_mesh_instance(&track);
            if self.track_mesh.is_none() {
                godot_warn!("WorldBody: track has no collider mesh to draw");
            }
        }

        self.world = Some(world);
        self.track = Some(track);
        self.lap_timer = lap_timer;
        self.start_pose = start_pose;
        self.grid_poses = grid_poses;

        if let Some(mesh) = self.track_mesh.clone() {
            self.base_mut().add_child(&mesh.upcast::<Node>());
        }

        true
    }

    /// 当前是否已经加载了赛道与世界。
    #[func]
    pub fn is_track_loaded(&self) -> bool {
        self.world.is_some() && self.track.is_some()
    }

    /// 赛道根目录（加载时传入的路径）。
    #[func]
    pub fn track_root(&self) -> GString {
        self.track
            .as_ref()
            .map(|track| GString::from(track.root().to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    /// 赛道布局（`roadcourse` / `oval` / ...）。
    #[func]
    pub fn track_layout(&self) -> GString {
        self.track
            .as_ref()
            .and_then(TrackModule::layout)
            .map(GString::from)
            .unwrap_or_default()
    }

    /// Jolt 世界里已安装的静态物体数量（赛道放进去后通常为 1）。
    #[func]
    pub fn track_object_count(&self) -> i64 {
        self.world
            .as_ref()
            .map_or(0, |world| world.track_object_count() as i64)
    }

    /// 当前车辆数。
    #[func]
    pub fn vehicle_count(&self) -> i64 {
        self.vehicles.len() as i64
    }

    /// 在赛道起点（或第一个可用发车格）生成一台车。
    #[func]
    pub fn spawn_vehicle(&mut self, car: Gd<AcCar>) -> Option<Gd<VehicleBody>> {
        let pose = self.spawn_pose_for(None)?;
        self.spawn_vehicle_at_pose(car, pose)
    }

    /// 直接用 AC 车目录路径生成一台车，省去手动创建 `AcCar` 资源。
    #[func]
    pub fn spawn_vehicle_from_path(&mut self, car_path: GString) -> Option<Gd<VehicleBody>> {
        let mut car = AcCar::new_gd();
        if !car.bind_mut().open(car_path) {
            godot_error!("WorldBody.spawn_vehicle_from_path: cannot open car directory");
            return None;
        }
        self.spawn_vehicle(car)
    }

    /// 在指定位置 / 朝向（绕 `+Y` 的 yaw，单位度）生成一台车。
    #[func]
    pub fn spawn_vehicle_at(
        &mut self,
        car: Gd<AcCar>,
        position: Vector3,
        yaw_deg: f32,
    ) -> Option<Gd<VehicleBody>> {
        let yaw = if yaw_deg.is_finite() {
            yaw_deg.to_radians()
        } else {
            0.0
        };
        let orientation = glam::Quat::from_rotation_y(yaw);
        let pose = Pose {
            position: glam::Vec3::new(position.x, position.y, position.z),
            orientation,
        };
        self.spawn_vehicle_at_pose(car, pose)
    }

    /// 在第 `index` 个发车格生成一台车；越界时按最近的有效格钳制。
    #[func]
    pub fn spawn_vehicle_on_grid(&mut self, car: Gd<AcCar>, index: i64) -> Option<Gd<VehicleBody>> {
        let pose = self.spawn_pose_for(Some(index))?;
        self.spawn_vehicle_at_pose(car, pose)
    }

    /// 把一台车送回它自己的初始位姿。
    #[func]
    pub fn reset_vehicle(&mut self, vehicle: Gd<VehicleBody>) {
        let (handle, pose) = {
            let bound = vehicle.bind();
            (bound.handle(), bound.initial_pose())
        };
        if let (Some(handle), Some(world)) = (handle, self.world.as_mut())
            && let Err(err) = world.reset_vehicle(handle, pose)
        {
            godot_error!("WorldBody.reset_vehicle: {}", err);
        }
    }

    /// 低模路面网格的显隐（不影响物理）。
    #[func]
    pub fn set_track_mesh_visible(&mut self, visible: bool) {
        if let Some(mesh) = self.track_mesh.as_mut() {
            mesh.set_visible(visible);
        }
    }

    /// 当前进行中的圈速，秒。起点线尚未越过时为 `0`。
    #[func]
    pub fn lap_time_s(&self) -> f32 {
        self.lap_timer
            .as_ref()
            .map_or(0.0, |timer| timer.lap_time_s())
    }

    /// 上一个完成圈，秒；没有完成圈时为 `-1`（GDScript 侧好判断）。
    #[func]
    pub fn last_lap_s(&self) -> f32 {
        self.lap_timer
            .as_ref()
            .and_then(LapTimer::last_lap_s)
            .unwrap_or(-1.0)
    }

    /// 最快圈，秒；没有完成圈时为 `-1`。
    #[func]
    pub fn best_lap_s(&self) -> f32 {
        self.lap_timer
            .as_ref()
            .and_then(LapTimer::best_lap_s)
            .unwrap_or(-1.0)
    }

    /// 已完成且计数的圈数。
    #[func]
    pub fn laps(&self) -> i64 {
        self.lap_timer
            .as_ref()
            .map_or(0, |timer| timer.laps() as i64)
    }

    /// 分段数量（含起点线那一段）。
    #[func]
    pub fn sector_count(&self) -> i64 {
        self.lap_timer
            .as_ref()
            .map_or(0, |timer| timer.sector_count() as i64)
    }

    /// 当前所在分段索引。
    #[func]
    pub fn current_sector(&self) -> i64 {
        self.lap_timer
            .as_ref()
            .map_or(0, |timer| timer.sector() as i64)
    }

    /// 进行中的圈里已经完成的分段用时。
    #[func]
    pub fn sector_times_s(&self) -> PackedFloat32Array {
        self.lap_timer
            .as_ref()
            .map(|timer| timer.sector_times_s().iter().copied().collect())
            .unwrap_or_default()
    }

    /// 上一个完整圈的分段用时。
    #[func]
    pub fn last_sector_times_s(&self) -> PackedFloat32Array {
        self.lap_timer
            .as_ref()
            .map(|timer| timer.last_sector_times_s().iter().copied().collect())
            .unwrap_or_default()
    }

    /// 计时清零（保留赛道与车辆）。
    #[func]
    pub fn reset_lap_timer(&mut self) {
        self.elapsed_s = 0.0;
        if let Some(timer) = self.lap_timer.as_mut() {
            timer.reset();
        }
    }
}

impl WorldBody {
    /// 生成车辆：装进世界 → 造节点 → 实例化车模 → 绑轮 → 登记。
    fn spawn_vehicle_at_pose(&mut self, mut car: Gd<AcCar>, pose: Pose) -> Option<Gd<VehicleBody>> {
        let spec = car.bind().vehicle_spec()?;
        let handle = {
            let world = self.world.as_mut()?;
            match world.add_vehicle(spec, pose) {
                Ok(handle) => handle,
                Err(err) => {
                    godot_error!("WorldBody.spawn_vehicle: {}", err);
                    return None;
                }
            }
        };

        let mut body = VehicleBody::new_alloc();
        body.set_position(Vector3::new(
            pose.position.x,
            pose.position.y,
            pose.position.z,
        ));
        body.set_quaternion(Quaternion::new(
            pose.orientation.x,
            pose.orientation.y,
            pose.orientation.z,
            pose.orientation.w,
        ));
        let display_name = car.bind().display_name().to_string();
        body.set_name(&StringName::from(display_name.as_str()));
        self.base_mut().add_child(&body.clone().upcast::<Node>());

        // 车模挂在车身节点下，车身节点只负责物理位姿；这样 `WHEEL_*` dummy 的
        // 静止变换就是作者值，车轮旋转不会污染车模。
        if let Some(root) = car.bind_mut().instantiate(body.clone().upcast::<Node>()) {
            let bound = body.bind_mut().bind_wheels(root);
            if bound < 4 {
                godot_warn!("WorldBody: only {} / 4 wheel dummies bound", bound);
            }
        }
        {
            let mut bound = body.bind_mut();
            bound.attach_to_world(handle);
            bound.set_initial_pose(pose);
        }

        if self.primary.is_none() {
            self.primary = Some(handle);
        }
        self.vehicles.push(body.clone());
        Some(body)
    }

    /// 发车位选择：`index` 有值走发车格，否则起点优先。
    fn spawn_pose_for(&self, index: Option<i64>) -> Option<Pose> {
        if let Some(index) = index
            && !self.grid_poses.is_empty()
        {
            let last = self.grid_poses.len() as i64 - 1;
            let slot = index.clamp(0, last) as usize;
            return Some(self.grid_poses[slot]);
        }

        self.start_pose
            .or_else(|| self.grid_poses.first().copied())
            .or_else(|| self.world.as_ref().map(|_| Pose::default()))
    }

    /// 卸掉当前赛道：释放车辆、网格、世界，但保留节点本身（可再次 `load_track`）。
    fn unload(&mut self) {
        // `Gd::upcast` 会拿走所有权，先把 drain 出来的资源接走，避免和
        // `self.base_mut()` 的借用撞在一起。
        let spawned = std::mem::take(&mut self.vehicles);
        for vehicle in spawned {
            self.base_mut()
                .remove_child(&vehicle.clone().upcast::<Node>());
            vehicle.free();
        }
        if let Some(mesh) = self.track_mesh.take() {
            self.base_mut().remove_child(&mesh.clone().upcast::<Node>());
            mesh.free();
        }

        self.world = None;
        self.track = None;
        self.lap_timer = None;
        self.start_pose = None;
        self.grid_poses.clear();
        self.primary = None;
        self.elapsed_s = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{GRID_SLOTS, GRID_SPACING_M};

    /// 发车格参数是物理层的输入，必须落在 `TrackModule::spawn_poses` 的钳制范围内。
    ///
    /// 编译期断言而不是运行时测试：这两个常量写错的代价是发车位全部落回线外，
    /// 那属于代码错误，不是环境错误。
    const _: () = assert!(GRID_SLOTS > 0 && GRID_SPACING_M >= 2.0 && GRID_SPACING_M <= 100.0);
}
