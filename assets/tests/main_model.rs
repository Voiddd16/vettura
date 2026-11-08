//! `CarFolder::main_model_path` selection rules, against synthetic folders.
//!
//! Rules under test (see `docs/kn5-format.md` / AcTools `GetMainCarFilename`):
//! 1. `data/lods.ini` `[LOD_HR] FILE` / `[LOD_0] FILE` wins when it points at an
//!    existing `*.kn5`;
//! 2. otherwise the largest `.kn5` directly inside the car folder wins;
//! 3. anything unstat-able or pointing nowhere never wins.

mod common;

use common::{TempDir, car_folder};
use vettura_assets::CarFolder;

fn main_model(root: &TempDir) -> Option<std::path::PathBuf> {
    CarFolder::open(root.path())
        .expect("synthetic car folder should open")
        .main_model_path()
}

fn file_name(path: &std::path::Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

#[test]
fn lods_ini_lod_hr_wins_over_larger_file() {
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=body.kn5\n")]);
    let model = fixture.write_sized("body.kn5", 64);
    fixture.write_sized("bigger.kn5", 4096);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn lods_ini_lod_0_is_used_when_lod_hr_missing() {
    let fixture = car_folder(&[("lods.ini", "[LOD_0]\nFILE=lod0.kn5\n")]);
    let model = fixture.write_sized("lod0.kn5", 128);
    fixture.write_sized("bigger.kn5", 4096);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn lods_ini_lod_hr_takes_priority_over_lod_0() {
    // 两段都存在时，LOD_HR 在前、先命中。
    let fixture = car_folder(&[(
        "lods.ini",
        "[LOD_0]\nFILE=lod0.kn5\n[LOD_HR]\nFILE=hr.kn5\n",
    )]);
    let model = fixture.write_sized("hr.kn5", 8);
    fixture.write_sized("lod0.kn5", 8);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn lods_ini_may_point_into_a_subdirectory() {
    // AC 的 lods.ini 允许相对路径；join(root, file) 之后即可命中。
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=lods/body.kn5\n")]);
    let model = fixture.write_sized("lods/body.kn5", 64);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn lods_ini_entry_without_a_kn5_extension_falls_back() {
    // 条目存在且文件真实，但不是 .kn5 → 跳过条目走最大文件回退。
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=notes.txt\n")]);
    fixture.write("notes.txt", b"not a model");
    let fallback = fixture.write_sized("body.kn5", 256);

    assert_eq!(main_model(&fixture), Some(fallback));
}

#[test]
fn lods_ini_uppercase_extension_counts_as_kn5() {
    // 大小写不敏感：body.KN5 也是合法主模型。
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=body.KN5\n")]);
    let model = fixture.write_sized("body.KN5", 64);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn lods_ini_entry_pointing_nowhere_falls_back() {
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=ghost.kn5\n")]);
    let fallback = fixture.write_sized("body.kn5", 256);

    assert_eq!(main_model(&fixture), Some(fallback));
}

#[test]
fn lods_ini_with_blank_file_entry_falls_back() {
    let fixture = car_folder(&[("lods.ini", "[LOD_HR]\nFILE=   \n[LOD_0]\nFILE=\n")]);
    let fallback = fixture.write_sized("body.kn5", 32);

    assert_eq!(main_model(&fixture), Some(fallback));
}

#[test]
fn lods_ini_without_sections_falls_back() {
    let fixture = car_folder(&[("lods.ini", "just some noise\n")]);
    let fallback = fixture.write_sized("body.kn5", 32);

    assert_eq!(main_model(&fixture), Some(fallback));
}

#[test]
fn picks_the_largest_kn5_in_the_folder_root() {
    let fixture = car_folder(&[]);
    let model = fixture.write_sized("car.kn5", 2048);
    fixture.write_sized("collider.kn5", 40);
    fixture.write_sized("small.kn5", 10);
    // 非 kn5 的文件再大也不参与候选。
    fixture.write_sized("huge_texture.dds", 8192);
    // 子目录里的 kn5 不参与（非递归扫描）。
    fixture.write_sized("lods/inner.kn5", 65536);

    assert_eq!(main_model(&fixture), Some(model));
}

#[test]
fn ties_are_broken_deterministically_in_favour_of_an_existing_file() {
    // 两个同样大小的 kn5：规则不保证选哪个，但必须选到一个真实存在的。
    let fixture = car_folder(&[]);
    fixture.write_sized("a.kn5", 128);
    fixture.write_sized("b.kn5", 128);

    let model = main_model(&fixture).expect("a tie must still pick a file");
    assert!(model.is_file());
    assert!(
        matches!(file_name(&model), Some("a.kn5") | Some("b.kn5")),
        "unexpected winner {}",
        model.display()
    );
}

#[test]
fn returns_none_without_any_kn5() {
    let fixture = car_folder(&[]);
    fixture.write_sized("collider.kn5u", 10); // 不同扩展名
    fixture.write_sized("readme.txt", 10);

    assert_eq!(main_model(&fixture), None);
}

#[test]
fn returns_none_for_an_empty_folder_with_only_data() {
    let fixture = car_folder(&[]);
    assert_eq!(main_model(&fixture), None);
}

#[cfg(unix)]
#[test]
fn skips_files_whose_metadata_cannot_be_read() {
    // 坏符号链接：read_dir 列得出它，metadata 读不到 → 不能当成 0 字节候选。
    let fixture = car_folder(&[]);
    let model = fixture.write_sized("real.kn5", 16);
    fixture.write_dangling_symlink("ghost.kn5", "nonexistent-target.kn5");

    assert_eq!(main_model(&fixture), Some(model));
}

#[cfg(unix)]
#[test]
fn returns_none_when_only_unstatable_kn5_files_exist() {
    let fixture = car_folder(&[]);
    fixture.write_dangling_symlink("ghost.kn5", "nonexistent-target.kn5");

    // 没有真实模型可比 → None，而不是返回一个打不开的路径。
    assert_eq!(main_model(&fixture), None);
}
