//! Discovery test against a real AC car. Ignored by default because it needs the
//! local test assets:
//!
//! ```bash
//! cargo test -p vettura-audio -- --ignored
//! ```

use std::path::PathBuf;

use vettura_audio::SoundManifest;

fn ferrari_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../vettura/mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(
                "../vettura/mods/F1 2008 by Cimmerian_Iter/assettocorsa/content/cars/cim_2008_ferrari",
            )
        })
}

#[test]
#[ignore = "needs the local AC car test assets"]
fn ferrari_sound_manifest_is_discovered() {
    let root = ferrari_root();
    if !root.is_dir() {
        eprintln!("skipping: {} is not available", root.display());
        return;
    }

    let manifest = SoundManifest::discover(&root)
        .expect("sfx discovery should not fail")
        .expect("Ferrari should ship sound");

    assert_eq!(manifest.car_id(), "cim_2008_ferrari");
    assert!(manifest.bank_path().is_file());
    assert!(manifest.bank_path().metadata().unwrap().len() > 1_000_000);

    for expected in [
        "engine_ext",
        "engine_int",
        "skid_ext",
        "skid_int",
        "gear_ext",
    ] {
        assert!(
            manifest.has_event(expected),
            "missing event {expected}: {:?}",
            manifest.event_names()
        );
    }

    println!("bank: {}", manifest.bank_path().display());
    println!("events: {:?}", manifest.event_names());
}
