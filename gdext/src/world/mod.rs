//! Jolt 世界节点：主项目进入赛车模拟的入口。
//!
//! 它做四件事，别的什么都不做：
//!
//! 1. **加载赛道**：`models*.ini` / `surfaces.ini` / `ai/fast_lane.ai` /
//!    `data/sections.ini` / `groove.ini`，把碰撞体装进 Jolt 静态网格，渲染侧
//!    优先实例化 `models.ini` 里的真实视觉 KN5（见 [`track_models`]），
//!    失败才退回同一份碰撞几何的低模 `MeshInstance3D`（见 [`track_mesh`]）；
//! 2. **生成车辆**：从 [`AcCar`](crate::ac_car::AcCar) 拿真实 `VehicleSpec`，
//!    落到 `start_pose` 或发车格上，把车模实例化到 [`VehicleBody`] 下并绑轮；
//! 3. **统一步进**：在自己的 `_physics_process` 里按固定步长推进 Jolt，再把
//!    `VehicleSnapshot` 推给每个 `VehicleBody`——子节点的 `_physics_process`
//!    不会重复步进物理；
//! 4. **计时**：用 `AiSpline` + `sections.ini` 驱动 `LapTimer`，对外只暴露
//!    圈速 / 分段 / 有效性这些标量；
//! 5. **遥测**：读 `user://telemetry.cfg`，每帧（按 `rate_hz` 节流）把车辆 /
//!    赛道 / 计时数据打成 JSON，经 UDP / TCP 发到配置的地址（见
//!    [`vettura_world::TelemetrySink`]）。
//!
//! 之所以放在 `vettura-godot` 而不是 `vettura-world`，是因为它需要依赖节点树与
//! `ProjectSettings`；`vettura-world` 保持 headless，仍可被 CLI 与测试直接复用。

use godot::classes::{INode3D, MeshInstance3D, Node, Node3D};
use godot::prelude::*;
use vettura_assets::TrackModule;
use vettura_physics::Pose;
use vettura_world::{
    AiDriver, DEFAULT_AI_SKILL, LapTimer, TELEMETRY_CONFIG_PATH, TelemetryConfig,
    TelemetryProtocol, TelemetrySample, TelemetryScope, TelemetrySink, VehicleHandle,
    VehicleSample, World,
};

use crate::ac_car::AcCar;
use crate::paths::native_path;
use crate::vehicle_body::VehicleBody;

mod track_mesh;
mod track_models;

/// 发车格长度（米）。`TrackModule` 自己会做坡度、净空、路面检查，这里只是告诉它
/// 想要几排。
const GRID_SLOTS: usize = 16;
/// 发车间隔（米）。
const GRID_SPACING_M: f32 = 8.0;
/// 遥测配置文件的轮询间隔（秒）：`user://telemetry.cfg` 可以在运行中被改写，
/// 每隔这么久比一次 mtime，变了就热加载。
const TELEMETRY_POLL_S: f32 = 0.5;

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
    /// 赛道真实视觉模型的容器节点（`models.ini` 里的 `*mesh*.kn5`）。
    /// 可视时替代 `track_mesh` 的低模，两者不会同时存在。
    track_models: Option<Gd<Node3D>>,
    lap_timer: Option<LapTimer>,
    /// 赛车的 AI 线：`fast_lane.ai` 存在时才有。`ai_controls` 每帧从这里求输入。
    ai: Option<AiDriver>,
    start_pose: Option<Pose>,
    grid_poses: Vec<Pose>,
    vehicles: Vec<Gd<VehicleBody>>,
    primary: Option<VehicleHandle>,
    elapsed_s: f32,
    /// 遥测输出端点（`user://telemetry.cfg` 驱动；默认整体禁用）。
    telemetry: TelemetrySink,
    /// 配置文件上次的 mtime，用于检测运行中改写（热更新）。
    telemetry_cfg_mtime: Option<std::time::SystemTime>,
    /// 配置文件轮询的计时器。
    telemetry_poll_s: f32,
    /// 上次遥测采样时刻（秒），按 `rate_hz` 节流；`None` = 还没采过（首帧立即发）。
    telemetry_last_sample_s: Option<f32>,
    /// `World::impacts()` 累积缓冲的已发送游标（该缓冲从不 drain）。
    telemetry_impact_cursor: usize,
}

#[godot_api]
impl INode3D for WorldBody {
    /// 节点就绪时读一次遥测配置。文件缺失 = 静默禁用（默认状态）。
    fn ready(&mut self) {
        self.load_telemetry_config(false);
    }

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
                vehicle.bind_mut().sync_snapshot(snapshot);
            }
        }

        if let (Some(timer), Some(handle)) = (self.lap_timer.as_mut(), self.primary)
            && let Some(snapshot) = world.snapshot(handle)
        {
            timer.update(snapshot.reference_position, self.elapsed_s);
        }

        // 遥测在物理步进与计时之后采样（`world` 的借用到此结束，下面重新独占 self）。
        self.poll_telemetry_config(delta as f32);
        self.emit_telemetry();
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
        // AI 线是可选的：赛道没有 `fast_lane.ai` 时只是没有 AI 输入源，
        // 赛道本身照常可用；解析失败则是数据损坏，必须让人看见。
        let ai = match track.ai_driver() {
            Ok(driver) => driver,
            Err(err) => {
                godot_warn!("WorldBody.load_track: AI line unusable: {}", err);
                None
            }
        };
        let start_pose = track.start_pose().ok().flatten();
        let grid_poses = track
            .spawn_poses(GRID_SLOTS, GRID_SPACING_M)
            .unwrap_or_default();

        if build_visual_mesh {
            // 先试真实视觉模型（`models.ini` 的 KN5）；一个都没加载成功才退回
            // 碰撞低模——两条路都有 warning，不会静默。
            let models = {
                let mut host = self.base().clone().upcast::<Node>();
                track_models::visual_models_instance(&track, &mut host)
            };
            if let Some(models) = models {
                self.track_models = Some(models);
            } else {
                self.track_mesh = track_mesh::collider_mesh_instance(&track);
                if self.track_mesh.is_none() {
                    godot_warn!("WorldBody: track has no collider mesh to draw");
                }
            }
        }

        self.world = Some(world);
        self.track = Some(track);
        self.lap_timer = lap_timer;
        self.ai = ai;
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

    /// 赛道视觉网格的显隐（真实模型或回退低模，不影响物理）。
    #[func]
    pub fn set_track_mesh_visible(&mut self, visible: bool) {
        if let Some(mesh) = self.track_mesh.as_mut() {
            mesh.set_visible(visible);
        }
        if let Some(models) = self.track_models.as_mut() {
            models.set_visible(visible);
        }
    }

    /// 让 AI 沿赛道线驾驶第 `index` 辆车，返回 `[油门, 刹车, 转向]`。
    ///
    /// - 返回空数组表示这条赛道没有 `fast_lane.ai`、车辆不存在或还没接上物理；
    /// - `AiDriver` 是有状态的纯追踪控制器：每次调用都以上一帧结束时的车辆位置
    ///   为准，所以调用顺序就是时间顺序，每帧调用一次即可；
    /// - 转向为 `-1..=1`，正数向左。
    #[func]
    pub fn ai_controls(&mut self, index: i64) -> PackedFloat32Array {
        let Some(world) = self.world.as_ref() else {
            return PackedFloat32Array::new();
        };
        let Some(driver) = self.ai.as_mut() else {
            return PackedFloat32Array::new();
        };
        let Some(vehicle) = self.vehicles.get(index.max(0) as usize) else {
            return PackedFloat32Array::new();
        };
        let Some(handle) = vehicle.bind().handle() else {
            return PackedFloat32Array::new();
        };
        let Some(view) = world.vehicle(handle) else {
            return PackedFloat32Array::new();
        };
        let input = driver.update_view(view);
        PackedFloat32Array::from(vec![input.throttle, input.brake, input.steering])
    }

    /// 把 `index` 号车放到比赛温度：发动机/机油到工作温度，轮胎到最佳抓地温度，
    /// 刹车片到最佳效率温度。
    ///
    /// 商业竞速游戏的 AI 永远从这个状态出发——发车格上它前面已经跑过暖胎圈。
    /// 不暖胎的代价实测极大：这台 F1 在冷机下只有 55% 功率、变速箱还会因为冷胎
    /// 高滑移拒绝升挡，圈速直接慢 20% 以上。要建模暖胎过程就让他自己跑几分钟。
    #[func]
    pub fn warm_up_vehicle(&mut self, index: i64) -> bool {
        let Some(world) = self.world.as_mut() else {
            return false;
        };
        let Some(vehicle) = self.vehicles.get(index.max(0) as usize) else {
            return false;
        };
        let Some(handle) = vehicle.bind().handle() else {
            return false;
        };
        world.warm_up_vehicle(handle).is_ok()
    }

    /// 当前 AI 难度档位（AC 的 AI level），`0..=110`。
    ///
    /// 没有 AI 司机（赛道没有 `fast_lane.ai`）时返回默认档。
    #[func]
    pub fn ai_skill(&self) -> i64 {
        self.ai
            .as_ref()
            .map_or(DEFAULT_AI_SKILL.round() as i64, |driver| {
                driver.skill().round() as i64
            })
    }

    /// 运行时切换 AI 难度档位（AC 的 AI level）：`0`（新手）～ `110`（Pro）。
    ///
    /// 它缩放的是司机的能力而不是车的性能：抓地使用率、刹车时机、预瞄距离、
    /// 方向盘可用行程、贴线精度、避让幅度。`110` 是 AC 社区里那个抓地超出
    /// 物理极限的「不可战胜」档。
    #[func]
    pub fn set_ai_skill(&mut self, skill: i64) {
        if let Some(driver) = self.ai.as_mut() {
            driver.set_skill(skill as f32);
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

    /// 上一个完整圈是否是有效圈（没有倒车/离线cut）。
    #[func]
    pub fn last_lap_valid(&self) -> bool {
        self.lap_timer
            .as_ref()
            .is_some_and(LapTimer::was_last_lap_valid)
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

    /// 重新读取 `user://telemetry.cfg` 并立即生效（文件缺失则整体禁用）。
    ///
    /// 配置文件通常会被自动热更新（见 `physics_process` 里的 mtime 轮询），
    /// 这个方法给 GDScript 一个手动强制重载的入口。
    #[func]
    pub fn reload_telemetry_config(&mut self) {
        self.load_telemetry_config(true);
    }

    /// 运行时直接开关遥测（不改配置文件；下次热更新会以文件内容为准）。
    #[func]
    pub fn set_telemetry_enabled(&mut self, enabled: bool) {
        let mut config = self.telemetry.config().clone();
        config.enabled = enabled;
        self.telemetry.reconfigure(config);
    }

    /// 遥测当前是否启用（读的是生效中的配置）。
    #[func]
    pub fn is_telemetry_enabled(&self) -> bool {
        self.telemetry.config().enabled
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
        if let Some(models) = self.track_models.take() {
            self.base_mut()
                .remove_child(&models.clone().upcast::<Node>());
            models.free();
        }

        self.world = None;
        self.track = None;
        self.lap_timer = None;
        self.ai = None;
        self.start_pose = None;
        self.grid_poses.clear();
        self.primary = None;
        self.elapsed_s = 0.0;
        // 世界换了，遥测的采样节流与碰撞游标都要清零；输出配置本身保留
        // （换赛道不应关掉遥测）。
        self.telemetry_last_sample_s = None;
        self.telemetry_impact_cursor = 0;
    }

    /// 读取 `user://telemetry.cfg` 并套用到输出端点。
    ///
    /// - `missing_is_warning = false`：首次启动文件不存在是正常状态，静默禁用；
    /// - `missing_is_warning = true`：手动重载 / 热更新时文件没了，要让人看见。
    fn load_telemetry_config(&mut self, missing_is_warning: bool) {
        let path = native_path(&GString::from(TELEMETRY_CONFIG_PATH));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if missing_is_warning {
                    godot_warn!(
                        "WorldBody: telemetry config {} not found, telemetry disabled",
                        TELEMETRY_CONFIG_PATH
                    );
                }
                self.telemetry.reconfigure(TelemetryConfig::default());
                self.telemetry_cfg_mtime = None;
                return;
            }
            Err(err) => {
                godot_warn!(
                    "WorldBody: cannot read {}: {} (keeping previous telemetry config)",
                    TELEMETRY_CONFIG_PATH,
                    err
                );
                return;
            }
        };

        self.telemetry_cfg_mtime = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();

        let config = TelemetryConfig::parse(&text);
        if config.enabled && config.any_section_enabled() {
            godot_print!(
                "WorldBody: telemetry enabled -> {} {}:{} @ {} Hz, scope {:?}",
                match config.protocol {
                    TelemetryProtocol::Udp => "udp",
                    TelemetryProtocol::Tcp => "tcp",
                },
                config.host,
                config.port,
                config.rate_hz,
                config.scope
            );
        }
        self.telemetry.reconfigure(config);
    }

    /// 每 0.5 秒比一次配置文件 mtime，变了就热加载。
    fn poll_telemetry_config(&mut self, delta: f32) {
        self.telemetry_poll_s += delta;
        if self.telemetry_poll_s < TELEMETRY_POLL_S {
            return;
        }
        self.telemetry_poll_s = 0.0;

        let path = native_path(&GString::from(TELEMETRY_CONFIG_PATH));
        let mtime = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if mtime != self.telemetry_cfg_mtime {
            self.load_telemetry_config(true);
        }
    }

    /// 按配置采样一帧并发出去（UDP 数据报 / TCP 流）。
    ///
    /// 必须在 `step` 与 `LapTimer::update` 之后调用：帧里装的是本帧最终状态。
    fn emit_telemetry(&mut self) {
        let config = self.telemetry.config().clone();
        if !config.enabled || !config.any_section_enabled() {
            return;
        }
        if !config.should_sample(self.elapsed_s, &mut self.telemetry_last_sample_s) {
            return;
        }
        let Some(world) = self.world.as_ref() else {
            return;
        };

        // `World::impacts()` 是累积缓冲且本 crate 从不 drain：这里只取游标之后的
        // 新增段，然后推进游标（与是否发送无关，避免以后打开时回灌历史）。
        if world.impacts().len() < self.telemetry_impact_cursor {
            self.telemetry_impact_cursor = 0;
        }
        let impacts = &world.impacts()[self.telemetry_impact_cursor..];
        self.telemetry_impact_cursor = world.impacts().len();

        let primary = self.primary;
        let mut samples = Vec::new();
        if config.vehicle.enabled {
            for (index, vehicle) in self.vehicles.iter().enumerate() {
                let Some(handle) = vehicle.bind().handle() else {
                    continue;
                };
                let in_scope = match &config.scope {
                    TelemetryScope::Ego => Some(handle) == primary,
                    TelemetryScope::All => true,
                    TelemetryScope::Indices(list) => list.contains(&index),
                };
                if !in_scope {
                    continue;
                }
                let snapshot = world.snapshot(handle);
                let groove = snapshot
                    .filter(|_| config.track.groove)
                    .map(|s| world.groove_progress_at(s.reference_position));
                samples.push(VehicleSample {
                    handle: handle.raw(),
                    index,
                    primary: Some(handle) == primary,
                    snapshot,
                    controls: Some(vehicle.bind().controls()),
                    groove,
                });
            }
        }

        let sample = TelemetrySample {
            time_s: self.elapsed_s,
            vehicles: samples,
            weather: if config.track.enabled && config.track.weather {
                Some(world.weather())
            } else {
                None
            },
            impacts,
            timing: if config.timing.enabled {
                self.lap_timer.as_ref()
            } else {
                None
            },
            primary: primary.map(|handle| handle.raw()),
        };
        self.telemetry.send(&sample);
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
