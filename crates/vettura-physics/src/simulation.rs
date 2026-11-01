use crate::{Controls, Pose, VehicleBackend};

/// Fixed-timestep driver for a [`VehicleBackend`].
///
/// Godot's `_physics_process` delta is usually stable, but rendering hitches
/// and debugger pauses are not. This keeps the solver on a fixed step and
/// caps catch-up work to avoid the "spiral of death".
pub struct Simulation {
    backend: Box<dyn VehicleBackend>,
    fixed_dt: f32,
    accumulator: f32,
    max_substeps: u32,
}

impl Simulation {
    pub fn new(backend: Box<dyn VehicleBackend>) -> Self {
        Self {
            backend,
            fixed_dt: 1.0 / 600.0,
            accumulator: 0.0,
            max_substeps: 8,
        }
    }

    pub fn with_fixed_dt(mut self, fixed_dt: f32) -> Self {
        self.fixed_dt = fixed_dt.max(1.0e-5);
        self
    }

    pub fn with_max_substeps(mut self, max_substeps: u32) -> Self {
        self.max_substeps = max_substeps.max(1);
        self
    }

    /// Advance by `frame_dt` seconds. Returns the number of solver substeps.
    pub fn step(&mut self, frame_dt: f32, controls: &Controls) -> u32 {
        if !frame_dt.is_finite() || frame_dt <= 0.0 {
            return 0;
        }

        self.accumulator += frame_dt;
        let controls = controls.clamped();
        let mut steps = 0;

        while self.accumulator >= self.fixed_dt && steps < self.max_substeps {
            self.backend.step(self.fixed_dt, &controls);
            self.accumulator -= self.fixed_dt;
            steps += 1;
        }

        if steps == self.max_substeps && self.accumulator > self.fixed_dt {
            // Drop the backlog instead of running an unbounded catch-up loop.
            self.accumulator = 0.0;
        }

        steps
    }

    pub fn reset(&mut self, pose: Pose) {
        self.accumulator = 0.0;
        self.backend.reset(pose);
    }

    pub fn state(&self) -> &crate::VehicleState {
        self.backend.state()
    }

    pub fn spec(&self) -> &crate::VehicleSpec {
        self.backend.spec()
    }

    pub fn backend(&self) -> &dyn VehicleBackend {
        self.backend.as_ref()
    }

    pub fn backend_mut(&mut self) -> &mut dyn VehicleBackend {
        self.backend.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VehicleSpec, VehicleState};
    use std::sync::{Arc, Mutex};

    /// backend 侧观察到的调用记录（步数、模拟秒数、最后一帧控制量）。
    #[derive(Default)]
    struct Recording {
        steps: u32,
        simulated_seconds: f32,
        last_controls: Controls,
    }

    /// 只数步数的测试 backend；通过共享的 [`Recording`] 把行为暴露给断言。
    struct CountingBackend {
        spec: VehicleSpec,
        state: VehicleState,
        recording: Arc<Mutex<Recording>>,
    }

    impl CountingBackend {
        fn new() -> (Self, Arc<Mutex<Recording>>) {
            let recording = Arc::new(Mutex::new(Recording::default()));
            let backend = Self {
                spec: VehicleSpec::default(),
                state: VehicleState::default(),
                recording: Arc::clone(&recording),
            };
            (backend, recording)
        }
    }

    impl VehicleBackend for CountingBackend {
        fn spec(&self) -> &VehicleSpec {
            &self.spec
        }
        fn state(&self) -> &VehicleState {
            &self.state
        }
        fn reset(&mut self, pose: Pose) {
            self.state = VehicleState {
                position: pose.position,
                orientation: pose.orientation,
                ..VehicleState::default()
            };
        }
        fn step(&mut self, dt: f32, controls: &Controls) {
            let mut recording = self.recording.lock().unwrap();
            recording.steps += 1;
            recording.simulated_seconds += dt;
            recording.last_controls = *controls;
        }
    }

    #[test]
    fn runs_exactly_the_fixed_steps_that_fit() {
        // 1/1024 与 1/64 都是二进制精确分数：16 × 1/1024 恰好等于 1/64，
        // 不依赖 f32 舍去位的运气（真实默认步长 1/600 边界上会差一两位）。
        // 上限放到 16，否则默认的 8 子步上限会先把帧截断。
        let (backend, recording) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend))
            .with_fixed_dt(1.0 / 1024.0)
            .with_max_substeps(16);

        assert_eq!(sim.step(1.0 / 64.0, &Controls::default()), 16);
        // 同一比例再跑一帧，仍然是精确的 16 步、无残余漂移。
        assert_eq!(sim.step(1.0 / 64.0, &Controls::default()), 16);

        let recording = recording.lock().unwrap();
        assert_eq!(recording.steps, 32);
        assert!((recording.simulated_seconds - 0.03125).abs() < 1e-6);
    }

    #[test]
    fn default_settings_cap_a_60hz_frame_at_eight_substeps() {
        // 默认 600 Hz 固定步 + 8 子步上限 → 单帧最多覆盖 8 × 1/600 = 13.3 ms。
        // 60fps 的 16.7 ms 帧必然吃满上限并丢弃残余：每个帧恒定 8 个子步，
        // 模拟时长是墙钟的 80%。这是“防止追帧死亡螺旋”的设计行为，
        // 测试把它钉住，避免以后误改。
        let (backend, recording) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend));

        for _ in 0..6 {
            assert_eq!(sim.step(1.0 / 60.0, &Controls::default()), 8);
        }

        let recording = recording.lock().unwrap();
        assert_eq!(recording.steps, 6 * 8);
        assert!((recording.simulated_seconds - 0.08).abs() < 1e-5);
    }

    #[test]
    fn caps_substeps_and_drops_the_backlog() {
        // 一秒的帧 + 上限 3 步：跑满 3 步后丢弃残余，而不是死循环追帧。
        let (backend, recording) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend)).with_max_substeps(3);

        assert_eq!(sim.step(1.0, &Controls::default()), 3);

        // 累加器已被清空：下一帧一个子步都不该补跑。
        assert_eq!(sim.step(1.0 / 1200.0, &Controls::default()), 0);
        assert_eq!(recording.lock().unwrap().steps, 3);
    }

    #[test]
    fn rejects_non_finite_and_non_positive_frames() {
        let (backend, recording) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend));

        for frame_dt in [0.0, -1.0 / 60.0, f32::NAN, f32::INFINITY] {
            assert_eq!(sim.step(frame_dt, &Controls::default()), 0);
        }

        // 坏帧不能污染累加器：紧随其后的好帧照常工作。
        assert_eq!(sim.step(1.0 / 600.0, &Controls::default()), 1);
        assert_eq!(recording.lock().unwrap().steps, 1);
    }

    #[test]
    fn fixed_dt_is_clamped_to_a_positive_floor() {
        // 0 或负数应当被夹到 1e-5 s，而不是永远跑不了或除零。
        let (backend, _) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend)).with_fixed_dt(0.0);

        // 2e-5 s 理论上正好 2 个 1e-5 步长；f32 舍入允许差一步，但不能跑飞。
        let steps = sim.step(2.0e-5, &Controls::default());
        assert!((1..=3).contains(&steps), "steps: {steps}");
    }

    #[test]
    fn controls_are_clamped_before_reaching_the_backend() {
        let (backend, recording) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend));

        let controls = Controls {
            throttle: 7.0,
            steering: -4.0,
            brake: -0.5,
            ..Controls::default()
        };
        assert_eq!(sim.step(1.0 / 600.0, &controls), 1);

        let observed = recording.lock().unwrap().last_controls;
        assert_eq!(observed.throttle, 1.0);
        assert_eq!(observed.steering, -1.0);
        assert_eq!(observed.brake, 0.0);
    }

    #[test]
    fn reset_clears_the_accumulator() {
        let (backend, _) = CountingBackend::new();
        let mut sim = Simulation::new(Box::new(backend)).with_fixed_dt(1.0 / 600.0);

        // 先留一点残余在累加器里（1.5 个步长 → 1 步 + 0.5 步残余）。
        assert_eq!(sim.step(1.5 / 600.0, &Controls::default()), 1);
        // 再补 0.6 步长：有残余时 0.5 + 0.6 ≥ 1 → 1 步。
        assert_eq!(sim.step(0.6 / 600.0, &Controls::default()), 1);

        sim.reset(Pose::default());
        // reset 之后累加器归零：同样的 0.6 步长不够一个完整步长。
        assert_eq!(sim.step(0.6 / 600.0, &Controls::default()), 0);
    }

    #[test]
    fn spec_and_state_forward_to_the_backend() {
        let (backend, _) = CountingBackend::new();
        let sim = Simulation::new(Box::new(backend));
        assert_eq!(sim.spec().mass_kg, 1200.0);
        assert_eq!(sim.state().gear, 1);
    }
}
