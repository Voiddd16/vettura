//! Optional integration test against a real Assetto Corsa car folder.
//!
//! Set `VETTURA_CAR_DIR` to a car folder containing `data.acd` or `data/`.

use std::path::Path;

use vettura_assets::CarFolder;

#[test]
fn maps_env_car_folder_when_present() {
    let Ok(path) = std::env::var("VETTURA_CAR_DIR") else {
        return;
    };

    let car = CarFolder::open(&path).expect("car folder should open");
    assert!(!car.data().is_empty());

    let spec = car.vehicle_spec().expect("physics mapping should succeed");
    assert!(spec.mass_kg > 0.0);
    assert!(spec.wheelbase_m > 0.0);
    assert_eq!(spec.wheels.len(), 4);

    println!(
        "{}: {:.0} kg, {:.2} m wheelbase, {} gears",
        spec.name,
        spec.mass_kg,
        spec.wheelbase_m,
        spec.engine.gear_ratios.len()
    );
}

#[test]
fn picks_the_main_model_of_env_car_folder() {
    let Ok(path) = std::env::var("VETTURA_CAR_DIR") else {
        return;
    };

    let car = CarFolder::open(&path).expect("car folder should open");
    let model = car
        .main_model_path()
        .expect("car folder should contain a .kn5");

    // 主模型必须落在车目录内、以 .kn5 结尾，并且明显大于 collider。
    assert_eq!(model.parent(), Some(Path::new(&path)));
    assert!(
        model
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kn5"))
    );
    assert_ne!(
        model.file_name().and_then(|n| n.to_str()),
        Some("collider.kn5")
    );

    // 能被 KN5 解析器完整读进来。
    let kn5 = vettura_kn5::Kn5::from_path(&model).expect("main model should parse");
    assert!(kn5.node_count() > 0);

    println!(
        "main model: {} ({} nodes)",
        model.display(),
        kn5.node_count()
    );
}
