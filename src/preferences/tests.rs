use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Barrier},
};
fn fixture() -> (crate::autostart::Fixture, Store) {
    let fixture = crate::autostart::Fixture::new();
    let store = Store::new(fixture.root.join("talis/settings.toml")).unwrap();
    (fixture, store)
}
fn patch(key: &str, value: impl Into<toml::Value>) -> Patch {
    Patch::from_iter([(key.into(), value.into())])
}

#[test]
fn resolution_preference_is_optional_typed_and_round_trips() {
    let (_fixture, store) = fixture();
    assert_eq!(
        load(Config::default(), None, Some(store.clone()))
            .unwrap()
            .config
            .scene
            .resolution,
        SceneResolution::Automatic
    );
    for (id, expected) in [
        ("1080p", SceneResolution::FullHd),
        ("4k", SceneResolution::FourK),
        ("1440p", SceneResolution::Qhd),
        ("automatic", SceneResolution::Automatic),
    ] {
        store.write(&patch("scene_resolution", id)).unwrap();
        assert_eq!(
            load(Config::default(), None, Some(store.clone()))
                .unwrap()
                .config
                .scene
                .resolution,
            expected
        );
    }
    assert!(store.write(&patch("scene_resolution", "720p")).is_err());
    assert_eq!(
        store.read().unwrap().scene_resolution,
        Some(SceneResolution::Automatic)
    );
    assert!(
        !fs::read_to_string(store.path())
            .unwrap()
            .contains("background")
    );
}

#[test]
fn missing_preferences_are_read_only_and_first_write_is_private() {
    let (fixture, store) = fixture();
    assert!(store.read().unwrap().fps.is_none());
    assert!(!fixture.root.join("talis").exists());
    store.write(&patch("fps", 30_i64)).unwrap();
    assert_eq!(store.read().unwrap().fps, Some(30));
    assert_eq!(
        fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = fs::read_to_string(store.path()).unwrap();
    assert!(!text.contains("background") && !text.contains("rotation_angle"));
    store.write(&patch("surface_opacity", 0.4_f64)).unwrap();
    assert_eq!(store.read().unwrap().fps, Some(30));
    assert!(
        fs::read_to_string(store.path().with_extension("toml.bak"))
            .unwrap()
            .contains("fps = 30")
    );
}
#[test]
fn corrupt_newer_unknown_and_nonfinite_settings_are_never_overwritten() {
    for text in [
        "bad = [",
        "schema_version=99\nfps=30",
        "unknown=true",
        "fps=0",
        "surface_brightness=nan",
        "arm_range_start=0.9\narm_range_end=0.2",
        "preferred_player='bad/player'",
    ] {
        let (_fixture, store) = fixture();
        fs::create_dir(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), text).unwrap();
        assert!(store.read().is_err(), "{text}");
        assert!(store.write(&patch("fps", 60_i64)).is_err(), "{text}");
        assert_eq!(fs::read_to_string(store.path()).unwrap(), text);
        let loaded = load(Config::default(), None, Some(store)).unwrap();
        assert!(loaded.issue.is_some());
        assert_eq!(loaded.config.render.fps, 60);
    }
}
#[test]
fn explicit_partial_config_overrides_only_supplied_fields() {
    let (fixture, store) = fixture();
    store.write(&patch("fps", 15_i64)).unwrap();
    store.write(&patch("surface_opacity", 0.6_f64)).unwrap();
    let explicit = fixture.root.join("profile.toml");
    fs::write(&explicit, "[artwork]\nrotation_rpm=45\n").unwrap();
    let loaded = load(Config::default(), Some(&explicit), Some(store)).unwrap();
    assert_eq!(loaded.config.render.fps, 15);
    assert_eq!(loaded.config.artwork.vinyl_opacity, 0.6);
    assert_eq!(loaded.config.artwork.rotation_rpm, 45.0);
    assert_eq!(
        fs::read_to_string(explicit).unwrap(),
        "[artwork]\nrotation_rpm=45\n"
    );
    let alias = fixture.root.join("alias.toml");
    fs::write(&alias, "[artwork]\ndiameter_percent=40\n").unwrap();
    assert_eq!(Config::load(&alias).unwrap().artwork.size_percent, 40.0);
}
#[test]
fn separate_writers_merge_fields_and_worker_flushes_without_idle_activity() {
    let (_fixture, store) = fixture();
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = [
        ("fps", toml::Value::Integer(30)),
        ("surface_brightness", toml::Value::Float(1.4)),
    ]
    .into_iter()
    .map(|(key, value)| {
        let store = store.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let worker = Worker::new(store).unwrap();
            barrier.wait();
            worker.save(1, patch(key, value)).unwrap();
            worker.flush().unwrap();
        })
    })
    .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(store.read().unwrap().fps, Some(30));
    assert_eq!(store.read().unwrap().surface_brightness, Some(1.4));
    let worker = Worker::new(store.clone()).unwrap();
    assert!(worker.poll().is_none());
    worker
        .save(1, patch("preferred_player", "automatic"))
        .unwrap();
    worker.flush().unwrap();
    assert_eq!(worker.poll().unwrap().0, 1);
    assert_eq!(
        store.read().unwrap().preferred_player.as_deref(),
        Some("automatic")
    );
}
#[test]
fn unsafe_targets_invalid_patches_and_backup_conflicts_preserve_data() {
    let (fixture, store) = fixture();
    fs::create_dir(store.path().parent().unwrap()).unwrap();
    let target = fixture.root.join("untouched");
    fs::write(&target, "keep").unwrap();
    symlink(&target, store.path()).unwrap();
    assert!(store.read().is_err());
    assert!(store.write(&patch("fps", 30_i64)).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    fs::remove_file(store.path()).unwrap();
    store.write(&patch("fps", 30_i64)).unwrap();
    let before = fs::read_to_string(store.path()).unwrap();
    assert!(store.write(&patch("schema_version", 2_i64)).is_err());
    assert!(store.write(&patch("asset_path", "test")).is_err());
    assert!(store.write(&patch("fps", 0_i64)).is_err());
    symlink(&target, store.path().with_extension("toml.bak")).unwrap();
    assert!(store.write(&patch("fps", 60_i64)).is_err());
    assert_eq!(fs::read_to_string(store.path()).unwrap(), before);
    assert_eq!(fs::read_to_string(target).unwrap(), "keep");
}
#[test]
fn unavailable_scene_does_not_erase_other_settings_or_saved_scene() {
    let (_fixture, store) = fixture();
    store.write(&patch("scene", "night")).unwrap();
    store.write(&patch("fps", 30_i64)).unwrap();
    let mut base = Config::default();
    base.scene.night = None;
    let loaded = load(base, None, Some(store.clone())).unwrap();
    assert_eq!(loaded.config.scene.variant, SceneVariant::Day);
    assert!(loaded.issue.is_some());
    assert_eq!(loaded.config.render.fps, 30);
    assert_eq!(store.read().unwrap().scene, Some(SceneVariant::Night));
}
#[test]
fn startup_values_are_live_and_legacy_import_never_overwrites_preferences() {
    let (_fixture, store) = fixture();
    let mut snapshot = Config::default();
    snapshot.render.fps = 15;
    snapshot.artwork.size_percent = 42.0;
    snapshot.scene.mode = crate::config::PreviewMode::Arm;
    snapshot.render.arm_interpolation = true;
    let baseline = startup_base(snapshot.clone());
    assert_eq!(baseline.render.fps, 60);
    assert_eq!(baseline.artwork.size_percent, 36.0);
    assert_eq!(baseline.scene.mode, crate::config::PreviewMode::Arm);
    assert!(baseline.render.arm_interpolation);
    store
        .import_if_missing(&Document::from_config(&snapshot, Some("spotify")))
        .unwrap();
    let imported = load(baseline, None, Some(store.clone())).unwrap();
    assert_eq!(imported.config.render.fps, 15);
    assert_eq!(imported.config.artwork.size_percent, 42.0);
    store.write(&patch("fps", 30_i64)).unwrap();
    store
        .import_if_missing(&Document::from_config(&snapshot, None))
        .unwrap();
    let current = load(startup_base(snapshot), None, Some(store)).unwrap();
    assert_eq!(current.config.render.fps, 30);
    assert_eq!(
        current.document.player().unwrap().as_deref(),
        Some("org.mpris.MediaPlayer2.spotify")
    );
}
