use godot::classes::Resource;
use godot::prelude::*;
use vettura_assets::CarFolder;
use vettura_physics::VehicleSpec;

/// Assetto Corsa 车辆目录资源。
///
/// 打开一个车目录，自动选择 `data.acd` 或已解包的 `data/` 目录，并把 INI /
/// LUT 数据映射为物理参数。当前只开放少量查询接口，完整物理与网格桥接属于
/// 后续工作。
///
/// 用法示例：
///
/// ```gdscript
/// var car := AcCar.new()
/// if car.open("res://cars/my_car"):
///     print(car.display_name(), " ", car.total_mass())
/// ```
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct AcCar {
    base: Base<Resource>,

    #[init(val = GString::new())]
    path: GString,

    car: Option<CarFolder>,
}

#[godot_api]
impl AcCar {
    /// 打开一个车目录。
    ///
    /// 成功后返回 `true`；失败会打印错误并返回 `false`。
    #[func]
    pub fn open(&mut self, path: GString) -> bool {
        match CarFolder::open(path.to_string()) {
            Ok(car) => {
                self.path = path;
                self.car = Some(car);
                true
            }
            Err(err) => {
                godot_error!("AC car load failed: {}", err);
                self.car = None;
                false
            }
        }
    }

    /// 当前是否已成功打开车目录。
    #[func]
    pub fn is_open(&self) -> bool {
        self.car.is_some()
    }

    /// 已解密/已读取的数据文件数量。
    #[func]
    pub fn data_file_count(&self) -> i64 {
        self.car.as_ref().map_or(0, |car| car.data().len() as i64)
    }

    /// 查询某个数据文件是否存在，例如 `car.ini`。
    #[func]
    pub fn has_data_file(&self, name: GString) -> bool {
        self.car
            .as_ref()
            .is_some_and(|car| car.data().get(&name.to_string()).is_some())
    }

    /// 车辆总质量，单位千克（kg）。未打开时返回 `0.0`。
    #[func]
    pub fn total_mass(&self) -> f64 {
        self.vehicle_spec()
            .map(|spec| spec.mass_kg as f64)
            .unwrap_or(0.0)
    }

    /// 轴距，单位米（m）。未打开时返回 `0.0`。
    #[func]
    pub fn wheelbase(&self) -> f64 {
        self.vehicle_spec()
            .map(|spec| spec.wheelbase_m as f64)
            .unwrap_or(0.0)
    }

    /// 车辆显示名，来自 `car.ini`；未打开时为空字符串。
    #[func]
    pub fn display_name(&self) -> GString {
        self.vehicle_spec()
            .map(|spec| GString::from(spec.name.as_str()))
            .unwrap_or_default()
    }

    /// 把车目录映射为 `VehicleSpec`。
    ///
    /// 内部在每次调用时重新解析 INI；如果映射失败会返回 `None` 并打印错误。
    pub fn vehicle_spec(&self) -> Option<VehicleSpec> {
        let car = self.car.as_ref()?;
        match car.vehicle_spec() {
            Ok(spec) => Some(spec),
            Err(err) => {
                godot_error!("AC physics mapping failed: {}", err);
                None
            }
        }
    }

    /// 访问底层车目录，给未来的网格/物理桥接使用。
    ///
    /// 当前没有暴露为 `#[func]`，因为 `CarFolder` 不是 Godot 类型。
    #[allow(dead_code)]
    pub fn car(&self) -> Option<&CarFolder> {
        self.car.as_ref()
    }
}
